//! Entity CRUD and search built-ins: documents, types, actions.
//!
//! The `entity-save-action` / `entity-delete-action` handlers authorize as
//! [`Actor::Builtin`], which keeps an MCP tool call, a WASM guest, or a
//! `.solx` script from creating or modifying an executable action — see
//! [`crate::check_action_change`].

use std::sync::Arc;

use serde_json::{json, Value};
use solx_config::EntityKind;
use solx_surface::entities::{ActionInput, TypeInput};
use solx_surface::managers::{ActionManager, DocManager, TypeManager};
use solx_surface::query::{ActionSearchQuery, ListOptions, SearchQuery};
use solx_surface::Actor;

use solx_surface::entities::DocumentInput;

use super::exclude::{apply_action_search, apply_doc_search, apply_list};
use super::{denial, parse_input, path_or_root, require_str, to_value};
use crate::{LocalActionManager, ScopeCheck};

/// Parse a `ListOptions`, expand its `excludeMode` for `kind`, and narrow it
/// to what the caller may read.
fn list_opts(
    kind: EntityKind,
    params: &Value,
    scopes: &ScopeCheck<'_>,
) -> Result<ListOptions, String> {
    let mut opts: ListOptions = parse_input(params)?;
    apply_list(kind, &mut opts, params, scopes)?;
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
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Document, params, scopes)?;
    let page = docs.list(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

pub(super) async fn doc_paths(
    params: &Value,
    docs: &Arc<dyn DocManager>,
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Document, params, scopes)?;
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
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Type, params, scopes)?;
    let page = types.list(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

pub(super) async fn type_paths(
    params: &Value,
    types: &Arc<dyn TypeManager>,
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Type, params, scopes)?;
    let page = types.paths(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

// ── action CRUD ──────────────────────────────────────────────────────────────
//
// These built-ins are the only route to action creation an MCP client, a
// WASM guest or a `.solx` script has: a guest imports nothing but
// `action-exec`, a script composes actions, and an MCP client calls action
// rows. They authorize as `Actor::Builtin` whatever principal the chain
// carries, which adds a refusal of Command and Webhook rows to the content
// checks REST and the desktop app share — see `crate::check_action_change`.

pub(super) async fn action_save(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
    local: &LocalActionManager,
    actor: Actor<'_>,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let input: ActionInput = parse_input(params)?;
    let path = path_or_root(params);
    local
        .authorize_action_change(actor, path, name, Some(&input))
        .await
        .map_err(denial)?;

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
    local: &LocalActionManager,
    actor: Actor<'_>,
) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let path = path_or_root(params);
    local
        .authorize_action_change(actor, path, name, None)
        .await
        .map_err(denial)?;
    actions
        .delete(path, name)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "deleted": true }))
}

pub(super) async fn action_list(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Action, params, scopes)?;
    let page = actions.list(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

pub(super) async fn action_paths(
    params: &Value,
    actions: &Arc<dyn ActionManager>,
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let opts = list_opts(EntityKind::Action, params, scopes)?;
    let page = actions.paths(opts).await.map_err(|e| e.to_string())?;
    to_value(&page)
}

// ── search ───────────────────────────────────────────────────────────────────

pub(super) async fn search_documents(
    params: &Value,
    docs: &Arc<dyn DocManager>,
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let mut query: SearchQuery = parse_input(params)?;
    apply_doc_search(&mut query, params, scopes)?;
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
    scopes: &ScopeCheck<'_>,
) -> Result<Value, String> {
    let mut query: ActionSearchQuery = parse_input(params)?;
    apply_action_search(&mut query, scopes)?;
    let page = actions.search(query).await.map_err(|e| e.to_string())?;
    to_value(&page)
}
