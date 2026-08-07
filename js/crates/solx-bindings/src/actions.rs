//! Neon binding for the `ActionManager` trait.
//!
//! Local construction is the most involved `open()` in this crate: it
//! mirrors `solx-manager::App::wire_local`'s actions step exactly —
//! seed the env-mappings allowlist, open `LocalActionManager` with its
//! four collaborators (config/types/docs/files), then wire its
//! `self_ref` (a `Weak` back-reference `LocalActionManager` uses to
//! recurse into its own `exec` for internal/wasm actions that call
//! other actions) before erasing to the trait object. `exec` itself
//! dispatches through the plain `ActionManager::exec` trait method, so
//! it's identical for local and remote handles — no cfg branch needed.

use std::path::PathBuf;
use std::sync::Arc;

use neon::prelude::*;
use serde_json::Value;
use solx_surface::entities::ActionInput;
use solx_surface::managers::ActionManager;
use solx_surface::query::ListOptions;
#[cfg(feature = "local")]
use solx_actions::LocalActionManager;

use crate::async_runtime::{run_async, SolxErrorExt, SOLX_ERROR_PREFIX};
use crate::config::JsConfigService;
use crate::docs::JsDocManager;
use crate::files::JsFileStore;
use crate::types::JsTypeManager;

pub struct JsActionManager {
    pub(crate) inner: Arc<dyn ActionManager>,
}

impl Finalize for JsActionManager {}

fn open(mut cx: FunctionContext) -> JsResult<JsBox<JsActionManager>> {
    let db_path = cx.argument::<JsString>(0)?.value(&mut cx);
    let config = cx.argument::<JsBox<JsConfigService>>(1)?;
    let types = cx.argument::<JsBox<JsTypeManager>>(2)?;
    let docs = cx.argument::<JsBox<JsDocManager>>(3)?;
    let files = cx.argument::<JsBox<JsFileStore>>(4)?;

    let config_inner = config.inner.clone();
    let types_inner = types.inner.clone();
    let docs_inner = docs.inner.clone();
    let files_inner = files.inner.clone();
    let db_path_buf = PathBuf::from(db_path);

    let open_result: solx_surface::error::Result<Arc<dyn ActionManager>> = (|| {
        #[cfg(feature = "local")]
        {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| solx_surface::error::SolxError::Other(e.to_string()))?;
            rt.block_on(async {
                solx_actions::internal::init_env_mappings(
                    config_inner.snapshot().env_mappings.unwrap_or_default(),
                );
                let concrete = Arc::new(
                    LocalActionManager::open(
                        &db_path_buf,
                        config_inner,
                        types_inner,
                        docs_inner,
                        files_inner,
                    )
                    .await?,
                );
                concrete.set_self_ref(Arc::downgrade(&concrete));
                Ok(concrete as Arc<dyn ActionManager>)
            })
        }
        #[cfg(not(feature = "local"))]
        {
            let _ = (config_inner, types_inner, docs_inner, files_inner, db_path_buf);
            Err::<Arc<dyn ActionManager>, _>(solx_surface::error::SolxError::Other(
                "no action manager backend compiled in (enable `local` or `client` feature)"
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
    Ok(cx.boxed(JsActionManager { inner: mgr }))
}

fn connect(mut cx: FunctionContext) -> JsResult<JsBox<JsActionManager>> {
    let server_url = cx.argument::<JsString>(0)?.value(&mut cx);
    let token = cx.argument::<JsString>(1)?.value(&mut cx);

    #[cfg(feature = "client")]
    {
        let remote = solx_client::RemoteActionManager::new(server_url, token);
        Ok(cx.boxed(JsActionManager {
            inner: Arc::new(remote) as Arc<dyn ActionManager>,
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

fn post(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsActionManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let input_json = cx.argument::<JsString>(3)?.value(&mut cx);

    let input: ActionInput = match serde_json::from_str(&input_json) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: bad input JSON: {e}"));
        }
    };

    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        ActionManager::post(&*inner, &path, &name, input).await
    })
}

fn get(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsActionManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        ActionManager::get(&*inner, &path, &name).await
    })
}

fn delete(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsActionManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        ActionManager::delete(&*inner, &path, &name).await
    })
}

fn list(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsActionManager>>(0)?;
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
    run_async(&mut cx, async move { ActionManager::list(&*inner, opts).await })
}

fn exec(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsActionManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let params_json: String = cx
        .argument_opt(3)
        .and_then(|v| v.downcast::<JsString, _>(&mut cx).ok().map(|s| s.value(&mut cx)))
        .unwrap_or_else(|| "{}".to_string());
    let params: Value = match serde_json::from_str(&params_json) {
        Ok(v) => v,
        Err(e) => {
            return cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: bad params JSON: {e}"));
        }
    };
    let inner = mgr.inner.clone();
    run_async(&mut cx, async move {
        ActionManager::exec(&*inner, &path, &name, params).await
    })
}

/// Register all action-manager functions on the parent module context.
pub fn register_actions_module(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("actionsOpen", open)?;
    cx.export_function("actionsConnect", connect)?;
    cx.export_function("actionsPost", post)?;
    cx.export_function("actionsGet", get)?;
    cx.export_function("actionsDelete", delete)?;
    cx.export_function("actionsList", list)?;
    cx.export_function("actionsExec", exec)?;
    Ok(())
}
