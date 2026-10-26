//! File-store built-ins: single-file put/get/delete, list, copy, and the
//! two recursive directory operations (`dir-copy`, `dir-delete`).
//!
//! These are general-purpose and unrestricted — Internal actions carry
//! the same trust level as the dispatcher itself, same as the OAuth
//! loopback. There is no sandboxing model for them the way there is for
//! untrusted WASM guests, with two exceptions: none of them writes or
//! deletes a locked action's pinned artifact (see `crate::lock`), and each
//! is held to the caller's `read:file`/`write:file` scopes (see
//! `super::scopes`). A directory operation checks every entry before
//! touching any.

use std::sync::Arc;

use base64::Engine as _;
use serde_json::{json, Value};
use solx_surface::managers::FileStore;
use solx_surface::scope::{Access, Need};
use solx_surface::Actor;

use super::require_str;
use crate::{LocalActionManager, ScopeCheck};

/// Refuse when `actor` may not write or delete any of `targets` — see
/// `LocalActionManager::authorize_file_change`.
async fn guard_pinned<'a>(
    local: &LocalActionManager,
    actor: Actor<'_>,
    targets: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    local
        .authorize_file_change(actor, targets)
        .await
        .map_err(super::denial)
}

pub(super) async fn file_put(
    params: &Value,
    files: &Arc<dyn FileStore>,
    local: &LocalActionManager,
    actor: Actor<'_>,
) -> Result<Value, String> {
    let rel_path = require_str(params, "rel_path")?;
    guard_pinned(local, actor, [rel_path]).await?;
    let content = require_str(params, "content")?;
    let encoding = params
        .get("encoding")
        .and_then(Value::as_str)
        .unwrap_or("utf8");
    let bytes = match encoding {
        "utf8" => content.as_bytes().to_vec(),
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(content)
            .map_err(|e| format!("invalid base64 content: {e}"))?,
        other => {
            return Err(format!(
                "unknown encoding '{other}'; expected \"utf8\" or \"base64\""
            ))
        }
    };
    let stored = files
        .put(rel_path, bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "rel_path": stored }))
}

pub(super) async fn file_get(params: &Value, files: &Arc<dyn FileStore>) -> Result<Value, String> {
    let rel_path = require_str(params, "rel_path")?;
    let bytes = files.get(rel_path).await.map_err(|e| e.to_string())?;
    let (content, encoding) = match String::from_utf8(bytes.clone()) {
        Ok(s) => (s, "utf8"),
        Err(_) => (
            base64::engine::general_purpose::STANDARD.encode(&bytes),
            "base64",
        ),
    };
    Ok(json!({ "rel_path": rel_path, "encoding": encoding, "content": content }))
}

pub(super) async fn file_delete(
    params: &Value,
    files: &Arc<dyn FileStore>,
    local: &LocalActionManager,
    actor: Actor<'_>,
) -> Result<Value, String> {
    let rel_path = require_str(params, "rel_path")?;
    guard_pinned(local, actor, [rel_path]).await?;
    files.delete(rel_path).await.map_err(|e| e.to_string())?;
    Ok(json!({ "deleted": true }))
}

/// Lists only the files the caller may read: the store has no SQL to
/// narrow, so each entry is checked.
pub(super) async fn file_list(
    params: &Value,
    files: &Arc<dyn FileStore>,
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let prefix = params.get("prefix").and_then(Value::as_str).unwrap_or("");
    let entries = files.list(prefix).await.map_err(|e| e.to_string())?;
    let entries = scopes.filter_file_reads(entries);
    Ok(json!({ "files": entries }))
}

pub(super) async fn file_copy(
    params: &Value,
    files: &Arc<dyn FileStore>,
    local: &LocalActionManager,
    actor: Actor<'_>,
) -> Result<Value, String> {
    let source = require_str(params, "source")?;
    let dest = require_str(params, "dest")?;
    guard_pinned(local, actor, [dest]).await?;
    let bytes = files.get(source).await.map_err(|e| e.to_string())?;
    files.put(dest, bytes).await.map_err(|e| e.to_string())?;
    Ok(json!({ "dest": dest }))
}

/// Recursively copies every entry under `source` to the matching relative
/// position under `dest`. Refused whole if the caller may not read every
/// entry, rather than copying only some.
pub(super) async fn dir_copy(
    params: &Value,
    files: &Arc<dyn FileStore>,
    local: &LocalActionManager,
    actor: Actor<'_>,
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let source = require_str(params, "source")?;
    let dest = require_str(params, "dest")?;
    let source_prefix = source.trim_end_matches('/');
    let dest_prefix = dest.trim_end_matches('/');
    let entries = files.list(source).await.map_err(|e| e.to_string())?;
    // The first entry the scopes don't grant decides it, so audit logs
    // once per copy rather than once per file.
    let unreadable = entries
        .iter()
        .find(|e| !scopes.grants(&Need::file(Access::Read, e)));
    if let Some(entry) = unreadable {
        scopes
            .require(&Need::file(Access::Read, entry))
            .map_err(|e| e.to_string())?;
    }
    let pairs: Vec<(&String, String)> = entries
        .iter()
        .map(|entry| {
            let rel = entry
                .strip_prefix(source_prefix)
                .unwrap_or(entry)
                .trim_start_matches('/');
            (entry, format!("{dest_prefix}/{rel}"))
        })
        .collect();
    guard_pinned(local, actor, pairs.iter().map(|(_, dest)| dest.as_str())).await?;
    let mut copied = Vec::with_capacity(pairs.len());
    for (entry, dest_entry) in pairs {
        let bytes = files.get(entry).await.map_err(|e| e.to_string())?;
        files
            .put(&dest_entry, bytes)
            .await
            .map_err(|e| e.to_string())?;
        copied.push(dest_entry);
    }
    Ok(json!({ "copied": copied }))
}

// ── dir-delete ───────────────────────────────────────────────────────────────
//
// The file-store trait only exposes single-file `delete`; tree removal
// needs a `list` + per-entry `delete`. This mirrors the structure of
// `dir-copy` exactly, just inverted, so the two stay symmetric.

pub(super) async fn dir_delete(
    params: &Value,
    files: &Arc<dyn FileStore>,
    local: &LocalActionManager,
    actor: Actor<'_>,
) -> Result<Value, String> {
    let rel_path = require_str(params, "rel_path")?;
    let prefix = rel_path.trim_end_matches('/');
    let entries = files.list(rel_path).await.map_err(|e| e.to_string())?;
    // Defensive: only delete entries actually under `prefix`. If a
    // concurrent write lands a sibling outside the subtree between `list`
    // and the per-entry `delete`, we don't follow it.
    let entries: Vec<&String> = entries
        .iter()
        .filter(|entry| *entry != prefix && entry.starts_with(&format!("{prefix}/")))
        .collect();
    guard_pinned(local, actor, entries.iter().map(|e| e.as_str())).await?;
    let mut deleted = Vec::with_capacity(entries.len());
    for entry in entries {
        files.delete(entry).await.map_err(|e| e.to_string())?;
        deleted.push(entry.clone());
    }
    Ok(json!({ "deleted": deleted }))
}
