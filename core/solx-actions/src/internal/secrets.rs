//! Per-caller secrets: `get-secret` / `set-secret`.
//!
//! Scoped to the action that *invoked* these built-ins (`ctx.caller`), not
//! to the `/builtin/secrets/get-secret` row that dispatched here — that row's own
//! `action_config` is always `null` (see `seed.rs`), so scoping to it, as
//! an earlier version did, meant no key ever resolved and both built-ins
//! were silently inert.
//!
//! An action wanting `get-secret`/`set-secret` must list the secret's name
//! in its own `action_config.secrets` and have a key (kept in the actions
//! table's `secret_key` column, not in the config). The key never leaves
//! the host: these return the decrypted *value*, never the key that
//! unlocked it.

use serde_json::{json, Value};

use crate::caller::Caller;

use super::require_str;

pub(super) async fn get_secret(params: &Value, caller: Option<&Caller>) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let key = caller_secret_key(caller, name)?;
    let value = crate::secrets::get_secret(name, key).await?;
    Ok(json!({ "value": value }))
}

pub(super) async fn set_secret(params: &Value, caller: Option<&Caller>) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let value = require_str(params, "value")?;
    let key = caller_secret_key(caller, name)?;
    crate::secrets::set_secret(name, value, key).await?;
    Ok(json!({ "set": true }))
}

/// Look up the calling action's AES key, if it may use `name`.
fn caller_secret_key<'a>(caller: Option<&'a Caller>, name: &str) -> Result<&'a str, String> {
    let caller = caller.ok_or_else(|| {
        format!(
            "secret '{name}' is scoped to the calling action, but this call has no \
             action caller — get-secret/set-secret can only be used from within an action"
        )
    })?;
    caller.secret_key(name).ok_or_else(|| {
        format!(
            "{caller} has no key for secret '{name}' (the name must be listed in its \
             action_config.secrets, and the action needs a key)"
        )
    })
}
