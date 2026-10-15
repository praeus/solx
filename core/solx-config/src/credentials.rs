//! Where solx's bearer tokens live, and the default [`TokenAuthority`]:
//! today's single shared root token.
//!
//! Tokens are kept in the OS credential manager (Windows Credential
//! Manager, macOS Keychain, Secret Service on Linux) under the service
//! [`KEYRING_SERVICE`], not in `solx-config.json`. Every host and client
//! resolves tokens through this module rather than reading `server_token`
//! itself.
//!
//! Two kinds of entry, each scoped to one appdata directory so isolated
//! appdata dirs (tests, `SOLX_APPDATA_DIR`) never share a token:
//! - the **root token** (`root:<appdata>`), which a host's HTTP server
//!   accepts;
//! - **client tokens** (`client:<appdata>:<server url>`), which a client
//!   presents to a remote server.
//!
//! `server_token` in the config file is the legacy location. [`root_token`]
//! moves a value found there into the credential manager and removes it
//! from the file. The file is still used when `token_store` is `"config"`,
//! or as a fallback (with a warning) when no credential manager is
//! available, e.g. a headless Linux box without Secret Service.

use std::sync::RwLock;

use async_trait::async_trait;
use base64::Engine as _;
use serde_json::Value;
use solx_surface::auth::{constant_time_eq, ActionCredential, Principal, TokenAuthority};
use solx_surface::error::{Result, SolxError};

use crate::ConfigService;

/// Credential-manager service name every solx token is stored under.
pub const KEYRING_SERVICE: &str = "solx";

const LEGACY_KEY: &str = "server_token";

/// A fresh random token: 32 bytes, base64.
pub fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ── root token ───────────────────────────────────────────────────────────────

/// The server-side root token, generated on first use. Moves a legacy
/// `server_token` out of the config file into the credential manager.
pub fn root_token(config: &ConfigService) -> Result<String> {
    if !uses_keyring(config) {
        return config.ensure_server_token();
    }
    let account = root_account(config);
    // Fast path: already migrated, nothing to write.
    if legacy_token(config).is_none() {
        match store().get(&account) {
            Ok(Some(token)) => return Ok(token),
            Ok(None) => {}
            Err(e) => return fall_back_to_config(config, &e),
        }
    }
    // Generate or migrate under the config file's cross-process lock, so two
    // hosts starting at once can't each mint a different token. A store
    // error aborts the write, leaving the config file untouched.
    let mut token = None;
    let mut store_error = None;
    let locked = config.mutate(|obj| {
        let legacy = obj
            .get(LEGACY_KEY)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let stored = store().get(&account).map_err(|e| {
            store_error = Some(e.clone());
            SolxError::Config(e)
        })?;
        // When both exist, the file's copy is the newer one: once migrated,
        // `server_token` is only written again by a fallback or rotation that
        // couldn't reach the credential manager (or by an older build). So it
        // wins — keeping the credential manager's would silently revive a
        // token that was rotated away.
        let value = match (stored.as_deref(), legacy) {
            (Some(stored), None) => stored.to_string(),
            (_, Some(legacy)) => legacy,
            (None, None) => generate_token(),
        };
        if stored.as_deref() != Some(value.as_str()) {
            if stored.is_some() {
                eprintln!(
                    "solx: solx-config.json had a newer server_token than the credential \
                     manager; moving it there"
                );
            }
            store_verified(&account, &value).inspect_err(|e| {
                store_error = Some(e.to_string());
            })?;
        }
        obj.remove(LEGACY_KEY);
        token = Some(value);
        Ok(())
    });
    // Only a credential-store failure falls back to the file; a config-file
    // error (lock, unparsable JSON) is reported as itself.
    match (locked, token, store_error) {
        (Ok(()), Some(token), _) => Ok(token),
        (Err(_), _, Some(e)) => fall_back_to_config(config, &e),
        (Err(e), _, None) => Err(e),
        (Ok(()), None, _) => Err(SolxError::Config("root token was not resolved".into())),
    }
}

/// Replace the root token with a new random one and return it. Hosts that
/// are already running keep accepting the old token until they restart.
pub fn rotate_root_token(config: &ConfigService) -> Result<String> {
    let token = generate_token();
    if uses_keyring(config) {
        let account = root_account(config);
        let mut stored = false;
        config.mutate(|obj| {
            match store_verified(&account, &token) {
                Ok(()) => {
                    obj.remove(LEGACY_KEY);
                    stored = true;
                }
                Err(e) => warn_keyring_unavailable(&e.to_string()),
            }
            if !stored {
                obj.insert(LEGACY_KEY.into(), Value::String(token.clone()));
            }
            Ok(())
        })?;
    } else {
        config.set(LEGACY_KEY, Value::String(token.clone()))?;
    }
    Ok(token)
}

/// Where the root token is kept right now: `"keyring"` or `"config"`.
pub fn root_token_location(config: &ConfigService) -> &'static str {
    if legacy_token(config).is_some() {
        "config"
    } else {
        "keyring"
    }
}

fn fall_back_to_config(config: &ConfigService, error: &str) -> Result<String> {
    warn_keyring_unavailable(error);
    config.ensure_server_token()
}

// ── client tokens ────────────────────────────────────────────────────────────

/// The token a client presents to the `solx-server` at `server_url`:
/// `SOLX_SERVER_TOKEN` from the environment, else [`stored_client_token`].
/// Never generates one.
pub fn client_token(config: &ConfigService, server_url: &str) -> Option<String> {
    std::env::var("SOLX_SERVER_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| stored_client_token(config, server_url))
}

/// The stored token for `server_url`, ignoring the environment, in order:
/// 1. the client token saved by [`set_client_token`];
/// 2. this appdata dir's root token, **only** when `server_url` is on
///    loopback: a client sharing its appdata dir with a server on this
///    machine. The root token is never offered to another host;
/// 3. a plaintext `server_token` in the config file (legacy, or
///    `token_store: "config"`).
pub fn stored_client_token(config: &ConfigService, server_url: &str) -> Option<String> {
    if uses_keyring(config) {
        if let Ok(Some(token)) = store().get(&client_account(config, server_url)) {
            return Some(token);
        }
        if is_loopback_url(server_url) {
            if let Ok(Some(token)) = store().get(&root_account(config)) {
                return Some(token);
            }
        }
    }
    legacy_token(config)
}

/// Whether `server_url`'s host is `localhost` or a loopback IP.
fn is_loopback_url(server_url: &str) -> bool {
    let Ok(url) = url::Url::parse(server_url.trim()) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Save the token a client uses for `server_url`. Falls back to the config
/// file's `server_token` when no credential manager is available.
pub fn set_client_token(config: &ConfigService, server_url: &str, token: &str) -> Result<()> {
    if uses_keyring(config) {
        match store_verified(&client_account(config, server_url), token) {
            Ok(()) => {
                // Drop any plaintext copy this one supersedes.
                return config.mutate(|obj| {
                    obj.remove(LEGACY_KEY);
                    Ok(())
                });
            }
            Err(e) => warn_keyring_unavailable(&e.to_string()),
        }
    }
    config.set(LEGACY_KEY, Value::String(token.to_string()))
}

/// Forget the token a client saved for `server_url`, including a plaintext
/// `server_token` in the config file.
pub fn forget_client_token(config: &ConfigService, server_url: &str) -> Result<()> {
    if uses_keyring(config) {
        if let Err(e) = store().delete(&client_account(config, server_url)) {
            eprintln!("solx: could not remove the saved token for {server_url}: {e}");
        }
    }
    config.mutate(|obj| {
        obj.remove(LEGACY_KEY);
        Ok(())
    })
}

/// Move a plaintext `server_token` into the credential manager as the
/// client token for `server_url`. For a config used only as a client's
/// connection settings (e.g. a desktop client's private config), where that
/// value was always a client token, never this dir's root token.
pub fn adopt_legacy_client_token(config: &ConfigService, server_url: &str) -> Result<()> {
    match legacy_token(config) {
        Some(token) if uses_keyring(config) => set_client_token(config, server_url, &token),
        _ => Ok(()),
    }
}

// ── storage ──────────────────────────────────────────────────────────────────

fn uses_keyring(config: &ConfigService) -> bool {
    config.snapshot().token_store.as_deref() != Some("config")
}

fn legacy_token(config: &ConfigService) -> Option<String> {
    config
        .snapshot()
        .server_token
        .filter(|s| !s.trim().is_empty())
}

fn root_account(config: &ConfigService) -> String {
    format!("root:{}", appdata_key(config))
}

fn client_account(config: &ConfigService, server_url: &str) -> String {
    let url = server_url.trim().trim_end_matches('/');
    format!("client:{}:{url}", appdata_key(config))
}

/// Stable short id of the appdata dir (FNV-1a over its canonical path), so
/// entries don't collide across dirs and don't spell out the path. Stable
/// across Rust versions, unlike `DefaultHasher`.
fn appdata_key(config: &ConfigService) -> String {
    let path =
        std::fs::canonicalize(config.appdata()).unwrap_or_else(|_| config.appdata().to_path_buf());
    let mut text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Write `token` and read it back, so a backend that silently drops writes
/// is caught before anything (like the config file's copy) is removed.
fn store_verified(account: &str, token: &str) -> Result<()> {
    let s = store();
    s.set(account, token).map_err(SolxError::Config)?;
    match s.get(account).map_err(SolxError::Config)? {
        Some(read) if read == token => Ok(()),
        _ => Err(SolxError::Config(format!(
            "credential manager did not return the token just stored under {account}"
        ))),
    }
}

fn warn_keyring_unavailable(error: &str) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        eprintln!(
            "solx: OS credential manager unavailable ({error}); keeping the bearer token in \
             plaintext in solx-config.json. Set \"token_store\": \"config\" to silence this."
        );
    });
}

/// A place secrets can be kept, by account name under [`KEYRING_SERVICE`].
trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> std::result::Result<Option<String>, String>;
    fn set(&self, account: &str, secret: &str) -> std::result::Result<(), String>;
    fn delete(&self, account: &str) -> std::result::Result<(), String>;
}

/// Set by [`use_memory_store_for_tests`].
#[cfg_attr(test, allow(dead_code))]
static USE_MEMORY_STORE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Keep tokens in a process-local map instead of the OS credential manager,
/// for the rest of this process. **Tests only**: call it before building an
/// `App` so a test run never writes to the real credential manager. A
/// runtime switch rather than a cargo feature on purpose: a dev-dependency
/// feature would also be compiled into the regular binaries `cargo test`
/// builds, which would then silently lose the real token.
#[doc(hidden)]
pub fn use_memory_store_for_tests() {
    USE_MEMORY_STORE.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(not(test))]
fn store() -> &'static dyn SecretStore {
    static MEMORY: std::sync::OnceLock<MemoryStore> = std::sync::OnceLock::new();
    if USE_MEMORY_STORE.load(std::sync::atomic::Ordering::SeqCst) {
        MEMORY.get_or_init(MemoryStore::default)
    } else {
        &KeyringStore
    }
}

#[cfg_attr(test, allow(dead_code))]
struct KeyringStore;

#[cfg_attr(test, allow(dead_code))]
impl KeyringStore {
    fn entry(account: &str) -> std::result::Result<keyring::Entry, String> {
        keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> std::result::Result<Option<String>, String> {
        match Self::entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, account: &str, secret: &str) -> std::result::Result<(), String> {
        Self::entry(account)?
            .set_password(secret)
            .map_err(|e| e.to_string())
    }

    fn delete(&self, account: &str) -> std::result::Result<(), String> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Default)]
struct MemoryStore {
    entries: std::sync::Mutex<std::collections::HashMap<String, String>>,
    unavailable: std::sync::atomic::AtomicBool,
}

impl MemoryStore {
    fn check(&self) -> std::result::Result<(), String> {
        if self.unavailable.load(std::sync::atomic::Ordering::SeqCst) {
            Err("no credential manager (test)".into())
        } else {
            Ok(())
        }
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> std::result::Result<Option<String>, String> {
        self.check()?;
        Ok(self.entries.lock().unwrap().get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> std::result::Result<(), String> {
        self.check()?;
        self.entries
            .lock()
            .unwrap()
            .insert(account.into(), secret.into());
        Ok(())
    }

    fn delete(&self, account: &str) -> std::result::Result<(), String> {
        self.check()?;
        self.entries.lock().unwrap().remove(account);
        Ok(())
    }
}

/// This crate's own tests get a store per thread, so a test can make it
/// unavailable without affecting the others running in parallel.
#[cfg(test)]
fn store() -> &'static dyn SecretStore {
    test_store()
}

#[cfg(test)]
fn test_store() -> &'static MemoryStore {
    thread_local! {
        static STORE: &'static MemoryStore = Box::leak(Box::default());
    }
    STORE.with(|s| *s)
}

// ── the default authority ────────────────────────────────────────────────────

/// One shared token for everything: every valid bearer is
/// [`Principal::Root`], and every external action is handed the same token.
/// This is solx's original trust model, unchanged.
pub struct SingleTokenAuthority {
    token: String,
    server_url: RwLock<Option<String>>,
}

impl SingleTokenAuthority {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            server_url: RwLock::new(None),
        }
    }

    /// Built from [`root_token`], generating it if this appdata dir has none.
    ///
    /// Until the host reports its own bound address (`set_server_url`),
    /// action credentials point at [`sibling_server_url`]: a process that
    /// serves no HTTP itself (stdio `solx-mcp`, or a host whose embedded
    /// server is off or failed to bind) still hands its Command actions the
    /// `solx-server` a sibling host on this appdata dir would run, which
    /// accepts this same root token.
    pub fn from_config(config: &ConfigService) -> Result<Self> {
        let authority = Self::new(root_token(config)?);
        authority.set_server_url(sibling_server_url(config));
        Ok(authority)
    }
}

/// Where a `solx-server` for this appdata dir listens, as reached from this
/// machine: `server_host`/`server_port`, with an unset or unspecified host
/// (`0.0.0.0`/`::`) reached over loopback.
fn sibling_server_url(config: &ConfigService) -> String {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    let snap = config.snapshot();
    let port = snap.server_port.unwrap_or(crate::DEFAULT_SERVER_PORT);
    let host = match snap.server_host.as_deref().map(str::parse::<IpAddr>) {
        Some(Ok(IpAddr::V4(ip))) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        Some(Ok(IpAddr::V6(ip))) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        Some(Ok(ip)) => ip,
        _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    format!("http://{}", SocketAddr::new(host, port))
}

#[async_trait]
impl TokenAuthority for SingleTokenAuthority {
    async fn authenticate(&self, bearer: &str) -> Option<Principal> {
        constant_time_eq(bearer.as_bytes(), self.token.as_bytes()).then_some(Principal::Root)
    }

    async fn issue_action_token(
        &self,
        _action_ref: &str,
        _invocation_id: &str,
    ) -> Option<ActionCredential> {
        let server_url = self.server_url.read().ok().and_then(|u| u.clone());
        Some(ActionCredential::new(server_url, self.token.clone()))
    }

    fn set_server_url(&self, url: String) {
        if let Ok(mut slot) = self.server_url.write() {
            *slot = Some(url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> (tempfile::TempDir, ConfigService) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        (dir, cfg)
    }

    fn set_unavailable(unavailable: bool) {
        test_store()
            .unavailable
            .store(unavailable, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn root_token_is_generated_once_and_kept_out_of_the_file() {
        let (_d, cfg) = config();
        let token = root_token(&cfg).unwrap();
        assert!(!token.is_empty());
        assert_eq!(root_token(&cfg).unwrap(), token);
        assert_eq!(cfg.snapshot().server_token, None);
        assert_eq!(root_token_location(&cfg), "keyring");
    }

    #[test]
    fn legacy_token_is_migrated_then_removed_from_the_file() {
        let (_d, cfg) = config();
        cfg.set(LEGACY_KEY, "legacy".into()).unwrap();
        assert_eq!(root_token(&cfg).unwrap(), "legacy");
        assert_eq!(cfg.snapshot().server_token, None);
        assert_eq!(
            store().get(&root_account(&cfg)).unwrap().as_deref(),
            Some("legacy")
        );
    }

    #[test]
    fn appdata_dirs_get_separate_root_tokens() {
        let (_a, a) = config();
        let (_b, b) = config();
        assert_ne!(root_token(&a).unwrap(), root_token(&b).unwrap());
    }

    #[test]
    fn unavailable_keyring_falls_back_to_the_file() {
        let (_d, cfg) = config();
        cfg.set(LEGACY_KEY, "legacy".into()).unwrap();
        set_unavailable(true);
        let token = root_token(&cfg);
        set_unavailable(false);
        assert_eq!(token.unwrap(), "legacy");
        // Nothing was removed, since nothing was stored elsewhere.
        assert_eq!(cfg.snapshot().server_token.as_deref(), Some("legacy"));
        assert_eq!(root_token_location(&cfg), "config");
    }

    #[test]
    fn config_token_store_keeps_using_the_file() {
        let (_d, cfg) = config();
        cfg.set("token_store", "config".into()).unwrap();
        let token = root_token(&cfg).unwrap();
        assert_eq!(cfg.snapshot().server_token.as_deref(), Some(token.as_str()));
        assert_eq!(store().get(&root_account(&cfg)).unwrap(), None);
    }

    #[test]
    fn rotate_replaces_the_root_token() {
        let (_d, cfg) = config();
        let old = root_token(&cfg).unwrap();
        let new = rotate_root_token(&cfg).unwrap();
        assert_ne!(old, new);
        assert_eq!(root_token(&cfg).unwrap(), new);
        assert_eq!(cfg.snapshot().server_token, None);
    }

    /// A rotation that could only reach the file must survive the
    /// credential manager coming back, not be replaced by the old token.
    #[test]
    fn rotation_during_an_outage_is_not_undone() {
        let (_d, cfg) = config();
        let old = root_token(&cfg).unwrap();
        set_unavailable(true);
        let new = rotate_root_token(&cfg);
        set_unavailable(false);
        let new = new.unwrap();
        assert_ne!(old, new);

        assert_eq!(root_token(&cfg).unwrap(), new);
        assert_eq!(cfg.snapshot().server_token, None);
        assert_eq!(
            store().get(&root_account(&cfg)).unwrap().as_deref(),
            Some(new.as_str())
        );
    }

    /// A token minted in the file during an outage is the one the host
    /// served with, so it is kept once the credential manager is back.
    #[test]
    fn token_minted_during_an_outage_is_kept() {
        let (_d, cfg) = config();
        let original = root_token(&cfg).unwrap();
        set_unavailable(true);
        let fallback = root_token(&cfg);
        set_unavailable(false);
        let fallback = fallback.unwrap();
        assert_ne!(original, fallback);

        assert_eq!(root_token(&cfg).unwrap(), fallback);
        assert_eq!(root_token_location(&cfg), "keyring");
    }

    #[test]
    fn client_token_prefers_its_own_entry_then_the_root_token() {
        let (_d, cfg) = config();
        let url = "http://127.0.0.1:8766";
        assert_eq!(stored_client_token(&cfg, url), None);

        let root = root_token(&cfg).unwrap();
        assert_eq!(
            stored_client_token(&cfg, url).as_deref(),
            Some(root.as_str())
        );

        set_client_token(&cfg, "http://127.0.0.1:8766/", "client-tok").unwrap();
        assert_eq!(
            stored_client_token(&cfg, url).as_deref(),
            Some("client-tok")
        );

        forget_client_token(&cfg, url).unwrap();
        assert_eq!(
            stored_client_token(&cfg, url).as_deref(),
            Some(root.as_str())
        );
    }

    #[test]
    fn root_token_is_never_offered_to_another_host() {
        let (_d, cfg) = config();
        root_token(&cfg).unwrap();
        for url in [
            "http://10.0.0.5:8766",
            "https://solx.example.com",
            "not a url",
        ] {
            assert_eq!(stored_client_token(&cfg, url), None, "{url}");
        }
        assert!(stored_client_token(&cfg, "http://localhost:8766").is_some());
        assert!(stored_client_token(&cfg, "http://[::1]:8766").is_some());

        set_client_token(&cfg, "http://10.0.0.5:8766", "remote-tok").unwrap();
        assert_eq!(
            stored_client_token(&cfg, "http://10.0.0.5:8766").as_deref(),
            Some("remote-tok")
        );
    }

    #[test]
    fn a_broken_config_file_is_an_error_not_a_fallback() {
        let (dir, cfg) = config();
        std::fs::write(dir.path().join("solx-config.json"), "[1, 2]").unwrap();
        // Forces the slow (locked) path, where the file is read raw.
        let err = root_token(&cfg).unwrap_err();
        assert!(err.to_string().contains("JSON object"), "{err}");
    }

    #[test]
    fn adopting_a_legacy_client_token_moves_it_out_of_the_file() {
        let (_d, cfg) = config();
        let url = "http://10.0.0.5:8766";
        cfg.set(LEGACY_KEY, "pasted".into()).unwrap();
        adopt_legacy_client_token(&cfg, url).unwrap();
        assert_eq!(cfg.snapshot().server_token, None);
        assert_eq!(stored_client_token(&cfg, url).as_deref(), Some("pasted"));
    }

    #[test]
    fn invalid_token_store_is_rejected() {
        let err = crate::validate(&serde_json::json!({ "token_store": "vault" })).unwrap_err();
        assert!(err.to_string().contains("token_store"), "{err}");
    }

    #[tokio::test]
    async fn single_token_authority_accepts_only_its_token() {
        let auth = SingleTokenAuthority::new("secret");
        assert_eq!(auth.authenticate("secret").await, Some(Principal::Root));
        assert_eq!(auth.authenticate("secreT").await, None);
        assert_eq!(auth.authenticate("").await, None);
    }

    #[tokio::test]
    async fn action_credential_carries_token_and_bound_url() {
        let auth = SingleTokenAuthority::new("secret");
        let cred = auth.issue_action_token("/a", "inv").await.unwrap();
        assert_eq!(cred.token, "secret");
        assert_eq!(cred.server_url, None);

        auth.set_server_url("http://127.0.0.1:1".into());
        let cred = auth.issue_action_token("/a", "inv").await.unwrap();
        assert_eq!(cred.server_url.as_deref(), Some("http://127.0.0.1:1"));
    }

    /// A process serving no HTTP of its own (stdio `solx-mcp`) still gives
    /// its Command actions a URL: the sibling host's, until it binds one.
    #[tokio::test]
    async fn from_config_defaults_to_the_sibling_server_url() {
        async fn url(auth: &SingleTokenAuthority) -> Option<String> {
            auth.issue_action_token("/a", "inv").await.unwrap().server_url
        }
        let (_d, cfg) = config();

        let auth = SingleTokenAuthority::from_config(&cfg).unwrap();
        assert_eq!(url(&auth).await.as_deref(), Some("http://127.0.0.1:8766"));
        auth.set_server_url("http://127.0.0.1:1".into());
        assert_eq!(url(&auth).await.as_deref(), Some("http://127.0.0.1:1"));

        cfg.set("server_port", 9000.into()).unwrap();
        cfg.set("server_host", "0.0.0.0".into()).unwrap();
        assert_eq!(sibling_server_url(&cfg), "http://127.0.0.1:9000");
        cfg.set("server_host", "::".into()).unwrap();
        assert_eq!(sibling_server_url(&cfg), "http://[::1]:9000");
        cfg.set("server_host", "10.0.0.5".into()).unwrap();
        assert_eq!(sibling_server_url(&cfg), "http://10.0.0.5:9000");
    }
}
