//! Neon binding for `solx-config::ConfigService`.
//!
//! `ConfigService` is **synchronous** (unlike the other
//! `Local*` managers), so the binding functions are sync too
//! — no `run_async` needed. Returns are JSON-encoded so the
//! wire format stays uniform.
//!
//! Each exported function prepends a `config` prefix so they
//! share the same `solx.node` namespace as the other managers.
//!
//! **Trait seam (TODO):** unlike `types.rs`/`files.rs`/`docs.rs`/
//! `actions.rs`, this binding holds the concrete `ConfigService`
//! because `solx_config` does not yet expose a `ConfigService`
//! trait in `solx_surface::managers` — and, per the local/client
//! design (see `docs/design-and-progress.md`), it never will be
//! proxied even once other managers can be: `solx-manager` itself
//! always uses the local `ConfigService` regardless of whether the
//! other managers are wired local or remote, because config is what
//! *tells* them which mode to use. So unlike the other bindings,
//! `ConfigService` construction is **not** gated by the `local`/
//! `client` features — it's unconditionally available.

use std::path::PathBuf;
use std::sync::Arc;

use neon::prelude::*;
use solx_config::ConfigService;
use solx_config::InstalledPackage;

use crate::async_runtime::{SolxErrorExt, SOLX_ERROR_PREFIX};

pub struct JsConfigService {
    /// Concrete until `solx_surface::managers::ConfigService`
    /// trait exists — see module docs. `pub(crate)` so
    /// `actions.rs` can pass `Arc<ConfigService>` into
    /// `LocalActionManager::open`, which needs it directly
    /// (env-mapping lookups, derived paths).
    pub(crate) inner: Arc<ConfigService>,
}

impl Finalize for JsConfigService {}

fn path_to_str(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Throw a `JsError` carrying the given solx-formatted message
/// and unify with the surrounding function's return type `T`.
/// Used at tail-position as `return throw_str::<JsValue, _>(cx, msg);`
/// or chained with `?`.
fn throw_str<'a, T: Value, C: Context<'a>>(cx: &mut C, msg: String) -> JsResult<'a, T> {
    cx.throw_error(msg)
}

/// Get a string argument by index, throwing if it's absent or
/// wrong type. Returns a `Result<String, String>` so callers can
/// convert with `throw_str` at tail-position.
fn get_string_arg(cx: &mut FunctionContext, idx: usize, name: &str) -> Result<String, String> {
    cx.argument::<JsString>(idx)
        .map(|s| s.value(cx))
        .map_err(|e| format!("{SOLX_ERROR_PREFIX}Invalid: {name} must be a string: {e:?}"))
}

/// Convert a `Result<T, solx_config::error::SolxError>` to a
/// `JsResult<JsBox<JsConfigService>>` by formatting the error
/// in solx prefix form. The caller provides the boxed JS handle
/// to use on success via a closure.
fn or_solx<'a, T, E: SolxErrorExt, F, C>(
    cx: &mut C,
    r: Result<T, E>,
    ok: F,
) -> JsResult<'a, JsBox<JsConfigService>>
where
    C: Context<'a>,
    F: FnOnce(&mut C, T) -> JsResult<'a, JsBox<JsConfigService>>,
{
    match r {
        Ok(v) => ok(cx, v),
        Err(e) => {
            let kind = e.kind_str();
            let detail = e.detail();
            throw_str(cx, format!("{SOLX_ERROR_PREFIX}{kind}: {detail}"))
        }
    }
}

fn encode<'a, C: Context<'a>>(cx: &mut C, v: &impl serde::Serialize) -> JsResult<'a, JsString> {
    match serde_json::to_string(v) {
        Ok(s) => Ok(cx.string(s)),
        Err(e) => throw_str(cx, format!("{SOLX_ERROR_PREFIX}Other: serialize failed: {e}")),
    }
}

fn open(mut cx: FunctionContext) -> JsResult<JsBox<JsConfigService>> {
    let appdata_arg = cx.argument_opt(0);
    let appdata_path: Option<PathBuf> = if let Some(v) = appdata_arg {
        if v.is_a::<JsString, _>(&mut cx) {
            let js_str = match v.downcast::<JsString, _>(&mut cx) {
                Ok(s) => s,
                Err(e) => return throw_str::<JsBox<JsConfigService>, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: {e:?}")),
            };
            Some(PathBuf::from(js_str.value(&mut cx)))
        } else if v.is_a::<JsUndefined, _>(&mut cx) || v.is_a::<JsNull, _>(&mut cx) {
            None
        } else {
            return throw_str::<JsBox<JsConfigService>, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: appdata must be a string"));
        }
    } else {
        None
    };

    // Unconditional (not feature-gated) — see module docs: config
    // is always local, in both local and client modes.
    match appdata_path {
        Some(p) => or_solx(&mut cx, ConfigService::open_in(p), |cx, svc| {
            Ok(cx.boxed(JsConfigService { inner: Arc::new(svc) }))
        }),
        None => or_solx(&mut cx, ConfigService::open(), |cx, svc| {
            Ok(cx.boxed(JsConfigService { inner: Arc::new(svc) }))
        }),
    }
}

fn appdata(mut cx: FunctionContext) -> JsResult<JsString> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    Ok(cx.string(path_to_str(mgr.inner.appdata())))
}

fn config_path(mut cx: FunctionContext) -> JsResult<JsString> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    Ok(cx.string(path_to_str(mgr.inner.config_path())))
}

fn raw_snapshot(mut cx: FunctionContext) -> JsResult<JsString> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    encode(&mut cx, &mgr.inner.raw_snapshot())
}

fn snapshot(mut cx: FunctionContext) -> JsResult<JsString> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    encode(&mut cx, &mgr.inner.snapshot())
}

fn get(mut cx: FunctionContext) -> JsResult<JsValue> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    let key = match get_string_arg(&mut cx, 1, "key") {
        Ok(s) => s,
        Err(e) => return throw_str::<JsValue, _>(&mut cx, e),
    };
    match mgr.inner.get(&key) {
        Some(v) => {
            let s = encode(&mut cx, &v)?;
            Ok(s.upcast::<JsValue>())
        }
        None => Ok(cx.undefined().upcast::<JsValue>()),
    }
}

fn set(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    let key = match get_string_arg(&mut cx, 1, "key") {
        Ok(s) => s,
        Err(e) => return throw_str::<JsUndefined, _>(&mut cx, e),
    };
    let value_json = match cx.argument::<JsString>(2) {
        Ok(s) => s.value(&mut cx),
        Err(e) => return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: value must be a JSON string: {e:?}")),
    };
    let value: serde_json::Value = match serde_json::from_str(&value_json) {
        Ok(v) => v,
        Err(e) => return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: bad value JSON: {e}")),
    };
    if let Err(e) = mgr.inner.set(&key, value) {
        return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}{}: {}", e.kind_str(), e.detail()));
    }
    Ok(cx.undefined())
}

fn patch(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    let patch_json = match cx.argument::<JsString>(1) {
        Ok(s) => s.value(&mut cx),
        Err(e) => return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: patch must be a JSON string: {e:?}")),
    };
    let patch: serde_json::Value = match serde_json::from_str(&patch_json) {
        Ok(v) => v,
        Err(e) => return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: bad patch JSON: {e}")),
    };
    if let Err(e) = mgr.inner.patch(patch) {
        return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}{}: {}", e.kind_str(), e.detail()));
    }
    Ok(cx.undefined())
}

fn path_accessor(mut cx: FunctionContext) -> JsResult<JsString> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    let which = match cx.argument::<JsString>(1) {
        Ok(s) => s.value(&mut cx),
        Err(e) => return throw_str::<JsString, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: which must be a string: {e:?}")),
    };
    let path = match which.as_str() {
        "dbDir" => mgr.inner.db_dir(),
        "docsDbPath" => mgr.inner.docs_db_path(),
        "actionsDbPath" => mgr.inner.actions_db_path(),
        "typesDbPath" => mgr.inner.types_db_path(),
        "filesDir" => mgr.inner.files_dir(),
        "searchIndexDir" => mgr.inner.search_index_dir(),
        "logsDir" => mgr.inner.logs_dir(),
        _ => {
            return throw_str::<JsString, _>(
                &mut cx,
                format!("{SOLX_ERROR_PREFIX}Invalid: unknown path accessor '{which}'"),
            );
        }
    };
    Ok(cx.string(path_to_str(&path)))
}

fn list_packages(mut cx: FunctionContext) -> JsResult<JsString> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    encode(&mut cx, &mgr.inner.list_packages())
}

fn register_package(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    let pkg_json = match cx.argument::<JsString>(1) {
        Ok(s) => s.value(&mut cx),
        Err(e) => return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: package must be a JSON string: {e:?}")),
    };
    let pkg: InstalledPackage = match serde_json::from_str(&pkg_json) {
        Ok(v) => v,
        Err(e) => return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}Invalid: bad package JSON: {e}")),
    };
    if let Err(e) = mgr.inner.register_package(pkg) {
        return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}{}: {}", e.kind_str(), e.detail()));
    }
    Ok(cx.undefined())
}

fn unregister_package(mut cx: FunctionContext) -> JsResult<JsUndefined> {
    let mgr = cx.argument::<JsBox<JsConfigService>>(0)?;
    let name = match get_string_arg(&mut cx, 1, "name") {
        Ok(s) => s,
        Err(e) => return throw_str::<JsUndefined, _>(&mut cx, e),
    };
    if let Err(e) = mgr.inner.unregister_package(&name) {
        return throw_str::<JsUndefined, _>(&mut cx, format!("{SOLX_ERROR_PREFIX}{}: {}", e.kind_str(), e.detail()));
    }
    Ok(cx.undefined())
}

/// Register all config-service functions on the parent module context.
pub fn register_config_module(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("configOpen", open)?;
    cx.export_function("configAppdata", appdata)?;
    cx.export_function("configConfigPath", config_path)?;
    cx.export_function("configRawSnapshot", raw_snapshot)?;
    cx.export_function("configSnapshot", snapshot)?;
    cx.export_function("configGet", get)?;
    cx.export_function("configSet", set)?;
    cx.export_function("configPatch", patch)?;
    cx.export_function("configPath", path_accessor)?;
    cx.export_function("configListPackages", list_packages)?;
    cx.export_function("configRegisterPackage", register_package)?;
    cx.export_function("configUnregisterPackage", unregister_package)?;
    Ok(())
}
