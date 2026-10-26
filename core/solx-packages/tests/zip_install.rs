//! Installing a package from a zip: unpacked under `packages_dir()`, nested
//! layouts unwrapped, hostile entries refused, directory removed on uninstall.

use std::io::Write;

use async_trait::async_trait;
use serde_json::Value;
use solx_config::ConfigService;
use solx_packages::{install_package_zip_bytes, uninstall_package};
use solx_scripts::CommandRunner;

/// No actions to lock: these tests are about what an install does around
/// its script, and the fake runner saves nothing.
struct NoActions;

#[async_trait]
impl solx_packages::PackageActions for NoActions {
    async fn actions_under(&self, _base: &str) -> solx_surface::error::Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }
    async fn set_locked(&self, _p: &str, _n: &str, _l: bool) -> solx_surface::error::Result<()> {
        Ok(())
    }
    async fn is_keyed(&self, _p: &str, _n: &str) -> solx_surface::error::Result<bool> {
        Ok(false)
    }
    async fn declared_scopes(
        &self,
        _p: &str,
        _n: &str,
    ) -> solx_surface::error::Result<Option<solx_surface::ScopeSet>> {
        Ok(None)
    }
}

struct NoopRunner;

#[async_trait]
impl CommandRunner for NoopRunner {
    async fn run(&self, _t: Vec<String>, _p: Option<Value>) -> solx_surface::error::Result<Value> {
        Ok(Value::Null)
    }
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

const MANIFEST: &[u8] = br#"{"name":"demo","version":"1.0.0"}"#;
const SCRIPT: &[u8] = b"json '{}'";

/// `install_package` moves the process-wide cwd into the package while its
/// script runs, so tests that install and then delete directories can't
/// overlap: one could be left standing inside another's directory.
static CWD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn cfg() -> (tempfile::TempDir, ConfigService) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(dir.path()).unwrap();
    (dir, cfg)
}

#[tokio::test]
async fn installs_from_a_top_level_folder_and_uninstall_deletes_it() {
    let _serial = CWD.lock().await;
    let (_d, cfg) = cfg();
    let zip = zip_of(&[
        ("demo-1.0/solx-package.json", MANIFEST),
        ("demo-1.0/install.solx", SCRIPT),
        ("demo-1.0/bin/tool", b"x"),
    ]);
    let out = install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, zip)
        .await
        .unwrap();
    let dir = cfg.packages_dir().join("demo");
    assert_eq!(out.package.name, "demo");
    assert!(dir.join("bin/tool").exists());
    assert_eq!(cfg.list_packages().len(), 1);
    // No staging leftovers.
    assert_eq!(std::fs::read_dir(cfg.packages_dir()).unwrap().count(), 1);

    uninstall_package(&NoopRunner, &cfg, "demo").await.unwrap();
    assert!(!dir.exists());
    assert!(cfg.list_packages().is_empty());
}

#[tokio::test]
async fn ignores_build_time_siblings_but_not_several_packages() {
    let _serial = CWD.lock().await;
    let (_d, cfg) = cfg();
    let zip = zip_of(&[
        ("demo/solx-package.json", MANIFEST),
        ("demo/install.solx", SCRIPT),
        ("lib/Cargo.toml", b"x"),
    ]);
    install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, zip)
        .await
        .unwrap();
    let packages = cfg.packages_dir();
    assert!(packages.join("demo/install.solx").exists());
    assert!(!packages.join("lib").exists());

    let two = zip_of(&[
        ("a/solx-package.json", MANIFEST),
        ("b/solx-package.json", MANIFEST),
    ]);
    assert!(install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, two)
        .await
        .is_err());
}

#[tokio::test]
async fn reinstall_replaces_files_and_failed_reinstall_keeps_the_old_ones() {
    let _serial = CWD.lock().await;
    let (_d, cfg) = cfg();
    let v1 = zip_of(&[
        ("solx-package.json", MANIFEST),
        ("install.solx", SCRIPT),
        ("old", b"1"),
    ]);
    install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, v1)
        .await
        .unwrap();

    let v2 = zip_of(&[
        ("solx-package.json", MANIFEST),
        ("install.solx", SCRIPT),
        ("new", b"2"),
    ]);
    install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, v2)
        .await
        .unwrap();
    let dir = cfg.packages_dir().join("demo");
    assert!(dir.join("new").exists() && !dir.join("old").exists());

    // No install.solx: install fails, so the previous version must survive.
    let bad = zip_of(&[("solx-package.json", MANIFEST), ("bad", b"3")]);
    assert!(install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, bad)
        .await
        .is_err());
    assert!(dir.join("new").exists() && !dir.join("bad").exists());
}

#[tokio::test]
async fn rejects_bad_zips_and_unsafe_names() {
    let _serial = CWD.lock().await;
    let (_d, cfg) = cfg();
    assert!(
        install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, b"nope".to_vec())
            .await
            .is_err()
    );
    let no_manifest = zip_of(&[("readme.md", b"hi"), ("other", b"x")]);
    assert!(install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, no_manifest)
        .await
        .is_err());
    let evil = zip_of(&[
        ("solx-package.json", br#"{"name":"../evil"}"#),
        ("install.solx", b""),
    ]);
    assert!(install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, evil)
        .await
        .is_err());
    let escape = zip_of(&[("../x", b"1")]);
    assert!(install_package_zip_bytes(&NoopRunner, &NoActions, &cfg, escape)
        .await
        .is_err());
    assert!(cfg.list_packages().is_empty());
}
