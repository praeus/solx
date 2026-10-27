pub mod actions;
#[cfg(feature = "registry")]
pub mod connect;
pub mod docs;
pub mod exclude;
pub mod files;
pub mod mcp;
pub mod refs;
pub mod types;
pub mod ui;
pub mod whoami;

use axum::Router;

use crate::state::AppState;

/// All data routes, merged into one router (auth middleware is layered on
/// by the caller — see `main.rs`). Takes `state` (rather than being
/// constructed later via `.with_state`, like the other route modules) only
/// because `mcp::router` needs a concrete `Arc<App>` at construction time —
/// the MCP service isn't itself state-generic the way the `Json`/`State`
/// extractor-based handlers elsewhere are.
pub fn router(state: &AppState) -> Router<AppState> {
    Router::new()
        .merge(types::router())
        .merge(docs::router())
        .merge(actions::router())
        .merge(files::router())
        .merge(whoami::router())
        .merge(mcp::router(state.app.clone()))
}
