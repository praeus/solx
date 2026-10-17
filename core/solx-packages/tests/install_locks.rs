//! An install locks every action it leaves under `/packages/<name>/` except
//! the manifest's `unlocked_actions`, after unlocking that package's own
//! actions so a reinstall can replace them — see the module doc on
//! `solx_packages`.

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};
use solx_config::ConfigService;
use solx_packages::{install_package, PackageActions};
use solx_scripts::CommandRunner;
use solx_surface::error::Result;

struct NoopRunner;

#[async_trait]
impl CommandRunner for NoopRunner {
    async fn run(&self, _tokens: Vec<String>, _piped: Option<Value>) -> Result<Value> {
        Ok(Value::Null)
    }
}

/// An action store that already holds what `install.solx` would have
/// saved, and records every lock change in order.
struct FakeActions {
    actions: Vec<(String, String)>,
    /// `path/name` of the actions that have scoped secrets.
    keyed: Vec<String>,
    log: Mutex<Vec<String>>,
}

impl FakeActions {
    fn new(refs: &[(&str, &str)]) -> Self {
        FakeActions {
            actions: refs.iter().map(|(p, n)| (p.to_string(), n.to_string())).collect(),
            keyed: Vec::new(),
            log: Mutex::new(Vec::new()),
        }
    }

    fn keyed(mut self, full: &str) -> Self {
        self.keyed.push(full.to_string());
        self
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

#[async_trait]
impl PackageActions for FakeActions {
    async fn actions_under(&self, base: &str) -> Result<Vec<(String, String)>> {
        // A string prefix, as `ListOptions::path_prefix` is.
        Ok(self
            .actions
            .iter()
            .filter(|(path, _)| path.starts_with(base))
            .cloned()
            .collect())
    }

    async fn set_locked(&self, path: &str, name: &str, locked: bool) -> Result<()> {
        let verb = if locked { "lock" } else { "unlock" };
        self.log.lock().unwrap().push(format!("{verb} {path}/{name}"));
        Ok(())
    }

    async fn has_secrets(&self, path: &str, name: &str) -> Result<bool> {
        Ok(self.keyed.contains(&format!("{path}/{name}")))
    }
}

/// A runner whose every command fails, as a broken `install.solx` would.
struct FailingRunner;

#[async_trait]
impl CommandRunner for FailingRunner {
    async fn run(&self, _tokens: Vec<String>, _piped: Option<Value>) -> Result<Value> {
        Err(solx_surface::error::SolxError::Other("script broke".into()))
    }
}

fn write_package(dir: &Path, name: &str, extra: Value) {
    let mut meta = json!({ "name": name, "version": "0.0.1" });
    for (k, v) in extra.as_object().unwrap() {
        meta.as_object_mut().unwrap().insert(k.clone(), v.clone());
    }
    std::fs::write(dir.join("solx-package.json"), meta.to_string()).unwrap();
    std::fs::write(dir.join("install.solx"), "json '{}'").unwrap();
}

#[tokio::test]
async fn install_unlocks_then_locks_its_own_actions_except_the_opted_out() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({ "unlocked_actions": ["scratch"] }));
    let actions = FakeActions::new(&[
        ("/packages/pkg", "run"),
        ("/packages/pkg", "scratch"),
        ("/packages/pkg/nested", "deep"),
        // Another package's path, and one that only shares a prefix.
        ("/packages/other", "run"),
        ("/packages/pkg-extra", "run"),
    ]);

    let outcome = install_package(&NoopRunner, &actions, &cfg, pkg.path())
        .await
        .unwrap();

    assert_eq!(
        actions.log(),
        vec![
            "unlock /packages/pkg/run",
            "unlock /packages/pkg/scratch",
            "unlock /packages/pkg/nested/deep",
            "lock /packages/pkg/run",
            "lock /packages/pkg/nested/deep",
        ]
    );
    assert_eq!(
        outcome.package.locked_actions,
        vec!["/packages/pkg/run", "/packages/pkg/nested/deep"]
    );
    assert_eq!(
        cfg.list_packages()[0].locked_actions,
        outcome.package.locked_actions
    );
}

#[tokio::test]
async fn a_malformed_opt_out_fails_the_install() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({ "unlocked_actions": "scratch" }));
    let err = install_package(&NoopRunner, &FakeActions::new(&[]), &cfg, pkg.path())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unlocked_actions"), "{err}");
}

#[tokio::test]
async fn an_opted_out_action_with_secrets_is_locked_anyway() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({ "unlocked_actions": ["login"] }));
    let actions = FakeActions::new(&[("/packages/pkg", "login")]).keyed("/packages/pkg/login");

    let outcome = install_package(&NoopRunner, &actions, &cfg, pkg.path())
        .await
        .unwrap();

    assert_eq!(outcome.package.locked_actions, vec!["/packages/pkg/login"]);
    assert!(
        outcome.warnings.iter().any(|w| w.contains("locked anyway")),
        "{:?}",
        outcome.warnings
    );
}

/// A reinstall unlocks before running the script; a script that fails must
/// not leave the previous install's actions unlocked.
#[tokio::test]
async fn a_failed_reinstall_relocks_what_was_locked() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({ "unlocked_actions": ["scratch"] }));
    let actions = FakeActions::new(&[("/packages/pkg", "run"), ("/packages/pkg", "scratch")]);
    install_package(&NoopRunner, &actions, &cfg, pkg.path())
        .await
        .unwrap();
    actions.log.lock().unwrap().clear();

    let err = install_package(&FailingRunner, &actions, &cfg, pkg.path())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("script broke"), "{err}");
    assert_eq!(
        actions.log(),
        vec![
            "unlock /packages/pkg/run",
            "unlock /packages/pkg/scratch",
            "lock /packages/pkg/run",
        ]
    );
}
