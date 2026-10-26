//! Execution dispatch: resolve an action, validate its params, and hand it
//! to the backend for its `action_type` — [`crate::exec`] (Command,
//! Webhook), [`crate::internal`], [`crate::wasm`], or [`crate::script`].

use std::sync::Arc;

use serde_json::Value;
use solx_surface::entities::{Action, ActionExecResult, ActionType};
use solx_surface::error::{Result, SolxError};
use solx_surface::path::full_ref;
use solx_surface::scope::Need;
use solx_surface::{ExcludeMode, ExecContext, Grant};
use uuid::Uuid;

use crate::caller::Caller;
use crate::{exec, internal, lock, script, wasm, LocalActionManager, ScopeCheck};

/// Per-action wall-clock ceiling from `action_config.timeout_secs`, using
/// the same untyped-config convention as `cwd`. `None` means the backend's
/// own default applies.
fn timeout_secs(action_config: &Option<Value>) -> Option<u64> {
    action_config.as_ref()?.get("timeout_secs")?.as_u64()
}

/// Params for an internal built-in run under [`ExcludeMode::Tooling`], with the
/// tooling exclusion mode forced on.
///
/// The entity built-ins (`entity-list-documents`, `entity-get-type`,
/// `search-documents`, ...) apply no configured exclusions unless asked, and
/// a model chooses its own arguments - so without this, anything excluded
/// from tooling would be one call away. Every spelling of the flag is
/// replaced rather than added beside: [`internal::normalize_params`] aliases
/// camelCase and snake_case, so a model-supplied `exclude_mode: "none"`
/// would otherwise survive next to ours.
///
/// Keyed on the action *type*, not on a `/builtin` path: an `Internal` row
/// saved anywhere else dispatches to the same handlers.
pub(crate) fn force_tooling_scope(params: Value) -> Value {
    let Value::Object(mut args) = params else {
        return params;
    };
    for key in [
        "excludeMode",
        "exclude_mode",
        "excludeHidden",
        "exclude_hidden",
    ] {
        args.remove(key);
    }
    args.insert("excludeMode".into(), Value::from("tooling"));
    Value::Object(args)
}

impl LocalActionManager {
    /// Hold one frame of a call chain to its [`ExcludeMode`]. Under `Tooling`,
    /// an action excluded from tooling (`exclusions.tooling`, the legacy
    /// `tool_exclude`/`mcp_exclude`, or `solx:hidden`) is refused as if it
    /// did not exist, and an internal built-in runs in the tooling scope.
    ///
    /// Runs on every frame, not just the first, which is the point: the MCP
    /// server's own check sees only the tool the model called, and that tool
    /// can be a Script or WASM action, or `action-start`, that calls another.
    fn enforce_exclusions(
        &self,
        mode: ExcludeMode,
        action: &Action,
        params: Value,
    ) -> Result<Value> {
        match mode {
            ExcludeMode::Tooling => {
                self.config
                    .exclusion_policy()
                    .guard_action(ExcludeMode::Tooling, action, false)?;
                Ok(if action.action_type == Some(ActionType::Internal) {
                    force_tooling_scope(params)
                } else {
                    params
                })
            }
            // User exclusions are a display convenience, not access
            // control, so the call chain doesn't enforce them.
            ExcludeMode::None | ExcludeMode::User => Ok(params),
        }
    }

    /// Resolve a WASM action's `bin_name` to bytes: try the shared
    /// artifact location first, then the action's own scratch space — or,
    /// for a locked action, only its pinned artifact, checked against its
    /// hash (see `crate::lock::load_artifact`).
    ///
    /// Returned behind an `Arc` because the bytes are handed to
    /// `wasm::exec`, which moves them onto the blocking pool to
    /// compile on a cache miss.
    async fn load_wasm_bytes(
        &self,
        action: &Action,
        reference: &str,
        bin_name: &str,
    ) -> Result<Arc<Vec<u8>>> {
        lock::load_artifact(&self.files, action, reference, bin_name, "wasm")
            .await
            .map(Arc::new)
    }

    /// Resolve a `Script` action's `bin_name` to its `.solx` source text —
    /// same lookup (and pin check) as [`Self::load_wasm_bytes`].
    async fn load_script_source(
        &self,
        action: &Action,
        reference: &str,
        bin_name: &str,
    ) -> Result<String> {
        let bytes = lock::load_artifact(&self.files, action, reference, bin_name, "script").await?;
        String::from_utf8(bytes).map_err(|e| {
            SolxError::Exec(format!(
                "script artifact '{bin_name}' is not valid UTF-8: {e}"
            ))
        })
    }

    /// Execute an action, optionally attributed to the action that invoked
    /// it. Equivalent to [`Self::exec_as_with`] with `invocation_id: None` —
    /// i.e. mint a fresh one at dispatch time, the behavior every existing
    /// caller of this method already gets.
    ///
    /// `ctx` is the chain's [`ExecContext`]. Its caller is `Some` only on a
    /// recursive hop from another action (a WASM guest's `action-exec`, a
    /// script's `exec` stage), and scopes `get-secret`/`set-secret` to the
    /// *calling* action's own secrets. Its principal and exclusion mode were
    /// fixed where the chain entered; under [`ExcludeMode::Tooling`] every
    /// frame is held to the tooling exclusions (see [`Self::enforce_exclusions`]).
    pub async fn exec_as(
        &self,
        path: &str,
        name: &str,
        params: Value,
        ctx: &ExecContext,
    ) -> Result<ActionExecResult> {
        self.exec_as_with(path, name, params, ctx, None).await
    }

    /// Like [`Self::exec_as`], but lets the caller fix the `invocation_id`
    /// up front instead of letting one be minted at dispatch time.
    ///
    /// Only [`Self::start_invocation`] ever passes `Some` — a detached run
    /// needs the id known *before* execution begins, so `action-stop`/
    /// `action-poll` have something to address before the run finishes (or
    /// even starts). `invocation_id.is_some()` doubles as "this is a
    /// detached run" for the timeout default below; every other caller goes
    /// through [`Self::exec_as`], which always passes `None`.
    ///
    /// **Why a manually boxed future, not `async fn`:** this method's
    /// `Internal` arm can dispatch to `action-start`, whose handler calls
    /// [`Self::start_invocation`], which `tokio::spawn`s a task that calls
    /// back into *this very method*. An `async fn`'s return type is an
    /// anonymous, structurally-inferred generator — with that indirect
    /// self-reference in the call graph, the compiler cannot resolve
    /// whether the type is `Send` (it would need to already know the answer
    /// to answer the question) and rejects it outright. Returning an
    /// explicit `Pin<Box<dyn Future + Send>>` gives the method a *nominal*
    /// type with a `Send` bound checked once against its own body, which
    /// every recursive call site (including the one inside
    /// `start_invocation`'s spawned task) can then simply trust — the same
    /// trick `#[async_trait]` uses under the hood for object-safe trait
    /// methods, applied by hand here since this isn't a trait method.
    pub fn exec_as_with<'a>(
        &'a self,
        path: &'a str,
        name: &'a str,
        params: Value,
        ctx: &'a ExecContext,
        invocation_id: Option<&'a str>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ActionExecResult>> + Send + 'a>>
    {
        Box::pin(self.exec_as_with_inner(path, name, params, ctx, invocation_id))
    }

    async fn exec_as_with_inner(
        &self,
        path: &str,
        name: &str,
        params: Value,
        ctx: &ExecContext,
        invocation_id: Option<&str>,
    ) -> Result<ActionExecResult> {
        let (action, secrets) = self.get_with_secrets(path, name).await?;
        let action_ref = full_ref(&action.path, &action.name)?;
        let params = self.enforce_exclusions(ctx.exclude_mode(), &action, params)?;
        // After the exclusions, so an excluded action still reads as
        // missing. Like them it runs on every frame: a client's call is
        // checked against its token, and each nested call against the
        // action that makes it.
        let scopes = ScopeCheck::for_exec(&self.config, ctx);
        scopes.require(&Need::Exec {
            path: &action.path,
            name: &action.name,
        })?;

        // Internal built-ins mix two wire-key conventions (camelCase
        // entity/search structs, snake_case raw-key handlers) — see
        // `internal::normalize_params`. Must run *before* validation, not
        // just before dispatch: several builtin schemas in
        // `solx-types/src/seed.rs` declare a required property under only
        // one spelling (`FilePutParams`' `rel_path`, `OauthAwaitParams`'
        // `state_value`, ...), so a caller guessing the other spelling would
        // fail validation here and never reach `run_internal` at all.
        // Scoped to `Internal` only — Command/Webhook/Script/Wasm actions are
        // user-authored external contracts (a webhook body, a command argv)
        // that must never get synthesized alias keys injected into them.
        let params = if action.action_type == Some(ActionType::Internal) {
            internal::normalize_params(&params)
        } else {
            params
        };

        if let Some(tr) = &action.param_type_ref {
            self.types.validate(&params, tr).await?;
        }

        let detached = invocation_id.is_some();
        let minted;
        let invocation_id: &str = match invocation_id {
            Some(id) => id,
            None => {
                minted = Uuid::new_v4().to_string();
                &minted
            }
        };
        // A detached run should not silently inherit the 300s sync default
        // — it is expected to outlive any one caller. `action_config`'s own
        // `timeout_secs` still wins either way.
        let effective_timeout = if detached {
            Some(
                timeout_secs(&action.action_config)
                    .unwrap_or_else(|| self.config.background_timeout_secs()),
            )
        } else {
            timeout_secs(&action.action_config)
        };

        let result = match action.action_type {
            Some(ActionType::Command) => {
                let fn_name = action.fn_name.as_deref().ok_or_else(|| {
                    SolxError::Exec("command action has no fn_name (the command to run)".into())
                })?;
                exec::run_command(
                    &self.config,
                    self.token_authority(),
                    self.console.clone(),
                    self.invocations.clone(),
                    &action_ref,
                    invocation_id,
                    fn_name,
                    &action.action_config,
                    &params,
                    effective_timeout,
                )
                .await?
            }
            Some(ActionType::Webhook) => {
                let url = action
                    .fn_name
                    .as_deref()
                    .ok_or_else(|| SolxError::Exec("webhook action has no fn_name (URL)".into()))?;
                exec::run_webhook(
                    &self.config,
                    self.console.clone(),
                    &action_ref,
                    invocation_id,
                    &action.path,
                    &action.name,
                    url,
                    &action.action_config,
                    &secrets,
                    &params,
                )
                .await?
            }
            Some(ActionType::Internal) => {
                let fn_name = action.fn_name.as_deref().ok_or_else(|| {
                    SolxError::Exec("internal action has no fn_name (operation)".into())
                })?;
                if !self.is_catalogued(&action.path, &action.name, fn_name) {
                    return Err(SolxError::Exec(format!(
                        "{action_ref} is not a built-in: internal actions come only from \
                         solx's own catalogue"
                    )));
                }
                internal::scopes::gate(fn_name, &params, &scopes)?;
                let ctx = internal::InternalCtx {
                    docs: self.docs.clone(),
                    types: self.types.clone(),
                    actions: self.self_arc()?,
                    files: self.files.clone(),
                    config: self.config.clone(),
                    local: self.self_arc()?,
                    registry: self.internal_registry(),
                    action_config: action.action_config.clone(),
                    exec: ctx.clone(),
                };
                internal::run_internal(fn_name, &params, &ctx)
                    .await
                    .map_err(SolxError::Exec)?
            }
            Some(ActionType::Wasm) => {
                let bin_name = action.bin_name.as_deref().ok_or_else(|| {
                    SolxError::Exec("wasm action has no bin_name (artifact)".into())
                })?;
                let bytes = self.load_wasm_bytes(&action, &action_ref, bin_name).await?;
                // A new caller frame: anything this guest invokes is invoked
                // by *this* action, not by whoever invoked it. `enter`
                // replaces the incoming caller rather than forwarding it, so
                // a guest can never reach an outer action's secret keys.
                // `with_invocation` rather than `from_action` so a detached
                // run's guest is stamped with the id `start_invocation`
                // already committed to the `invocations` row — for a plain
                // `exec_as` call this is exactly the freshly-minted id above,
                // so behavior is unchanged from before this method existed.
                let frame = ctx.enter(
                    Caller::with_invocation(&action_ref, Some(secrets.clone()), invocation_id)
                        .with_grant(Grant::for_action(action.action_config.as_ref())),
                );
                // WASM execution reports its own success/message (a guest can
                // report a handled failure without erroring the host call), so
                // it returns a full ActionExecResult directly rather than
                // going through the common Value-wrapping below.
                return wasm::exec(
                    self.self_arc()?,
                    self.files.clone(),
                    bytes,
                    action.fn_name.as_deref(),
                    &params,
                    frame,
                    effective_timeout,
                )
                .await;
            }
            Some(ActionType::Script) => {
                let bin_name = action.bin_name.as_deref().ok_or_else(|| {
                    SolxError::Exec("script action has no bin_name (artifact)".into())
                })?;
                let source = self.load_script_source(&action, &action_ref, bin_name).await?;
                // Same reasoning as the Wasm arm above: a fresh caller frame
                // for this action's own identity, not the incoming one — a
                // script's nested `exec` calls must never reach an outer
                // action's secret keys.
                let runner = script::ActionCommandRunner {
                    actions: self.self_arc()?,
                    ctx: ctx.enter(
                        Caller::with_invocation(&action_ref, Some(secrets.clone()), invocation_id)
                            .with_grant(Grant::for_action(action.action_config.as_ref())),
                    ),
                };
                let initial =
                    std::collections::HashMap::from([("params".to_string(), params.clone())]);
                let run = solx_scripts::execute_script_with_vars(&runner, &source, initial);
                let budget = std::time::Duration::from_secs(
                    effective_timeout.unwrap_or(exec::DEFAULT_TIMEOUT_SECS),
                );
                tokio::time::timeout(budget, run).await.map_err(|_| {
                    SolxError::Exec(format!("script action {action_ref} timed out"))
                })??
            }
            None => {
                return Err(SolxError::Exec(format!(
                    "action {action_ref} has no action_type to execute"
                )))
            }
        };

        Ok(ActionExecResult {
            action: action_ref,
            result,
            success: true,
            message: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seed;
    use crate::test_support::*;
    use solx_config::ConfigService;
    use solx_docs::LocalDocManager;
    use solx_files::LocalFileStore;
    use solx_surface::entities::ActionInput;
    use solx_surface::managers::{ActionManager, DocManager, FileStore, TypeManager};
    use solx_types::LocalTypeManager;

    #[test]
    fn the_tooling_scope_replaces_every_spelling_of_the_mode() {
        let got = force_tooling_scope(serde_json::json!({
            "pathPrefix": "/a",
            "exclude_mode": "none",
            "excludeHidden": false,
        }));
        assert_eq!(
            got,
            serde_json::json!({"pathPrefix": "/a", "excludeMode": "tooling"})
        );
    }

    /// The tooling scope keys on the action type, not a `/builtin` path.
    #[tokio::test]
    async fn the_tooling_scope_forces_the_mode_on_internal_rows() {
        let (_d, _cfg, m) = setup().await;
        let (action, _) = m
            .get_with_secrets(seed::DOCUMENT_PATH, "entity-list-documents")
            .await
            .unwrap();
        let params = m
            .enforce_exclusions(
                ExcludeMode::Tooling,
                &action,
                serde_json::json!({"excludeMode": "none"}),
            )
            .unwrap();
        assert_eq!(params, serde_json::json!({"excludeMode": "tooling"}));

        let params = m
            .enforce_exclusions(
                ExcludeMode::None,
                &action,
                serde_json::json!({"excludeMode": "none"}),
            )
            .unwrap();
        assert_eq!(params, serde_json::json!({"excludeMode": "none"}));
    }

    /// Proves the aliasing chokepoint lives ahead of param-type validation,
    /// not just ahead of dispatch: `FilePutParams` requires `rel_path`
    /// (`solx-types/src/seed.rs`), so a `run_internal`-only fix would have
    /// rejected this call at `self.types.validate` before ever reaching the
    /// handler. Going through `exec` end-to-end (not `run_internal`
    /// directly) is what exercises that ordering.
    #[tokio::test]
    async fn exec_accepts_camel_case_for_a_required_snake_case_builtin_param() {
        let (_d, _c, m) = setup_wired().await;
        let result = m
            .exec(
                "/builtin/file",
                "file-put",
                serde_json::json!({"relPath": "a.txt", "content": "hi"}),
            )
            .await
            .unwrap();
        assert_eq!(
            result.result.get("rel_path").and_then(Value::as_str),
            Some("a.txt")
        );
    }

    #[tokio::test]
    async fn exec_command_resolves_fn_name_through_the_allowlist() {
        let (_d, c, m) = setup().await;
        // fn_name is a key into `command_actions`, never a literal command —
        // register it first. Echo a bare number so the output is valid JSON
        // on both cmd.exe and sh without quote handling.
        allow_command(&c, "echo-42", "echo 42");
        let input = ActionInput {
            action_type: Some(ActionType::Command),
            fn_name: Some("echo-42".into()),
            ..Default::default()
        };
        m.save("/tools", "echo", input).await.unwrap();

        let res = m
            .exec("/tools", "echo", serde_json::json!({}))
            .await
            .unwrap();
        assert!(res.success);
        assert_eq!(res.result, serde_json::json!(42));
    }

    #[tokio::test]
    async fn exec_command_with_unregistered_key_is_denied() {
        let (_d, _c, m) = setup().await;
        // Deny-by-default: an empty/absent allowlist rejects every key,
        // including one that happens to look like a real shell command.
        let input = ActionInput {
            action_type: Some(ActionType::Command),
            fn_name: Some("echo 42".into()),
            ..Default::default()
        };
        m.save("/tools", "echo", input).await.unwrap();

        let err = m
            .exec("/tools", "echo", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("command_actions"), "{err}");
    }

    /// Minimal single-shot HTTP server for the webhook logging tests below —
    /// always replies 200 with a tiny JSON body, regardless of what was sent.
    async fn start_ok_server() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 8192];
                    let _ = stream.read(&mut buf).await;
                    let body = r#"{"ok":true}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body,
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn exec_webhook_logs_start_and_success_to_the_console() {
        let (_d, c, m) = setup().await;
        let (url, server) = start_ok_server().await;
        allow_webhook_prefix(&c, "http://127.0.0.1:");

        m.save(
            "/tools",
            "hook",
            ActionInput {
                action_type: Some(ActionType::Webhook),
                fn_name: Some(url),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let res = m
            .exec("/tools", "hook", serde_json::json!({"x": 1}))
            .await
            .unwrap();
        assert!(res.success);

        let entries = m
            .console()
            .read("/tools/hook", None, 10)
            .await
            .unwrap()
            .entries;
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].source, "webhook");
        assert!(entries[0]
            .message
            .as_deref()
            .unwrap()
            .starts_with("POST http"));
        assert!(entries[1].message.as_deref().unwrap().contains("succeeded"));
        // Params must never be logged — they can carry secrets a nested
        // reader of the console has no business seeing.
        assert!(!entries
            .iter()
            .any(|e| e.message.as_deref().unwrap_or("").contains("\"x\"")));

        server.abort();
    }

    #[tokio::test]
    async fn exec_webhook_logs_failure_to_the_console() {
        let (_d, c, m) = setup().await;
        allow_webhook_prefix(&c, "http://127.0.0.1:");
        m.save(
            "/tools",
            "deadhook",
            ActionInput {
                action_type: Some(ActionType::Webhook),
                // Nothing listens here — a fast, deterministic transport failure.
                fn_name: Some("http://127.0.0.1:1".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let err = m.exec("/tools", "deadhook", serde_json::json!({})).await;
        assert!(err.is_err());

        let entries = m
            .console()
            .read("/tools/deadhook", None, 10)
            .await
            .unwrap()
            .entries;
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[1].level, "warn");
        assert!(entries[1].message.as_deref().unwrap().contains("failed"));
    }

    #[tokio::test]
    async fn exec_webhook_with_unlisted_url_is_denied_before_any_request() {
        let (_d, _c, m) = setup().await;
        // Deny-by-default, and no console entries at all — the allowlist
        // check happens before the "POST ..." start log is written.
        m.save(
            "/tools",
            "hook",
            ActionInput {
                action_type: Some(ActionType::Webhook),
                fn_name: Some("http://127.0.0.1:1".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let err = m
            .exec("/tools", "hook", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("allowed_base_urls"), "{err}");

        let entries = m
            .console()
            .read("/tools/hook", None, 10)
            .await
            .unwrap()
            .entries;
        assert!(entries.is_empty(), "{entries:?}");
    }

    // ── entity-save-action: no self-granting shell ───────────────────────
    //
    // These go through `exec` on `/builtin/action/entity-save-action`, which is the
    // exact path an MCP tool call, a WASM guest's `action-exec`, and a
    // `.solx` script all take. A direct `m.save(...)` is the CLI's path and
    // stays allowed — that's the whole distinction being enforced.

    // Only ever used for `entity-save-action`/`entity-delete-action`, which
    // live under `ACTION_PATH` (action-entity CRUD, alongside the async
    // start/stop/poll/cancelled actions), not the flat `/builtin` root.
    async fn exec_builtin(
        m: &Arc<LocalActionManager>,
        fn_name: &str,
        params: Value,
    ) -> Result<Value> {
        m.exec(seed::ACTION_PATH, fn_name, params)
            .await
            .map(|r| r.result)
    }

    #[tokio::test]
    async fn entity_save_action_refuses_to_create_command_or_webhook() {
        let (_d, _cfg, m) = setup_wired().await;

        for ty in ["command", "webhook"] {
            let err = exec_builtin(
                &m,
                "entity-save-action",
                serde_json::json!({
                    "path": "/evil", "name": "shell",
                    "actionType": ty, "fnName": "rm -rf /"
                }),
            )
            .await
            .unwrap_err();
            assert!(err.to_string().contains("use the solx CLI"), "{ty}: {err}");
            assert!(
                m.get("/evil", "shell").await.is_err(),
                "{ty} was created anyway"
            );
        }
    }

    /// `save` is a merge-upsert, so a payload with no `action_type` at all
    /// would otherwise silently rewrite an existing Command action's shell
    /// command. The guard has to consult the stored row, not just the input.
    #[tokio::test]
    async fn entity_save_action_refuses_to_repoint_an_existing_command() {
        let (_d, _cfg, m) = setup_wired().await;
        m.save(
            "/tools",
            "safe",
            ActionInput {
                action_type: Some(ActionType::Command),
                fn_name: Some("echo 42".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let err = exec_builtin(
            &m,
            "entity-save-action",
            serde_json::json!({ "path": "/tools", "name": "safe", "fnName": "rm -rf /" }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("use the solx CLI"), "{err}");
        assert_eq!(
            m.get("/tools", "safe")
                .await
                .unwrap()
                .fn_name
                .as_deref(),
            Some("echo 42")
        );
    }

    #[tokio::test]
    async fn entity_delete_action_refuses_to_remove_a_command() {
        let (_d, _cfg, m) = setup_wired().await;
        m.save(
            "/tools",
            "safe",
            ActionInput {
                action_type: Some(ActionType::Command),
                fn_name: Some("echo 42".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let err = exec_builtin(
            &m,
            "entity-delete-action",
            serde_json::json!({ "path": "/tools", "name": "safe" }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("use the solx CLI"), "{err}");
        assert!(
            m.get("/tools", "safe").await.is_ok(),
            "action was deleted anyway"
        );
    }

    /// The lockdown is limited to the two executable types — everything else
    /// an agent legitimately does through this built-in still works.
    #[tokio::test]
    async fn entity_save_action_still_allows_non_executable_types() {
        let (_d, _cfg, m) = setup_wired().await;
        exec_builtin(
            &m,
            "entity-save-action",
            serde_json::json!({
                "path": "/tools", "name": "w",
                "actionType": "wasm", "binName": "x.wasm"
            }),
        )
        .await
        .unwrap();
        assert!(m.get("/tools", "w").await.is_ok());
    }

    // ── entity-save-action: keyed actions are root-only ─────────────────

    #[tokio::test]
    async fn entity_save_action_refuses_to_set_a_key() {
        let (_d, _cfg, m) = setup_wired().await;
        let err = exec_builtin(
            &m,
            "entity-save-action",
            serde_json::json!({
                "path": "/tools", "name": "w",
                "actionType": "wasm", "binName": "x.wasm",
                "actionConfig": { "secrets": { "S": TEST_KEY } }
            }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("only root"), "{err}");
        assert!(m.get("/tools", "w").await.is_err());
    }

    /// Not just the key: adding a name would extend the key to another
    /// secret, and repointing `binName` would run new code with it.
    #[tokio::test]
    async fn entity_save_action_refuses_to_modify_a_keyed_action() {
        let (_d, _cfg, m) = setup_wired().await;
        m.files
            .put(&solx_files::shared_action_file_path("good.wasm"), b"\0asm".to_vec())
            .await
            .unwrap();
        m.save(
            "/pkg",
            "w",
            ActionInput {
                action_type: Some(ActionType::Wasm),
                bin_name: Some("good.wasm".into()),
                action_config: Some(serde_json::json!({ "secrets": { "S": TEST_KEY } })),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        for change in [
            serde_json::json!({ "binName": "evil.wasm" }),
            serde_json::json!({ "actionConfig": { "secrets": { "S": "***", "OTHER": "***" } } }),
            serde_json::json!({ "actionConfig": {} }),
        ] {
            let mut params = serde_json::json!({ "path": "/pkg", "name": "w" });
            params
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            let err = exec_builtin(&m, "entity-save-action", params)
                .await
                .unwrap_err();
            // Declaring secrets needs root; any other change meets the lock
            // that having secrets implies.
            let err = err.to_string();
            assert!(
                err.contains("only root") || err.contains("is locked"),
                "{change}: {err}"
            );
        }
        let kept = m.get("/pkg", "w").await.unwrap();
        assert_eq!(kept.bin_name.as_deref(), Some("good.wasm"));
    }

    #[tokio::test]
    async fn exec_wasm_missing_artifact_errors() {
        let (_d, _c, m) = setup().await;
        let input = ActionInput {
            action_type: Some(ActionType::Wasm),
            bin_name: Some("x.wasm".into()),
            ..Default::default()
        };
        m.save("/tools", "w", input).await.unwrap();
        let err = m
            .exec("/tools", "w", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("x.wasm"), "{err}");
    }

    #[tokio::test]
    async fn exec_wasm_without_bin_name_errors() {
        let (_d, _c, m) = setup().await;
        let input = ActionInput {
            action_type: Some(ActionType::Wasm),
            ..Default::default()
        };
        m.save("/tools", "w", input).await.unwrap();
        let err = m
            .exec("/tools", "w", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("bin_name"), "{err}");
    }

    // ── Script action tests ─────────────────────────────────────────────────

    /// Like `setup_wired`, but also hands back the `FileStore` so a test can
    /// upload a `.solx` artifact before posting the action that references it.
    async fn setup_script() -> (
        tempfile::TempDir,
        Arc<LocalActionManager>,
        Arc<dyn FileStore>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Arc::new(ConfigService::open_in(dir.path()).unwrap());
        let types: Arc<dyn TypeManager> = Arc::new(
            LocalTypeManager::open(&dir.path().join("types.db"))
                .await
                .unwrap(),
        );
        let docs: Arc<dyn DocManager> = Arc::new(
            LocalDocManager::open(&dir.path().join("docs.db"), types.clone())
                .await
                .unwrap(),
        );
        let files: Arc<dyn FileStore> = Arc::new(LocalFileStore::new(dir.path().join("files")));
        let m = LocalActionManager::open(
            &dir.path().join("actions.db"),
            cfg,
            types,
            docs,
            files.clone(),
        )
        .await
        .unwrap();
        let m = Arc::new(m);
        m.set_self_ref(Arc::downgrade(&m));
        (dir, m, files)
    }

    async fn post_script_artifact(files: &Arc<dyn FileStore>, name: &str, source: &str) {
        files
            .put(
                &solx_files::shared_action_file_path(name),
                source.as_bytes().to_vec(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn exec_script_reads_params_and_calls_nested_action() {
        let (_d, m, files) = setup_script().await;
        // The bare `exec /builtin/action/entity-list-actions` (no `json`
        // wrapping needed) becomes the script's result directly: the callee's
        // whole ActionExecResult, JSON-encoded — same as `handle_exec` in the
        // CLI. The else branch is a plain string value, no `json` stage.
        post_script_artifact(
            &files,
            "hello.solx",
            "if $params.go == true then exec /builtin/action/entity-list-actions; else; \"skipped\"; endif",
        )
        .await;
        m.save(
            "/tools",
            "hello",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some("hello.solx".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let ran = m
            .exec("/tools", "hello", serde_json::json!({"go": true}))
            .await
            .unwrap();
        assert_eq!(ran.result["success"], serde_json::json!(true));
        // The nested action's result is a list page (an object).
        assert!(
            ran.result["result"].is_object(),
            "expected a list-page object, got: {:?}",
            ran.result
        );

        let skipped = m
            .exec("/tools", "hello", serde_json::json!({"go": false}))
            .await
            .unwrap();
        assert_eq!(skipped.result, serde_json::json!("skipped"));
    }

    #[tokio::test]
    async fn exec_script_logs_each_stage_to_the_console() {
        let (_d, m, files) = setup_script().await;
        post_script_artifact(
            &files,
            "count.solx",
            "exec /builtin/action/entity-list-actions; exec /builtin/document/entity-list-documents",
        )
        .await;
        m.save(
            "/tools",
            "count",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some("count.solx".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        m.exec("/tools", "count", serde_json::json!({}))
            .await
            .unwrap();

        let entries = m
            .console()
            .read("/tools/count", None, 10)
            .await
            .unwrap()
            .entries;
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].source, "script");
        assert_eq!(
            entries[0].message.as_deref(),
            Some("exec /builtin/action/entity-list-actions")
        );
        assert_eq!(
            entries[1].message.as_deref(),
            Some("exec /builtin/document/entity-list-documents")
        );
        // Both stages share the one script invocation's identity.
        assert_eq!(entries[0].invocation_id, entries[1].invocation_id);
    }

    #[tokio::test]
    async fn exec_script_logs_a_stage_failure() {
        let (_d, m, files) = setup_script().await;
        post_script_artifact(&files, "bad.solx", "exec /builtin/nope").await;
        m.save(
            "/tools",
            "bad",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some("bad.solx".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let err = m.exec("/tools", "bad", serde_json::json!({})).await;
        assert!(err.is_err());

        let entries = m
            .console()
            .read("/tools/bad", None, 10)
            .await
            .unwrap()
            .entries;
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[1].level, "warn");
        assert!(entries[1].message.as_deref().unwrap().contains("failed"));
    }

    #[tokio::test]
    async fn exec_script_missing_bin_name_errors() {
        let (_d, _c, m) = setup().await;
        let input = ActionInput {
            action_type: Some(ActionType::Script),
            ..Default::default()
        };
        m.save("/tools", "s", input).await.unwrap();
        let err = m
            .exec("/tools", "s", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("bin_name"), "{err}");
    }

    #[tokio::test]
    async fn exec_script_missing_artifact_errors() {
        let (_d, _c, m) = setup().await;
        let input = ActionInput {
            action_type: Some(ActionType::Script),
            bin_name: Some("missing.solx".into()),
            ..Default::default()
        };
        m.save("/tools", "s", input).await.unwrap();
        let err = m
            .exec("/tools", "s", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing.solx"), "{err}");
    }

    #[tokio::test]
    async fn exec_script_unsupported_stage_verb_errors() {
        let (_d, m, files) = setup_script().await;
        post_script_artifact(&files, "bad.solx", "post doc /a --json '{}'").await;
        m.save(
            "/tools",
            "bad",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some("bad.solx".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let err = m
            .exec("/tools", "bad", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("unsupported script stage"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn exec_script_action_not_blocked_by_executable_action_guard() {
        let (_d, m, _files) = setup_script().await;
        // Unlike `command`/`webhook`, `script` may be created through the
        // guarded `entity-save-action` built-in the same way `wasm` can.
        exec_builtin(
            &m,
            "entity-save-action",
            serde_json::json!({
                "path": "/tools", "name": "s",
                "actionType": "script", "binName": "x.solx"
            }),
        )
        .await
        .unwrap();
        assert!(m.get("/tools", "s").await.is_ok());
    }

    #[tokio::test]
    async fn exec_script_times_out() {
        let (_d, m, files) = setup_script().await;
        post_script_artifact(&files, "slow.solx", "wait 5").await;
        m.save(
            "/tools",
            "slow",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some("slow.solx".into()),
                action_config: Some(serde_json::json!({ "timeout_secs": 1 })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let err = m
            .exec("/tools", "slow", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
    }
}
