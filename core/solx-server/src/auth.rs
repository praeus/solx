use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use solx_surface::error::SolxError;

use crate::error::ApiError;
use crate::state::AppState;

/// Bearer-token middleware: asks `state.auth` who `Authorization: Bearer
/// <token>` belongs to, and on success stores the resulting
/// [`solx_surface::auth::Principal`] in the request's extensions for
/// handlers that need it. With the default authority there is one shared
/// `server_token`, so every accepted caller is `Principal::Root`. The server
/// binds `127.0.0.1` by default. When `server_host` puts it on the network,
/// this check is the only gate.
///
/// A rejection is a 401 whose body is the usual `SolxError` JSON
/// (`{"kind":"unauthorized",...}`), so clients can tell "no token" from
/// "wrong token" instead of seeing an empty body.
pub async fn require_bearer(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let Some(token) = provided else {
        return Err(SolxError::Unauthorized(
            "missing `Authorization: Bearer <token>` header".into(),
        )
        .into());
    };
    match state.auth.authenticate(token).await {
        Some(principal) => {
            request.extensions_mut().insert(principal);
            Ok(next.run(request).await)
        }
        None => Err(SolxError::Unauthorized(
            "the bearer token is not valid for this solx-server".into(),
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use axum::routing::get;
    use axum::{middleware, Extension, Router};
    use solx_surface::auth::Principal;

    use super::require_bearer;
    use crate::state::AppState;

    /// An accepted bearer reaches the handler with its `Principal` attached.
    #[tokio::test]
    async fn accepted_bearer_carries_its_principal() {
        let dir = tempfile::tempdir().unwrap();
        // Never touch the real OS credential manager from a test.
        solx_config::credentials::use_memory_store_for_tests();
        let app = solx_manager::App::build_local_in(dir.path()).await.unwrap();
        let token = solx_config::credentials::root_token(&app.config).unwrap();
        let state = AppState::new(app).unwrap();

        let router = Router::new()
            .route(
                "/who",
                get(|Extension(p): Extension<Principal>| async move { format!("{p:?}") }),
            )
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_bearer,
            ))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let client = reqwest::Client::new();
        let ok = client
            .get(format!("http://{addr}/who"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status(), 200);
        assert_eq!(ok.text().await.unwrap(), "Root");

        let bad = client
            .get(format!("http://{addr}/who"))
            .bearer_auth("not-the-token")
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 401);
    }
}
