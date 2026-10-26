//! Fixtures shared by the crate's unit tests.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use solx_config::ConfigService;
use solx_docs::LocalDocManager;
use solx_files::LocalFileStore;
use solx_surface::managers::{DocManager, FileStore, TypeManager};
use solx_types::LocalTypeManager;

use crate::LocalActionManager;

/// Open a manager over `actions_db`, with its sibling type/doc/file stores
/// and config all rooted in `dir`.
pub(crate) async fn open_manager(
    dir: &Path,
    actions_db: &Path,
) -> (Arc<ConfigService>, LocalActionManager) {
    let cfg = Arc::new(ConfigService::open_in(dir).unwrap());
    let types: Arc<dyn TypeManager> =
        Arc::new(LocalTypeManager::open(&dir.join("types.db")).await.unwrap());
    let docs: Arc<dyn DocManager> = Arc::new(
        LocalDocManager::open(&dir.join("docs.db"), types.clone())
            .await
            .unwrap(),
    );
    let files: Arc<dyn FileStore> = Arc::new(LocalFileStore::new(dir.join("files")));
    let m = LocalActionManager::open(actions_db, cfg.clone(), types, docs, files)
        .await
        .unwrap();
    (cfg, m)
}

pub(crate) async fn setup() -> (tempfile::TempDir, Arc<ConfigService>, LocalActionManager) {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, m) = open_manager(dir.path(), &dir.path().join("actions.db")).await;
    (dir, cfg, m)
}

/// A manager wired for recursive execution, as `solx-manager` does it.
pub(crate) async fn setup_wired() -> (
    tempfile::TempDir,
    Arc<ConfigService>,
    Arc<LocalActionManager>,
) {
    let (dir, cfg, m) = setup().await;
    let m = Arc::new(m);
    m.set_self_ref(Arc::downgrade(&m));
    (dir, cfg, m)
}

/// Registers `command` under `command_actions` keyed by `key`, then
/// saves a Command action whose `fn_name` is that key. `fn_name` is
/// never a literal shell string once the allowlist is in effect — see
/// `docs/next-steps.md` §1.
pub(crate) fn allow_command(cfg: &ConfigService, key: &str, command: &str) {
    cfg.register_command(
        key,
        solx_config::CommandDef {
            command: command.into(),
            description: None,
            cwd: None,
        },
    )
    .unwrap();
}

/// Appends `prefix` to the `allowed_base_urls` allowlist.
pub(crate) fn allow_webhook_prefix(cfg: &ConfigService, prefix: &str) {
    cfg.add_allowed_base_url(prefix).unwrap();
}

/// A valid action key: 32 bytes, base64-encoded.
pub(crate) const TEST_KEY: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";

pub(crate) fn cfg_with_secret(key: &str) -> Value {
    serde_json::json!({ "cwd": "/work", "secrets": { "API_TOKEN": key } })
}

/// In-memory stand-in for the OS credential manager behind the scoped
/// secret store, cleared each time it is built. `fn` pointers can't
/// capture state, so the map lives in a static.
pub(crate) fn fake_secret_backend() -> crate::secrets::SecretBackendOverride {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static FAKE_SECRETS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    fn store() -> &'static Mutex<HashMap<String, String>> {
        FAKE_SECRETS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    store().lock().unwrap().clear();
    crate::secrets::SecretBackendOverride {
        get: |name| Ok(store().lock().unwrap().get(name).cloned()),
        set: |name, blob| {
            store()
                .lock()
                .unwrap()
                .insert(name.to_string(), blob.to_string());
            Ok(())
        },
        delete: |name| {
            store().lock().unwrap().remove(name);
            Ok(())
        },
    }
}
