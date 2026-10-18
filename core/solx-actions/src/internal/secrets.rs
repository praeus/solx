//! Per-caller secrets: `get-secret` / `set-secret`.
//!
//! Scoped to the action that *invoked* these built-ins (`ctx.caller`), not
//! to the `/builtin/secrets/get-secret` row that dispatched here — that row's own
//! `action_config` is always `null` (see `seed.rs`), so scoping to it, as
//! an earlier version did, meant no key ever resolved and both built-ins
//! were silently inert.
//!
//! An action wanting `get-secret`/`set-secret` must list the secret's name
//! in its own `action_config.secrets`. The value is stored under the
//! action's secret scope (its package, or the action itself) and encrypted
//! with that scope's key, which solx keeps in the actions database. The key
//! never leaves the host: these return the decrypted *value*, never the key
//! that unlocked it.

use serde_json::{json, Value};

use solx_surface::SecretHandle;

use crate::caller::Caller;

use super::require_str;

pub(super) async fn get_secret(params: &Value, caller: Option<&Caller>) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let secret = caller_secret(caller, name)?;
    let value = crate::secrets::get_secret(&secret).await?;
    Ok(json!({ "value": value }))
}

pub(super) async fn set_secret(params: &Value, caller: Option<&Caller>) -> Result<Value, String> {
    let name = require_str(params, "name")?;
    let value = require_str(params, "value")?;
    let secret = caller_secret(caller, name)?;
    crate::secrets::set_secret(&secret, value).await?;
    Ok(json!({ "set": true }))
}

/// Secret `name` of the calling action, if it may use it.
fn caller_secret(caller: Option<&Caller>, name: &str) -> Result<SecretHandle, String> {
    let caller = caller.ok_or_else(|| {
        format!(
            "secret '{name}' is scoped to the calling action, but this call has no \
             action caller — get-secret/set-secret can only be used from within an action"
        )
    })?;
    caller.secret(name).ok_or_else(|| {
        format!(
            "{caller} can't use secret '{name}': it isn't listed in the action's \
             action_config.secrets"
        )
    })
}
