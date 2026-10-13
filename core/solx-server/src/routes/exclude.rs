//! `excludeMode` handling shared by the entity routes.
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
use solx_config::EntityKind;
use solx_surface::query::{ExcludeMode, ListOptions, SearchQuery};

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

/// Expand the request's mode (or the default) into `opts.exclude_paths`.
pub fn apply(state: &AppState, kind: EntityKind, opts: &mut ListOptions) {
    state
        .app
        .config
        .exclusion_policy()
        .apply(kind, opts, DEFAULT_MODE);
}

/// [`apply`] for document search.
pub fn apply_search(state: &AppState, query: &mut SearchQuery) {
    state
        .app
        .config
        .exclusion_policy()
        .apply_search(query, DEFAULT_MODE);
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
