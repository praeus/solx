//! `GET /ui/*`: the web UI, served without a token from the file store's
//! `web/` directory ([`solx_files::WEB_UI_DIR`]).
//!
//! `/files/*` can't serve it: it sits behind the bearer gate, and a browser
//! can't add `Authorization` to a navigation or a `<script src>`. Widgets
//! avoid that because the already-running UI fetches their bytes with its
//! token; the UI itself has nothing to start from. Its code holds no
//! secrets, so it's public, read-only, and limited to that one directory.
//! The token the UI uses stays in the browser's `localStorage`, which the
//! `/connect` page fills on this same origin.
//!
//! Writes to `web/` are refused to everything but an operator (see
//! `solx_actions::lock::web_ui_target`): code served here runs next to
//! every browser user's token. Nothing is exposed until someone publishes
//! the UI; until then every path is a 404.

use axum::extract::{Path, State};
use axum::http::header::{HeaderValue, X_CONTENT_TYPE_OPTIONS};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use solx_surface::managers::Solx;

use crate::error::ApiError;
use crate::routes::files::serve_bytes;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // Relative asset URLs (`./assets/…`) resolve against the directory,
        // so the bare `/ui` has to become `/ui/`.
        .route("/ui", get(|| async { Redirect::permanent("/ui/") }))
        .route("/ui/", get(index))
        .route("/ui/*rel_path", get(get_ui_file))
}

async fn index(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    serve(&state, "", &headers).await
}

async fn get_ui_file(
    State(state): State<AppState>,
    Path(rel_path): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serve(&state, &rel_path, &headers).await
}

/// `web/<rel_path>`, or the directory's `index.html` for a path ending in
/// `/`. The store normalizes `rel_path` and rejects `..`, so nothing
/// outside `web/` is reachable.
async fn serve(state: &AppState, rel_path: &str, headers: &HeaderMap) -> Result<Response, ApiError> {
    let mut rel_path = format!("{}/{rel_path}", solx_files::WEB_UI_DIR);
    if rel_path.ends_with('/') {
        rel_path.push_str("index.html");
    }
    let bytes = state.app.files().get(&rel_path).await?;
    let mut response = serve_bytes(&rel_path, bytes, headers).into_response();
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}
