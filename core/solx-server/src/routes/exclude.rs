//! `excludeMode` and scope handling shared by the entity routes.
//!
//! Scopes: a single-entity route checks the token's read or write scope
//! for that entity ([`require_entity`]); a list, search or path facet is
//! narrowed to what it may read ([`apply`]). With the root token both are
//! no-ops.
//!
//! These routes are what the web UI and other REST clients read, so a
//! request that names no mode gets [`DEFAULT_MODE`] (`user`). A caller that
//! wants something else says so: the agent harness running in the UI sends
//! `excludeMode=tooling` through the builtins, and `solx-client` always
//! sends an explicit mode so a proxied `solx-cli`/`solx-mcp` behaves exactly
//! as it would against local managers.
//!
//! Exec (`POST /actions/{ref}`) is deliberately not filtered: the agent
//! widget reaches the builtins that way, and hiding `/builtin/*` from the
//! user's lists must not break it.

use serde::Deserialize;
use solx_actions::guard::scope::resource_of;
use solx_actions::ScopeCheck;
use solx_config::EntityKind;
use solx_surface::query::{ExcludeMode, ListOptions, SearchQuery};
use solx_surface::scope::{Access, Need, Resource};
use solx_surface::Principal;

use crate::error::ApiError;
use crate::state::AppState;

/// The mode an entity route applies when the request names none.
pub const DEFAULT_MODE: ExcludeMode = ExcludeMode::User;

/// `?excludeMode=` on the single-entity routes (get/put/delete), whose
/// path already carries the reference.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeParam {
    #[serde(default)]
    pub exclude_mode: Option<ExcludeMode>,
}

impl ModeParam {
    pub fn mode(&self) -> ExcludeMode {
        self.exclude_mode.unwrap_or(DEFAULT_MODE)
    }
}

/// Expand the request's mode (or the default) into `opts.exclude_paths`,
/// and narrow the list to what `principal`'s token may read.
pub fn apply(
    state: &AppState,
    principal: &Principal,
    kind: EntityKind,
    opts: &mut ListOptions,
) -> Result<(), ApiError> {
    let config = &state.app.config;
    config.exclusion_policy().apply(kind, opts, DEFAULT_MODE);
    ScopeCheck::for_principal(config, principal)
        .narrow_reads(resource_of(kind), &mut opts.include_paths)?;
    Ok(())
}

/// [`apply`] for document search.
pub fn apply_search(
    state: &AppState,
    principal: &Principal,
    query: &mut SearchQuery,
) -> Result<(), ApiError> {
    let config = &state.app.config;
    config.exclusion_policy().apply_search(query, DEFAULT_MODE);
    ScopeCheck::for_principal(config, principal)
        .narrow_reads(Resource::Document, &mut query.include_paths)?;
    Ok(())
}

/// Refuse a call `principal`'s token may not make: reading or writing one
/// entity or file. Action and file writes are checked by the `authorize_*`
/// entry points instead, and exec by dispatch.
pub fn require(state: &AppState, principal: &Principal, need: &Need<'_>) -> Result<(), ApiError> {
    ScopeCheck::for_principal(&state.app.config, principal).require(need)?;
    Ok(())
}

/// [`require`] for reading or writing the entity `name` in `path`.
pub fn require_entity(
    state: &AppState,
    principal: &Principal,
    kind: EntityKind,
    access: Access,
    path: &str,
    name: &str,
) -> Result<(), ApiError> {
    require(
        state,
        principal,
        &Need::Resource {
            access,
            resource: resource_of(kind),
            path,
            name,
        },
    )
}

/// Refuse direct access to an excluded entity — see
/// `solx_manager::App::guard_excluded`.
pub async fn guard(
    state: &AppState,
    kind: EntityKind,
    mode: &ModeParam,
    path: &str,
    name: &str,
    write: bool,
) -> Result<(), ApiError> {
    state
        .app
        .guard_excluded(kind, mode.mode(), path, name, write)
        .await?;
    Ok(())
}
