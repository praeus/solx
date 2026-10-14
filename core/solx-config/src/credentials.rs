//! Where solx's bearer tokens come from, and the default
//! [`TokenAuthority`]: today's single shared `server_token`.
//!
//! Every host and client resolves tokens through here rather than reading
//! `server_token` itself, so changing where the token is stored (e.g. the OS
//! credential manager instead of `solx-config.json`) only touches this
//! module.

use std::sync::RwLock;

use async_trait::async_trait;
use solx_surface::auth::{constant_time_eq, ActionCredential, Principal, TokenAuthority};
use solx_surface::error::Result;

use crate::ConfigService;

/// The server-side root token, generated and persisted on first use.
pub fn root_token(config: &ConfigService) -> Result<String> {
    config.ensure_server_token()
}

/// The token a client presents to a remote `solx-server`:
/// `SOLX_SERVER_TOKEN` from the environment, else the configured
/// `server_token`. Unlike [`root_token`], never generates one.
pub fn client_token(config: &ConfigService) -> Option<String> {
    std::env::var("SOLX_SERVER_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            config
                .snapshot()
                .server_token
                .filter(|s| !s.trim().is_empty())
        })
}

/// One shared token for everything: every valid bearer is
/// [`Principal::Root`], and every external action is handed the same token.
/// This is solx's original trust model, unchanged.
pub struct SingleTokenAuthority {
    token: String,
    server_url: RwLock<Option<String>>,
}

impl SingleTokenAuthority {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            server_url: RwLock::new(None),
        }
    }

    /// Built from [`root_token`], generating it if this appdata dir has none.
    pub fn from_config(config: &ConfigService) -> Result<Self> {
        Ok(Self::new(root_token(config)?))
    }
}

#[async_trait]
impl TokenAuthority for SingleTokenAuthority {
    async fn authenticate(&self, bearer: &str) -> Option<Principal> {
        constant_time_eq(bearer.as_bytes(), self.token.as_bytes()).then_some(Principal::Root)
    }

    async fn issue_action_token(
        &self,
        _action_ref: &str,
        _invocation_id: &str,
    ) -> Option<ActionCredential> {
        let server_url = self.server_url.read().ok().and_then(|u| u.clone());
        Some(ActionCredential::new(server_url, self.token.clone()))
    }

    fn set_server_url(&self, url: String) {
        if let Ok(mut slot) = self.server_url.write() {
            *slot = Some(url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn single_token_authority_accepts_only_its_token() {
        let auth = SingleTokenAuthority::new("secret");
        assert_eq!(auth.authenticate("secret").await, Some(Principal::Root));
        assert_eq!(auth.authenticate("secreT").await, None);
        assert_eq!(auth.authenticate("").await, None);
    }

    #[tokio::test]
    async fn action_credential_carries_token_and_bound_url() {
        let auth = SingleTokenAuthority::new("secret");
        let cred = auth.issue_action_token("/a", "inv").await.unwrap();
        assert_eq!(cred.token, "secret");
        assert_eq!(cred.server_url, None);

        auth.set_server_url("http://127.0.0.1:1".into());
        let cred = auth.issue_action_token("/a", "inv").await.unwrap();
        assert_eq!(cred.server_url.as_deref(), Some("http://127.0.0.1:1"));
    }
}
