//! Action execution backends.
//!
//! * **Command** — `fn_name` is never the literal shell command. It's a key
//!   resolved against `ConfigService::command_actions` (the
//!   `command_actions` allowlist in `solx-config.json`); the matching
//!   [`solx_config::CommandDef`]'s `command` field is what actually runs.
//!   **Deny-by-default**: an unregistered key is a hard error, not a
//!   fallback to running the key text itself.
//! * **Webhook** — `fn_name` is the literal URL to call, but the URL must
//!   start with one of the prefixes in `ConfigService::allowed_webhook_base_urls`
//!   (also `solx-config.json`) or dispatch is refused before any network
//!   call. Auth/headers come from `action_config`; OAuth token exchange
//!   (bearer, refresh_token, service_account, authorization_code) is
//!   resolved via [`crate::auth::resolve_auth`]. Defaults to POSTing
//!   `params` as a JSON body; `action_config.method: "GET"` sends `params`
//!   as query-string parameters instead, with no body.
//!
//! Both allowlists are deny-by-default: an unset or empty allowlist rejects
//! every Command/Webhook action, not the old behavior of permitting
//! everything until configured. See `docs/next-steps.md` §1 for why (ported
//! from old `sol`'s `command_actions` key-indirection and
//! `allowed_webhook_base_urls` prefix allowlist, with the default flipped).
//!
//! `Wasm`-typed actions are executed by [`crate::wasm`], not here —
//! see [`crate::LocalActionManager::exec`]'s `Wasm` arm.

use serde_json::Value;

mod command;
mod webhook;

pub use command::run_command;
pub(crate) use command::{prepare_command, PreparedCommand};
pub use webhook::run_webhook;

/// Wall-clock ceiling on a command, overridable per action via
/// `action_config.timeout_secs`. Also reused by `crate::script` as the
/// default ceiling on a whole `Script` action's execution.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// What one invocation of an action needs, whatever its backend: which
/// action, which run of it, its stored config, and the call-time params.
pub struct ActionInvoke<'a> {
    pub action_ref: &'a str,
    pub invocation_id: &'a str,
    pub action_config: &'a Option<Value>,
    pub params: &'a Value,
}
