//! `GET /auth/whoami`: what the presented token is and may do, so a client
//! or the UI can show it and grey out what it can't do.
//!
//! Behind the bearer gate like every data route, and answered for any
//! token. `scopes` is `null` for the root token, which no scope limits, and
//! the token's scope strings otherwise. `scope_enforcement` says whether
//! those scopes are enforced yet.

use axum::extract::State;
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde_json::{json, Value};
use solx_surface::scope::Grant;
use solx_surface::Principal;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/auth/whoami", get(whoami))
}

async fn whoami(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Json<Value> {
    let scopes = match principal.grant() {
        Grant::Unlimited => Value::Null,
        Grant::Scoped(set) => json!(set.as_ref()),
    };
    let enforcement = state.app.config.scope_enforcement().name();
    let mut body = match &principal {
        Principal::Root => json!({ "kind": "root" }),
        Principal::Client { id, label, .. } => {
            json!({ "kind": "client", "id": id, "label": label })
        }
        Principal::Action {
            action_ref,
            invocation_id,
            ..
        } => json!({
            "kind": "action",
            "action": action_ref,
            "invocation_id": invocation_id,
        }),
    };
    body["scopes"] = scopes;
    body["scope_enforcement"] = json!(enforcement);
    Json(body)
}
