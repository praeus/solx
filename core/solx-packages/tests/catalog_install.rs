//! Installing catalog packages from a folder of zips: names come from the
//! manifest inside, requirements install first, unknown names are refused.

use std::io::Write;

use async_trait::async_trait;
use serde_json::Value;
use solx_config::ConfigService;
use solx_packages::catalog::{
    catalog_view, dependents, install_from_catalog, resolve, EntryState, PackageSource,
};
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
    async fn has_secrets(&self, _p: &str, _n: &str) -> solx_surface::error::Result<bool> {
        Ok(false)
    }
}

struct NoopRunner;

#[async_trait]
impl CommandRunner for NoopRunner {
    async fn run(&self, _t: Vec<String>, _p: Option<Value>) -> solx_surface::error::Result<Value> {
        Ok(Value::Null)
    }
}

/// See `zip_install.rs`: installs move the process-wide cwd.
static CWD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn write_zip(dir: &std::path::Path, file: &str, name: &str, version: &str) {
    let mut w = zip::ZipWriter::new(std::fs::File::create(dir.join(file)).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    w.start_file(format!("{name}/solx-package.json"), opts)
        .unwrap();
    w.write_all(format!(r#"{{"name":"{name}","version":"{version}"}}"#).as_bytes())
        .unwrap();
    w.start_file(format!("{name}/install.solx"), opts).unwrap();
    w.write_all(b"json '{}'").unwrap();
    w.finish().unwrap();
}

#[tokio::test]
async fn installs_requirements_first_from_a_folder_source() {
    let _serial = CWD.lock().await;
    let home = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(home.path()).unwrap();
    let zips = tempfile::tempdir().unwrap();
    // CI artifact style names: the manifest, not the file name, is trusted.
    write_zip(
        zips.path(),
        "zip-solx-reddit-any.zip",
        "solx-reddit",
        "0.1.0",
    );
    write_zip(zips.path(), "quickjs.zip", "solx-quickjs", "0.1.0");
    write_zip(zips.path(), "junk.zip", "not-in-catalog", "1.0.0");

    let source = PackageSource::Dir(zips.path().to_path_buf());
    let release = resolve(&source).await.unwrap();
    assert_eq!(release.assets.len(), 3);

    let view = catalog_view(&cfg, &source, &release);
    let state = |v: &solx_packages::catalog::CatalogView, n: &str| {
        v.entries.iter().find(|e| e.entry.name == n).unwrap().state
    };
    assert_eq!(state(&view, "solx-reddit"), EntryState::Available);
    assert_eq!(state(&view, "solx-git"), EntryState::NotReleased);

    let out = install_from_catalog(&NoopRunner, &NoActions, &cfg, &release, "solx-reddit")
        .await
        .unwrap();
    assert_eq!(out.package.name, "solx-reddit");
    assert!(out.warnings.iter().any(|w| w.contains("solx-quickjs")));
    let names: Vec<_> = cfg.list_packages().into_iter().map(|p| p.name).collect();
    assert!(names.contains(&"solx-quickjs".to_string()));
    assert_eq!(dependents(&cfg, "solx-quickjs"), ["solx-reddit"]);
    let view = catalog_view(&cfg, &source, &release);
    assert_eq!(state(&view, "solx-reddit"), EntryState::Installed);

    let err = install_from_catalog(&NoopRunner, &NoActions, &cfg, &release, "not-in-catalog")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not in the package catalog"));
    let err = install_from_catalog(&NoopRunner, &NoActions, &cfg, &release, "solx-git")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("solx-git"));
}

#[tokio::test]
async fn a_newer_release_is_an_update() {
    let _serial = CWD.lock().await;
    let home = tempfile::tempdir().unwrap();
    let cfg = ConfigService::open_in(home.path()).unwrap();
    let zips = tempfile::tempdir().unwrap();
    write_zip(zips.path(), "a.zip", "solx-names", "0.1.0");
    let source = PackageSource::Dir(zips.path().to_path_buf());
    let release = resolve(&source).await.unwrap();
    install_from_catalog(&NoopRunner, &NoActions, &cfg, &release, "solx-names")
        .await
        .unwrap();

    write_zip(zips.path(), "b.zip", "solx-names", "0.2.0");
    let release = resolve(&source).await.unwrap();
    let view = catalog_view(&cfg, &source, &release);
    let names = view
        .entries
        .iter()
        .find(|e| e.entry.name == "solx-names")
        .unwrap();
    assert_eq!(names.state, EntryState::UpdateAvailable);
    assert_eq!(names.available_version.as_deref(), Some("0.2.0"));
}
