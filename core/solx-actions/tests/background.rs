//! Background commands end to end, against `fixtures/fake_background.rs`:
//! one process serving many calls, started on first use, stopped with its
//! whole process tree, and held to its start action's scopes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use solx_actions::guard::scope::read_audit_log;
use solx_actions::LocalActionManager;
use solx_config::{CommandDef, ConfigService};
use solx_surface::entities::{ActionInput, ActionType};
use solx_surface::managers::{ActionManager, DocManager, FileStore, TypeManager};

const PKG: &str = "/packages/fake";

fn fake_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_solx-fake-background"))
}

async fn setup() -> (
    tempfile::TempDir,
    Arc<ConfigService>,
    Arc<LocalActionManager>,
) {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, actions) = manager_in(dir.path()).await;
    cfg.set("stop_grace_secs", json!(1)).unwrap();
    (dir, cfg, actions)
}

/// A manager on `dir`: a second one on the same dir stands in for another
/// solx process on the machine.
async fn manager_in(dir: &std::path::Path) -> (Arc<ConfigService>, Arc<LocalActionManager>) {
    let cfg = Arc::new(ConfigService::open_in(dir).unwrap());
    let types: Arc<dyn TypeManager> = Arc::new(
        solx_types::LocalTypeManager::open(&dir.join("types.db"))
            .await
            .unwrap(),
    );
    let docs: Arc<dyn DocManager> = Arc::new(
        solx_docs::LocalDocManager::open(&dir.join("docs.db"), types.clone())
            .await
            .unwrap(),
    );
    let files: Arc<dyn FileStore> = Arc::new(solx_files::LocalFileStore::new(dir.join("files")));
    let actions = Arc::new(
        LocalActionManager::open(&dir.join("actions.db"), cfg.clone(), types, docs, files)
            .await
            .unwrap(),
    );
    actions.set_self_ref(Arc::downgrade(&actions));
    (cfg, actions)
}

fn register(cfg: &ConfigService, key: &str, command: String) {
    cfg.register_command(
        key,
        CommandDef {
            command,
            description: None,
            cwd: None,
        },
    )
    .unwrap();
}

/// A Command action in `path` belonging to background `fake/session`.
async fn save_bg(
    actions: &LocalActionManager,
    path: &str,
    name: &str,
    key: &str,
    operation: &str,
    scopes: Option<&[&str]>,
) {
    let mut config = json!({"background": {
        "backgroundNamespace": "fake",
        "backgroundName": "session",
        "backgroundOperation": operation,
    }});
    if let Some(scopes) = scopes {
        config["scopes"] = json!(scopes);
    }
    actions
        .save(
            path,
            name,
            ActionInput {
                action_type: Some(ActionType::Command),
                fn_name: Some(key.into()),
                action_config: Some(config),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

/// The fake package: a start action running the fake (with `args`), a
/// stop action, and one action per op the fake knows.
async fn fake_package(
    cfg: &ConfigService,
    actions: &LocalActionManager,
    args: &str,
    start_scopes: Option<&[&str]>,
) {
    register(cfg, "fake-bg", format!("{} {args}", fake_exe().display()));
    save_bg(actions, PKG, "start", "fake-bg", "start", start_scopes).await;
    save_bg(actions, PKG, "stop", "fake-bg", "stop", None).await;
    for op in ["echo", "sleep", "pid", "crash", "host", "grandchild"] {
        save_bg(actions, PKG, op, "fake-bg", op, None).await;
    }
}

async fn call(actions: &LocalActionManager, name: &str, params: Value) -> Value {
    actions.exec(PKG, name, params).await.unwrap().result
}

fn process_alive(pid: u64) -> bool {
    if cfg!(windows) {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    } else {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    }
}

async fn wait_dead(pid: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !process_alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn a_second_start_returns_the_running_instance() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", None).await;

    let first = call(&actions, "start", json!({})).await;
    let second = call(&actions, "start", json!({})).await;
    assert_eq!(first["alreadyRunning"], false);
    assert_eq!(second["alreadyRunning"], true);
    assert_eq!(first["pid"], second["pid"]);
    assert_eq!(first["startAction"], "/packages/fake/start");
    actions.stop_backgrounds().await;
}

#[tokio::test]
async fn concurrent_first_operations_spawn_one_process() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", None).await;

    let calls: Vec<_> = (0..10)
        .map(|_| {
            let a = actions.clone();
            tokio::spawn(async move { call(&a, "pid", json!({})).await })
        })
        .collect();
    let mut pids = Vec::new();
    for c in calls {
        pids.push(c.await.unwrap());
    }
    assert!(pids.iter().all(|p| p == &pids[0]), "{pids:?}");
    assert_eq!(actions.list_backgrounds().len(), 1);
    actions.stop_backgrounds().await;
}

#[tokio::test]
async fn concurrent_calls_get_their_own_answers_out_of_order() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", None).await;
    call(&actions, "start", json!({})).await;

    let a = actions.clone();
    let slow = tokio::spawn(async move {
        let r = call(&a, "sleep", json!({"ms": 800, "value": "slow"})).await;
        (r, Instant::now())
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let fast = call(&actions, "echo", json!({"value": "fast"})).await;
    let fast_done = Instant::now();
    let (slow, slow_done) = slow.await.unwrap();
    assert_eq!(fast, json!({"value": "fast"}));
    assert_eq!(slow, json!("slow"));
    assert!(fast_done < slow_done, "the fast answer waited behind the slow one");
    actions.stop_backgrounds().await;
}

#[tokio::test]
async fn an_operation_with_no_start_action_says_so() {
    let (_d, cfg, actions) = setup().await;
    register(&cfg, "fake-bg", fake_exe().display().to_string());
    save_bg(&actions, PKG, "echo", "fake-bg", "echo", None).await;

    let err = actions.exec(PKG, "echo", json!({})).await.unwrap_err();
    assert!(err.to_string().contains("has no start action"), "{err}");
}

#[tokio::test]
async fn stop_exits_cleanly_and_is_a_no_op_when_stopped() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", None).await;
    let pid = call(&actions, "pid", json!({})).await.as_u64().unwrap();

    let stopped = call(&actions, "stop", json!({})).await;
    assert_eq!(stopped["wasRunning"], true);
    assert!(wait_dead(pid).await, "the background outlived stop");
    assert!(actions.list_backgrounds().is_empty());

    let again = call(&actions, "stop", json!({})).await;
    assert_eq!(again["wasRunning"], false);
}

#[tokio::test]
async fn stop_kills_a_background_that_ignores_shutdown() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "--ignore-shutdown", None).await;
    let pid = call(&actions, "pid", json!({})).await.as_u64().unwrap();

    let began = Instant::now();
    call(&actions, "stop", json!({})).await;
    assert!(began.elapsed() >= Duration::from_secs(1), "killed before the grace period");
    assert!(wait_dead(pid).await, "the background outlived stop");
}

#[tokio::test]
async fn a_crash_fails_pending_calls_and_the_next_call_starts_afresh() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", None).await;
    let first = call(&actions, "pid", json!({})).await;

    let a = actions.clone();
    let pending =
        tokio::spawn(async move { a.exec(PKG, "sleep", json!({"ms": 10_000})).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = actions.exec(PKG, "crash", json!({})).await;

    let err = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("a pending call outlived the crash")
        .unwrap()
        .unwrap_err();
    assert!(err.to_string().contains("background"), "{err}");

    let second = call(&actions, "pid", json!({})).await;
    assert_ne!(first, second);
    actions.stop_backgrounds().await;
}

#[tokio::test]
async fn stop_backgrounds_leaves_no_grandchild_behind() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", None).await;
    let grandchild = call(&actions, "grandchild", json!({})).await.as_u64().unwrap();
    assert!(process_alive(grandchild));

    actions.stop_backgrounds().await;
    assert!(wait_dead(grandchild).await, "the grandchild outlived stop_backgrounds");
}

/// One-shot Command targets for the background's `exec` calls.
async fn targets(cfg: &ConfigService, actions: &LocalActionManager) {
    register(cfg, "hello", "echo 42".into());
    for path in ["/t", "/u", "/v"] {
        actions
            .save(
                path,
                "hello",
                ActionInput {
                    action_type: Some(ActionType::Command),
                    fn_name: Some("hello".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
}

fn host_exec(path: &str) -> Value {
    json!({"op": "exec", "params": {"path": path, "name": "hello", "params": {}}})
}

#[tokio::test]
async fn a_backgrounds_calls_are_held_to_its_start_actions_scopes() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", Some(&["exec:/t/*"])).await;
    targets(&cfg, &actions).await;

    let inside = call(&actions, "host", host_exec("/t")).await;
    assert_eq!(inside["result"], json!(42));

    // `/v` only here: refusals are recorded once per process, so a need
    // another test also refuses might be on that test's log instead.
    let outside = actions.exec(PKG, "host", host_exec("/v")).await.unwrap_err();
    assert!(outside.to_string().contains("exec:/v/hello"), "{outside}");
    let audit = read_audit_log(&cfg).unwrap();
    assert!(
        audit
            .iter()
            .any(|e| e.refused && e.need.contains("/v/hello")),
        "{audit:?}"
    );
    actions.stop_backgrounds().await;
}

#[tokio::test]
async fn waking_through_a_wider_operation_does_not_widen_the_scopes() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", Some(&["exec:/t/*"])).await;
    targets(&cfg, &actions).await;
    // The operation that wakes it declares more than the start action.
    save_bg(&actions, PKG, "host", "fake-bg", "host", Some(&["exec:/u/*"])).await;

    let err = actions.exec(PKG, "host", host_exec("/u")).await.unwrap_err();
    assert!(err.to_string().contains("exec:/u/hello"), "{err}");
    actions.stop_backgrounds().await;
}

#[tokio::test]
async fn console_print_lands_on_the_calling_actions_console() {
    let (_d, cfg, actions) = setup().await;
    fake_package(&cfg, &actions, "", None).await;

    call(
        &actions,
        "host",
        json!({"op": "console.print", "params": {"level": "info", "message": "hello"}}),
    )
    .await;
    let read = actions
        .console()
        .read("/packages/fake/host", None, 50)
        .await
        .unwrap();
    assert!(
        read.entries
            .iter()
            .any(|e| e.message.as_deref() == Some("hello")),
        "{:?}",
        read.entries
    );
    actions.stop_backgrounds().await;
}

#[tokio::test]
async fn another_process_on_the_machine_is_refused_with_its_details() {
    let (dir, cfg, first) = setup().await;
    fake_package(&cfg, &first, "", None).await;
    call(&first, "start", json!({})).await;

    let (_, second) = manager_in(dir.path()).await;
    let err = second.exec(PKG, "pid", json!({})).await.unwrap_err();
    let shown = err.to_string();
    assert!(shown.contains("another solx process"), "{shown}");
    assert!(shown.contains(&std::process::id().to_string()), "{shown}");

    // Once the first has stopped it, the second may start its own.
    call(&first, "stop", json!({})).await;
    assert!(second.exec(PKG, "pid", json!({})).await.is_ok());
    second.stop_backgrounds().await;
}

#[tokio::test]
async fn a_crash_releases_the_lock() {
    let (dir, cfg, first) = setup().await;
    fake_package(&cfg, &first, "", None).await;
    call(&first, "start", json!({})).await;
    let _ = first.exec(PKG, "crash", json!({})).await;

    let (_, second) = manager_in(dir.path()).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match second.exec(PKG, "start", json!({})).await {
            Ok(_) => break,
            Err(e) if Instant::now() < deadline => {
                assert!(e.to_string().contains("another solx process"), "{e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => panic!("the lock outlived the crash: {e}"),
        }
    }
    second.stop_backgrounds().await;
}

#[tokio::test]
async fn a_start_waits_for_the_lock_when_configured_to() {
    let (dir, cfg, first) = setup().await;
    fake_package(&cfg, &first, "", None).await;
    call(&first, "start", json!({})).await;

    let (second_cfg, second) = manager_in(dir.path()).await;
    second_cfg.set("background_lock_wait_secs", json!(10)).unwrap();
    let waiting = {
        let second = second.clone();
        tokio::spawn(async move { second.exec(PKG, "pid", json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!waiting.is_finished(), "the second start didn't wait");
    call(&first, "stop", json!({})).await;

    let pid = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the waiting start never got the lock")
        .unwrap()
        .unwrap();
    assert!(pid.result.is_u64());
    second.stop_backgrounds().await;
}
