use std::hash::{Hash, Hasher};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::Deserialize;
use solx_actions::ScopeCheck;
use solx_surface::managers::Solx;
use solx_surface::scope::{Access, Need};
use solx_surface::{Actor, Principal};
use solx_surface::wire::{FileListResponse, FilePutResponse};

use crate::error::ApiError;
use crate::state::AppState;

/// File content is transferred as raw bytes, so this is a plain byte
/// ceiling rather than the base64-inflated one it used to be.
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct FileListQuery {
    /// Absent means "everything" — the store treats `""` as the root.
    #[serde(default)]
    prefix: Option<String>,
}

/// `/files` (list) plus `/files/{*rel_path}` (one file's bytes).
///
/// Unlike the entity routes, the capture here is used verbatim: a file's
/// `rel_path` is already a relative path with no path/name split, and
/// `solx-files` does its own normalization and traversal rejection.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/files", get(list_files))
        .route(
            "/files/*rel_path",
            get(get_file).put(put_file).delete(delete_file),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

/// Only the root token may replace a locked action's pinned artifact — see
/// `solx_manager::App::authorize_file_change`. A Command action calling back
/// with its own token is held to that too.
async fn put_file(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(rel_path): Path<String>,
    bytes: Bytes,
) -> Result<Json<FilePutResponse>, ApiError> {
    state
        .app
        .authorize_file_change(Actor::for_principal(&principal), &[rel_path.as_str()])
        .await?;
    let rel_path = state.app.files().put(&rel_path, bytes.to_vec()).await?;
    Ok(Json(FilePutResponse { rel_path }))
}

/// The `FileStore` trait doesn't persist a content type (that lives on a
/// `FileRef` in document metadata), so the response type is guessed from
/// the extension and falls back to `application/octet-stream`.
///
/// ## Why this sends validators
///
/// The store is mutable and addressed by a stable path, so the bytes behind
/// `/files/widgets/foo.js` change every time that widget is rebuilt. This
/// response used to carry `Content-Type` and nothing else -- no
/// `Cache-Control`, no `ETag`, no `Last-Modified` -- which leaves a browser
/// free to reuse a stored copy on its own heuristics, with nothing in the
/// response that could reveal it had gone stale. The failure mode is
/// miserable to debug: you rebuild a widget, upload it, reload, and keep
/// running yesterday's code while every byte on the server is correct.
///
/// `no-cache` does not mean "don't store" -- it means "revalidate before
/// reusing", which is exactly right for a mutable store. The `ETag` is what
/// keeps that cheap: an unchanged file revalidates to a bodiless `304`
/// rather than resending itself.
///
/// The hash is `DefaultHasher`, not a digest crate: an ETag only has to
/// change when the content does, and nothing here is a security boundary.
async fn get_file(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(rel_path): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // Judged as the path the store will read, so an alias (`a//b`, `a`)
    // is checked as the file it names.
    let stored = solx_files::normalize_rel_path(&rel_path)?;
    ScopeCheck::for_principal(&state.app.config, &principal)
        .require(&Need::file(Access::Read, &stored))?;
    let bytes = state.app.files().get(&rel_path).await?;
    Ok(serve_bytes(&rel_path, bytes, &headers))
}

/// `bytes` (the file at `rel_path`) as a response with the validators
/// described on `get_file`, or a bodiless `304` when `headers` already
/// hold its tag. Shared with `routes::ui`, which serves the same store.
pub fn serve_bytes(rel_path: &str, bytes: Vec<u8>, headers: &HeaderMap) -> Response {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let etag = format!("\"{:016x}\"", hasher.finish());

    // A conditional request lists one or more candidate tags, or `*`.
    let fresh = headers
        .get(IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|c| c.trim() == etag || c.trim() == "*"));
    if fresh {
        return (
            StatusCode::NOT_MODIFIED,
            [(ETAG, etag), (CACHE_CONTROL, "no-cache".to_string())],
        )
            .into_response();
    }

    let content_type = mime_guess::from_path(rel_path)
        .first_or_octet_stream()
        .to_string();
    (
        [
            (CONTENT_TYPE, content_type),
            (ETAG, etag),
            (CACHE_CONTROL, "no-cache".to_string()),
        ],
        bytes,
    )
        .into_response()
}

/// Only the root token may remove a locked action's pinned artifact.
async fn delete_file(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(rel_path): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .app
        .authorize_file_change(Actor::for_principal(&principal), &[rel_path.as_str()])
        .await?;
    state.app.files().delete(&rel_path).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lists only the files the token may read: the store has no SQL to
/// narrow, so each entry is checked.
async fn list_files(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<FileListQuery>,
) -> Result<Json<FileListResponse>, ApiError> {
    let scopes = ScopeCheck::for_principal(&state.app.config, &principal);
    scopes.require_some_reads(solx_surface::scope::Resource::File)?;
    let paths = state
        .app
        .files()
        .list(query.prefix.as_deref().unwrap_or(""))
        .await?;
    Ok(Json(FileListResponse {
        paths: scopes.filter_file_reads(paths),
    }))
}
