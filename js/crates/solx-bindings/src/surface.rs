//! `solx-surface` re-exports + path/normalize helpers exposed to JS.

use neon::prelude::*;

const SOLX_ERROR_PREFIX: &str = "SolxError::";

fn validate_path(mut cx: FunctionContext) -> JsResult<JsString> {
    let path = cx.argument::<JsString>(0)?.value(&mut cx);
    match solx_surface::path::normalize_path(&path) {
        Ok(normalized) => Ok(cx.string(normalized)),
        Err(e) => {
            let kind = match e {
                solx_surface::error::SolxError::Invalid(_) => "Invalid",
                _ => "Other",
            };
            let detail = match e {
                solx_surface::error::SolxError::Invalid(s) => s,
                _ => format!("{e:?}"),
            };
            cx.throw_error(format!("{SOLX_ERROR_PREFIX}{kind}: {detail}"))
        }
    }
}

fn validate_name(mut cx: FunctionContext) -> JsResult<JsString> {
    let name = cx.argument::<JsString>(0)?.value(&mut cx);
    match solx_surface::path::validate_name(&name) {
        Ok(()) => Ok(cx.string(name)),
        Err(e) => cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: {e:?}")),
    }
}

fn full_ref(mut cx: FunctionContext) -> JsResult<JsString> {
    let path = cx.argument::<JsString>(0)?.value(&mut cx);
    let name = cx.argument::<JsString>(1)?.value(&mut cx);
    match solx_surface::path::full_ref(&path, &name) {
        Ok(r) => Ok(cx.string(r)),
        Err(e) => cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: {e:?}")),
    }
}

fn split_ref(mut cx: FunctionContext) -> JsResult<JsObject> {
    let full = cx.argument::<JsString>(0)?.value(&mut cx);
    let (path, name) = match solx_surface::path::split_ref(&full) {
        Ok(p) => p,
        Err(e) => {
            return cx.throw_error(format!("{SOLX_ERROR_PREFIX}Invalid: {e:?}"));
        }
    };
    let obj = cx.empty_object();
    let path_handle = cx.string(path);
    let name_handle = cx.string(name);
    obj.set(&mut cx, "path", path_handle)?;
    obj.set(&mut cx, "name", name_handle)?;
    Ok(obj)
}

pub fn register_surface_module(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("surfaceValidatePath", validate_path)?;
    cx.export_function("surfaceValidateName", validate_name)?;
    cx.export_function("surfaceFullRef", full_ref)?;
    cx.export_function("surfaceSplitRef", split_ref)?;
    Ok(())
}
