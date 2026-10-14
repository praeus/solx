use std::sync::Arc;

use solx_manager::App;
use solx_surface::auth::TokenAuthority;

/// Shared state threaded through the bearer-auth middleware and every
/// route handler.
#[derive(Clone)]
pub struct AppState {
    pub app: Arc<App>,
    /// Decides which bearer tokens `require_bearer` accepts.
    pub auth: Arc<dyn TokenAuthority>,
}

impl AppState {
    /// State for serving `app`, using its own token authority. Fails for a
    /// remote `App`, which has no local managers to serve.
    pub fn new(app: Arc<App>) -> anyhow::Result<Self> {
        let auth = app.authority().ok_or_else(|| {
            anyhow::anyhow!(
                "cannot serve a remote App: it has no local managers or token authority"
            )
        })?;
        Ok(Self { app, auth })
    }
}
