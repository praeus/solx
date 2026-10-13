//! Neon binding for the `FileStore` trait.
//!
//! Exposed under `require("solx.node")`:
//!   - `filesNew(root) -> JsFileStore`              (sync; constructor)
//!   - `filesPut(mgr, relPath, base64Bytes) -> Promise<string>`  (returns normalized rel)
//!   - `filesGet(mgr, relPath) -> Promise<string>`                (returns base64 bytes)
//!   - `filesDelete(mgr, relPath) -> Promise<null>`
//!   - `filesList(mgr, prefix) -> Promise<string>`                 (returns JSON array)
//!
//! **Wire format:** all byte payloads cross the JS boundary as
//! base64-encoded UTF-8 strings (inside the JSON envelope). The TS
//! side decodes/encodes transparently so consumers get a real
//! `Uint8Array`.
//!
//! **Trait seam:** the handle holds `Arc<dyn FileStore>` (the
//! trait from `solx_surface::managers`), not the concrete
//! `LocalFileStore`. `new()` (local mode) and `connect()` (client
//! mode, via `solx_client::RemoteFileStore`) both box into the
//! same trait object — every method below dispatches through the
//! trait. The on-disk root path is captured at the TS layer for a
//! local store (it has the value already, since it opened the
//! store) rather than round-tripping through the binding — the
//! trait doesn't expose `root()` and we don't want to downcast.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use neon::prelude::*;
#[cfg(feature = "local")]
use solx_files::LocalFileStore;
use solx_surface::managers::FileStore;

use crate::async_runtime::{run_async, SOLX_ERROR_PREFIX};

pub struct JsFileStore {
    pub(crate) inner: Arc<dyn FileStore>,
}

impl Finalize for JsFileStore {}

fn b64_engine() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// Decode a base64 string into raw bytes. Used for the inbound
/// `put` payload.
fn decode_b64(s: &str) -> solx_surface::error::Result<Vec<u8>> {
    b64_engine()
        .decode(s.as_bytes())
        .map_err(|e| solx_surface::error::SolxError::Other(format!("bad base64: {e}")))
}

fn new(mut cx: FunctionContext) -> JsResult<JsBox<JsFileStore>> {
    let root = cx.argument::<JsString>(0)?.value(&mut cx);
    // Trait seam: today we always construct the local impl
    // and box it. A future `cfg(client)` branch here would
    // open an HTTP-backed `Arc<dyn FileStore>` instead.
    let store: Arc<dyn FileStore> = {
        #[cfg(feature = "local")]
        {
            Arc::new(LocalFileStore::new(PathBuf::from(root)))
        }
        #[cfg(not(feature = "local"))]
        {
            // No store backend is currently compiled in.
            let _ = root;
            return cx.throw_error(format!(
                "{SOLX_ERROR_PREFIX}Other: no file-store backend compiled in \
                 (enable `local` or `client` feature)"
            ));
        }
    };
    Ok(cx.boxed(JsFileStore { inner: store }))
}

/// Construct a `FileStore` backed by a remote `solx-server` over
/// HTTP. Synchronous — no I/O happens until the first call.
fn connect(mut cx: FunctionContext) -> JsResult<JsBox<JsFileStore>> {
    let server_url = cx.argument::<JsString>(0)?.value(&mut cx);
    let token = cx.argument::<JsString>(1)?.value(&mut cx);

    #[cfg(feature = "client")]
    {
        let remote = solx_client::RemoteFileStore::new(server_url, token);
        Ok(cx.boxed(JsFileStore {
            inner: Arc::new(remote) as Arc<dyn FileStore>,
        }))
    }
    #[cfg(not(feature = "client"))]
    {
        let _ = (server_url, token);
        cx.throw_error(format!(
            "{SOLX_ERROR_PREFIX}Other: no client backend compiled in (enable the `client` feature)"
        ))
    }
}

fn put(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsFileStore>>(0)?;
    let rel_path = cx.argument::<JsString>(1)?.value(&mut cx);
    let b64 = cx.argument::<JsString>(2)?.value(&mut cx);
    let bytes = match decode_b64(&b64) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: {e:?}"));
        }
    };
    let inner = mgr.inner.clone();
    let rel_path = rel_path.clone();
    run_async(&mut cx, async move {
        FileStore::put(&*inner, &rel_path, bytes).await
    })
}

fn get(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsFileStore>>(0)?;
    let rel_path = cx.argument::<JsString>(1)?.value(&mut cx);
    let inner = mgr.inner.clone();
    let rel_path = rel_path.clone();
    run_async(&mut cx, async move {
        // We must return a base64-encoded string (Rust struct
        // serialization can't round-trip a `Vec<u8>` as a JS
        // value cleanly without going through `JsBuffer`, and
        // we want the `run_async` JSON-string wire format to
        // stay uniform). The TS side decodes this.
        let bytes = FileStore::get(&*inner, &rel_path).await?;
        Ok(b64_engine().encode(&bytes))
    })
}

fn delete(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsFileStore>>(0)?;
    let rel_path = cx.argument::<JsString>(1)?.value(&mut cx);
    let inner = mgr.inner.clone();
    let rel_path = rel_path.clone();
    run_async(&mut cx, async move {
        FileStore::delete(&*inner, &rel_path).await
    })
}

fn list(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsFileStore>>(0)?;
    let prefix = cx.argument::<JsString>(1)?.value(&mut cx);
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        let items = FileStore::list(&*inner, &prefix).await?;
        // Return as a JSON array of strings.
        Ok(serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string()))
    })
}

/// Register all file-store functions on the parent module context.
pub fn register_files_module(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("filesNew", new)?;
    cx.export_function("filesConnect", connect)?;
    cx.export_function("filesPut", put)?;
    cx.export_function("filesGet", get)?;
    cx.export_function("filesDelete", delete)?;
    cx.export_function("filesList", list)?;
    // `filesRoot` is intentionally NOT registered: it
    // required downcasting the trait object back to the
    // concrete `LocalFileStore`. The TS layer captures the
    // root at construction time and exposes it via
    // `LocalFileStore.root()`.
    Ok(())
}
