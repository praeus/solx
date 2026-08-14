//! Neon binding for the `DocManager` trait.
//!
//! Mirrors `types.rs` exactly in shape (`Arc<dyn DocManager>`,
//! `run_async` for every I/O method). The only wrinkle is
//! construction: local mode needs a `TypeManager` handle (documents
//! are validated against their type across databases) in addition to
//! the db/index paths, while client mode only needs a server URL and
//! token — `solx-server` already knows how to wire its own local
//! `TypeManager` internally.

use std::path::PathBuf;
use std::sync::Arc;

use neon::prelude::*;
use solx_surface::entities::DocumentInput;
use solx_surface::managers::DocManager;
use solx_surface::query::{ListOptions, SearchQuery};
#[cfg(feature = "local")]
use solx_docs::LocalDocManager;

use crate::async_runtime::{run_async, SolxErrorExt, SOLX_ERROR_PREFIX};
use crate::types::JsTypeManager;

pub struct JsDocManager {
    pub(crate) inner: Arc<dyn DocManager>,
}

impl Finalize for JsDocManager {}

fn open(mut cx: FunctionContext) -> JsResult<JsBox<JsDocManager>> {
    let db_path = cx.argument::<JsString>(0)?.value(&mut cx);
    let index_dir = cx.argument::<JsString>(1)?.value(&mut cx);
    let types = cx.argument::<JsBox<JsTypeManager>>(2)?;
    let types_inner = types.inner.clone();

    let db_path_buf = PathBuf::from(db_path);
    let index_dir_buf = PathBuf::from(index_dir);

    let open_result: solx_surface::error::Result<Arc<dyn DocManager>> = (|| {
        #[cfg(feature = "local")]
        {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| solx_surface::error::SolxError::Other(e.to_string()))?;
            let local =
                rt.block_on(LocalDocManager::open(&db_path_buf, &index_dir_buf, types_inner))?;
            Ok(Arc::new(local) as Arc<dyn DocManager>)
        }
        #[cfg(not(feature = "local"))]
        {
            let _ = (db_path_buf, index_dir_buf, types_inner);
            Err::<Arc<dyn DocManager>, _>(solx_surface::error::SolxError::Other(
                "no doc manager backend compiled in (enable `local` or `client` feature)"
                    .to_string(),
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
    Ok(cx.boxed(JsDocManager { inner: mgr }))
}

fn connect(mut cx: FunctionContext) -> JsResult<JsBox<JsDocManager>> {
    let server_url = cx.argument::<JsString>(0)?.value(&mut cx);
    let token = cx.argument::<JsString>(1)?.value(&mut cx);

    #[cfg(feature = "client")]
    {
        let remote = solx_client::RemoteDocManager::new(server_url, token);
        Ok(cx.boxed(JsDocManager {
            inner: Arc::new(remote) as Arc<dyn DocManager>,
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
    let mgr = cx.argument::<JsBox<JsDocManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let input_json = cx.argument::<JsString>(3)?.value(&mut cx);

    let input: DocumentInput = match serde_json::from_str(&input_json) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: bad input JSON: {e}"));
        }
    };

    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        DocManager::save(&*inner, &path, &name, input).await
    })
}

fn get(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsDocManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        DocManager::get(&*inner, &path, &name).await
    })
}

fn delete(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsDocManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        DocManager::delete(&*inner, &path, &name).await
    })
}

fn list(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsDocManager>>(0)?;
    let opts_json: String = cx
        .argument_opt(1)
        .and_then(|v| v.downcast::<JsString, _>(&mut cx).ok().map(|s| s.value(&mut cx)))
        .unwrap_or_else(|| "{}".to_string());
    let opts: ListOptions = match serde_json::from_str(&opts_json) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: bad options JSON: {e}"));
        }
    };
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move { DocManager::list(&*inner, opts).await })
}

fn search(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsDocManager>>(0)?;
    let query_json = cx.argument::<JsString>(1)?.value(&mut cx);
    let query: SearchQuery = match serde_json::from_str(&query_json) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: bad query JSON: {e}"));
        }
    };
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        DocManager::search(&*inner, query).await
    })
}

/// Register all doc-manager functions on the parent module context.
pub fn register_docs_module(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("docsOpen", open)?;
    cx.export_function("docsConnect", connect)?;
    cx.export_function("docsSave", save)?;
    cx.export_function("docsGet", get)?;
    cx.export_function("docsDelete", delete)?;
    cx.export_function("docsList", list)?;
    cx.export_function("docsSearch", search)?;
    Ok(())
}
