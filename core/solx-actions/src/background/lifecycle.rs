//! Starting a background (explicitly or on first use) and stopping it.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use solx_surface::background::{BackgroundConfig, BackgroundOp};
use solx_surface::entities::{Action, ActionType};
use solx_surface::error::{Result, SolxError};
use solx_surface::managers::ActionManager;
use solx_surface::path::full_ref;
use solx_surface::scope::package_of;
use solx_surface::{ActionSecrets, Caller, ExcludeMode, ExecContext, Grant, ListOptions};
use tokio::sync::{mpsc, oneshot, watch};

use super::contain::Containment;
use super::handle::{BackgroundHandle, Spawned};
use super::lock::BackgroundLock;
use super::{reader, Key};
use crate::exec::{prepare_command, ActionInvoke, PreparedCommand};
use crate::LocalActionManager;

/// Set on a background's process: `<backgroundNamespace>/<backgroundName>`.
/// Tells `solx-package-lib` to speak the protocol rather than read one
/// request and exit.
const ENV_BACKGROUND: &str = "SOLX_BACKGROUND";

impl LocalActionManager {
    /// The `start` operation: spawn `action`'s command as the background, or
    /// return the running instance's status. `ctx` is the chain the start
    /// runs in; the background's own calls fall back to it.
    pub(crate) async fn start_background(
        &self,
        action: &Action,
        secrets: &ActionSecrets,
        bg: &BackgroundConfig,
        ctx: &ExecContext,
    ) -> Result<Value> {
        let key = bg.key();
        let lock = self.backgrounds.lock_for(&key);
        let _held = lock.lock().await;
        if let Some(running) = self.backgrounds.get(&key) {
            return Ok(running.status(true));
        }
        // Before anything spawns: another solx process may already run it.
        let mut lock = BackgroundLock::acquire(
            &self.config,
            &key,
            Duration::from_secs(self.config.background_lock_wait_secs()),
        )
        .await?;

        let start_ref = full_ref(&action.path, &action.name)?;
        let fn_name = action.fn_name.as_deref().ok_or_else(|| {
            SolxError::Exec("command action has no fn_name (the command to run)".into())
        })?;
        let PreparedCommand {
            mut cmd,
            command,
            log_dir,
        } = prepare_command(&self.config, &start_ref, &action.action_config, fn_name)?;
        cmd.env(ENV_BACKGROUND, format!("{}/{}", key.0, key.1))
            .kill_on_drop(true);
        Containment::prepare(&mut cmd);
        lock.inherit_into(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| {
            SolxError::Exec(format!(
                "spawn background command key '{fn_name}' ('{command}'): {e}"
            ))
        })?;
        let containment = match Containment::attach(&child) {
            Ok(c) => c,
            Err(e) => {
                let _ = child.kill().await;
                return Err(SolxError::Exec(format!(
                    "contain background {}/{}: {e}",
                    key.0, key.1
                )));
            }
        };
        lock.set_pid(child.id());
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(SolxError::Exec("background spawned without its pipes".into()));
        };

        let (writer, lines) = mpsc::unbounded_channel();
        let (exit_tx, exit) = watch::channel(None);
        let (kill, kill_rx) = oneshot::channel();
        let handle = Arc::new(BackgroundHandle::new(Spawned {
            key: key.clone(),
            start_ref,
            start_path: action.path.clone(),
            start_config: action.action_config.clone(),
            start_secrets: secrets.clone(),
            start_ctx: ctx.clone(),
            pid: child.id(),
            writer,
            exit,
            kill,
            containment,
            lock,
            running: Arc::downgrade(&self.backgrounds.running),
        }));
        self.backgrounds
            .running
            .lock()
            .unwrap()
            .insert(key, handle.clone());

        let weak = Arc::downgrade(&handle);
        reader::spawn_writer(stdin, lines);
        reader::spawn_stderr(stderr, log_dir);
        reader::spawn_reader(
            stdout,
            weak.clone(),
            self.self_ref.get().cloned().unwrap_or_default(),
        );
        reader::spawn_waiter(child, kill_rx, exit_tx, weak);
        tracing::info!(
            namespace = %bg.background_namespace,
            name = %bg.background_name,
            pid = ?handle.pid,
            "background started"
        );
        Ok(handle.status(false))
    }

    /// The running instance for an operation, started through its package's
    /// start action when there is none.
    ///
    /// The start runs through `exec_as` as the operation's action, so the
    /// scope check applies to it like any other nested call, and the process
    /// runs with the *start* action's command and scopes whichever operation
    /// arrived first.
    pub(crate) async fn ensure_background(
        &self,
        action: &Action,
        secrets: &ActionSecrets,
        bg: &BackgroundConfig,
        invoke: &ActionInvoke<'_>,
        ctx: &ExecContext,
    ) -> Result<Arc<BackgroundHandle>> {
        let key = bg.key();
        if let Some(running) = self.backgrounds.get(&key) {
            return Ok(running);
        }
        let start = self.find_start_action(action, bg).await?;
        let frame = ctx.enter(
            Caller::with_invocation(invoke.action_ref, Some(secrets.clone()), invoke.invocation_id)
                .with_grant(Grant::for_action(&action.path, action.action_config.as_ref())),
        );
        self.exec_as(&start.path, &start.name, json!({}), &frame)
            .await?;
        self.backgrounds.get(&key).ok_or_else(|| {
            SolxError::Exec(format!(
                "background {}/{} exited right after it started",
                key.0, key.1
            ))
        })
    }

    /// The Command action with `bg`'s namespace and name and operation
    /// `start`, in the same package as `action` (or, outside a package, in
    /// the same path). `list` filters by path prefix, never on
    /// `action_config`, so this matches in Rust.
    async fn find_start_action(&self, action: &Action, bg: &BackgroundConfig) -> Result<Action> {
        let prefix = match package_of(&action.path) {
            Some(package) => format!("/packages/{package}"),
            None => action.path.clone(),
        };
        let mut offset = 0;
        loop {
            let page = self
                .list(ListOptions {
                    path_prefix: Some(prefix.clone()),
                    limit: Some(200),
                    offset: Some(offset),
                    exclude_mode: Some(ExcludeMode::None),
                    ..Default::default()
                })
                .await?;
            let count = page.items.len();
            let found = page.items.into_iter().find(|a| {
                a.action_type == Some(ActionType::Command)
                    && (package_of(&action.path).is_some() || a.path == action.path)
                    && BackgroundConfig::from_action_config(a.action_config.as_ref())
                        .ok()
                        .flatten()
                        .is_some_and(|c| c.key() == bg.key() && c.op() == BackgroundOp::Start)
            });
            if let Some(found) = found {
                return Ok(found);
            }
            offset += count;
            if count == 0 || offset >= page.total {
                break;
            }
        }
        Err(SolxError::Exec(format!(
            "background {}/{} isn't running and has no start action: add a Command \
             action under {prefix} with action_config.background \
             {{\"backgroundNamespace\": \"{}\", \"backgroundName\": \"{}\", \
             \"backgroundOperation\": \"start\"}}",
            bg.background_namespace, bg.background_name, bg.background_namespace, bg.background_name
        )))
    }

    /// The `stop` operation: `shutdown`, `stop_grace_secs`, then kill.
    /// Stopping something that isn't running is a no-op.
    pub(crate) async fn stop_background(&self, key: &Key) -> Value {
        let lock = self.backgrounds.lock_for(key);
        let _held = lock.lock().await;
        let Some(handle) = self.backgrounds.get(key) else {
            return json!({
                "backgroundNamespace": key.0,
                "backgroundName": key.1,
                "status": "stopped",
                "wasRunning": false,
            });
        };
        handle
            .shutdown(Duration::from_secs(self.config.stop_grace_secs()))
            .await;
        json!({
            "backgroundNamespace": key.0,
            "backgroundName": key.1,
            "status": "stopped",
            "wasRunning": true,
        })
    }
}
