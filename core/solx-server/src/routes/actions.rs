use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Extension, Json, Router};
use axum_extra::extract::Query;
use serde::Deserialize;
use serde_json::{json, Value};
use solx_surface::auth::NO_PRINCIPAL_HEADER;
use solx_surface::entities::{Action, ActionExecResult, ActionInput};
use solx_surface::managers::Solx;
use solx_surface::query::{ActionSearchQuery, ListOptions, Page, PathFacet};
use solx_surface::Principal;

use solx_config::EntityKind;

use crate::error::ApiError;
use crate::routes::exclude::{self, ModeParam};
use crate::routes::refs::split_url_ref;
use crate::state::AppState;

/// `/actions` (the collection) plus `/actions/{*ref}` (one action), and the
/// top-level `/actions-search`.
///
/// Execution is `POST` on the action's own URL, with the params as the
/// body — RFC 9110's "resource-specific processing of the request
/// payload". Unlike a static `/actions/exec/...` segment, this shares the
/// exact path pattern of the CRUD routes and so can't shadow an action
/// stored at any particular reference.
///
/// Search lives at the top-level `/actions-search` rather than
/// `/actions/search`, for the same reason document search lives at `/search`
/// rather than `/docs/search` — see `super::docs::router`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/actions", get(list_actions))
        .route(
            "/actions/*ref",
            get(get_action)
                .put(save_action)
                .delete(delete_action)
                .post(exec_action),
        )
        .route("/actions-paths", get(action_paths))
        .route("/actions-search", get(search_actions))
}

async fn save_action(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
    Json(input): Json<ActionInput>,
) -> Result<Json<Action>, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Action, &mode, &path, &name, true).await?;
    let action = state.app.actions().save(&path, &name, input).await?;
    Ok(Json(action))
}

async fn get_action(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
) -> Result<Json<Action>, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Action, &mode, &path, &name, false).await?;
    let action = state.app.actions().get(&path, &name).await?;
    Ok(Json(action))
}

async fn delete_action(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
) -> Result<StatusCode, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Action, &mode, &path, &name, false).await?;
    state.app.actions().delete(&path, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_actions(
    State(state): State<AppState>,
    Query(mut opts): Query<ListOptions>,
) -> Result<Json<Page<Action>>, ApiError> {
    exclude::apply(&state, EntityKind::Action, &mut opts);
    let page = state.app.actions().list(opts).await?;
    Ok(Json(page))
}

async fn action_paths(
    State(state): State<AppState>,
    Query(mut opts): Query<ListOptions>,
) -> Result<Json<Page<PathFacet>>, ApiError> {
    exclude::apply(&state, EntityKind::Action, &mut opts);
    let page = state.app.actions().paths(opts).await?;
    Ok(Json(page))
}

/// Split across two `Query` extractors rather than one
/// `Query<ActionSearchQuery>`: axum's `Query` (via `serde_urlencoded`) can't
/// deserialize a `#[serde(flatten)]`ed field — numeric fields like `limit`
/// come back as strings and fail with "invalid type: string, expected
/// usize". Each extractor re-parses the same query string into its own
/// flat (non-nested) struct, which `serde_urlencoded` handles fine.
async fn search_actions(
    State(state): State<AppState>,
    Query(list): Query<ListOptions>,
    Query(SearchTerm { q, exclude_hidden }): Query<SearchTerm>,
) -> Result<Json<Page<Action>>, ApiError> {
    let mut query = ActionSearchQuery {
        list,
        q,
        exclude_hidden,
    };
    query.list.exclude_mode = query.effective_mode();
    query.exclude_hidden = false;
    exclude::apply(&state, EntityKind::Action, &mut query.list);
    let page = state.app.actions().search(query).await?;
    Ok(Json(page))
}

#[derive(Deserialize)]
struct SearchTerm {
    /// Deprecated: `?exclude_hidden=true` is `?excludeMode=tooling`, which
    /// wins when both are sent.
    #[serde(default, alias = "excludeHidden")]
    exclude_hidden: bool,
    #[serde(default)]
    q: Option<String>,
}

/// Execute the action. The body is optional so a parameterless action can
/// be invoked with an empty `POST` (no `Content-Type` needed) rather than
/// forcing callers to send a literal `{}`.
///
/// Runs as the [`Principal`] `require_bearer` authenticated — the route
/// layer always inserts one before this handler runs — unless the caller
/// sends `X-Solx-Principal: none` ([`NO_PRINCIPAL_HEADER`]), which runs it
/// with no principal, like a local `exec`. `solx-client`'s `exec` does, so
/// the CLI and its scripts can't reach root-only built-ins just because
/// they present the loopback root token.
async fn exec_action(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(reference): Path<String>,
    body: Option<Json<Value>>,
) -> Result<Json<ActionExecResult>, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    let params = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
    let drop_principal = headers
        .get(NO_PRINCIPAL_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("none"));
    let actions = state.app.actions();
    let result = if drop_principal {
        actions.exec(&path, &name, params).await?
    } else {
        actions.exec_for(&principal, &path, &name, params).await?
    };
    Ok(Json(result))
}
