//! An install locks every action it leaves under `/packages/<name>/` except
//! the manifest's `unlocked_actions`, after unlocking that package's own
//! actions so a reinstall can replace them — see the module doc on
//! `solx_packages`.

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};
use solx_config::ConfigService;
use solx_packages::{
    guard_package_script_action, install_package, uninstall_package, PackageActions,
};
use solx_scripts::CommandRunner;
use solx_surface::entities::ActionInput;
use solx_surface::error::Result;
use solx_surface::ScopeSet;

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
    /// `path/name` of the actions that can't be locked (artifact missing).
    unlockable: Vec<String>,
    /// `path/name` and declared scopes of the actions that declare some.
    scoped: Vec<(String, ScopeSet)>,
    log: Mutex<Vec<String>>,
}

impl FakeActions {
    fn new(refs: &[(&str, &str)]) -> Self {
        FakeActions {
            actions: refs.iter().map(|(p, n)| (p.to_string(), n.to_string())).collect(),
            keyed: Vec::new(),
            unlockable: Vec::new(),
            scoped: Vec::new(),
            log: Mutex::new(Vec::new()),
        }
    }

    fn keyed(mut self, full: &str) -> Self {
        self.keyed.push(full.to_string());
        self
    }

    fn unlockable(mut self, full: &str) -> Self {
        self.unlockable.push(full.to_string());
        self
    }

    fn scoped(mut self, full: &str, scopes: &[&str]) -> Self {
        self.scoped
            .push((full.to_string(), ScopeSet::parse(scopes).unwrap()));
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
        let full = format!("{path}/{name}");
        if locked && self.unlockable.contains(&full) {
            return Err(solx_surface::error::SolxError::Invalid(format!(
                "cannot lock {full}: artifact not found"
            )));
        }
        let verb = if locked { "lock" } else { "unlock" };
        self.log.lock().unwrap().push(format!("{verb} {full}"));
        Ok(())
    }

    async fn is_keyed(&self, path: &str, name: &str) -> Result<bool> {
        Ok(self.keyed.contains(&format!("{path}/{name}")))
    }

    async fn declared_scopes(&self, path: &str, name: &str) -> Result<Option<ScopeSet>> {
        let full = format!("{path}/{name}");
        Ok(self
            .scoped
            .iter()
            .find(|(f, _)| *f == full)
            .map(|(_, s)| s.clone()))
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

/// An action with secrets is never unlocked, so content gets no window
/// during the script to swap the code that runs with the package's key.
#[tokio::test]
async fn a_reinstall_leaves_actions_with_secrets_locked() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({}));
    let actions = FakeActions::new(&[("/packages/pkg", "login"), ("/packages/pkg", "run")])
        .keyed("/packages/pkg/login");

    install_package(&NoopRunner, &actions, &cfg, pkg.path())
        .await
        .unwrap();

    assert_eq!(
        actions.log(),
        vec![
            "unlock /packages/pkg/run",
            "lock /packages/pkg/login",
            "lock /packages/pkg/run",
        ]
    );
}

/// One action that can't be locked doesn't leave the ones after it open.
#[tokio::test]
async fn a_failed_lock_still_locks_the_rest() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({}));
    let actions = FakeActions::new(&[("/packages/pkg", "a"), ("/packages/pkg", "b")])
        .unlockable("/packages/pkg/a");

    let err = install_package(&NoopRunner, &actions, &cfg, pkg.path())
        .await
        .unwrap_err();

    assert!(err.to_string().contains("locking /packages/pkg/a"), "{err}");
    assert!(actions.log().contains(&"lock /packages/pkg/b".to_string()));
    assert!(cfg.list_packages().is_empty());
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

/// Stands in for the CLI's `save action <ref> [--json <input>]` and `delete action
/// <ref>`, which apply `guard_package_script_action` to the action's path
/// and, for a save, its input.
struct GuardingRunner;

#[async_trait]
impl CommandRunner for GuardingRunner {
    async fn run(&self, tokens: Vec<String>, _piped: Option<Value>) -> Result<Value> {
        let (path, _) = solx_surface::path::split_ref(&tokens[2])?;
        let body = tokens
            .iter()
            .position(|t| t == "--json")
            .and_then(|i| tokens.get(i + 1));
        let input: Option<ActionInput> = match (tokens[0].as_str(), body) {
            ("save", Some(body)) => Some(serde_json::from_str(body)?),
            ("save", None) => Some(ActionInput::default()),
            _ => None,
        };
        guard_package_script_action(&path, input.as_ref())?;
        Ok(Value::Null)
    }
}

fn write_script(dir: &Path, file: &str, script: &str) {
    std::fs::write(dir.join(file), script).unwrap();
}

#[tokio::test]
async fn a_script_may_change_its_own_actions() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({}));
    write_script(
        pkg.path(),
        "install.solx",
        "save action /packages/pkg/a; save action /packages/pkg/nested/b; \
         delete action /packages/pkg/old",
    );
    install_package(&GuardingRunner, &FakeActions::new(&[]), &cfg, pkg.path())
        .await
        .unwrap();
}

/// The secret key is chosen by path, so an action saved under another
/// package's path would run with that package's key.
#[tokio::test]
async fn a_script_may_not_change_another_packages_actions() {
    for target in [
        "/packages/other/steal",
        "/packages//other/steal",
        "/packages/pkg-extra/steal",
        "/packages/steal",
        "/tools/steal",
    ] {
        for verb in ["save", "delete"] {
            let appdata = tempfile::tempdir().unwrap();
            let cfg = ConfigService::open_in(appdata.path()).unwrap();
            let pkg = tempfile::tempdir().unwrap();
            write_package(pkg.path(), "pkg", json!({}));
            write_script(pkg.path(), "install.solx", &format!("{verb} action {target}"));
            let err = install_package(&GuardingRunner, &FakeActions::new(&[]), &cfg, pkg.path())
                .await
                .unwrap_err();
            assert!(
                err.to_string().contains("only save or delete actions under /packages/pkg/"),
                "{verb} {target}: {err}"
            );
            assert!(cfg.list_packages().is_empty());
        }
    }
}

#[tokio::test]
async fn an_uninstall_script_is_confined_too() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({}));
    install_package(&NoopRunner, &FakeActions::new(&[]), &cfg, pkg.path())
        .await
        .unwrap();
    write_script(pkg.path(), "uninstall.solx", "delete action /packages/other/run");
    let err = uninstall_package(&GuardingRunner, &cfg, "pkg")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("under /packages/pkg/"), "{err}");
}

/// Outside a package script the guard lets everything through.
#[test]
fn the_guard_is_a_no_op_outside_a_script() {
    guard_package_script_action("/packages/other", None).unwrap();
}

/// What a script saves may declare only scopes its manifest lists, besides
/// the package's own `exec`. The manifest's list is recorded canonical.
#[tokio::test]
async fn a_script_may_declare_only_the_manifests_scopes() {
    let save = |scopes: &str| {
        format!("save action /packages/pkg/a --json {{\"actionConfig\": {{\"scopes\": {scopes}}}}}")
    };
    async fn install(script: String) -> (Result<()>, Vec<solx_config::InstalledPackage>) {
        let appdata = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(appdata.path()).unwrap();
        let pkg = tempfile::tempdir().unwrap();
        write_package(
            pkg.path(),
            "pkg",
            json!({ "scopes": ["write:script", "exec:/builtin/web/*", "write:script"] }),
        );
        write_script(pkg.path(), "install.solx", &script);
        let result = install_package(&GuardingRunner, &FakeActions::new(&[]), &cfg, pkg.path())
            .await
            .map(|_| ());
        (result, cfg.list_packages())
    }

    for ok in [
        r#"["exec:/builtin/web/http-request","write:script"]"#,
        r#"["exec:/packages/pkg/*"]"#,
        "[]",
    ] {
        let (result, packages) = install(save(ok)).await;
        result.unwrap_or_else(|e| panic!("{ok}: {e}"));
        assert_eq!(
            packages[0].granted_scopes,
            ["exec:/builtin/web/*", "write:script"],
            "{ok}"
        );
    }
    for (bad, names) in [
        (r#"["exec:/builtin/env/get-env"]"#, "exec:/builtin/env/get-env"),
        (r#"["exec:/packages/other/*"]"#, "exec:/packages/other/*"),
        (r#"["all"]"#, "all"),
    ] {
        let (result, packages) = install(save(bad)).await;
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("manifest doesn't list") && err.contains(names),
            "{bad}: {err}"
        );
        assert!(packages.is_empty());
    }
}

#[tokio::test]
async fn a_manifest_with_a_bad_scope_is_refused() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({ "scopes": ["exec:"] }));
    let err = install_package(&NoopRunner, &FakeActions::new(&[]), &cfg, pkg.path())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("'scopes'"), "{err}");
}

/// An action the script didn't save keeps its stored scopes; one the
/// manifest no longer covers is warned about.
#[tokio::test]
async fn a_leftover_action_beyond_the_manifest_is_warned_about() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({ "scopes": ["write:script"] }));
    let actions = FakeActions::new(&[("/packages/pkg", "old"), ("/packages/pkg", "new")])
        .scoped("/packages/pkg/old", &["write:command"])
        .scoped("/packages/pkg/new", &["write:script"]);
    let outcome = install_package(&NoopRunner, &actions, &cfg, pkg.path())
        .await
        .unwrap();
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
    assert!(outcome.warnings[0].contains("/packages/pkg/old"));
    assert!(outcome.warnings[0].contains("write:command"));
}

/// A nested install would run its script under another package's name.
struct NestedUninstallRunner<'a> {
    cfg: &'a ConfigService,
}

#[async_trait]
impl CommandRunner for NestedUninstallRunner<'_> {
    async fn run(&self, _tokens: Vec<String>, _piped: Option<Value>) -> Result<Value> {
        uninstall_package(&NoopRunner, self.cfg, "other").await?;
        Ok(Value::Null)
    }
}

#[tokio::test]
async fn a_script_may_not_install_or_uninstall_a_package() {
    let appdata = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(appdata.path()).unwrap();
    let pkg = tempfile::tempdir().unwrap();
    write_package(pkg.path(), "pkg", json!({}));
    let runner = NestedUninstallRunner { cfg: &cfg };
    let err = install_package(&runner, &FakeActions::new(&[]), &cfg, pkg.path())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("can't uninstall a package"), "{err}");
}

/// `a/b` would put its actions under `/packages/a/`, in package `a`'s
/// secret scope.
#[tokio::test]
async fn a_package_name_must_be_one_path_segment() {
    for name in ["other/steal", "..", ".hidden", "a b"] {
        let appdata = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(appdata.path()).unwrap();
        let pkg = tempfile::tempdir().unwrap();
        write_package(pkg.path(), name, json!({}));
        let err = install_package(&NoopRunner, &FakeActions::new(&[]), &cfg, pkg.path())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("package name"), "{name}: {err}");
    }
}
