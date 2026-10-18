//! Secret retrieval layer.
//!
//! Centralises three ways a value referenced from `action_config.auth`
//! can be sourced at call time:
//!
//! | Shape                                      | Source                                          |
//! |--------------------------------------------|--------------------------------------------------|
//! | Plain `String`                              | Inline in `action_config.auth` (existing)       |
//! | `{"keyring_service": …, "keyring_account": …}` | OS credential manager via the `keyring` crate |
//! | String env-var *name* (`*_env` field)       | In-process WASM env store                       |
//!
//! ## Why a separate module?
//!
//! The dispatcher needs to read the same kind of secret in three places
//! (`exchange_refresh_token`, `exchange_service_account_token`, and
//! `dispatch_oauth_token_exchange`). Centralising the resolution keeps
//! the precedence rules and the keyring-failure semantics in one spot,
//! and gives us a place to add caching if we ever need it.
//!
//! ## Failure model
//!
//! The `resolve` function returns `Err(String)` with a human-readable
//! message on any failure. The error is propagated all the way back
//! through `dispatch_webhook` into the action result; callers see the
//! provider-agnostic message rather than the raw `keyring::Error`.
//!
//! ## Test seams
//!
//! Tests inject a fake loader via [`set-secret_loader_for_tests`] so
//! they do not require a real OS credential manager (and so they can
//! cover the keyring-failure path deterministically). The default
//! loader is the real `keyring::Entry::get_password`.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use solx_surface::SecretHandle;

/// Alternate shape accepted by [`resolve_str`] and friends.
///
/// `keyring_service` is a per-app namespace (conventionally
/// `"sol-manager"`); `keyring_account` is the unique identifier within
/// that namespace (typically the action name, or some random ID for a
/// one-off secret).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct KeyringRef {
    pub keyring_service: String,
    pub keyring_account: String,
}

/// Inspect a JSON value and decide how to resolve it.
///
/// * `Value::String(s)`        → inline value (return immediately)
/// * `Value::Object({..})`     → expect `keyring_service` + `keyring_account`
/// * anything else              → treat as `Err`
///
/// The `*_env` indirection is handled *outside* this module by the
/// caller; `resolve` itself is the final hop.
pub async fn resolve(value: &Value) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Object(_) => {
            let r: KeyringRef = serde_json::from_value(value.clone()).map_err(|e| {
                format!(
                    "keyring reference must have 'keyring_service' and \
                     'keyring_account' string fields; got: {e}"
                )
            })?;
            load_keyring_with_test_override(&r).await
        }
        Value::Null => Err("secret value is null; provide an inline string, a keyring reference, or a *-env pointer".to_string()),
        other => Err(format!(
            "secret value has unsupported JSON type {}; expected string or object",
            type_name(other)
        )),
    }
}

fn type_name(v: &Value) -> &'static str {
    type_name_of(v)
}

/// Public re-export of [`type_name`] so callers (e.g. `webhook_auth`)
/// can describe the shape of a malformed auth field without depending
/// on the private helper.
pub fn type_name_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Resolve a string-or-keyring-ref to a `String`. Convenience wrapper
/// around [`resolve`] for the common "read one field out of auth"
/// case.
///
/// `field_name` is used to build a helpful error message ("auth's
/// 'private_key' is …") so callers don't need to interpolate.
pub async fn resolve_in_field(
    parent: &str,
    field_name: &str,
    value: &Value,
) -> Result<String, String> {
    resolve(value)
        .await
        .map_err(|e| format!("{parent}'s '{field_name}' {e}"))
}

/// Load a secret from the OS credential manager via the `keyring`
/// crate. Errors are converted to human-readable strings here so the
/// dispatcher doesn't need to know about `keyring::Error`.
///
/// On platforms where `keyring::Entry::get_password` returns
/// `NoEntry`, the error is "no entry under service/account"; on
/// permission-denied it's "access denied by credential manager".
pub fn load_keyring(r: &KeyringRef) -> Result<String, String> {
    if r.keyring_service.is_empty() || r.keyring_account.is_empty() {
        return Err(format!(
            "keyring reference requires non-empty 'keyring_service' \
             and 'keyring_account' (got service='{}', account='{}')",
            r.keyring_service, r.keyring_account
        ));
    }
    let entry = keyring::Entry::new(&r.keyring_service, &r.keyring_account).map_err(|e| {
        format!(
            "keyring entry '{}/{}' open failed: {e}",
            r.keyring_service, r.keyring_account
        )
    })?;
    entry.get_password().map_err(|e| {
        let service = &r.keyring_service;
        let account = &r.keyring_account;
        format!(
            "keyring lookup '{service}/{account}' failed: {e} \
             (this commonly means the entry was never stored -- set it with \
             `cmdkey /generic:{service}\\{account} /user:ignored /pass:\"<value>\"` on Windows, \
             `security add-generic-password -s {service} -a {account} -w \"<value>\"` on macOS, \
             or `secret-tool store service {service} account {account}` on Linux)"
        )
    })
}

// ── Test seam ────────────────────────────────────────────────────────────────

/// Type of the test-only secret loader. Receives the same `KeyringRef`
/// the real loader would receive, returns whatever the test wants.
pub type SecretLoaderFn = fn(&KeyringRef) -> Result<String, String>;

static TEST_LOADER: std::sync::OnceLock<std::sync::Mutex<Option<SecretLoaderFn>>> =
    std::sync::OnceLock::new();

fn test_loader_slot() -> &'static std::sync::Mutex<Option<SecretLoaderFn>> {
    TEST_LOADER.get_or_init(|| std::sync::Mutex::new(None))
}

/// Install a fake [`SecretLoaderFn`] for tests. Pass `None` to restore
/// the real `keyring`-backed loader.
pub fn set_secret_loader_for_tests(loader: Option<SecretLoaderFn>) {
    if let Ok(mut g) = test_loader_slot().lock() {
        *g = loader;
    }
}

/// Try the test loader first; fall back to the real implementation.
///
/// The real backend is synchronous OS credential-manager IPC — DPAPI on
/// Windows, Security.framework on macOS, and a **blocking D-Bus round trip**
/// to gnome-keyring/KWallet on Linux, which can stall for seconds or hang
/// outright on a locked keyring. It therefore runs on the blocking pool.
/// The test seam stays synchronous so the existing `fn`-pointer fakes work
/// unchanged.
pub async fn load_keyring_with_test_override(r: &KeyringRef) -> Result<String, String> {
    if let Ok(g) = test_loader_slot().lock() {
        if let Some(loader) = *g {
            return loader(r);
        }
    }
    let owned = r.clone();
    tokio::task::spawn_blocking(move || load_keyring(&owned))
        .await
        .map_err(|e| format!("keyring task panicked: {e}"))?
}

// ── Scoped secret store ──────────────────────────────────────────────────────
//
// Unlike `resolve`/`KeyringRef` above (which serve keyring-ref `auth`
// fields), this store backs the `get-secret`/`set-secret` built-ins and
// the `*_secret` auth fields. Every value is encrypted with its scope's
// 32-byte key before it is written to the OS credential manager, and that
// key lives in the actions database (`secret_keys`), never beside it — so
// reading a secret back out needs both the keyring entry *and* the key.
// Entries are addressed by a `SecretHandle`, whose account is
// `<scope>/<name>`: two scopes never share an entry, so one package can't
// read, overwrite or (through self-heal) delete another's secret. The host
// never hands a key to a guest; it only resolves one for the action that
// is currently calling in.

const SECRETS_KEYRING_SERVICE: &str = "sol-secrets";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// A fresh base64 AES-256 key for a secret scope, from the OS RNG.
pub(crate) fn generate_key() -> String {
    base64::engine::general_purpose::STANDARD.encode(Aes256Gcm::generate_key(&mut OsRng))
}

/// Decode a base64 key and enforce it is exactly 32 bytes (AES-256).
pub(crate) fn decode_key(key_b64: &str) -> Result<[u8; KEY_LEN], String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(key_b64)
        .map_err(|e| format!("secret key is not valid base64: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("secret key must decode to {KEY_LEN} bytes, got {}", v.len()))
}

fn encrypt(value: &str, key_b64: &str) -> Result<String, String> {
    let key_bytes = decode_key(key_b64)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, value.as_bytes())
        .map_err(|e| format!("secret encryption failed: {e}"))?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ciphertext);
    Ok(base64::engine::general_purpose::STANDARD.encode(blob))
}

/// Decrypt `blob_b64` with an already-decoded key. Any failure here means
/// the *entry* itself can't be trusted (wrong key or corrupted blob), as
/// opposed to a malformed `key_b64` argument, which `decode_key` rejects
/// before this is ever called.
fn decrypt_with_key(blob_b64: &str, key_bytes: &[u8; KEY_LEN]) -> Result<String, String> {
    let blob = base64::engine::general_purpose::STANDARD
        .decode(blob_b64)
        .map_err(|e| format!("stored secret is not valid base64: {e}"))?;
    if blob.len() < NONCE_LEN {
        return Err("stored secret is too short to contain a nonce".to_string());
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key_bytes));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| "secret decryption failed (wrong key or corrupted entry)".to_string())?;
    String::from_utf8(plaintext).map_err(|e| format!("decrypted secret is not valid UTF-8: {e}"))
}

/// Encrypt `value` with the handle's key and write it to the OS credential
/// manager under the handle's account.
pub async fn set_secret(secret: &SecretHandle, value: &str) -> Result<(), String> {
    // AES-GCM stays inline: hardware-accelerated and microseconds on
    // secret-sized payloads. Only the keyring hop is worth offloading.
    let blob = encrypt(value, secret.key())?;
    raw_set(secret.account(), &blob).await
}

/// Read the value stored for `secret`, decrypting with its key. Returns
/// `Ok(None)` if no entry exists. Also returns `Ok(None)` — after clearing
/// the entry — if one exists but can't be decrypted (wrong key or
/// corrupted blob): a self-heal so a scope whose key was regenerated isn't
/// stuck behind a dead entry, it just goes through its normal "nothing
/// persisted yet" path again. The entry is the scope's own, so this never
/// touches another scope's secret. Still returns `Err` if the key itself
/// is malformed, since clearing the entry wouldn't fix that.
pub async fn get_secret(secret: &SecretHandle) -> Result<Option<String>, String> {
    let key_bytes = decode_key(secret.key())?;
    let account = secret.account();
    match raw_get(account).await? {
        Some(blob) => match decrypt_with_key(&blob, &key_bytes) {
            Ok(value) => Ok(Some(value)),
            Err(e) => {
                tracing::warn!(
                    "secret '{account}' could not be decrypted ({e}); clearing the \
                     corrupted entry so it can be re-created"
                );
                raw_delete(account).await?;
                Ok(None)
            }
        },
        None => Ok(None),
    }
}

/// Remove the entry stored for `secret`, if any.
pub async fn delete_secret(secret: &SecretHandle) -> Result<(), String> {
    raw_delete(secret.account()).await
}

// ── Raw keyring I/O (test-overridable) ───────────────────────────────────────

fn keyring_entry(name: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SECRETS_KEYRING_SERVICE, name)
        .map_err(|e| format!("keyring entry '{SECRETS_KEYRING_SERVICE}/{name}' open failed: {e}"))
}

fn real_raw_get(name: &str) -> Result<Option<String>, String> {
    match keyring_entry(name)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!(
            "keyring lookup '{SECRETS_KEYRING_SERVICE}/{name}' failed: {e}"
        )),
    }
}

fn real_raw_set(name: &str, blob: &str) -> Result<(), String> {
    keyring_entry(name)?
        .set_password(blob)
        .map_err(|e| format!("keyring write '{SECRETS_KEYRING_SERVICE}/{name}' failed: {e}"))
}

fn real_raw_delete(name: &str) -> Result<(), String> {
    match keyring_entry(name)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!(
            "keyring delete '{SECRETS_KEYRING_SERVICE}/{name}' failed: {e}"
        )),
    }
}

/// Test-only override for the raw keyring backend, so tests can cover
/// the encrypt/decrypt and scoping logic without touching the real OS
/// credential manager. Mirrors the `TEST_LOADER` seam above, but for
/// the name/value store rather than `KeyringRef`.
pub struct SecretBackendOverride {
    pub get: fn(&str) -> Result<Option<String>, String>,
    pub set: fn(&str, &str) -> Result<(), String>,
    pub delete: fn(&str) -> Result<(), String>,
}

static TEST_BACKEND: std::sync::OnceLock<std::sync::Mutex<Option<SecretBackendOverride>>> =
    std::sync::OnceLock::new();

fn test_backend_slot() -> &'static std::sync::Mutex<Option<SecretBackendOverride>> {
    TEST_BACKEND.get_or_init(|| std::sync::Mutex::new(None))
}

/// Install a fake backend for tests. Pass `None` to restore the real
/// keyring-backed implementation.
pub fn set_secret_backend_for_tests(backend: Option<SecretBackendOverride>) {
    if let Ok(mut g) = test_backend_slot().lock() {
        *g = backend;
    }
}

// Each of these checks the (synchronous) test seam first, dropping its
// guard before any await, and otherwise pushes the blocking keyring call
// onto the blocking pool.

async fn raw_get(name: &str) -> Result<Option<String>, String> {
    if let Ok(g) = test_backend_slot().lock() {
        if let Some(b) = g.as_ref() {
            return (b.get)(name);
        }
    }
    let owned = name.to_string();
    tokio::task::spawn_blocking(move || real_raw_get(&owned))
        .await
        .map_err(|e| format!("keyring task panicked: {e}"))?
}

async fn raw_set(name: &str, blob: &str) -> Result<(), String> {
    if let Ok(g) = test_backend_slot().lock() {
        if let Some(b) = g.as_ref() {
            return (b.set)(name, blob);
        }
    }
    let (owned, blob) = (name.to_string(), blob.to_string());
    tokio::task::spawn_blocking(move || real_raw_set(&owned, &blob))
        .await
        .map_err(|e| format!("keyring task panicked: {e}"))?
}

async fn raw_delete(name: &str) -> Result<(), String> {
    if let Ok(g) = test_backend_slot().lock() {
        if let Some(b) = g.as_ref() {
            return (b.delete)(name);
        }
    }
    let owned = name.to_string();
    tokio::task::spawn_blocking(move || real_raw_delete(&owned))
        .await
        .map_err(|e| format!("keyring task panicked: {e}"))?
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;

    #[tokio::test]
    async fn inline_string_resolves_directly() {
        assert_eq!(resolve(&json!("abc")).await.unwrap(), "abc");
    }

    #[tokio::test]
    async fn null_value_errors_with_helpful_message() {
        let err = resolve(&json!(null)).await.unwrap_err();
        assert!(err.contains("secret value is null"), "{err}");
        assert!(err.contains("*-env"), "{err}");
    }

    #[tokio::test]
    async fn number_value_errors_with_type_name() {
        let err = resolve(&json!(42)).await.unwrap_err();
        assert!(err.contains("unsupported JSON type number"), "{err}");
    }

    #[test]
    fn keyring_shape_round_trips() {
        // The shape must serialise to two non-empty strings.
        let v = json!({"keyring_service": "sol", "keyring_account": "google"});
        let r: KeyringRef = serde_json::from_value(v).unwrap();
        assert_eq!(r.keyring_service, "sol");
        assert_eq!(r.keyring_account, "google");
    }

    #[tokio::test]
    async fn malformed_keyring_shape_errors_without_calling_loader() {
        // Single-field object — should not match the KeyringRef shape.
        let err = resolve(&json!({"keyring_service": "sol"}))
            .await
            .unwrap_err();
        assert!(err.contains("missing field"), "{err}");
    }

    #[test]
    fn empty_keyring_fields_rejected_before_loader_runs() {
        let r = KeyringRef {
            keyring_service: "".into(),
            keyring_account: "x".into(),
        };
        let err = load_keyring(&r).unwrap_err();
        assert!(err.contains("non-empty"), "{err}");
    }

    #[tokio::test]
    #[serial(secrets_loader)]
    async fn loader_test_seam_overrides_real_implementation() {
        set_secret_loader_for_tests(Some(|_| Ok("from-test-loader".into())));
        let v = json!({"keyring_service": "sol", "keyring_account": "any"});
        let r = KeyringRef {
            keyring_service: "sol".into(),
            keyring_account: "any".into(),
        };
        assert_eq!(
            load_keyring_with_test_override(&r).await.unwrap(),
            "from-test-loader"
        );
        // Also covers the resolve path.
        assert_eq!(resolve(&v).await.unwrap(), "from-test-loader");
        // Restore.
        set_secret_loader_for_tests(None);
    }

    #[tokio::test]
    #[serial(secrets_loader)]
    async fn loader_test_seam_error_surfaces_in_resolve() {
        set_secret_loader_for_tests(Some(|r| {
            Err(format!(
                "synthetic: {}/{}",
                r.keyring_service, r.keyring_account
            ))
        }));
        let v = json!({"keyring_service": "sol", "keyring_account": "missing"});
        let err = resolve(&v).await.unwrap_err();
        assert!(err.contains("synthetic: sol/missing"), "{err}");
        set_secret_loader_for_tests(None);
    }

    // ── Scoped secret store ──────────────────────────────────────────────────

    /// In-memory fake keyring for the `SecretBackendOverride` seam. Fn
    /// pointers can't capture state, so the map lives in a static.
    static FAKE_STORE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, String>>,
    > = std::sync::OnceLock::new();

    fn fake_store() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
        FAKE_STORE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
    }

    fn install_fake_backend() {
        fake_store().lock().unwrap().clear();
        set_secret_backend_for_tests(Some(SecretBackendOverride {
            get: |name| Ok(fake_store().lock().unwrap().get(name).cloned()),
            set: |name, blob| {
                fake_store()
                    .lock()
                    .unwrap()
                    .insert(name.to_string(), blob.to_string());
                Ok(())
            },
            delete: |name| {
                fake_store().lock().unwrap().remove(name);
                Ok(())
            },
        }));
    }

    fn key_a() -> String {
        base64::engine::general_purpose::STANDARD.encode([1u8; KEY_LEN])
    }

    fn key_b() -> String {
        base64::engine::general_purpose::STANDARD.encode([2u8; KEY_LEN])
    }

    /// `MY_SECRET` in `scope`, opened with `key`.
    fn handle_in(scope: &str, key: String) -> SecretHandle {
        let cfg = json!({ "secrets": ["MY_SECRET"] });
        solx_surface::ActionSecrets::new(scope, Some(&cfg), Some(key))
            .resolve("MY_SECRET")
            .unwrap()
    }

    fn handle(key: String) -> SecretHandle {
        handle_in("package:p", key)
    }

    #[tokio::test]
    #[serial(secrets_backend)]
    async fn set_then_get_secret_round_trips_with_the_right_key() {
        install_fake_backend();
        set_secret(&handle(key_a()), "top-secret-value")
            .await
            .unwrap();
        assert_eq!(
            get_secret(&handle(key_a())).await.unwrap(),
            Some("top-secret-value".to_string())
        );
        set_secret_backend_for_tests(None);
    }

    #[tokio::test]
    #[serial(secrets_backend)]
    async fn get_secret_with_wrong_key_self_heals_by_clearing_entry() {
        install_fake_backend();
        set_secret(&handle(key_a()), "top-secret-value")
            .await
            .unwrap();
        assert_eq!(get_secret(&handle(key_b())).await.unwrap(), None);
        // The corrupted entry was cleared, not just masked -- even the
        // originally-correct key now finds nothing.
        assert_eq!(get_secret(&handle(key_a())).await.unwrap(), None);
        set_secret_backend_for_tests(None);
    }

    #[tokio::test]
    #[serial(secrets_backend)]
    async fn get_secret_with_malformed_key_errors_without_deleting_entry() {
        install_fake_backend();
        set_secret(&handle(key_a()), "top-secret-value")
            .await
            .unwrap();
        let bad_key = base64::engine::general_purpose::STANDARD.encode(b"not-32-bytes");
        let err = get_secret(&handle(bad_key)).await.unwrap_err();
        assert!(err.contains("must decode to"), "{err}");
        // A malformed key argument is a caller config problem, not a
        // corrupted entry -- the stored secret must survive untouched.
        assert_eq!(
            get_secret(&handle(key_a())).await.unwrap(),
            Some("top-secret-value".to_string())
        );
        set_secret_backend_for_tests(None);
    }

    #[tokio::test]
    #[serial(secrets_backend)]
    async fn get_secret_missing_entry_returns_none() {
        install_fake_backend();
        assert_eq!(get_secret(&handle(key_a())).await.unwrap(), None);
        set_secret_backend_for_tests(None);
    }

    #[tokio::test]
    #[serial(secrets_backend)]
    async fn delete_secret_removes_the_entry() {
        install_fake_backend();
        set_secret(&handle(key_a()), "value").await.unwrap();
        delete_secret(&handle(key_a())).await.unwrap();
        assert_eq!(get_secret(&handle(key_a())).await.unwrap(), None);
        set_secret_backend_for_tests(None);
    }

    /// Two scopes using the same name have separate entries: neither reads
    /// the other's value, and one failing to decrypt (self-heal) never
    /// deletes the other's.
    #[tokio::test]
    #[serial(secrets_backend)]
    async fn scopes_sharing_a_name_never_touch_each_other() {
        install_fake_backend();
        let a = handle_in("package:a", key_a());
        let b = handle_in("package:b", key_b());
        assert_eq!(a.account(), "package:a/MY_SECRET");
        set_secret(&a, "for-a").await.unwrap();
        set_secret(&b, "for-b").await.unwrap();
        assert_eq!(get_secret(&a).await.unwrap().as_deref(), Some("for-a"));
        assert_eq!(get_secret(&b).await.unwrap().as_deref(), Some("for-b"));

        // b's key is regenerated: b self-heals, a is untouched.
        let regenerated = handle_in("package:b", key_a());
        assert_eq!(get_secret(&regenerated).await.unwrap(), None);
        assert_eq!(get_secret(&a).await.unwrap().as_deref(), Some("for-a"));
        set_secret_backend_for_tests(None);
    }

    #[test]
    fn generated_keys_are_valid_and_distinct() {
        let (k1, k2) = (generate_key(), generate_key());
        assert!(decode_key(&k1).is_ok());
        assert_ne!(k1, k2);
    }

    #[tokio::test]
    #[serial(secrets_backend)]
    async fn short_key_is_rejected() {
        install_fake_backend();
        let short_key = base64::engine::general_purpose::STANDARD.encode([1u8; 16]);
        let err = set_secret(&handle(short_key), "value")
            .await
            .unwrap_err();
        assert!(err.contains("must decode to 32 bytes"), "{err}");
        set_secret_backend_for_tests(None);
    }
}
