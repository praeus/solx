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
    /// The token registry behind `auth`, for `/auth/exchange`. With it,
    /// the root token is accepted from loopback peers only.
    #[cfg(feature = "registry")]
    pub registry: Option<Arc<solx_registry::Registry>>,
    /// Per-IP backoff after a failed connect-code exchange.
    #[cfg(feature = "registry")]
    pub exchange_backoff: Arc<crate::routes::connect::Backoff>,
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
        Ok(Self {
            #[cfg(feature = "registry")]
            registry: app.registry(),
            #[cfg(feature = "registry")]
            exchange_backoff: Default::default(),
            app,
            auth,
        })
    }

    /// Whether the root token must come from a loopback peer: whenever the
    /// token registry is on, so a network-facing server never accepts the
    /// all-powerful token from the network.
    pub fn root_needs_loopback(&self) -> bool {
        #[cfg(feature = "registry")]
        return self.registry.is_some();
        #[cfg(not(feature = "registry"))]
        return false;
    }
}
