//! A spawned Command action is handed `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN`
//! by the manager's token authority, and its own `SOLX_ACTION_REF`. The removed `action_config.env` map is
//! ignored, so it can neither override them nor add variables of its own.

use std::sync::Arc;

use serde_json::{json, Value};
use solx_actions::LocalActionManager;
use solx_config::credentials::SingleTokenAuthority;
use solx_surface::auth::TokenAuthority;
use solx_surface::entities::{ActionInput, ActionType};
use solx_surface::managers::{ActionManager, DocManager, FileStore, TypeManager};

/// Prints the three injected variables plus an ordinary one, space-separated.
fn print_env() -> &'static str {
    if cfg!(windows) {
        "echo %SOLX_SERVER_TOKEN% %SOLX_SERVER_URL% %SOLX_ACTION_REF% %SOLX_TEST_OTHER%"
    } else {
        "echo \"$SOLX_SERVER_TOKEN $SOLX_SERVER_URL $SOLX_ACTION_REF $SOLX_TEST_OTHER\""
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
    let out = run_print_env(None).await;
    assert_eq!(out[..2], ["issued-token", "http://127.0.0.1:9"]);
}

#[tokio::test]
async fn command_receives_its_own_action_ref() {
    let out = run_print_env(None).await;
    assert_eq!(out[2], "/t/print-env");
}

#[tokio::test]
async fn action_config_env_is_ignored() {
    let out = run_print_env(Some(json!({
        "env": {
            "SOLX_SERVER_TOKEN": "stale-token",
            "SOLX_SERVER_URL": "http://elsewhere",
            "SOLX_ACTION_REF": "/someone/else",
            "SOLX_TEST_OTHER": "not-forwarded",
        }
    })))
    .await;
    assert_eq!(out[..3], ["issued-token", "http://127.0.0.1:9", "/t/print-env"]);
    assert!(!out.iter().any(|v| v == "not-forwarded"), "{out:?}");
}

// ── per-invocation tokens from the token registry ───────────────────────────

/// Writes the child's token to [`TOKEN_FILE`] in its working directory
/// (the appdata dir), then runs for about `secs` seconds and exits with
/// `exit_code`. No quotes: `cmd /C` doesn't read Rust's escaped ones.
fn write_token_then_wait(secs: u32, exit_code: u8) -> String {
    if cfg!(windows) {
        format!(
            "echo %SOLX_SERVER_TOKEN%>{TOKEN_FILE}&& ping -n {} 127.0.0.1 >nul && exit /b {exit_code}",
            secs + 1
        )
    } else {
        format!("printf %s \"$SOLX_SERVER_TOKEN\" > {TOKEN_FILE}; sleep {secs}; exit {exit_code}")
    }
}

const TOKEN_FILE: &str = "token.txt";

struct RegistrySetup {
    _dir: tempfile::TempDir,
    actions: Arc<LocalActionManager>,
    registry: Arc<solx_registry::Registry>,
    token_file: std::path::PathBuf,
}

async fn registry_setup(secs: u32, exit_code: u8, timeout_secs: Option<u64>) -> RegistrySetup {
    solx_config::credentials::use_memory_store_for_tests();
    let (dir, cfg, actions) = setup().await;
    let registry = solx_registry::Registry::open(cfg.clone()).await.unwrap();
    actions.set_token_authority(registry.clone());
    let token_file = dir.path().join(TOKEN_FILE);
    cfg.register_command(
        "token",
        solx_config::CommandDef {
            command: write_token_then_wait(secs, exit_code),
            description: None,
            cwd: None,
        },
    )
    .unwrap();
    actions
        .save(
            "/t",
            "token",
            ActionInput {
                action_type: Some(ActionType::Command),
                fn_name: Some("token".into()),
                action_config: timeout_secs.map(|t| json!({ "timeout_secs": t })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    RegistrySetup {
        _dir: dir,
        actions,
        registry,
        token_file,
    }
}

/// The token the child wrote, once it has.
async fn child_token(file: &std::path::Path) -> String {
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(file) {
            let token = text.trim().to_string();
            if !token.is_empty() {
                return token;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the command never wrote its token");
}

fn is_action(principal: Option<solx_surface::Principal>) -> bool {
    matches!(
        principal,
        Some(solx_surface::Principal::Action { ref action_ref, .. }) if action_ref == "/t/token"
    )
}

/// Runs `/t/token` to completion, checking its token authenticates while it
/// runs and is rejected once it has ended. Returns how the run ended.
async fn token_lives_exactly_as_long_as_the_run(s: &RegistrySetup) -> bool {
    let run = {
        let actions = s.actions.clone();
        tokio::spawn(async move { actions.exec("/t", "token", json!({})).await })
    };
    let token = child_token(&s.token_file).await;
    assert!(is_action(s.registry.authenticate(&token).await));
    let ended = run.await.unwrap();
    assert_eq!(s.registry.authenticate(&token).await, None);
    ended.is_ok_and(|out| out.success)
}

#[tokio::test]
async fn an_action_token_works_while_the_command_runs_and_not_after() {
    let s = registry_setup(1, 0, None).await;
    assert!(token_lives_exactly_as_long_as_the_run(&s).await);
}

/// The child's token carries exactly the scopes its action declares, and
/// none when it declares none.
#[tokio::test]
async fn an_action_token_carries_the_declared_scopes() {
    use solx_surface::{Grant, ScopeSet};

    async fn child_grant(s: &RegistrySetup) -> Grant {
        let run = {
            let actions = s.actions.clone();
            tokio::spawn(async move { actions.exec("/t", "token", json!({})).await })
        };
        let token = child_token(&s.token_file).await;
        let principal = s.registry.authenticate(&token).await.unwrap();
        run.await.unwrap().unwrap();
        std::fs::remove_file(&s.token_file).unwrap();
        principal.grant().clone()
    }

    let s = registry_setup(1, 0, None).await;
    assert_eq!(child_grant(&s).await, Grant::none());

    let declared = ["exec:/builtin/web/http-request", "write:document:/media/*"];
    s.actions
        .save(
            "/t",
            "token",
            ActionInput {
                action_config: Some(json!({ "scopes": declared })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        child_grant(&s).await,
        Grant::scoped(ScopeSet::parse(declared).unwrap())
    );
}

#[tokio::test]
async fn a_failing_command_releases_its_token() {
    let s = registry_setup(1, 3, None).await;
    assert!(!token_lives_exactly_as_long_as_the_run(&s).await);
}

#[tokio::test]
async fn a_timed_out_command_releases_its_token() {
    // Short: the shell is killed at the timeout, but on Windows its `ping`
    // outlives it and holds the output pipe until it ends (see `run_command`).
    let s = registry_setup(3, 0, Some(1)).await;
    assert!(!token_lives_exactly_as_long_as_the_run(&s).await);
}

#[tokio::test]
#[serial_test::serial]
async fn a_detached_run_releases_its_token_at_completion() {
    solx_actions::set_long_lived_host(true);
    let s = registry_setup(2, 0, None).await;
    let started = s
        .actions
        .start_invocation("/t", "token", json!({}), solx_surface::ExecContext::local().info())
        .await
        .unwrap();
    let token = child_token(&s.token_file).await;
    // `start` has returned; the run hasn't finished, so the token still works.
    assert!(is_action(s.registry.authenticate(&token).await));

    let id = started["invocation_id"].as_str().unwrap();
    let polled = s.actions.poll_invocation(id, Some(30)).await.unwrap();
    assert_eq!(polled["status"], "ok", "{polled}");
    assert_eq!(s.registry.authenticate(&token).await, None);
    solx_actions::set_long_lived_host(false);
}
