//! The tasks behind a running background: one writer owning its stdin, one
//! reader dispatching what arrives on its stdout, one draining stderr into
//! its log, and one waiting for it to exit.
//!
//! None holds the handle strongly except while handling a line, so
//! dropping the manager drops the handle, which kills the process (see
//! [`super::contain`]), which ends them all.

use std::path::PathBuf;
use std::sync::Weak;

use serde::Deserialize;
use serde_json::{json, Value};
use solx_surface::background::{
    CommandRequest, CommandResponse, Message, OP_CONSOLE_PRINT, OP_EXEC,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, watch};

use super::handle::BackgroundHandle;
use crate::LocalActionManager;

pub(crate) fn spawn_writer(mut stdin: ChildStdin, mut lines: mpsc::UnboundedReceiver<String>) {
    tokio::spawn(async move {
        while let Some(mut line) = lines.recv().await {
            line.push('\n');
            if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                break;
            }
        }
        // Every sender is gone (the handle dropped): closing stdin is the
        // background's cue that nobody is listening any more.
    });
}

/// Owns the shell: reports how it exited, and kills it when asked to, or
/// when the handle (holding `kill`'s sender) drops.
pub(crate) fn spawn_waiter(
    mut child: Child,
    kill: oneshot::Receiver<()>,
    exit: watch::Sender<Option<String>>,
    handle: Weak<BackgroundHandle>,
) {
    tokio::spawn(async move {
        let how = tokio::select! {
            status = child.wait() => match status {
                Ok(status) => format!("exited, {status}"),
                Err(e) => format!("wait failed: {e}"),
            },
            _ = kill => {
                let _ = child.kill().await;
                "killed".to_string()
            }
        };
        let _ = exit.send(Some(how.clone()));
        if let Some(handle) = handle.upgrade() {
            // A grandchild could still hold stdout open; it goes too.
            handle.containment.kill();
            handle.close(&format!("background {how}"));
            handle.release_lock();
        }
    });
}

/// stderr isn't part of the protocol: it goes to the start action's log, so
/// a chatty child can never fill the pipe and stall.
pub(crate) fn spawn_stderr(stderr: ChildStderr, log_dir: PathBuf) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut log = match tokio::fs::create_dir_all(&log_dir).await {
            Ok(()) => tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_dir.join("background.log"))
                .await
                .ok(),
            Err(_) => None,
        };
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "solx_actions::background", "{line}");
            if let Some(file) = log.as_mut() {
                let _ = file.write_all(format!("{line}\n").as_bytes()).await;
            }
        }
    });
}

pub(crate) fn spawn_reader(
    stdout: ChildStdout,
    handle: Weak<BackgroundHandle>,
    actions: Weak<LocalActionManager>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let reason = loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break "background closed its stdout".to_string(),
                Err(e) => break format!("reading the background's stdout failed: {e}"),
            };
            if line.trim().is_empty() {
                continue;
            }
            let Some(handle) = handle.upgrade() else {
                return;
            };
            match serde_json::from_str::<Message>(&line) {
                Ok(Message::Response(response)) => handle.complete(response),
                Ok(Message::Request(request)) => {
                    // Its own task, so a slow `exec` doesn't hold up the
                    // responses behind it.
                    let actions = actions.clone();
                    tokio::spawn(async move {
                        let response = serve_host_request(&handle, &actions, request).await;
                        let _ = handle.send(&Message::Response(response));
                    });
                }
                // Stdout belongs to the protocol: anything else means the
                // stream can't be trusted from here on.
                Err(e) => {
                    let shown: String = line.chars().take(200).collect();
                    break format!("background wrote a line that isn't a protocol message ({e}): {shown}");
                }
            }
        };
        if let Some(handle) = handle.upgrade() {
            handle.close(&reason);
            handle.containment.kill();
        }
    });
}

#[derive(Deserialize)]
struct ExecParams {
    path: String,
    name: String,
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
struct PrintParams {
    #[serde(default = "default_level")]
    level: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    invocation_id: Option<String>,
}

fn default_level() -> String {
    "info".into()
}

/// A call the background makes into the host.
async fn serve_host_request(
    handle: &BackgroundHandle,
    actions: &Weak<LocalActionManager>,
    request: CommandRequest,
) -> CommandResponse {
    let id = request.id;
    let Some(actions) = actions.upgrade() else {
        return CommandResponse::failure(id, "the host is shutting down");
    };
    match request.op.as_str() {
        OP_EXEC => {
            let p: ExecParams = match serde_json::from_value(request.params) {
                Ok(p) => p,
                Err(e) => return CommandResponse::failure(id, format!("exec: {e}")),
            };
            // Through `exec_as`, as the start action: the scope check and
            // exclusions are the ones any nested call from it gets, and a
            // refusal is recorded the same way.
            let frame = handle.host_frame(request.invocation_id.as_deref());
            match actions.exec_as(&p.path, &p.name, p.params, &frame.ctx).await {
                Ok(result) => match serde_json::to_value(result) {
                    Ok(v) => CommandResponse::success(id, v),
                    Err(e) => CommandResponse::failure(id, e.to_string()),
                },
                Err(e) => CommandResponse::failure(id, e.to_string()),
            }
        }
        OP_CONSOLE_PRINT => {
            let p: PrintParams = match serde_json::from_value(request.params) {
                Ok(p) => p,
                Err(e) => return CommandResponse::failure(id, format!("console.print: {e}")),
            };
            let invocation_id = p.invocation_id.or(request.invocation_id);
            let frame = handle.host_frame(invocation_id.as_deref());
            match actions
                .console()
                .print(
                    &frame.action_ref,
                    &frame.invocation_id,
                    None,
                    &p.level,
                    "command",
                    p.message,
                    p.data,
                )
                .await
            {
                Ok(seq) => CommandResponse::success(id, json!({ "seq": seq })),
                Err(e) => CommandResponse::failure(id, e.to_string()),
            }
        }
        op => CommandResponse::failure(id, format!("unknown host op '{op}'")),
    }
}
