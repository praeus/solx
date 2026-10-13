//! Exclusion handling for the entity builtins.
//!
//! Builtins default to [`ExcludeMode::None`]: `install.solx`/`uninstall.solx`
//! scripts, WASM guests and packages drive these same handlers, and a
//! `delete doc … --if-exists` that silently skipped a user-hidden document
//! would leak it past an uninstall. A model-facing caller (the agent
//! harness) opts in with `excludeMode: "tooling"`; the deprecated
//! `excludeHidden: true` still means the same.
//!
//! Two halves, mirroring the HTTP and Tauri surfaces:
//! * [`gate`] runs before dispatch and refuses direct access (get, save,
//!   delete, the document field ops) to an excluded entity.
//! * [`apply_list`] / [`apply_action_search`] / [`apply_doc_search`] expand
//!   the configured patterns into the query before a list or search.

use serde_json::Value;
use solx_config::{ConfigService, EntityKind};
use solx_surface::query::{ActionSearchQuery, ExcludeMode, ListOptions, SearchQuery};

use super::{doc_path_or_root, path_or_root, InternalCtx};

/// The mode a builtin call asked for. `run_internal` has already aliased
/// `excludeMode`/`exclude_mode` (and `excludeHidden`/`exclude_hidden`), so
/// only the snake_case spelling is read.
pub(super) fn requested_mode(params: &Value) -> Result<Option<ExcludeMode>, String> {
    if let Some(v) = params.get("exclude_mode").filter(|v| !v.is_null()) {
        return serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|_| format!("invalid excludeMode {v}: expected none, user or tooling"));
    }
    if params.get("exclude_hidden").and_then(Value::as_bool) == Some(true) {
        return Ok(Some(ExcludeMode::Tooling));
    }
    Ok(None)
}

/// Which entity a single-entity builtin touches, and whether it writes.
fn target<'a>(fn_name: &str, params: &'a Value) -> Option<(EntityKind, &'a str, bool)> {
    use EntityKind::*;
    let (kind, write, path) = match fn_name {
        "entity-get-document" | "get-field" => (Document, false, path_or_root(params)),
        "entity-save-document" | "set-field" => (Document, true, path_or_root(params)),
        "entity-delete-document" => (Document, false, path_or_root(params)),
        "get-field-at-path" => (Document, false, doc_path_or_root(params)),
        "set-field-at-path" => (Document, true, doc_path_or_root(params)),
        "entity-get-type" | "entity-delete-type" => (Type, false, path_or_root(params)),
        "entity-save-type" => (Type, true, path_or_root(params)),
        "entity-get-action" | "entity-delete-action" => (Action, false, path_or_root(params)),
        "entity-save-action" => (Action, true, path_or_root(params)),
        _ => return None,
    };
    Some((kind, path, write))
}

/// Refuse direct access to an entity excluded under the requested mode.
/// A no-op for every builtin that doesn't address one entity, and for any
/// call that didn't ask for a mode.
pub(super) async fn gate(fn_name: &str, params: &Value, ctx: &InternalCtx) -> Result<(), String> {
    let Some((kind, path, write)) = target(fn_name, params) else {
        return Ok(());
    };
    let mode = match requested_mode(params)? {
        None | Some(ExcludeMode::None) => return Ok(()),
        Some(m) => m,
    };
    // A missing name is the handler's error to report, not ours.
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return Ok(());
    };
    let policy = ctx.config.exclusion_policy();
    policy
        .guard(kind, mode, path, name, write)
        .map_err(|e| e.to_string())?;
    // Under `Tooling` an action can also hide itself with `solx:hidden`,
    // which only the row can tell us.
    if kind == EntityKind::Action && mode == ExcludeMode::Tooling {
        if let Ok(action) = ctx.actions.get(path, name).await {
            policy
                .guard_action(mode, &action, write)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Expand the requested mode's patterns into a list query. Builtins
/// default to [`ExcludeMode::None`] (see the module docs).
pub(super) fn apply_list(
    kind: EntityKind,
    opts: &mut ListOptions,
    params: &Value,
    config: &ConfigService,
) -> Result<(), String> {
    if opts.exclude_mode.is_none() {
        opts.exclude_mode = requested_mode(params)?;
    }
    config
        .exclusion_policy()
        .apply(kind, opts, ExcludeMode::None);
    Ok(())
}

/// [`apply_list`] for `search-actions`, folding in the deprecated
/// `excludeHidden` flag the query struct carries itself.
pub(super) fn apply_action_search(query: &mut ActionSearchQuery, config: &ConfigService) {
    query.list.exclude_mode = query.effective_mode();
    query.exclude_hidden = false;
    config
        .exclusion_policy()
        .apply(EntityKind::Action, &mut query.list, ExcludeMode::None);
}

/// [`apply_list`] for `search-documents`.
pub(super) fn apply_doc_search(
    query: &mut SearchQuery,
    params: &Value,
    config: &ConfigService,
) -> Result<(), String> {
    if query.exclude_mode.is_none() {
        query.exclude_mode = requested_mode(params)?;
    }
    config
        .exclusion_policy()
        .apply_search(query, ExcludeMode::None);
    Ok(())
}
