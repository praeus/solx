//! Bearer-token authority: the seam between "a bearer token arrived" and
//! "who is this, and what may they do".
//!
//! Today there is one implementation, `solx_config::credentials::SingleTokenAuthority`,
//! which is the original single shared `server_token`: every valid bearer is
//! [`Principal::Root`], and every external action is handed that same token.
//! The trait exists so a token registry (per-client and per-invocation
//! tokens with expirations) can replace it without touching `solx-server`'s
//! middleware or `solx-actions`' Command spawn.

use std::any::Any;
use std::fmt;

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
    /// `None` when no HTTP server is bound in this process, so there is
    /// nothing to call back into.
    pub server_url: Option<String>,
    pub token: String,
    _guard: Option<Box<dyn Any + Send + Sync>>,
}

impl ActionCredential {
    pub fn new(server_url: Option<String>, token: String) -> Self {
        Self {
            server_url,
            token,
            _guard: None,
        }
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
            .finish()
    }
}

#[async_trait]
pub trait TokenAuthority: Send + Sync {
    /// Resolve a presented bearer token. `None` rejects it.
    async fn authenticate(&self, bearer: &str) -> Option<Principal>;

    /// Credential for one external action invocation. `None` means the
    /// action gets no server access.
    async fn issue_action_token(
        &self,
        action_ref: &str,
        invocation_id: &str,
    ) -> Option<ActionCredential>;

    /// Record where this process serves HTTP, once the host has bound it,
    /// so issued action credentials can carry `SOLX_SERVER_URL`.
    fn set_server_url(&self, url: String);
}

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
