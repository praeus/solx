//! A test-only internal handler that reports what an `InternalCallCtx`
//! carries — the principal, the caller, the fail-closed
//! `external_principal()` verdict, and the exclusion mode — so tests can check that each entry point
//! (REST, `/mcp`, nested frames, detached `action-start` runs) delivers them.
//!
//! Compiled only under `cfg(test)` or the `test-probe` feature, which
//! dependents enable from `[dev-dependencies]` alone, and seeded at
//! [`PATH`]/[`NAME`] only then. It has to be seeded: `save` refuses to
//! create an `Internal` row, and dispatch runs only seeded ones.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use solx_surface::internal_actions::{
    InternalActionHandler, InternalActionRegistry, InternalCallCtx, SeedAction,
};
use solx_surface::Principal;

pub const FN_NAME: &str = "test-principal-probe";
/// Where the probe is seeded.
pub const PATH: &str = "/builtin/test";
pub const NAME: &str = "probe";

/// Stable string form of a principal, for assertions.
pub fn describe(principal: &Principal) -> String {
    match principal {
        Principal::Root => "root".into(),
        Principal::Client { id, .. } => format!("client:{id}"),
        Principal::Action { action_ref, .. } => format!("action:{action_ref}"),
    }
}

struct Probe;

#[async_trait]
impl InternalActionHandler for Probe {
    async fn call(
        &self,
        _params: &Value,
        ctx: &InternalCallCtx,
    ) -> std::result::Result<Value, String> {
        Ok(json!({
            "principal": ctx.exec.principal().map(describe),
            "caller": ctx.exec.caller().map(|c| c.action_ref().to_string()),
            "external": ctx.external_principal().map(describe),
            "excludeMode": ctx.exec.exclude_mode(),
        }))
    }
}

pub(crate) fn seeds() -> Vec<SeedAction> {
    vec![SeedAction {
        path: PATH,
        name: NAME,
        fn_name: FN_NAME,
        description: "Test only: report the principal, caller and exclusion mode.",
        param_type: None,
    }]
}

pub(crate) fn plugin() -> InternalActionRegistry {
    let mut registry = InternalActionRegistry::new();
    registry.register(&[FN_NAME], Arc::new(Probe));
    registry
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use serial_test::serial;
    use solx_surface::entities::{ActionInput, ActionType};
    use solx_surface::managers::ActionManager;
    use solx_surface::{ExecContext, Principal};

    use super::{NAME, PATH};
    use crate::caller::Caller;
    use crate::set_long_lived_host;
    use crate::test_support::setup_wired;
    use crate::LocalActionManager;

    async fn save_script(m: &LocalActionManager, name: &str, source: &str) {
        save_script_as(m, name, source, vec![]).await;
    }

    async fn save_script_as(
        m: &LocalActionManager,
        name: &str,
        source: &str,
        capabilities: Vec<String>,
    ) {
        let bin = format!("{name}.solx");
        m.files
            .put(
                &solx_files::shared_action_file_path(&bin),
                source.as_bytes().to_vec(),
            )
            .await
            .unwrap();
        m.save(
            "/t",
            name,
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some(bin),
                capabilities,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn exec_for_delivers_the_principal_and_exec_delivers_none() {
        let (_d, _cfg, m) = setup_wired().await;

        let authed = m
            .exec_for(&Principal::Root, PATH, NAME, json!({}))
            .await
            .unwrap();
        assert_eq!(
            authed.result,
            json!({"principal": "root", "caller": null, "external": "root", "excludeMode": "none"})
        );

        let local = m.exec(PATH, NAME, json!({})).await.unwrap();
        assert_eq!(
            local.result,
            json!({"principal": null, "caller": null, "external": null, "excludeMode": "none"})
        );
    }

    /// A nested frame inherits the principal for attribution, but the
    /// caller it now has makes `external_principal()` refuse it.
    #[tokio::test]
    async fn nested_script_exec_inherits_the_principal_but_is_not_external() {
        let (_d, _cfg, m) = setup_wired().await;
        save_script(&m, "outer", "exec /builtin/test/probe").await;

        let ran = m
            .exec_for(&Principal::Root, "/t", "outer", json!({}))
            .await
            .unwrap();
        assert_eq!(
            ran.result["result"],
            json!({"principal": "root", "caller": "/t/outer", "external": null, "excludeMode": "none"})
        );
    }

    /// The `tokio::spawn` in `start_invocation` must carry both values:
    /// losing the caller would turn an action's detached run into an
    /// apparently external root call.
    #[tokio::test]
    #[serial]
    async fn detached_runs_keep_principal_and_caller() {
        set_long_lived_host(true);
        let (_d, _cfg, m) = setup_wired().await;

        let poll_result = |started: serde_json::Value| {
            let m = m.clone();
            async move {
                let id = started["invocation_id"].as_str().unwrap().to_string();
                let polled = m.poll_invocation(&id, Some(10)).await.unwrap();
                assert_eq!(polled["status"], solx_console::invocations::status::OK);
                polled["result"].clone()
            }
        };

        let direct = m
            .start_invocation(
                PATH,
                NAME,
                json!({}),
                ExecContext::for_principal(Principal::Root).info(),
            )
            .await
            .unwrap();
        assert_eq!(
            poll_result(direct).await,
            json!({"principal": "root", "caller": null, "external": "root", "excludeMode": "none"})
        );

        let from_action = m
            .start_invocation(
                PATH,
                NAME,
                json!({}),
                ExecContext::for_principal(Principal::Root)
                    .enter(Caller::from_action("/t/outer", None))
                    .info(),
            )
            .await
            .unwrap();
        assert_eq!(
            poll_result(from_action).await,
            json!({"principal": "root", "caller": "/t/outer", "external": null, "excludeMode": "none"})
        );

        // End to end through the `action-start` builtin, from a script.
        save_script(
            &m,
            "starter",
            r#"exec /builtin/action/start --json '{"path": "/builtin/test", "name": "probe"}'"#,
        )
        .await;
        let ran = m
            .exec_for(&Principal::Root, "/t", "starter", json!({}))
            .await
            .unwrap();
        assert_eq!(
            poll_result(ran.result["result"].clone()).await,
            json!({"principal": "root", "caller": "/t/starter", "external": null, "excludeMode": "none"})
        );

        // The tooling mode crosses the same `tokio::spawn`.
        let ran = m
            .exec_tooling(None, "/t", "starter", json!({}))
            .await
            .unwrap();
        assert_eq!(
            poll_result(ran.result["result"].clone()).await,
            json!({"principal": null, "caller": "/t/starter", "external": null, "excludeMode": "tooling"})
        );

        set_long_lived_host(false);
    }

    /// `exec_tooling` holds the whole chain, not just its first frame.
    #[tokio::test]
    async fn the_tooling_scope_reaches_nested_frames() {
        let (_d, _cfg, m) = setup_wired().await;
        save_script(&m, "outer", "exec /builtin/test/probe").await;

        let ran = m
            .exec_tooling(Some(&Principal::Root), "/t", "outer", json!({}))
            .await
            .unwrap();
        assert_eq!(
            ran.result["result"],
            json!({"principal": "root", "caller": "/t/outer", "external": null, "excludeMode": "tooling"})
        );
    }

    /// A model allowed to call a script can't use it to reach an action
    /// excluded from tooling, and the refusal reads like a missing action.
    #[tokio::test]
    #[serial]
    async fn excluded_actions_are_refused_at_every_hop_in_the_tooling_scope() {
        let (_d, _cfg, m) = setup_wired().await;
        save_script_as(
            &m,
            "hidden",
            "exec /builtin/test/probe",
            vec![solx_config::CAP_HIDDEN.into()],
        )
        .await;
        save_script(&m, "outer", "exec /t/hidden").await;

        let direct = m
            .exec_tooling(None, "/t", "hidden", json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(direct, solx_surface::SolxError::NotFound(_)),
            "{direct:?}"
        );

        let nested = m
            .exec_tooling(None, "/t", "outer", json!({}))
            .await
            .unwrap_err();
        assert!(nested.to_string().contains("/t/hidden"), "{nested}");

        set_long_lived_host(true);
        let detached = m
            .start_invocation("/t", "hidden", json!({}), ExecContext::tooling(None).info())
            .await
            .unwrap_err();
        assert!(
            matches!(detached, solx_surface::SolxError::NotFound(_)),
            "{detached:?}"
        );
        set_long_lived_host(false);

        // Outside the tooling scope the same chain runs.
        let ran = m.exec("/t", "outer", json!({})).await.unwrap();
        assert_eq!(ran.result["result"]["result"]["caller"], json!("/t/hidden"));
    }
}
