//! One running background: its process, the calls waiting on it, and the
//! identity its own calls back into the host run as.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use solx_surface::background::{CommandRequest, CommandResponse, Message, OP_SHUTDOWN};
use solx_surface::error::{Result, SolxError};
use solx_surface::{ActionSecrets, Caller, ExecContext, Grant};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

use super::contain::Containment;
use super::lock::BackgroundLock;
use super::{Key, Running};

/// A call the host has sent and not yet had answered, by `invocation_id`:
/// what the background's own calls made while serving it run under.
#[derive(Clone)]
struct Inflight {
    action_ref: String,
    ctx: ExecContext,
}

/// What a call from the background back into the host runs as.
pub(crate) struct HostFrame {
    pub ctx: ExecContext,
    /// The console its output goes to.
    pub action_ref: String,
    pub invocation_id: String,
}

pub(crate) struct BackgroundHandle {
    pub(crate) key: Key,
    /// The start action: the process runs its command, and everything the
    /// background calls is checked against its scopes.
    pub(crate) start_ref: String,
    start_path: String,
    start_config: Option<Value>,
    start_secrets: ActionSecrets,
    /// The chain the start ran in, for calls the background makes on its
    /// own rather than while serving one of ours.
    start_ctx: ExecContext,
    pub(crate) pid: Option<u32>,
    pub(crate) started_at: DateTime<Utc>,

    next_id: AtomicU64,
    /// Lines to the single stdin writer task.
    writer: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, oneshot::Sender<CommandResponse>>>,
    inflight: Mutex<HashMap<String, Inflight>>,

    closed: AtomicBool,
    /// Set once, by the waiter task, to how the process ended.
    exit: watch::Receiver<Option<String>>,
    /// Sending (or dropping) makes the waiter kill the shell.
    kill: Mutex<Option<oneshot::Sender<()>>>,
    pub(crate) containment: Containment,
    /// The manager's map, to take this handle out of when it closes.
    running: Weak<Running>,
    /// Held until the process tree is gone ([`Self::release_lock`]), or
    /// with the handle. Last, so it drops after `containment` has killed
    /// the tree.
    lock: Mutex<Option<BackgroundLock>>,
}

pub(crate) struct Spawned {
    pub key: Key,
    pub start_ref: String,
    pub start_path: String,
    pub start_config: Option<Value>,
    pub start_secrets: ActionSecrets,
    pub start_ctx: ExecContext,
    pub pid: Option<u32>,
    pub writer: mpsc::UnboundedSender<String>,
    pub exit: watch::Receiver<Option<String>>,
    pub kill: oneshot::Sender<()>,
    pub containment: Containment,
    pub lock: BackgroundLock,
    pub running: Weak<Running>,
}

impl BackgroundHandle {
    pub(crate) fn new(s: Spawned) -> Self {
        BackgroundHandle {
            key: s.key,
            start_ref: s.start_ref,
            start_path: s.start_path,
            start_config: s.start_config,
            start_secrets: s.start_secrets,
            start_ctx: s.start_ctx,
            pid: s.pid,
            started_at: Utc::now(),
            next_id: AtomicU64::new(1),
            writer: s.writer,
            pending: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            exit: s.exit,
            kill: Mutex::new(Some(s.kill)),
            containment: s.containment,
            running: s.running,
            lock: Mutex::new(Some(s.lock)),
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub(crate) fn status(&self, already_running: bool) -> Value {
        json!({
            "backgroundNamespace": self.key.0,
            "backgroundName": self.key.1,
            "status": "running",
            "pid": self.pid,
            "startedAt": self.started_at.to_rfc3339(),
            "startAction": self.start_ref,
            "alreadyRunning": already_running,
        })
    }

    pub(crate) fn send(&self, message: &Message) -> Result<()> {
        let line = serde_json::to_string(message)?;
        self.writer
            .send(line)
            .map_err(|_| SolxError::Exec(self.gone()))
    }

    fn gone(&self) -> String {
        let how = self.exit.borrow().clone();
        format!(
            "background {}/{} is no longer running{}",
            self.key.0,
            self.key.1,
            how.map(|h| format!(" ({h})")).unwrap_or_default()
        )
    }

    /// Send `op` and wait for its response. `invocation` is the call this
    /// belongs to: its id goes on the request, and calls the background
    /// makes while serving it run under `ctx`.
    pub(crate) async fn call(
        &self,
        op: &str,
        params: Value,
        invocation: Option<(&str, &str, &ExecContext)>,
        timeout: Duration,
    ) -> Result<CommandResponse> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let invocation_id = invocation.map(|(invocation_id, action_ref, ctx)| {
            self.inflight.lock().unwrap().insert(
                invocation_id.to_string(),
                Inflight {
                    action_ref: action_ref.to_string(),
                    ctx: ctx.clone(),
                },
            );
            invocation_id.to_string()
        });
        // Cleans up on every way out, a dropped (cancelled) call included.
        let _guard = CallGuard {
            handle: self,
            id,
            invocation_id: invocation_id.clone(),
        };
        // Checked after registering: `close` sets the flag before failing
        // what's pending, so a call either gets failed there or sees it here.
        if self.is_closed() {
            return Err(SolxError::Exec(self.gone()));
        }
        self.send(&Message::Request(CommandRequest {
            id,
            op: op.to_string(),
            params,
            invocation_id,
        }))?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(SolxError::Exec(self.gone())),
            Err(_) => Err(SolxError::Exec(format!(
                "background {}/{} did not answer '{op}' within {}s",
                self.key.0,
                self.key.1,
                timeout.as_secs()
            ))),
        }
    }

    /// Complete the host's request `response.id`. Unknown ids (a call that
    /// already timed out) are dropped.
    pub(crate) fn complete(&self, response: CommandResponse) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&response.id) {
            let _ = tx.send(response);
        }
    }

    /// What a call from the background runs as: under the chain of the call
    /// it's serving when it names one, else the chain that started it, and
    /// always as the start action, so it's checked against that action's
    /// scopes whichever operation woke the background.
    pub(crate) fn host_frame(&self, invocation_id: Option<&str>) -> HostFrame {
        let serving = invocation_id
            .and_then(|id| self.inflight.lock().unwrap().get(id).cloned().map(|f| (id, f)));
        let (base, action_ref, invocation_id) = match serving {
            Some((id, f)) => (f.ctx, f.action_ref, id.to_string()),
            None => (
                self.start_ctx.clone(),
                self.start_ref.clone(),
                Uuid::new_v4().to_string(),
            ),
        };
        let caller = Caller::with_invocation(
            &self.start_ref,
            Some(self.start_secrets.clone()),
            &invocation_id,
        )
        .with_grant(Grant::for_action(
            &self.start_path,
            self.start_config.as_ref(),
        ));
        HostFrame {
            ctx: base.enter(caller),
            action_ref,
            invocation_id,
        }
    }

    /// Mark this background gone: out of the manager's map, and every
    /// pending call failed with `reason`. Idempotent.
    pub(crate) fn close(self: &Arc<Self>, reason: &str) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(running) = self.running.upgrade() {
            let mut map = running.lock().unwrap();
            if map.get(&self.key).is_some_and(|h| Arc::ptr_eq(h, self)) {
                map.remove(&self.key);
            }
        }
        let pending: Vec<_> = self.pending.lock().unwrap().drain().collect();
        for (id, tx) in pending {
            let _ = tx.send(CommandResponse::failure(id, reason));
        }
        tracing::info!(
            namespace = %self.key.0,
            name = %self.key.1,
            "background closed: {reason}"
        );
    }

    /// Let another solx process start this background. Only once the
    /// process and whatever it started are gone: released any earlier, a
    /// second host could open the database the first still has open.
    pub(crate) fn release_lock(&self) {
        self.lock.lock().unwrap().take();
    }

    async fn wait_exit(&self, within: Duration) -> bool {
        let mut exit = self.exit.clone();
        let exited = tokio::time::timeout(within, exit.wait_for(Option::is_some))
            .await
            .is_ok();
        exited
    }

    /// Ask the process to exit, give it `grace`, then kill its whole tree.
    pub(crate) async fn shutdown(self: &Arc<Self>, grace: Duration) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let asked = self
            .send(&Message::Request(CommandRequest {
                id,
                op: OP_SHUTDOWN.into(),
                params: Value::Null,
                invocation_id: None,
            }))
            .is_ok();
        if !(asked && self.wait_exit(grace).await) {
            if let Some(kill) = self.kill.lock().unwrap().take() {
                let _ = kill.send(());
            }
        }
        // Whatever the shell left behind goes too, and with it the last
        // holder of the stdout pipe.
        self.containment.kill();
        self.wait_exit(Duration::from_secs(5)).await;
        self.close("background stopped");
    }
}

struct CallGuard<'a> {
    handle: &'a BackgroundHandle,
    id: u64,
    invocation_id: Option<String>,
}

impl Drop for CallGuard<'_> {
    fn drop(&mut self) {
        self.handle.pending.lock().unwrap().remove(&self.id);
        if let Some(id) = &self.invocation_id {
            self.handle.inflight.lock().unwrap().remove(id);
        }
    }
}
