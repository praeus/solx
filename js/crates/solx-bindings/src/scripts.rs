//! Neon binding exposing `solx-scripts`' pipeline language, scoped to
//! action execution.
//!
//! There is no `ScriptRunner` trait in solx-core — the only seam is
//! `solx_scripts::CommandRunner`. solx-core's own narrow implementation
//! for `Script`-typed actions (`solx-actions/src/script.rs`,
//! `ActionCommandRunner`) is pinned to the concrete
//! `Arc<LocalActionManager>` (it uses `exec_as` for caller
//! attribution), so it can't be reused here. `TraitActionRunner` below
//! is the same two-verb grammar (`exec <path/name> [--json '<params>']`
//! and `json <literal>`) built on `Arc<dyn ActionManager>` instead, so
//! it works identically whether the actions handle is local or a
//! `solx-client` remote proxy.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use neon::prelude::*;
use serde_json::Value;
use solx_scripts::CommandRunner;
use solx_surface::error::{Result, SolxError};
use solx_surface::managers::ActionManager;
use solx_surface::path::split_ref;

use crate::actions::JsActionManager;
use crate::async_runtime::run_async;

struct TraitActionRunner {
    actions: Arc<dyn ActionManager>,
}

#[async_trait]
impl CommandRunner for TraitActionRunner {
    async fn run(&self, tokens: Vec<String>, piped: Option<Value>) -> Result<Value> {
        match tokens.first().map(String::as_str) {
            Some("exec") => self.run_exec(&tokens, piped).await,
            Some("json") => run_json(&tokens),
            Some(other) => Err(SolxError::Invalid(format!(
                "unsupported script stage '{other}' (scripts only support \
                 'exec <path/name> [--json '<params>']' and 'json <value>')"
            ))),
            None => Ok(Value::Null),
        }
    }
}

impl TraitActionRunner {
    async fn run_exec(&self, tokens: &[String], piped: Option<Value>) -> Result<Value> {
        let (reference, json) = parse_exec_stage(tokens)?;
        let params = match json {
            Some(j) => serde_json::from_str(&j)
                .map_err(|e| SolxError::Invalid(format!("parse --json params: {e}")))?,
            None => piped.unwrap_or_else(|| Value::Object(Default::default())),
        };
        let (path, name) = split_ref(&reference)?;
        let result = self.actions.exec(&path, &name, params).await?;
        serde_json::to_value(&result)
            .map_err(|e| SolxError::Invalid(format!("serialize exec result: {e}")))
    }
}

fn parse_exec_stage(tokens: &[String]) -> Result<(String, Option<String>)> {
    let mut reference: Option<String> = None;
    let mut json: Option<String> = None;
    let mut i = 1;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--json" | "-j" => {
                i += 1;
                let value = tokens
                    .get(i)
                    .ok_or_else(|| SolxError::Invalid("exec: '--json' requires a value".into()))?;
                json = Some(value.clone());
            }
            other => {
                if reference.is_some() {
                    return Err(SolxError::Invalid(format!(
                        "exec: unexpected argument '{other}'"
                    )));
                }
                reference = Some(other.to_string());
            }
        }
        i += 1;
    }
    let reference = reference.ok_or_else(|| {
        SolxError::Invalid("exec requires an action reference, e.g. 'exec /pkg/name'".into())
    })?;
    Ok((reference, json))
}

fn run_json(tokens: &[String]) -> Result<Value> {
    if tokens.len() != 2 {
        return Err(SolxError::Invalid(
            "json takes exactly one argument, e.g. 'json \"hello\"' or 'json 5'".into(),
        ));
    }
    serde_json::from_str(&tokens[1])
        .map_err(|e| SolxError::Invalid(format!("parse json value: {e}")))
}

fn run(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsActionManager>>(0)?;
    let source = cx.argument::<JsString>(1)?.value(&mut cx);
    let params: Value = match cx
        .argument_opt(2)
        .and_then(|v| v.downcast::<JsString, _>(&mut cx).ok().map(|s| s.value(&mut cx)))
    {
        Some(json) => match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => {
                return cx.throw_error(format!(
                    "{}Invalid: bad params JSON: {e}",
                    crate::async_runtime::SOLX_ERROR_PREFIX
                ));
            }
        },
        None => Value::Null,
    };

    let actions = mgr.inner.clone();
    run_async(&mut cx, async move {
        let runner = TraitActionRunner { actions };
        let initial = HashMap::from([("params".to_string(), params)]);
        solx_scripts::execute_script_with_vars(&runner, &source, initial).await
    })
}

/// Register all script-runner functions on the parent module context.
pub fn register_scripts_module(cx: &mut ModuleContext) -> NeonResult<()> {
    cx.export_function("scriptsRun", run)?;
    Ok(())
}
