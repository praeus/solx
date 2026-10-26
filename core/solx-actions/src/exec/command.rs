//! `Command` action backend — see the [`super`] module doc.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use solx_config::ConfigService;
use solx_surface::action_scopes;
use solx_surface::auth::TokenAuthority;
use solx_surface::error::{Result, SolxError};
use solx_surface::path::split_ref;
use tokio::io::AsyncWriteExt;

use solx_console::loopback as console_loopback;
use solx_console::{ConsoleStore, InvocationStore};

use super::{ActionInvoke, DEFAULT_TIMEOUT_SECS};

/// Set on every spawned Command action that has a credential — see
/// [`run_command`].
const ENV_SERVER_URL: &str = "SOLX_SERVER_URL";
const ENV_SERVER_TOKEN: &str = "SOLX_SERVER_TOKEN";

/// Turn an action ref into a filesystem-safe path segment for
/// [`ConfigService::logs_dir`] — `/packages/solx-omniparse/process-file` ->
/// `packages_solx-omniparse_process-file`.
fn log_dir_slug(action_ref: &str) -> String {
    action_ref
        .trim_start_matches('/')
        .chars()
        .map(|c| if c == '/' { '_' } else { c })
        .collect()
}

/// `action_config.env` used to be forwarded to the child. It was removed:
/// every package used it only to set `SOLX_SERVER_URL`, which solx now
/// injects itself, and it was readable by anyone who could `get` the action
/// (`crate::mask` doesn't cover it). Rows saved before the removal still
/// carry the map, so it's ignored — and only a key other than that one is
/// worth a warning, since that's the only case whose behavior changed.
fn warn_on_removed_env(action_ref: &str, action_config: &Option<Value>) {
    let Some(env) = action_config
        .as_ref()
        .and_then(|c| c.get("env"))
        .and_then(Value::as_object)
    else {
        return;
    };
    for key in env.keys().filter(|k| k.as_str() != ENV_SERVER_URL) {
        tracing::warn!(
            action_ref = %action_ref,
            env_key = %key,
            "action_config.env is no longer supported and is ignored; set the \
             variable in the command_actions entry's command instead",
        );
    }
}

/// Run a `Command` action. `fn_name` is a key resolved against the
/// `command_actions` allowlist (see the module doc); the matching
/// `CommandDef.command` is what actually executes. Params are passed as
/// JSON on stdin only (no env var) — no payload size limit, and it doesn't
/// leak into process listings.
///
/// Fully async: the child is spawned with `tokio::process`, so an action
/// that takes minutes parks no thread. This used to be a synchronous
/// `std::process` call made directly from `exec_as`, which meant every
/// running command held an async *worker* thread — only `num_cpus` of
/// those exist, so a handful of concurrent commands could wedge the whole
/// process, HTTP routes included.
///
/// `invocation_id` is minted by the caller (`exec_as_with`), not here — a
/// detached `action-start` run needs the id fixed *before* execution begins
/// so `action-stop`/`action-poll` have something to address.
pub async fn run_command(
    cfg: &ConfigService,
    authority: Option<&Arc<dyn TokenAuthority>>,
    console: Arc<ConsoleStore>,
    invocations: Arc<InvocationStore>,
    invoke: &ActionInvoke<'_>,
    fn_name: &str,
    timeout_secs: Option<u64>,
) -> Result<Value> {
    let &ActionInvoke {
        action_ref,
        invocation_id,
        action_config,
        params,
    } = invoke;

    // `fn_name` is a key, not a command — resolve it against the
    // `command_actions` allowlist before anything else runs. Deny-by-default:
    // an unregistered key (including every key when the allowlist itself is
    // unset) is a hard error naming the fix, not a fallback to running the
    // key text as a shell command.
    let def = cfg.command_actions().remove(fn_name).ok_or_else(|| {
        SolxError::Exec(format!(
            "command key '{fn_name}' is not registered in solx-config.json's \
             'command_actions' allowlist; add an entry there to allow this \
             command to run"
        ))
    })?;

    // The allowlist's own `cwd` wins over the action's `action_config.cwd`
    // when both are set — the allowlist author decides where an approved
    // command runs, not the (potentially untrusted) action that invokes it.
    let cwd = def
        .cwd
        .clone()
        .or_else(|| {
            action_config
                .as_ref()
                .and_then(|c| c.get("cwd"))
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| cfg.appdata().to_string_lossy().into_owned());
    let command = def.command.as_str();

    let params_json = serde_json::to_string(params)?;
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    // `SOL_LOG_DIR` is the env var `solx-omniparse`/`solx-media` already read
    // for their own file logging (a convention carried over from old sol) —
    // this is what actually turns it on. It was never set here before, so
    // that logging has been silently inert regardless of what any package
    // does on its own. One subdirectory per action ref, so two actions in
    // the same package (e.g. omniparse's process/write variants) don't
    // collide on one shared file.
    let log_dir = cfg.logs_dir().join(log_dir_slug(action_ref));

    // A one-shot credential letting this specific invocation POST to its
    // own console over loopback HTTP — see `solx_console::loopback` for
    // why this replaced reading the child's stderr. Best-effort: if the
    // loopback can't start, the command still runs, it just has no console
    // (or cancellation-check) access for this run. `registration` must stay
    // bound (not `_`) for the rest of this function — dropping it
    // deregisters the token, and we want that to happen automatically on
    // every exit path below, not just the success path.
    let registration =
        console_loopback::register(console, invocations, action_ref, invocation_id).await;

    let timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));

    // How this invocation reaches `solx-server`: handed to the child as
    // `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN`, the only supported way for an
    // action to get a token. Valid for the invocation's timeout, and
    // released below once the child is done. Kept bound like
    // `registration`: an authority's guard on it revokes the token on drop,
    // as a fallback for early returns and panics. It carries exactly the
    // scopes the action runs with (`action_scopes`), so the child can do
    // what its action asked for and nothing more; one that declares none
    // (or a malformed list, which a save refuses) gets a token that can do
    // nothing but call its own package.
    let path = split_ref(action_ref)
        .map(|(path, _)| path)
        .unwrap_or_default();
    let scopes = action_scopes(&path, action_config.as_ref());
    let credential = match authority {
        Some(authority) => {
            authority
                .issue_action_token(action_ref, invocation_id, timeout, &scopes)
                .await
        }
        None => None,
    };

    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg(flag)
        .arg(command)
        .current_dir(&cwd)
        .env("SOL_LOG_DIR", &log_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // So a timed-out or cancelled exec reaps the child instead of
        // leaking it.
        //
        // Caveat: this kills the *shell* we spawned, not its descendants.
        // Neither Windows nor POSIX terminates a process tree on request
        // without extra machinery (a Job Object / a process group), so a
        // long-running grandchild — `sh -c 'sleep 600'` spawning `sleep`,
        // or `cmd /C ping ...` spawning `ping` — outlives the timeout and
        // keeps the inherited stdout pipe open. Killing the tree properly
        // would mean putting each command in its own job/process group.
        .kill_on_drop(true);
    if let Some(reg) = &registration {
        cmd.env("SOLX_CONSOLE_URL", reg.url())
            .env("SOLX_CONSOLE_TOKEN", reg.token())
            .env("SOLX_CONTROL_URL", reg.control_url());
    }

    if let Some(c) = &credential {
        cmd.env(ENV_SERVER_TOKEN, &c.token);
        if let Some(url) = &c.server_url {
            cmd.env(ENV_SERVER_URL, url);
        }
    }
    warn_on_removed_env(action_ref, action_config);
    let mut child = cmd.spawn().map_err(|e| {
        SolxError::Exec(format!("spawn command key '{fn_name}' ('{command}'): {e}"))
    })?;

    // Feed stdin *concurrently* with draining stdout/stderr. Writing it all
    // up front deadlocks whenever the payload exceeds the pipe buffer
    // (~64KB) and the child doesn't drain stdin before producing output:
    // the child blocks writing stdout, we block writing stdin, neither
    // moves. Dropping the handle after the write closes the pipe so the
    // child sees EOF.
    let mut stdin = child.stdin.take();
    let feed = async move {
        if let Some(mut pipe) = stdin.take() {
            let _ = pipe.write_all(params_json.as_bytes()).await;
            let _ = pipe.shutdown().await;
        }
    };

    let wait = async {
        let (_, out) = tokio::join!(feed, child.wait_with_output());
        out
    };

    let result = match tokio::time::timeout(timeout, wait).await {
        Ok(Ok(output)) => command_result(fn_name, output),
        Ok(Err(e)) => Err(SolxError::Exec(e.to_string())),
        Err(_) => Err(SolxError::Exec(format!(
            "command key '{fn_name}' timed out after {}s",
            timeout.as_secs()
        ))),
    };

    // The child is gone (on timeout, `kill_on_drop` reaped it when `wait`
    // dropped), so its token is no longer needed, however it ended.
    if let (Some(authority), Some(credential)) = (authority, &credential) {
        authority.release(credential).await;
    }
    result
}

fn command_result(fn_name: &str, output: std::process::Output) -> Result<Value> {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SolxError::Exec(format!(
            "command key '{fn_name}' failed ({}): {}",
            output.status,
            stderr.trim()
        )));
    }
    Ok(serde_json::from_str::<Value>(stdout.trim()).unwrap_or(Value::String(stdout)))
}
