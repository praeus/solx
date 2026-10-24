//! Redeeming a connect code at a `solx-server`'s `POST /auth/exchange` —
//! the one unauthenticated call a client makes, to get its own token.

use serde::Deserialize;
use serde_json::json;
use solx_surface::error::SolxError;

use crate::error::{read_response, transport_err};
use crate::http::collection_url;

/// What `/auth/exchange` returns for a redeemed code.
#[derive(Deserialize)]
pub struct Exchanged {
    pub token: String,
    pub client_id: String,
    pub expires_at: Option<String>,
    /// Where the server says clients should reach it. Can differ from the
    /// URL the code was redeemed at (`server_public_url`).
    pub server_url: Option<String>,
    /// The server's root pin, when it serves solx-certs certificates. Only
    /// as trustworthy as the connection it came over: worth keeping when
    /// that was already verified (a pin, `server_ca_file`, the OS store).
    #[serde(default)]
    pub tls_pin: Option<String>,
}

impl Exchanged {
    /// The pin to keep for `server_url` after this exchange, when the
    /// client had none: the one the server reported, for an `https://`
    /// server. The exchange went over a connection this machine already
    /// trusted, so from then on it trusts only that root.
    pub fn adopted_pin(&self, server_url: &str) -> Option<String> {
        self.tls_pin
            .clone()
            .filter(|pin| server_url.starts_with("https://") && crate::tls::looks_like_pin(pin))
    }
}

impl std::fmt::Debug for Exchanged {
    // Never print the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Exchanged")
            .field("client_id", &self.client_id)
            .field("expires_at", &self.expires_at)
            .field("server_url", &self.server_url)
            .field("tls_pin", &self.tls_pin)
            .finish()
    }
}

/// Redeem `code` (any spacing) at `server_url` for a client token, over
/// `http` (a [`crate::http_client`], so an `https://` server's root is
/// trusted the same way the managers will trust it).
pub async fn exchange_connect_code(
    http: &reqwest::Client,
    server_url: &str,
    code: &str,
) -> Result<Exchanged, SolxError> {
    let url = collection_url(server_url, "auth/exchange")?;
    let resp = http
        .post(url)
        .json(&json!({ "code": code }))
        .send()
        .await
        .map_err(transport_err)?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(SolxError::Invalid(format!(
            "{server_url} doesn't accept connect codes (an older solx-server, or one built \
             without the token registry); ask for a token instead"
        )));
    }
    read_response(resp).await
}
