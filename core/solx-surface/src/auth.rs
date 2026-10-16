//! Bearer-token authority: the seam between "a bearer token arrived" and
//! "who is this, and what may they do".
//!
//! Two implementations:
//! - `solx_config::credentials::SingleTokenAuthority`, the original single
//!   shared `server_token`: every valid bearer is [`Principal::Root`], and
//!   every external action is handed that same token;
//! - `solx_registry::Registry` (the default-on `registry` feature), which
//!   adds per-client and per-invocation tokens with expirations.
//!
//! `solx-server`'s middleware and `solx-actions`' Command spawn see only
//! this trait, so swapping one for the other touches neither.

use std::any::Any;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;

/// Who a presented bearer token belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// The root token (today's single `server_token`), or an in-process host.
    Root,
    /// A client issued its own token (not issued by the single-token authority).
    Client { id: String, label: Option<String> },
    /// One external action invocation (not issued by the single-token authority).
    Action {
        action_ref: String,
        invocation_id: String,
    },
}

/// What a spawned Command action is handed so it can call back into
/// `solx-server`, as `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN`.
///
/// Holding it keeps the token valid. An authority that issues per-invocation
/// tokens attaches a guard that revokes the token on drop, so the caller
/// should keep the credential bound until the action finishes — the same
/// pattern as `solx_console::loopback::Registration`.
pub struct ActionCredential {
    /// `None` when the authority knows of no server to call back into. The
    /// default authority always has one: this process's bound server, else
    /// the one a sibling host on the same appdata dir would serve.
    pub server_url: Option<String>,
    pub token: String,
    /// Opaque handle the issuing authority uses to find this credential
    /// again in [`TokenAuthority::release`]. `None` for a credential that
    /// was not issued per invocation.
    pub id: Option<String>,
    _guard: Option<Box<dyn Any + Send + Sync>>,
}

impl ActionCredential {
    pub fn new(server_url: Option<String>, token: String) -> Self {
        Self {
            server_url,
            token,
            id: None,
            _guard: None,
        }
    }

    /// Attach the authority's handle for [`TokenAuthority::release`].
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Attach a value whose `Drop` revokes the token.
    pub fn with_guard(mut self, guard: impl Any + Send + Sync) -> Self {
        self._guard = Some(Box::new(guard));
        self
    }
}

impl fmt::Debug for ActionCredential {
    // Never print the token.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionCredential")
            .field("server_url", &self.server_url)
            .field("token", &"***")
            .field("id", &self.id)
            .finish()
    }
}

#[async_trait]
pub trait TokenAuthority: Send + Sync {
    /// Resolve a presented bearer token. `None` rejects it.
    async fn authenticate(&self, bearer: &str) -> Option<Principal>;

    /// Credential for one external action invocation, valid for at least
    /// `ttl` (the invocation's own timeout). `None` means the action gets no
    /// server access.
    async fn issue_action_token(
        &self,
        action_ref: &str,
        invocation_id: &str,
        ttl: Duration,
    ) -> Option<ActionCredential>;

    /// Revoke `credential` now that its invocation has finished. A no-op for
    /// an authority whose credentials aren't per invocation.
    async fn release(&self, _credential: &ActionCredential) {}

    /// Record where this process serves HTTP, once the host has bound it,
    /// so issued action credentials can carry `SOLX_SERVER_URL`.
    fn set_server_url(&self, url: String);
}

/// Request header a client sends (with the value `none`) to run an action
/// with **no** principal, though its bearer token authenticates as one.
///
/// `ActionManager::exec` means "no principal", and `solx-client`'s remote
/// `exec` keeps that meaning across the wire with this header. Without it,
/// the CLI in remote mode, and every `.solx` script it runs, would act as
/// `Root` (it presents the loopback root token) and could reach the
/// root-only built-ins. A client can only drop its principal this way,
/// never raise it.
pub const NO_PRINCIPAL_HEADER: &str = "x-solx-principal";

/// Byte comparison whose running time doesn't depend on where the first
/// mismatch is, so a network caller can't recover a token one byte at a
/// time. A length mismatch returns early, which reveals only the length.
/// Tokens have a fixed length, so that leaks nothing useful.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn constant_time_eq_matches_only_identical_bytes() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
