//! A spawned Command action is handed `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN`
//! by the manager's token authority, and `action_config.env` can't override
//! either one.

use std::sync::Arc;

use serde_json::{json, Value};
use solx_actions::LocalActionManager;
use solx_config::credentials::SingleTokenAuthority;
use solx_surface::auth::TokenAuthority;
use solx_surface::entities::{ActionInput, ActionType};
use solx_surface::managers::{ActionManager, DocManager, FileStore, TypeManager};

/// Prints the two injected variables plus an ordinary one, space-separated.
fn print_env() -> &'static str {
    if cfg!(windows) {
        "echo %SOLX_SERVER_TOKEN% %SOLX_SERVER_URL% %SOLX_TEST_OTHER%"
    } else {
        "echo \"$SOLX_SERVER_TOKEN $SOLX_SERVER_URL $SOLX_TEST_OTHER\""
    }
}

async fn setup() -> (
    tempfile::TempDir,
    Arc<solx_config::ConfigService>,
    Arc<LocalActionManager>,
) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Arc::new(solx_config::ConfigService::open_in(dir.path()).unwrap());
    let types: Arc<dyn TypeManager> = Arc::new(
        solx_types::LocalTypeManager::open(&dir.path().join("types.db"))
            .await
            .unwrap(),
    );
    let docs: Arc<dyn DocManager> = Arc::new(
        solx_docs::LocalDocManager::open(&dir.path().join("docs.db"), types.clone())
            .await
            .unwrap(),
    );
    let files: Arc<dyn FileStore> =
        Arc::new(solx_files::LocalFileStore::new(dir.path().join("files")));
    let actions = Arc::new(
        LocalActionManager::open(
            &dir.path().join("actions.db"),
            cfg.clone(),
            types,
            docs,
            files,
        )
        .await
        .unwrap(),
    );
    actions.set_self_ref(Arc::downgrade(&actions));
    (dir, cfg, actions)
}

async fn run_print_env(action_config: Option<Value>) -> Vec<String> {
    let (_d, cfg, actions) = setup().await;
    let authority = Arc::new(SingleTokenAuthority::new("issued-token"));
    authority.set_server_url("http://127.0.0.1:9".into());
    actions.set_token_authority(authority);

    cfg.register_command(
        "print-env",
        solx_config::CommandDef {
            command: print_env().into(),
            description: None,
            cwd: None,
        },
    )
    .unwrap();
    actions
        .save(
            "/t",
            "print-env",
            ActionInput {
                action_type: Some(ActionType::Command),
                fn_name: Some("print-env".into()),
                action_config,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let out = actions.exec("/t", "print-env", json!({})).await.unwrap();
    assert!(out.success);
    out.result
        .as_str()
        .expect("command output is plain text")
        .split_whitespace()
        .map(String::from)
        .collect()
}

#[tokio::test]
async fn command_receives_server_url_and_token() {
    let out = run_print_env(Some(json!({ "env": { "SOLX_TEST_OTHER": "kept" } }))).await;
    assert_eq!(out, ["issued-token", "http://127.0.0.1:9", "kept"]);
}

#[tokio::test]
async fn action_config_env_cannot_override_injected_vars() {
    let out = run_print_env(Some(json!({
        "env": {
            "SOLX_SERVER_TOKEN": "stale-token",
            "SOLX_SERVER_URL": "http://elsewhere",
            "SOLX_TEST_OTHER": "kept",
        }
    })))
    .await;
    assert_eq!(out, ["issued-token", "http://127.0.0.1:9", "kept"]);
}
