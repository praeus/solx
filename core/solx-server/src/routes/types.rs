use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Extension;
use axum::{Json, Router};
use axum_extra::extract::Query;
use solx_surface::entities::{TypeEntity, TypeInput};
use solx_surface::managers::Solx;
use solx_surface::path::split_ref;
use solx_surface::query::{ListOptions, Page, PathFacet};
use solx_surface::scope::Access;
use solx_surface::wire::ValidateRequest;
use solx_surface::Principal;

use solx_config::EntityKind;

use crate::error::ApiError;
use crate::routes::exclude::{self, ModeParam};
use crate::routes::refs::split_url_ref;
use crate::state::AppState;

/// `/types` (the collection) plus `/types/{*ref}` (one type), and the
/// top-level `/validate`.
///
/// There is no `resolve` route: `TypeManager::resolve` is defined as
/// `split_ref` + `get` (solx-types/src/lib.rs), which is exactly what
/// `GET /types/{*ref}` does. `RemoteTypeManager` implements the trait
/// method client-side against this same route.
///
/// Validation is a top-level `/validate` for the same reason search is —
/// see [`super::docs::router`].
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/types", get(list_types))
        .route(
            "/types/*ref",
            get(get_type).put(save_type).delete(delete_type),
        )
        .route("/types-paths", get(type_paths))
        .route("/validate", post(validate_type))
}

async fn save_type(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
    Json(input): Json<TypeInput>,
) -> Result<Json<TypeEntity>, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Type, &mode, &path, &name, true).await?;
    exclude::require_entity(
        &state,
        &principal,
        EntityKind::Type,
        Access::Write,
        &path,
        &name,
    )?;
    let entity = state.app.types().save(&path, &name, input).await?;
    Ok(Json(entity))
}

async fn get_type(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
) -> Result<Json<TypeEntity>, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Type, &mode, &path, &name, false).await?;
    exclude::require_entity(
        &state,
        &principal,
        EntityKind::Type,
        Access::Read,
        &path,
        &name,
    )?;
    let entity = state.app.types().get(&path, &name).await?;
    Ok(Json(entity))
}

async fn delete_type(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(reference): Path<String>,
    Query(mode): Query<ModeParam>,
) -> Result<StatusCode, ApiError> {
    let (path, name) = split_url_ref(&reference)?;
    exclude::guard(&state, EntityKind::Type, &mode, &path, &name, false).await?;
    exclude::require_entity(
        &state,
        &principal,
        EntityKind::Type,
        Access::Write,
        &path,
        &name,
    )?;
    state.app.types().delete(&path, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_types(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(mut opts): Query<ListOptions>,
) -> Result<Json<Page<TypeEntity>>, ApiError> {
    exclude::apply(&state, &principal, EntityKind::Type, &mut opts)?;
    let page = state.app.types().list(opts).await?;
    Ok(Json(page))
}

async fn type_paths(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(mut opts): Query<ListOptions>,
) -> Result<Json<Page<PathFacet>>, ApiError> {
    exclude::apply(&state, &principal, EntityKind::Type, &mut opts)?;
    let page = state.app.types().paths(opts).await?;
    Ok(Json(page))
}

async fn validate_type(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<ValidateRequest>,
) -> Result<StatusCode, ApiError> {
    // Validating reads the type. A reference that doesn't split is the
    // validator's error to report.
    if let Ok((path, name)) = split_ref(&req.type_ref) {
        exclude::require_entity(
            &state,
            &principal,
            EntityKind::Type,
            Access::Read,
            &path,
            &name,
        )?;
    }
    state
        .app
        .types()
        .validate(&req.value, &req.type_ref)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
