use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Extension;
use axum::{Json, Router};
use axum_extra::extract::Query;
use solx_surface::entities::{Document, DocumentInput};
use solx_surface::managers::Solx;
use solx_surface::query::{ListOptions, Page, PathFacet, SearchQuery};
use solx_surface::scope::Access;
use solx_surface::Principal;

use solx_config::EntityKind;

use crate::error::ApiError;
use crate::routes::exclude::{self, ModeParam};
use crate::routes::refs::split_url_ref;
use crate::state::AppState;

/// `/docs` (the collection) plus `/docs/{*ref}` (one document), and the
/// top-level `/search`.
///
/// Search lives at `/search` rather than `/docs/search` because a static
/// segment beside a catch-all wins the match for *every* method it is
/// registered under and 405s the rest — so `/docs/search` would make a
/// document named `search` at the root unreachable.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/docs", get(list_docs))
        .route("/docs/*ref", get(get_doc).put(save_doc).delete(delete_doc))
        .route("/docs-paths", get(doc_paths))
        .route("/search", get(search_docs))
}

async fn save_doc(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
    Json(input): Json<DocumentInput>,
) -> Result<Json<Document>, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Document, &mode, &path, &name, true).await?;
    exclude::require_entity(
        &state,
        &principal,
        EntityKind::Document,
        Access::Write,
        &path,
        &name,
    )?;
    let doc = state.app.docs().save(&path, &name, input).await?;
    Ok(Json(doc))
}

async fn get_doc(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
) -> Result<Json<Document>, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Document, &mode, &path, &name, false).await?;
    exclude::require_entity(
        &state,
        &principal,
        EntityKind::Document,
        Access::Read,
        &path,
        &name,
    )?;
    let doc = state.app.docs().get(&path, &name).await?;
    Ok(Json(doc))
}

async fn delete_doc(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
) -> Result<StatusCode, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Document, &mode, &path, &name, false).await?;
    exclude::require_entity(
        &state,
        &principal,
        EntityKind::Document,
        Access::Write,
        &path,
        &name,
    )?;
    state.app.docs().delete(&path, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_docs(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(mut opts): Query<ListOptions>,
) -> Result<Json<Page<Document>>, ApiError> {
    exclude::apply(&state, &principal, EntityKind::Document, &mut opts)?;
    let page = state.app.docs().list(opts).await?;
    Ok(Json(page))
}

async fn doc_paths(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(mut opts): Query<ListOptions>,
) -> Result<Json<Page<PathFacet>>, ApiError> {
    exclude::apply(&state, &principal, EntityKind::Document, &mut opts)?;
    let page = state.app.docs().paths(opts).await?;
    Ok(Json(page))
}

async fn search_docs(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(mut query): Query<SearchQuery>,
) -> Result<Json<Page<Document>>, ApiError> {
    exclude::apply_search(&state, &principal, &mut query)?;
    let results = state.app.docs().search(query).await?;
    Ok(Json(results))
}
