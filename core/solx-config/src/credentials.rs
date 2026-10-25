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
    // A `#pin=` doesn't make it another server.
    let url = server_url.split('#').next().unwrap_or_default();
    let url = url.trim().trim_end_matches('/');
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

/// Where an authority's action credentials point: [`sibling_server_url`]
/// until the host reports its own bound address via `set_server_url`.
/// Shared by [`SingleTokenAuthority`] and the token registry.
pub struct ServerUrlSlot(RwLock<Option<String>>);

impl ServerUrlSlot {
    pub fn new(initial: Option<String>) -> Self {
        Self(RwLock::new(initial))
    }

    pub fn get(&self) -> Option<String> {
        self.0.read().ok().and_then(|u| u.clone())
    }

    pub fn set(&self, url: String) {
        if let Ok(mut slot) = self.0.write() {
            *slot = Some(url);
        }
    }
}

/// One shared token for everything: every valid bearer is
/// [`Principal::Root`], and every external action is handed the same token.
/// This is solx's original trust model, unchanged.
pub struct SingleTokenAuthority {
    token: String,
    server_url: ServerUrlSlot,
}

impl SingleTokenAuthority {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            server_url: ServerUrlSlot::new(None),
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
pub fn sibling_server_url(config: &ConfigService) -> String {
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
        _ttl: std::time::Duration,
    ) -> Option<ActionCredential> {
        Some(ActionCredential::new(
            self.server_url.get(),
            self.token.clone(),
        ))
    }

    fn set_server_url(&self, url: String) {
        self.server_url.set(url);
    }
}

// ── connect input ────────────────────────────────────────────────────────────

/// What a person typed or pasted into the one "code, link or token" field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectCredential {
    /// An 8-digit connect code, digits only, to redeem at `/auth/exchange`.
    Code(String),
    /// A bearer token, used as entered (trimmed).
    Token(String),
}

/// [`parse_connect_input`]'s result: the credential plus the server it is
/// for, when one is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectInput {
    pub credential: ConnectCredential,
    /// Without a `#pin=`; that is in `pin`.
    pub server_url: Option<String>,
    /// The server's root pin, from the link or the server address.
    pub pin: Option<String>,
}

/// Classify the single connect field, shared by the CLI and the desktop
/// client (`web/web/src/api/connectInput.ts` is the TypeScript twin, with
/// the same test cases):
/// - a link containing `#code=` is a code, and its origin becomes the
///   server URL when `server_url` is empty. Its `&pin=`, if any, is the
///   server's root pin;
/// - exactly 8 digits once spaces and dashes are removed is a code;
/// - anything else non-empty is a token, trimmed.
///
/// `server_url` may carry a pin too (`https://host:8766#pin=…`). A pin in
/// both places must be the same pin.
///
/// Tokens are 44-character base64, never exactly 8 digits, so this can't
/// be ambiguous. A malformed code (7 digits, say) comes back as a token and
/// fails verification like any other wrong token.
pub fn parse_connect_input(server_url: Option<&str>, input: &str) -> Result<ConnectInput> {
    let (given, given_pin) = match server_url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => {
            let (base, pin) = split_server_pin(url)?;
            (Some(base), pin)
        }
        None => (None, None),
    };
    let input = input.trim();
    if input.is_empty() {
        return Err(SolxError::Invalid(
            "enter a connect code, a connect link or a token".into(),
        ));
    }
    if let Some((before, fragment)) = input.split_once("#code=") {
        let (code, rest) = fragment.split_once('&').unwrap_or((fragment, ""));
        let digits = connect_code_digits(code)
            .ok_or_else(|| SolxError::Invalid("the connect link's code is not 8 digits".into()))?;
        let link_pin = pin_param(rest)?;
        let pin = match (given_pin, link_pin) {
            (Some(a), Some(b)) if a != b => {
                return Err(SolxError::Invalid(
                    "the server address and the connect link carry different pins".into(),
                ))
            }
            (a, b) => a.or(b),
        };
        let server_url = given.or_else(|| connect_link_base(before));
        if let Some(url) = &server_url {
            require_https_for_pin(url, pin.as_deref())?;
        }
        return Ok(ConnectInput {
            credential: ConnectCredential::Code(digits),
            server_url,
            pin,
        });
    }
    let credential = match connect_code_digits(input) {
        Some(digits) => ConnectCredential::Code(digits),
        None => ConnectCredential::Token(input.to_string()),
    };
    Ok(ConnectInput {
        credential,
        server_url: given,
        pin: given_pin,
    })
}

// ── pins ─────────────────────────────────────────────────────────────────────
//
// A pin is `base64url(SHA-256(root SubjectPublicKeyInfo))`, unpadded: the
// root a server's TLS chain must end in (see `docs/tls-plan.md`). It
// travels as a URL fragment, `https://host:8766#pin=…`, in connect links,
// in `server_url` and in `SOLX_SERVER_URL`, so a pinned client stores it
// wherever it stores the URL. A fragment is never sent to the server.

/// Whether `pin` looks like a pin: 43 base64url characters (32 bytes).
pub fn is_pin(pin: &str) -> bool {
    pin.len() == 43
        && base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(pin)
            .is_ok_and(|bytes| bytes.len() == 32)
}

/// `server_url` without its fragment and trailing slash, and the `pin` in
/// its fragment, if any. A fragment's other parameters are dropped; a
/// malformed pin, or a pin on anything but `https://`, is an error, never
/// ignored.
pub fn split_server_pin(server_url: &str) -> Result<(String, Option<String>)> {
    let (base, fragment) = server_url
        .trim()
        .split_once('#')
        .unwrap_or((server_url.trim(), ""));
    let base = base.trim_end_matches('/').to_string();
    let pin = pin_param(fragment)?;
    require_https_for_pin(&base, pin.as_deref())?;
    Ok((base, pin))
}

/// A pin says the server must prove itself over TLS. Over `http://` it
/// would protect nothing, and a code or token sent there travels in
/// cleartext, so refuse rather than ignore it.
fn require_https_for_pin(server_url: &str, pin: Option<&str>) -> Result<()> {
    let https = server_url
        .get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
    if pin.is_some() && !https {
        return Err(SolxError::Invalid(format!(
            "{server_url} carries a server pin, but a pinned server must be reached over https://"
        )));
    }
    Ok(())
}

/// `server_url` (any fragment dropped) with `#pin=<pin>` when there is a
/// pin.
pub fn with_server_pin(server_url: &str, pin: Option<&str>) -> String {
    let base = server_url.split('#').next().unwrap_or_default();
    let base = base.trim().trim_end_matches('/');
    match pin {
        Some(pin) => format!("{base}#pin={pin}"),
        None => base.to_string(),
    }
}

/// The `pin` among `&`-separated `key=value` fragment parameters.
fn pin_param(fragment: &str) -> Result<Option<String>> {
    let Some(pin) = fragment
        .split('&')
        .find_map(|param| param.strip_prefix("pin="))
    else {
        return Ok(None);
    };
    let pin = pin.trim();
    if !is_pin(pin) {
        return Err(SolxError::Invalid(format!(
            "{pin:?} is not a server pin (43 base64url characters)"
        )));
    }
    Ok(Some(pin.to_string()))
}

/// The server a connect link (`<public url>/connect`, fragment already cut
/// off) points at: the link minus its `/connect`, so a public URL under a
/// reverse-proxy prefix (`https://example.com/solx`) keeps the prefix.
fn connect_link_base(link: &str) -> Option<String> {
    let mut url = url::Url::parse(link).ok().filter(|u| u.has_host())?;
    url.set_query(None);
    let path = url.path().trim_end_matches('/');
    let path = path.strip_suffix("/connect").unwrap_or(path).to_string();
    url.set_path(&path);
    Some(url.as_str().trim_end_matches('/').to_string())
}

/// The 8 digits of a connect code typed as `12345678`, `1234-5678` or
/// `1234 5678`, or `None` if `input` is anything else.
pub fn connect_code_digits(input: &str) -> Option<String> {
    let input = input.trim();
    if !input
        .chars()
        .all(|c| c.is_ascii_digit() || c == '-' || c == ' ')
    {
        return None;
    }
    let digits: String = input.chars().filter(char::is_ascii_digit).collect();
    (digits.len() == 8).then_some(digits)
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
        let ttl = std::time::Duration::from_secs(60);
        let cred = auth.issue_action_token("/a", "inv", ttl).await.unwrap();
        assert_eq!(cred.token, "secret");
        assert_eq!(cred.server_url, None);

        auth.set_server_url("http://127.0.0.1:1".into());
        let cred = auth.issue_action_token("/a", "inv", ttl).await.unwrap();
        assert_eq!(cred.server_url.as_deref(), Some("http://127.0.0.1:1"));
    }

    /// A process serving no HTTP of its own (stdio `solx-mcp`) still gives
    /// its Command actions a URL: the sibling host's, until it binds one.
    #[tokio::test]
    async fn from_config_defaults_to_the_sibling_server_url() {
        async fn url(auth: &SingleTokenAuthority) -> Option<String> {
            let ttl = std::time::Duration::from_secs(60);
            auth.issue_action_token("/a", "inv", ttl)
                .await
                .unwrap()
                .server_url
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

    fn code(digits: &str) -> ConnectCredential {
        ConnectCredential::Code(digits.into())
    }

    #[test]
    fn digits_with_spaces_or_dashes_are_codes() {
        for input in ["12345678", "1234-5678", " 1234 5678 "] {
            let parsed = parse_connect_input(Some("http://10.0.0.5:8766"), input).unwrap();
            assert_eq!(parsed.credential, code("12345678"), "{input:?}");
            assert_eq!(parsed.server_url.as_deref(), Some("http://10.0.0.5:8766"));
        }
    }

    #[test]
    fn a_connect_link_is_a_code_and_supplies_the_server() {
        let link = "http://10.0.0.5:8766/connect#code=12345678";
        let parsed = parse_connect_input(None, link).unwrap();
        assert_eq!(parsed.credential, code("12345678"));
        assert_eq!(parsed.server_url.as_deref(), Some("http://10.0.0.5:8766"));

        // A reverse-proxy prefix in the public URL survives.
        let parsed =
            parse_connect_input(None, "https://example.com/solx/connect#code=12345678").unwrap();
        assert_eq!(
            parsed.server_url.as_deref(),
            Some("https://example.com/solx")
        );

        // An explicit server wins over the link's origin.
        let parsed = parse_connect_input(Some("https://solx.example.com/"), link).unwrap();
        assert_eq!(
            parsed.server_url.as_deref(),
            Some("https://solx.example.com")
        );

        assert!(parse_connect_input(None, "http://h/connect#code=123").is_err());
    }

    const PIN: &str = "q83vASNFZ4mrze8BI0VniavN7wEjRWeJq83vASNFZ4k";

    #[test]
    fn a_pin_comes_from_the_link_or_the_server_address() {
        let link = format!("https://10.0.0.5:8766/connect#code=12345678&pin={PIN}");
        let parsed = parse_connect_input(None, &link).unwrap();
        assert_eq!(parsed.credential, code("12345678"));
        assert_eq!(parsed.server_url.as_deref(), Some("https://10.0.0.5:8766"));
        assert_eq!(parsed.pin.as_deref(), Some(PIN));

        let address = format!("https://10.0.0.5:8766/#pin={PIN}");
        let parsed = parse_connect_input(Some(&address), "1234-5678").unwrap();
        assert_eq!(parsed.server_url.as_deref(), Some("https://10.0.0.5:8766"));
        assert_eq!(parsed.pin.as_deref(), Some(PIN));
        let parsed = parse_connect_input(Some(&address), &generate_token()).unwrap();
        assert_eq!(parsed.pin.as_deref(), Some(PIN));

        // The same pin twice is fine; a different one, or a malformed one,
        // is refused.
        assert!(parse_connect_input(Some(&address), &link).is_ok());
        let other = format!("https://h#pin={}", "A".repeat(43));
        assert!(parse_connect_input(Some(&other), &link).is_err());
        assert!(parse_connect_input(None, "https://h/connect#code=12345678&pin=abc").is_err());
        assert!(parse_connect_input(Some("https://h#pin=abc"), "12345678").is_err());

        // A pin never rides on plain HTTP, from the address or the link.
        assert!(parse_connect_input(Some(&format!("http://h#pin={PIN}")), "12345678").is_err());
        let plain_link = format!("http://h:8766/connect#code=12345678&pin={PIN}");
        assert!(parse_connect_input(None, &plain_link).is_err());
        assert!(parse_connect_input(Some("http://h:8766"), &link).is_err());

        assert_eq!(parse_connect_input(None, "12345678").unwrap().pin, None);
    }

    #[test]
    fn pins_ride_on_the_server_url() {
        let pinned = with_server_pin("https://h:8766/", Some(PIN));
        assert_eq!(pinned, format!("https://h:8766#pin={PIN}"));
        assert_eq!(
            split_server_pin(&pinned).unwrap(),
            ("https://h:8766".to_string(), Some(PIN.to_string()))
        );
        assert_eq!(with_server_pin(&pinned, None), "https://h:8766");
        assert_eq!(
            split_server_pin("http://h").unwrap(),
            ("http://h".to_string(), None)
        );

        // The token saved for a server is found with or without its pin.
        let (_dir, cfg) = config();
        set_client_token(&cfg, "https://h:8766", "tok").unwrap();
        assert_eq!(
            stored_client_token(&cfg, &pinned).as_deref(),
            Some("tok")
        );
    }

    #[test]
    fn a_generated_token_is_a_token() {
        let token = generate_token();
        assert_eq!(token.len(), 44);
        let parsed = parse_connect_input(None, &format!(" {token} ")).unwrap();
        assert_eq!(parsed.credential, ConnectCredential::Token(token));
        assert_eq!(parsed.server_url, None);
    }

    #[test]
    fn seven_digits_are_a_token_and_empty_input_is_refused() {
        let parsed = parse_connect_input(None, "1234567").unwrap();
        assert_eq!(
            parsed.credential,
            ConnectCredential::Token("1234567".into())
        );
        assert!(parse_connect_input(None, "   ").is_err());
    }
}
