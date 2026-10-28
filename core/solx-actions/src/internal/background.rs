//! `/builtin/background/*`: see and stop the background commands this host
//! runs (see `crate::background`). Gated like every other built-in, by
//! `exec:` on its path, so an action needs that scope to stop another
//! package's background; an operator at a local entry point can do both.

use serde_json::{json, Value};

use super::*;

/// `background-list` `{backgroundNamespace?}`: every running instance's
/// status, optionally one namespace's.
pub(super) fn list(params: &Value, ctx: &InternalCtx) -> Result<Value, String> {
    let namespace = params.get("backgroundNamespace").and_then(Value::as_str);
    let backgrounds: Vec<Value> = ctx
        .local
        .list_backgrounds()
        .into_iter()
        .filter(|b| namespace.is_none_or(|ns| b["backgroundNamespace"] == ns))
        .collect();
    Ok(json!({ "backgrounds": backgrounds }))
}

/// `background-stop` `{backgroundNamespace, backgroundName}`: what the
/// package's own `stop` action does, without needing one. A no-op when it
/// isn't running.
pub(super) async fn stop(params: &Value, ctx: &InternalCtx) -> Result<Value, String> {
    let key = (
        require_str(params, "backgroundNamespace")?.to_string(),
        require_str(params, "backgroundName")?.to_string(),
    );
    Ok(ctx.local.stop_background(&key).await)
}
