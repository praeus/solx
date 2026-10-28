//! Background commands: a Command action whose `action_config` has a
//! `background` object runs in a long-lived child that serves many calls
//! over its stdin and stdout, instead of one process per call. See
//! `docs/background-commands-plan.md` for the design, and
//! [`solx_surface::background`] for the wire protocol.
//!
//! At most one instance runs per `(backgroundNamespace, backgroundName)`
//! per [`LocalActionManager`] — a field on it like `running`, not a process
//! global, so tests stay isolated. An instance starts on first use and
//! stops on its `stop` action, on [`LocalActionManager::stop_backgrounds`],
//! or when the manager drops. Across processes, a file lock keeps it to one
//! per machine and appdata dir (see [`lock`]).

mod contain;
mod handle;
mod lifecycle;
mod lock;
mod reader;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use solx_surface::background::{BackgroundConfig, BackgroundOp};
use solx_surface::entities::Action;
use solx_surface::error::{Result, SolxError};
use solx_surface::{ActionSecrets, ExecContext};

use crate::exec::{ActionInvoke, DEFAULT_TIMEOUT_SECS};
use crate::LocalActionManager;
use handle::BackgroundHandle;

/// `(backgroundNamespace, backgroundName)`.
pub(crate) type Key = (String, String);
pub(crate) type Running = Mutex<HashMap<Key, Arc<BackgroundHandle>>>;

#[derive(Default)]
pub(crate) struct BackgroundManager {
    running: Arc<Running>,
    /// Serializes starts and stops per key, so concurrent first calls spawn
    /// one process.
    locks: Mutex<HashMap<Key, Arc<tokio::sync::Mutex<()>>>>,
}

impl BackgroundManager {
    fn lock_for(&self, key: &Key) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_default()
            .clone()
    }

    fn get(&self, key: &Key) -> Option<Arc<BackgroundHandle>> {
        self.running
            .lock()
            .unwrap()
            .get(key)
            .filter(|h| !h.is_closed())
            .cloned()
    }

    /// Every running instance's status.
    pub(crate) fn list(&self) -> Vec<Value> {
        let mut all: Vec<_> = self
            .running
            .lock()
            .unwrap()
            .values()
            .filter(|h| !h.is_closed())
            .map(|h| h.status(true))
            .collect();
        all.sort_by_key(|s| s.to_string());
        all
    }
}

impl LocalActionManager {
    /// The Command arm's branch for an action with `action_config.background`.
    pub(crate) async fn exec_background(
        &self,
        action: &Action,
        secrets: &ActionSecrets,
        bg: &BackgroundConfig,
        invoke: &ActionInvoke<'_>,
        ctx: &ExecContext,
        timeout_secs: Option<u64>,
    ) -> Result<Value> {
        match bg.op() {
            BackgroundOp::Start => self.start_background(action, secrets, bg, ctx).await,
            BackgroundOp::Stop => Ok(self.stop_background(&bg.key()).await),
            BackgroundOp::Call(op) => {
                let handle = self
                    .ensure_background(action, secrets, bg, invoke, ctx)
                    .await?;
                let timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
                let response = handle
                    .call(
                        op,
                        invoke.params.clone(),
                        Some((invoke.invocation_id, invoke.action_ref, ctx)),
                        timeout,
                    )
                    .await?;
                if response.ok {
                    Ok(response.result.unwrap_or(Value::Null))
                } else {
                    Err(SolxError::Exec(response.error.unwrap_or_else(|| {
                        format!("background operation '{op}' failed")
                    })))
                }
            }
        }
    }

    /// Stop every running background: on host shutdown. Each gets
    /// `stop_grace_secs` to exit on `shutdown`, concurrently.
    pub async fn stop_backgrounds(&self) {
        let keys: Vec<Key> = self.backgrounds.running.lock().unwrap().keys().cloned().collect();
        futures::future::join_all(keys.iter().map(|key| self.stop_background(key))).await;
    }

    /// Every running background's status, for `/builtin/background/list`.
    pub fn list_backgrounds(&self) -> Vec<Value> {
        self.backgrounds.list()
    }
}
