//! Neon binding for the `TypeManager` trait.
//!
//! All async work is done by spawning onto the long-lived binding
//! tokio runtime (see `async_runtime`). The closures passed to
//! `run_async` return `impl Future`, not `Result` directly, so
//! libsql's internal `tokio::spawn` calls all land in the same
//! runtime as our awaiters.
//!
//! **Trait seam:** the handle holds `Arc<dyn TypeManager>`
//! (the trait from `solx_surface::managers`), not the concrete
//! `LocalTypeManager`. The local impl is constructed in `open()`
//! (local mode) and a `solx_client::RemoteTypeManager` in
//! `connect()` (client mode), both boxed into the same trait
//! object — every method below dispatches through the trait and
//! is unaware of which one it got.

use std::sync::Arc;

use neon::prelude::*;
use solx_surface::managers::TypeManager;
use solx_surface::query::ListOptions;
#[cfg(feature = "local")]
use solx_types::LocalTypeManager;

use crate::async_runtime::{run_async, SolxErrorExt, SOLX_ERROR_PREFIX};

/// The handle returned by `typesOpen`/`typesConnect`. Holds the
/// manager plus a (currently empty) placeholder for a per-manager
/// runtime if we ever need one for fire-and-forget background work.
pub struct JsTypeManager {
    pub(crate) inner: Arc<dyn TypeManager>,
}

impl Finalize for JsTypeManager {}

fn open(mut cx: FunctionContext) -> JsResult<JsBox<JsTypeManager>> {
    let path = cx.argument::<JsString>(0)?.value(&mut cx);
    // We open the database synchronously on the JS thread using
    // a one-shot current-thread runtime. This is acceptable
    // because `open()` is a one-time setup cost; subsequent
    // calls (`save`, `get`, etc.) all use the long-lived
    // binding runtime via `run_async`.
    //
    // The `#[cfg(feature = "local")]` / `#[cfg(feature = "client")]`
    // branches below are the seam where a future HTTP-backed
    // client impl would construct its own `Arc<dyn TypeManager>`
    // and return it the same way.
    let path_buf = std::path::PathBuf::from(path);
    let open_result: solx_surface::error::Result<Arc<dyn TypeManager>> = (|| {
        #[cfg(feature = "local")]
        {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| solx_surface::error::SolxError::Other(e.to_string()))?;
            let local = rt.block_on(LocalTypeManager::open(&path_buf))?;
            Ok(Arc::new(local) as Arc<dyn TypeManager>)
        }
        #[cfg(not(feature = "local"))]
        {
            // No manager backend is currently compiled in. A
            // future `client` feature would open an HTTP-backed
            // impl here instead.
            Err::<Arc<dyn TypeManager>, _>(solx_surface::error::SolxError::Other(
                "no type manager backend compiled in (enable `local` or `client` feature)".to_string(),
            ))
        }
    })();
    let mgr = match open_result {
        Ok(m) => m,
        Err(e) => {
            let kind = e.kind_str();
            let detail = e.detail();
            return cx.throw_error(format!("{SOLX_ERROR_PREFIX}{kind}: {detail}"));
        }
    };
    Ok(cx.boxed(JsTypeManager { inner: mgr }))
}

/// Construct a `TypeManager` backed by a remote `solx-server`
/// over HTTP. Synchronous — building the `reqwest::Client` does
/// no I/O, unlike `open()`'s database setup.
fn connect(mut cx: FunctionContext) -> JsResult<JsBox<JsTypeManager>> {
    let server_url = cx.argument::<JsString>(0)?.value(&mut cx);
    let token = cx.argument::<JsString>(1)?.value(&mut cx);

    #[cfg(feature = "client")]
    {
        let remote = solx_client::RemoteTypeManager::new(server_url, token);
        Ok(cx.boxed(JsTypeManager {
            inner: Arc::new(remote) as Arc<dyn TypeManager>,
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

fn save(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsTypeManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let input_json = cx.argument::<JsString>(3)?.value(&mut cx);

    let input: solx_surface::entities::TypeInput = match serde_json::from_str(&input_json) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!(
                "{SOLX_ERROR_PREFIX}Invalid: bad input JSON: {e}"
            ));
        }
    };

    let inner = mgr.inner.clone();
    let path = path.clone();
    let name = name.clone();
    run_async(&mut cx, async move {
        TypeManager::save(&*inner, &path, &name, input).await
    })
}

fn get(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsTypeManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let inner = mgr.inner.clone();
    let path = path.clone();
    let name = name.clone();
    run_async(&mut cx, async move {
        TypeManager::get(&*inner, &path, &name).await
    })
}

fn delete(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsTypeManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let inner = mgr.inner.clone();
    let path = path.clone();
    let name = name.clone();
    run_async(&mut cx, async move {
        TypeManager::delete(&*inner, &path, &name).await
    })
}

fn list(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsTypeManager>>(0)?;
    let opts_json: String = cx
        .argument_opt(1)
        .and_then(|v| v.downcast::<JsString, _>(&mut cx).ok().map(|s| s.value(&mut cx)))
        .unwrap_or_else(|| "{}".to_string());
    let opts: ListOptions = match serde_json::from_str(&opts_json) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!(
                "{SOLX_ERROR_PREFIX}Invalid: bad options JSON: {e}"
            ));
        }
    };
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move { TypeManager::list(&*inner, opts).await })
}

fn resolve(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsTypeManager>>(0)?;
    let type_ref = cx.argument::<JsString>(1)?.value(&mut cx);
    let inner = mgr.inner.clone();
    let type_ref = type_ref.clone();
    run_async(&mut cx, async move {
        TypeManager::resolve(&*inner, &type_ref).await
    })
}

fn validate(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsTypeManager>>(0)?;
    let value_json = cx.argument::<JsString>(1)?.value(&mut cx);
    let type_ref = cx.argument::<JsString>(2)?.value(&mut cx);
    let value: serde_json::Value = match serde_json::from_str(&value_json) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!(
                "{SOLX_ERROR_PREFIX}Invalid: bad value JSON: {e}"
            ));
        }
    };
    let inner = mgr.inner.clone();
    let type_ref = type_ref.clone();
    run_async(&mut cx, async move {
        TypeManager::validate(&*inner, &value, &type_ref).await
    })
}

/// Register all type-manager functions on the parent module context.
pub fn register_types_module(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("typesOpen", open)?;
    cx.export_function("typesConnect", connect)?;
    cx.export_function("typesSave", save)?;
    cx.export_function("typesGet", get)?;
    cx.export_function("typesDelete", delete)?;
    cx.export_function("typesList", list)?;
    cx.export_function("typesResolve", resolve)?;
    cx.export_function("typesValidate", validate)?;
    Ok(())
}
