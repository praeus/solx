//! The wire protocol between a host and a background command — a
//! long-lived Command action child serving many calls over its stdin and
//! stdout (see `docs/background-commands-plan.md`).
//!
//! Newline-delimited JSON: each line is one [`Message`], a request or a
//! response, correlated by `id`. Both sides send requests, and each
//! allocates ids for its own, so ids are unique per direction only.
//! `solx-package-lib` mirrors these types for the child's side; it doesn't
//! depend on this crate, so a change here is a change there too.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Host → background: stop serving and exit. The background answers it
/// (or simply exits) and the host kills it after `stop_grace_secs`.
pub const OP_SHUTDOWN: &str = "shutdown";
/// Background → host: run an action, `{path, name, params}`. The result is
/// the action's `ActionExecResult`.
pub const OP_EXEC: &str = "exec";
/// Background → host: write to a console,
/// `{level, message, data?, invocation_id?}`.
pub const OP_CONSOLE_PRINT: &str = "console.print";

/// One line of the protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Message {
    Request(CommandRequest),
    Response(CommandResponse),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRequest {
    pub id: u64,
    pub op: String,
    #[serde(default)]
    pub params: Value,
    /// The invocation this call belongs to. Host → background, so console
    /// output and cancellation checks map to the right run; a background
    /// passes it back on the host calls it makes while serving that call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResponse {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CommandResponse {
    pub fn success(id: u64, result: Value) -> Self {
        CommandResponse {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: u64, error: impl Into<String>) -> Self {
        CommandResponse {
            id,
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}

/// `action_config.background`: which background a Command action belongs
/// to, and what calling it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundConfig {
    pub background_name: String,
    pub background_namespace: String,
    pub background_operation: String,
}

/// What a [`BackgroundConfig`]'s `backgroundOperation` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundOp<'a> {
    /// Spawn the process, or return the running one's status.
    Start,
    /// Shut the process down; a no-op when nothing is running.
    Stop,
    /// Forward the call's params to the process as this op.
    Call(&'a str),
}

impl BackgroundConfig {
    /// The `background` object of `action_config`, if it has one. Present
    /// but malformed is an error, not "not a background": a typo shouldn't
    /// turn an operation into a one-shot run of the start command.
    pub fn from_action_config(action_config: Option<&Value>) -> Result<Option<Self>, String> {
        let Some(raw) = action_config.and_then(|c| c.get("background")) else {
            return Ok(None);
        };
        let config: BackgroundConfig = serde_json::from_value(raw.clone())
            .map_err(|e| format!("action_config.background: {e}"))?;
        if config.background_name.trim().is_empty()
            || config.background_namespace.trim().is_empty()
            || config.background_operation.trim().is_empty()
        {
            return Err("action_config.background: backgroundName, backgroundNamespace and \
                        backgroundOperation must be non-empty"
                .into());
        }
        if config.background_operation == OP_SHUTDOWN {
            return Err(format!(
                "action_config.background: '{OP_SHUTDOWN}' is reserved; use \
                 backgroundOperation \"stop\""
            ));
        }
        Ok(Some(config))
    }

    pub fn op(&self) -> BackgroundOp<'_> {
        match self.background_operation.as_str() {
            "start" => BackgroundOp::Start,
            "stop" => BackgroundOp::Stop,
            op => BackgroundOp::Call(op),
        }
    }

    /// The key at most one running instance has per host.
    pub fn key(&self) -> (String, String) {
        (
            self.background_namespace.clone(),
            self.background_name.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_carry_their_kind() {
        let line = serde_json::to_string(&Message::Request(CommandRequest {
            id: 7,
            op: "echo".into(),
            params: json!({"a": 1}),
            invocation_id: None,
        }))
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap(),
            json!({"kind": "request", "id": 7, "op": "echo", "params": {"a": 1}})
        );
        let back: Message =
            serde_json::from_str(r#"{"kind":"response","id":7,"ok":false,"error":"no"}"#).unwrap();
        assert_eq!(back, Message::Response(CommandResponse::failure(7, "no")));
    }

    #[test]
    fn background_config_parses_and_rejects() {
        let cfg = json!({"background": {
            "backgroundName": "session",
            "backgroundNamespace": "solx-firefox",
            "backgroundOperation": "open-tab"
        }});
        let bg = BackgroundConfig::from_action_config(Some(&cfg))
            .unwrap()
            .unwrap();
        assert_eq!(bg.op(), BackgroundOp::Call("open-tab"));
        assert_eq!(bg.key(), ("solx-firefox".into(), "session".into()));

        assert_eq!(
            BackgroundConfig::from_action_config(Some(&json!({"cwd": "x"}))).unwrap(),
            None
        );
        assert!(BackgroundConfig::from_action_config(Some(
            &json!({"background": {"backgroundName": "s"}})
        ))
        .is_err());
        assert!(BackgroundConfig::from_action_config(Some(&json!({"background": {
            "backgroundName": "s", "backgroundNamespace": "n", "backgroundOperation": "shutdown"
        }})))
        .is_err());
    }
}
