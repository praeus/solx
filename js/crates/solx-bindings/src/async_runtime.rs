//! Internal plumbing for the Neon binding layer.
//!
//! The async-over-sync problem: every `Local*` manager in solx-core
//! is async (uses `tokio::spawn` internally via libsql). The Neon
//! callback API is sync, and the host process (vitest, bun, plain
//! Node) may or may not have a tokio runtime active on the JS
//! thread.
//!
//! Our solution: a single, **long-lived, dedicated tokio runtime**
//! owned by the binding, accessed via a globally-`OnceLock` `Handle`.
//! The runtime is **multi-threaded** (so libsql's internal `spawn`
//! calls have a place to run), and we use `Handle::block_on` to
//! run any future synchronously from any thread.
//!
//! Critically, we do NOT use this thread as a worker of the host
//! runtime (which is what causes the
//! "Cannot start a runtime from within a runtime" panic). We only
//! enter the binding runtime via `Handle::block_on`.
//!
//! Errors are thrown as `JsError` with the message format
//! `"SolxError::<Kind>: <detail>"` so the TS `SolxError.fromMessage`
//! helper can reconstruct a typed `SolxError`.

use std::sync::OnceLock;

use neon::prelude::*;
use neon::types::Deferred;
use serde::Serialize;
use solx_surface::error::SolxError;

pub const SOLX_ERROR_PREFIX: &str = "SolxError::";

static BINDING_RUNTIME: OnceLock<std::sync::Arc<tokio::runtime::Runtime>> = OnceLock::new();

/// Get (or create) the long-lived binding runtime.
pub fn binding_runtime() -> &'static std::sync::Arc<tokio::runtime::Runtime> {
    BINDING_RUNTIME.get_or_init(|| {
        std::sync::Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("solx-binding")
                .build()
                .expect("could not build solx binding tokio runtime"),
        )
    })
}

/// The kind discriminant of a `SolxError` — mirrors the Rust enum.
pub trait SolxErrorExt {
    fn kind_str(&self) -> &'static str;
    fn detail(&self) -> String;
}

impl SolxErrorExt for SolxError {
    fn kind_str(&self) -> &'static str {
        match self {
            SolxError::NotFound(_) => "NotFound",
            SolxError::Invalid(_) => "Invalid",
            SolxError::Validation(_) => "Validation",
            SolxError::Conflict(_) => "Conflict",
            SolxError::Io(_) => "Io",
            SolxError::Db(_) => "Db",
            SolxError::Exec(_) => "Exec",
            SolxError::Config(_) => "Config",
            SolxError::Other(_) => "Other",
        }
    }
    fn detail(&self) -> String {
        match self {
            SolxError::NotFound(s)
            | SolxError::Invalid(s)
            | SolxError::Validation(s)
            | SolxError::Conflict(s)
            | SolxError::Io(s)
            | SolxError::Db(s)
            | SolxError::Exec(s)
            | SolxError::Config(s)
            | SolxError::Other(s) => s.clone(),
        }
    }
}

/// Spawn an async operation on the long-lived binding runtime,
/// returning a `JsPromise` resolved with the JSON-serialized success
/// value or rejected with a `SolxError` message.
///
/// The future is spawned directly on the binding runtime
/// (multi-thread) so that libsql's internal `tokio::spawn`
/// calls have a runtime to schedule on. We do NOT use
/// `block_on` from this binding because that would require
/// a thread to be a runtime worker, which is incompatible
/// with the host's tokio runtime (vitest, bun, etc.).
pub fn run_async<'a, F, T>(
    cx: &mut FunctionContext<'a>,
    fut: F,
) -> JsResult<'a, JsPromise>
where
    F: std::future::Future<Output = solx_surface::error::Result<T>> + Send + 'static,
    T: Serialize + Send + 'static,
{
    let channel = cx.channel();
    let (deferred, promise) = cx.promise();
    let runtime = binding_runtime().clone();
    let join = runtime.spawn(async move {
        let result = fut.await;
        let result_str = match &result {
            Ok(v) => serde_json::to_string(v).unwrap_or_else(|e| {
                format!("\"{SOLX_ERROR_PREFIX}Other: serialize failed: {e}\"")
            }),
            Err(e) => {
                let kind = e.kind_str();
                let detail = e.detail();
                format!("ERR:{SOLX_ERROR_PREFIX}{kind}: {detail}")
            }
        };
        channel.send(move |mut tcx| -> NeonResult<()> {
            if let Some(err_msg) = result_str.strip_prefix("ERR:") {
                let err = tcx.error(err_msg.to_string())?;
                deferred.reject(&mut tcx, err);
            } else {
                let value = tcx.string(result_str);
                deferred.resolve(&mut tcx, value);
            }
            Ok(())
        });
    });
    drop(join);
    Ok(promise)
}
