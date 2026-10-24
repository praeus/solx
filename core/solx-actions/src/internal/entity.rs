//! Entity CRUD and search built-ins: documents, types, actions.
//!
//! The `entity-save-action` / `entity-delete-action` handlers apply the
//! shell-/webhook-guard that prevents an MCP tool call, a WASM guest, or a
//! `.solx` script from creating or modifying an executable action — see
//! [`super::guard_executable_action`] for the shared helper.

use std::sync::Arc;

use serde_json::{json, Value};
use solx_config::{ConfigService, EntityKind};
use solx_surface::entities::{ActionInput, TypeInput};
use solx_surface::error::SolxError;
use solx_surface::managers::{ActionManager, DocManager, TypeManager};
use solx_surface::ExecContext;
use solx_surface::query::{ActionSearchQuery, ListOptions, SearchQuery};

use solx_surface::entities::DocumentInput;

use super::exclude::{apply_action_search, apply_doc_search, apply_list};
use super::{guard_executable_action, parse_input, path_or_root, require_str, to_value};

/// Parse a `ListOptions` and expand its `excludeMode` for `kind`.
fn list_opts(
    kind: EntityKind,
    params: &Value,
    config: &ConfigService,
) -> Result<ListOptions, String> {
    let mut opts: ListOptions = parse_input(params)?;
    apply_list(kind, &mut opts, params, config)?;
    Ok(opts)
}

// ── document CRUD ────────────────────────────────────────────────────────────

pub(super) async fn doc_save(params: &Value, docs: &Arc<dyn DocManager>) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let input: DocumentInput = parse_input(params)?;
    let doc = docs
        .save(path_or_root(params), name, input)
        .await
        .map_err(|e| e.to_string())?;
    to_value(&doc)
}

pub(super) async fn doc_get(params: &Value, docs: &Arc<dyn DocManager>) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let doc = docs
        .get(path_or_root(params), name)
        .await
        .map_err(|e| e.to_string())?;
    to_value(&doc)
}

pub(super) async fn doc_delete(
    params: &Value,
    docs: &Arc<dyn DocManager>,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    docs.delete(path_or_root(params), name)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "deleted": true }))
}

pub(super) async fn doc_list(
    params: &Value,
    docs: &Arc<dyn DocManager>,
    config: &ConfigService,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Document, params, config)?;
    let page = docs.list(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

pub(super) async fn doc_paths(
    params: &Value,
    docs: &Arc<dyn DocManager>,
    config: &ConfigService,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Document, params, config)?;
    let page = docs.paths(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

// ── type CRUD ────────────────────────────────────────────────────────────────

pub(super) async fn type_save(
    params: &Value,
    types: &Arc<dyn TypeManager>,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let input: TypeInput = parse_input(params)?;
    let ty = types
        .save(path_or_root(params), name, input)
        .await
        .map_err(|e| e.to_string())?;
    to_value(&ty)
}

pub(super) async fn type_get(
    params: &Value,
    types: &Arc<dyn TypeManager>,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let ty = types
        .get(path_or_root(params), name)
        .await
        .map_err(|e| e.to_string())?;
    to_value(&ty)
}

pub(super) async fn type_delete(
    params: &Value,
    types: &Arc<dyn TypeManager>,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    types
        .delete(path_or_root(params), name)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "deleted": true }))
}

pub(super) async fn type_list(
    params: &Value,
    types: &Arc<dyn TypeManager>,
    config: &ConfigService,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Type, params, config)?;
    let page = types.list(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

pub(super) async fn type_paths(
    params: &Value,
    types: &Arc<dyn TypeManager>,
    config: &ConfigService,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Type, params, config)?;
    let page = types.paths(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

// ── action CRUD ──────────────────────────────────────────────────────────────
//
// `action_save` / `action_delete` apply the executable-action guard that
// closes MCP/guest/script routes to creating, modifying, or removing
// Command and Webhook actions — see `super::guard_executable_action`.
//
// The guard lives *here*, on the built-in, rather than in the managers,
// because this built-in is the only route to action creation those callers
// have: a WASM guest imports nothing but `action-exec`, a `.solx` script
// composes actions, and an MCP client calls action rows. Everything they can
// do arrives through this function.
//
// The CLI and the HTTP surface deliberately do **not** apply that one
// guard. Both call `ActionManager::save` directly, which is what lets
// `solx save action` register a Command, and equally what lets solx-web's
// action editor and solx-quickjs write rows over `PUT /actions/{ref}` (known
// gap 7 in docs/security-design.md).
//
// Every content guard besides it is shared with HTTP (for anyone but the
// root token) and the desktop app's main window, and this built-in applies
// it always, whatever principal the chain carries: no write under another
// package's `/packages/<name>/`, no keyed action, no change to a locked
// one. See `crate::guard_content_action_change`.

pub(super) async fn action_save(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
    exec: &ExecContext,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let input: ActionInput = parse_input(params)?;
    let path = path_or_root(params);

    // Apply the guards using the *existing* row (so a merge upsert that
    // omits `action_type` can't silently rewrite what an established
    // Command action shells out to) and the *incoming* one.
    let existing = match actions.get(path, name).await {
        Ok(a) => Some(a),
        Err(SolxError::NotFound(_)) => None,
        Err(e) => return Err(e.to_string()),
    };
    guard_executable_action(
        existing.as_ref().and_then(|a| a.action_type),
        input.action_type,
        path,
        name,
        "create or modify",
    )?;
    crate::guard_content_action_change(
        existing.as_ref(),
        Some(&input),
        path,
        name,
        crate::content::acting_action(exec),
    )?;

    let a = actions
        .save(path, name, input)
        .await
        .map_err(|e| e.to_string())?;
    to_value(&a)
}

/// `excludeMode: "tooling"` (or the older `excludeHidden: true`) makes a
/// hidden action indistinguishable from a missing one. That check runs
/// before dispatch, in `super::exclude::gate`, so it covers save and delete
/// too - see that module.
pub(super) async fn action_get(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let path = path_or_root(params);
    let a = actions.get(path, name).await.map_err(|e| e.to_string())?;
    to_value(&a)
}

pub(super) async fn action_delete(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
    exec: &ExecContext,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let path = path_or_root(params);
    // Same reasoning as `action_save`: if these callers can't create or
    // modify an executable action, they shouldn't be able to remove one
    // either.
    let existing = match actions.get(path, name).await {
        Ok(a) => Some(a),
        Err(SolxError::NotFound(_)) => None,
        Err(e) => return Err(e.to_string()),
    };
    guard_executable_action(
        existing.as_ref().and_then(|a| a.action_type),
        None,
        path,
        name,
        "delete",
    )?;
    crate::guard_content_action_change(
        existing.as_ref(),
        None,
        path,
        name,
        crate::content::acting_action(exec),
    )?;
    actions
        .delete(path, name)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "deleted": true }))
}

pub(super) async fn action_list(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
    config: &ConfigService,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Action, params, config)?;
    let page = actions.list(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

pub(super) async fn action_paths(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
    config: &ConfigService,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Action, params, config)?;
    let page = actions.paths(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

// ── search ───────────────────────────────────────────────────────────────────

pub(super) async fn search_documents(
    params: &Value,
    docs: &Arc<dyn DocManager>,
    config: &ConfigService,
) -> Result<Value, String> {
    let mut query: SearchQuery = parse_input(params)?;
    apply_doc_search(&mut query, params, config)?;
    let results = docs.search(query).await.map_err(|e| e.to_string())?;
    to_value(&results)
}

/// Full-text (FTS5) + structured filter search over the action catalogue —
/// `q` matches `path`/`name`/`caption`/`description`/`category`/`phrases`,
/// ranked by relevance, and composes with the same `path_prefix`/
/// `filter_field`/date filters as `entity-list-actions`.
pub(super) async fn search_actions(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
    config: &ConfigService,
) -> Result<Value, String> {
    let mut query: ActionSearchQuery = parse_input(params)?;
    apply_action_search(&mut query, config);
    let page = actions.search(query).await.map_err(|e| e.to_string())?;
    to_value(&page)
}
