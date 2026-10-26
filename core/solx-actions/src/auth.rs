//! `action_config.auth` block resolver for the Webhook dispatcher.
//!
//! Reads `action_config.auth` (free-form JSON) at call time and resolves
//! it to a `Bearer` header value, if any. Four auth types are supported:
//!
//! * `bearer` — `{type: "bearer", token: "..."}` → static header.
//! * `oauth_refresh` — refresh-token grant exchange against `token_uri`.
//! * `oauth_service_account` — JWT-bearer grant exchange against `token_uri`.
//! * `oauth_authorization_code` — sentinel; the dispatcher itself runs
//!   the `code` → `access_token` exchange (see `dispatch_webhook`).
//!
//! Access tokens are cached in-memory per action name and refreshed
//! within 60 seconds of expiry.
//!
//! Credentials ([`CREDENTIAL_FIELDS`]) are never inline in the config —
//! `crate::secrets::config` refuses them on save. Each comes from either a
//! pointer into the scoped secret store (`client_secret_secret`, etc — see
//! `secrets.rs`), decrypted with the action's scope key and allowed only for
//! a name listed in its `action_config.secrets`:
//!
//! ```json
//! { "action_config": {
//!     "secrets": ["GOOGLE_OAUTH_CLIENT_SECRET"],
//!     "auth": { "type": "oauth_refresh", "client_secret_secret": "GOOGLE_OAUTH_CLIENT_SECRET", ... }
//! } }
//! ```
//!
//! or a reference to an entry in the OS credential manager:
//!
//! ```json
//! { "type": "oauth_service_account",
//!   "client_email": "sa@project.iam.gserviceaccount.com",
//!   "private_key": { "keyring_service": "sol", "keyring_account": "google-svc-pk" },
//!   "token_uri": "https://oauth2.googleapis.com/token" }
//! ```
//!
//! When the provider returns a new `refresh_token` during an
//! `oauth_refresh` exchange, the resolver persists it back to wherever
//! it was originally sourced from (scoped secret store or keyring
//! reference) so rotation is not lost. Set
//! `action_config.auth.persist_rotated_refresh` to `false` to disable
//! this and let callers handle rotation themselves (default: `true`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Deserialize;
use serde_json::{json, Value};

use solx_config::ConfigService;
use solx_surface::{ActionSecrets, SecretHandle};

/// The `auth` fields that hold credentials. Each is resolved through
/// [`secret_field`] and has a `<field>_secret` pointer form; an inline
/// string at any of them is refused on save (see `crate::secrets::config`).
pub(crate) const CREDENTIAL_FIELDS: &[&str] = &[
    "token",
    "client_id",
    "client_secret",
    "refresh_token",
    "private_key",
];

// ── Cache ────────────────────────────────────────────────────────────────────

type CacheEntry = (String, Instant); // (token, expires_at)

static AUTH_CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    AUTH_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// solx identity is `(path, name)`, not `name` alone — two actions with the
/// same bare name under different paths are a normal configuration (e.g.
/// `/prod/refresh-token` and `/staging/refresh-token`). The cache key must
/// incorporate both, or two such actions would silently share (and clobber)
/// each other's cached bearer token.
fn cache_key(path: &str, name: &str) -> String {
    solx_surface::path::full_ref(path, name).unwrap_or_else(|_| format!("{path}/{name}"))
}

/// Clear the cached access token for the action at `path`/`name`. Useful
/// for tests and for an explicit "log out and force refresh" path.
pub fn clear_auth_cache(path: &str, name: &str) {
    if let Ok(mut c) = cache().lock() {
        c.remove(&cache_key(path, name));
    }
}

fn cache_get(key: &str) -> Option<String> {
    cache().lock().ok().and_then(|c| {
        c.get(key).and_then(|(tok, expires_at)| {
            if *expires_at > Instant::now() {
                Some(tok.clone())
            } else {
                None
            }
        })
    })
}

fn cache_put(key: &str, token: String, expires_in_secs: u64) {
    // Refresh 60s before the actual expiry to avoid races.
    let ttl = expires_in_secs.saturating_sub(60).max(1);
    if let Ok(mut c) = cache().lock() {
        c.insert(
            key.to_string(),
            (token, Instant::now() + Duration::from_secs(ttl)),
        );
    }
}

// ── Token endpoint ───────────────────────────────────────────────────────────

/// `auth.token_uri`, checked against the outbound allowlist, and a client
/// whose redirects are held to the same list.
///
/// Called before any credential is resolved: the request carries the
/// action's client secret, refresh token or signed assertion, so an
/// unlisted endpoint must be refused while there is still nothing to leak.
fn token_endpoint(
    config: &ConfigService,
    auth: &Value,
    auth_type_label: &str,
) -> Result<(String, reqwest::Client), String> {
    let token_uri = require_inline_string(auth, "token_uri", auth_type_label)?;
    crate::net::check_outbound_url(config, &token_uri)
        .map_err(|e| format!("auth.type='{auth_type_label}': token_uri: {e}"))?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(crate::net::redirect_policy(config))
        .build()
        .map_err(|e| format!("failed to build auth http client: {e}"))?;
    Ok((token_uri, client))
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Outcome of `oauth_refresh`: the new access token plus any rotated
/// refresh token the provider returned.
#[derive(Debug, Default)]
struct RefreshOutcome {
    access_token: String,
    /// When the provider returned a different `refresh_token` than
    /// the one we sent, this is the new value.
    rotated_refresh_token: Option<String>,
    /// Server-reported `expires_in` (seconds) for the access token.
    /// `None` when the provider omits the field; the resolver falls
    /// back to a 1h cache TTL in that case.
    expires_in_secs: Option<u64>,
}

/// Convert the full set of auth-resolution outcomes into a single
/// `Authorization` header value (without the `Authorization:` prefix).
///
/// Returns `Ok(None)` when:
/// * the action has no `auth` block, OR
/// * `auth.type == "oauth_authorization_code"` (the sentinel that the
///   dispatcher handles itself).
///
/// On `oauth_refresh` success the function **also** persists any
/// rotated `refresh_token` returned by the provider back to its scoped
/// secret (or its keyring account), so the next call sees the new token.
/// Persistence is best-effort: any error is logged at `warn` and does
/// **not** fail the auth resolution.
///
/// `secrets` are the executing action's own, for `<field>_secret`
/// pointers. `config` holds the outbound allowlist that a `token_uri` must
/// match, like every other URL solx sends a request to.
pub async fn resolve_auth(
    path: &str,
    name: &str,
    config: &ConfigService,
    action_config: &Value,
    secrets: &ActionSecrets,
) -> Result<Option<String>, String> {
    let key = cache_key(path, name);
    let Some(auth) = action_config.get("auth") else {
        return Ok(None);
    };
    let auth_type = auth
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "action_config.auth is missing required field 'type'".to_string())?;

    // Cache fast path for all three types.
    if let Some(tok) = cache_get(&key) {
        return Ok(Some(format!("Bearer {tok}")));
    }

    match auth_type {
        "bearer" => {
            // Resolve through the standard secret pipeline so the
            // bearer token can come from a keyring reference or the scoped
            // secret store, matching the OAuth types. The token-ttl
            // `ttl_secs` field is inline-only — the per-action TTL
            // doesn't have a keyring/secret shape.
            let token =
                secret_field(secrets, auth, "token", Some("token_secret"), "bearer").await?;
            let ttl = auth.get("ttl_secs").and_then(Value::as_u64).unwrap_or(3600);
            cache_put(&key, token.clone(), ttl);
            Ok(Some(format!("Bearer {token}")))
        }
        "oauth_refresh" => {
            let outcome = exchange_refresh_token(name, config, secrets, auth).await?;
            // Touch the rotated_refresh_token field so the compiler
            // doesn't flag it as unused — the persistence side-effect
            // inside `exchange_refresh_token` already wrote the new
            // value back to wherever the refresh token was sourced from.
            let _ = outcome.rotated_refresh_token;
            // Respect the server-reported `expires_in` when the
            // provider sends one. Fall back to 1h.
            let ttl_secs = outcome.expires_in_secs.unwrap_or(3600);
            cache_put(&key, outcome.access_token.clone(), ttl_secs);
            Ok(Some(format!("Bearer {}", outcome.access_token)))
        }
        "oauth_service_account" => {
            let token = exchange_service_account_token(config, secrets, auth).await?;
            let ttl = auth.get("ttl_secs").and_then(Value::as_u64).unwrap_or(3600);
            cache_put(&key, token.clone(), ttl);
            Ok(Some(format!("Bearer {token}")))
        }
        "oauth_authorization_code" => {
            // The `oauth_authorization_code` auth type is handled by
            // `dispatch_webhook` itself (which knows the request URL and
            // body shape); from the resolver's perspective it just means
            // "no Bearer header to inject". The dispatcher performs the
            // form-encoded POST to the action's `fn_name` (the token
            // endpoint) and returns the token JSON as the action result.
            Ok(None)
        }
        other => Err(format!(
            "unsupported action_config.auth.type '{other}'; \
             expected 'bearer', 'oauth_refresh', 'oauth_service_account', \
             or 'oauth_authorization_code'"
        )),
    }
}

// ── Refresh-token grant (RFC 6749 §6) ─────────────────────────────────────────

/// Owned form of the inputs needed for an RFC 6749 §6 refresh-token grant.
/// We deliberately avoid `Deserialize` here because borrowing the strings
/// out of `&Value` makes serde's lifetime checker unhappy — pulling them
/// into owned `String`s is simpler and the values are tiny.
///
/// `client_id`, `client_secret`, and `refresh_token` can each be a keyring
/// reference or a pointer into the scoped secret store
/// (`client_id_secret`, `client_secret_secret`, `refresh_token_secret` —
/// see [`secret_field`]).  `token_uri` and `scope` remain in
/// `action_config.auth` since they are not secrets; `token_uri` is read and
/// checked first, by [`token_endpoint`].
#[derive(Debug)]
struct RefreshRequest {
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

impl RefreshRequest {
    async fn from_value(secrets: &ActionSecrets, auth: &Value) -> Result<Self, String> {
        Ok(Self {
            client_id: secret_field(
                secrets,
                auth,
                "client_id",
                Some("client_id_secret"),
                "oauth_refresh",
            )
            .await?,
            client_secret: secret_field(
                secrets,
                auth,
                "client_secret",
                Some("client_secret_secret"),
                "oauth_refresh",
            )
            .await?,
            refresh_token: secret_field(
                secrets,
                auth,
                "refresh_token",
                Some("refresh_token_secret"),
                "oauth_refresh",
            )
            .await?,
        })
    }
}

/// Resolve one credential field, in order:
///
/// 1. `auth.<field>` as a keyring-ref object.
/// 2. `auth.<field>_secret` — name of a secret in the scoped secret
///    store, decrypted with the action's scope key; the name must be listed in
///    the action's `action_config.secrets`.
/// 3. `Err` — neither was present.
///
/// An inline string at `auth.<field>` is an error rather than a value:
/// save refuses them, so one here predates that check and is not used.
///
/// `secret_field_name` selects which `<field>_secret` field to look at;
/// pass `None` to opt out of the secret-store indirection entirely.
pub(crate) async fn secret_field(
    secrets: &ActionSecrets,
    auth: &Value,
    field: &str,
    secret_field_name: Option<&str>,
    auth_type_label: &str,
) -> Result<String, String> {
    // 1. Keyring reference.
    if let Some(v) = auth.get(field) {
        match v {
            Value::String(s) if !s.is_empty() => {
                return Err(format!(
                    "auth.type='{auth_type_label}': field '{field}' is an inline credential, \
                     which is not used; re-save the action with a keyring reference or a \
                     '{field}_secret' pointer"
                ));
            }
            Value::Object(_) => {
                return crate::secrets::resolve_in_field(
                    &format!("auth.type='{auth_type_label}'"),
                    field,
                    v,
                )
                .await;
            }
            Value::Null | Value::String(_) => { /* fall through to secret-store pointer */ }
            _ => {
                return Err(format!(
                    "auth.type='{auth_type_label}': field '{field}' has \
                     unsupported JSON type {}; expected string or \
                     '{{ \"keyring_service\": …, \"keyring_account\": … }}'",
                    crate::secrets::type_name_of(v)
                ));
            }
        }
    }
    // 2. Scoped secret-store indirection.
    if let Some(name) = secret_field_name.and_then(|f| auth.get(f).and_then(Value::as_str)) {
        if !name.is_empty() {
            let secret = secrets.resolve(name).ok_or_else(|| {
                format!(
                    "auth.type='{auth_type_label}': field '{field}' points to secret \
                     '{name}', which isn't listed in action_config.secrets"
                )
            })?;
            let value = crate::secrets::get_secret(&secret).await.map_err(|e| {
                format!("auth.type='{auth_type_label}': failed to read secret '{name}': {e}")
            })?;
            return value.filter(|v| !v.is_empty()).ok_or_else(|| {
                format!("auth.type='{auth_type_label}': secret '{name}' is not set")
            });
        }
    }
    let secret_hint = match secret_field_name {
        Some(name) => format!("or via '{name}' secret-store pointer"),
        None => String::new(),
    };
    Err(format!(
        "auth.type='{auth_type_label}' requires field '{field}' \
         (keyring reference{comma}{secret_hint})",
        comma = if secret_hint.is_empty() { "" } else { ", " }
    ))
}

/// Like [`secret_field`], but returns `Ok(None)` instead of `Err` when
/// *nothing at all* is configured for `field` — some OAuth flows
/// legitimately have no value here (e.g. a public/PKCE client has no
/// `client_secret`). If something *is* configured (keyring reference or a
/// `_secret` pointer) but fails to resolve, that's still a hard error — a
/// configured-but-broken secret should never fail silently.
pub(crate) async fn optional_secret_field(
    secrets: &ActionSecrets,
    auth: &Value,
    field: &str,
    secret_field_name: Option<&str>,
    auth_type_label: &str,
) -> Result<Option<String>, String> {
    let inline_present = auth.get(field).is_some();
    let pointer_present = secret_field_name
        .and_then(|f| auth.get(f).and_then(Value::as_str))
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if !inline_present && !pointer_present {
        return Ok(None);
    }
    secret_field(secrets, auth, field, secret_field_name, auth_type_label)
        .await
        .map(Some)
}

/// Variant of [`secret_field`] for non-secret fields. Inline string only.
fn require_inline_string(
    auth: &Value,
    field: &str,
    auth_type_label: &str,
) -> Result<String, String> {
    auth.get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("auth.type='{auth_type_label}' missing required field '{field}'"))
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
    /// Optional per RFC 6749 §5.1; many providers rotate refresh tokens
    /// on each successful response. We treat the response as
    /// "rotated" only when the field is **present and non-empty AND
    /// different** from the value we sent.
    #[serde(default)]
    refresh_token: Option<String>,
}

/// Where the rotated refresh token should land. Mirrors the resolution
/// source the action was originally configured with — keeping the
/// secret in the same place preserves any audit / scanning
/// expectations.
#[derive(Debug, Clone)]
enum RefreshStorage {
    /// The action has `"refresh_token": { "keyring_service": …, "keyring_account": … }`
    /// — write the new value through the keyring.
    Keyring { service: String, account: String },
    /// The action has `"refresh_token_secret": "<name>"` — write the new
    /// value through the scoped secret store.
    Secret(SecretHandle),
}

impl RefreshStorage {
    /// `None` when there is nowhere to persist to — which can't happen for
    /// a refresh token that just resolved, since it came from one of these.
    fn from_auth(secrets: &ActionSecrets, auth: &Value) -> Option<Self> {
        if let Some(name) = auth.get("refresh_token_secret").and_then(Value::as_str) {
            if let Some(secret) = secrets.resolve(name) {
                return Some(RefreshStorage::Secret(secret));
            }
        }
        if let Some(obj) = auth.get("refresh_token").and_then(Value::as_object) {
            if let (Some(service), Some(account)) = (
                obj.get("keyring_service").and_then(Value::as_str),
                obj.get("keyring_account").and_then(Value::as_str),
            ) {
                return Some(RefreshStorage::Keyring {
                    service: service.to_string(),
                    account: account.to_string(),
                });
            }
        }
        None
    }
}

async fn exchange_refresh_token(
    name: &str,
    config: &ConfigService,
    secrets: &ActionSecrets,
    auth: &Value,
) -> Result<RefreshOutcome, String> {
    let (token_uri, client) = token_endpoint(config, auth, "oauth_refresh")?;
    let req = RefreshRequest::from_value(secrets, auth).await?;

    let body = json!({
        "grant_type": "refresh_token",
        "client_id": req.client_id,
        "client_secret": req.client_secret,
        "refresh_token": req.refresh_token,
    });
    let resp = client
        .post(token_uri)
        .form(&body)
        .send()
        .await
        .map_err(|e| format!("oauth_refresh token endpoint POST failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "oauth_refresh token endpoint returned HTTP {status}: {text}"
        ));
    }
    let parsed: RefreshResponse = resp
        .json()
        .await
        .map_err(|e| format!("oauth_refresh response is not valid JSON: {e}"))?;

    let rotated = parsed
        .refresh_token
        .as_ref()
        .filter(|tok| !tok.is_empty() && **tok != req.refresh_token)
        .cloned();

    // Persistence of rotated tokens is opt-out (`auth.persist_rotated_refresh`).
    // The default is `true` because most providers rotate, but actions that
    // want to manage the new token themselves can disable this.
    let persistence_enabled = auth
        .get("persist_rotated_refresh")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    if let (Some(new_rt), true) = (rotated.as_ref(), persistence_enabled) {
        persist_rotated_refresh_token(name, secrets, auth, new_rt).await;
    } else if rotated.is_some() {
        tracing::info!(
            "received rotated refresh_token for action '{name}' but \
             persistence is disabled (auth.persist_rotated_refresh=false); \
             new token is NOT persisted"
        );
    }

    Ok(RefreshOutcome {
        access_token: parsed.access_token,
        rotated_refresh_token: rotated,
        expires_in_secs: parsed.expires_in,
    })
}

/// Persist a rotated refresh token. Best-effort: any error is logged at
/// `warn` and *does not* propagate to the caller — the access token
/// has already been issued, so the most we can do is tell the user
/// "the next refresh may fail; rotate manually".
async fn persist_rotated_refresh_token(
    name: &str,
    secrets: &ActionSecrets,
    auth: &Value,
    new_refresh_token: &str,
) {
    match RefreshStorage::from_auth(secrets, auth) {
        None => {
            tracing::warn!(
                "action '{name}' has nowhere to persist a rotated refresh_token; \
                 the next refresh may fail"
            );
        }
        Some(RefreshStorage::Keyring { service, account }) => {
            if let Err(e) = crate::secrets::check_keyring_service(&service) {
                tracing::warn!("rotated refresh_token not persisted: {e}");
                return;
            }
            let entry = match keyring::Entry::new(&service, &account) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        "keyring entry '{service}/{account}' open failed; \
                         rotated refresh_token not persisted: {e}"
                    );
                    return;
                }
            };
            if let Err(e) = entry.set_password(new_refresh_token) {
                tracing::warn!(
                    "keyring entry '{service}/{account}' set_password failed; \
                     rotated refresh_token not persisted: {e}"
                );
            }
        }
        Some(RefreshStorage::Secret(secret)) => {
            if let Err(e) = crate::secrets::set_secret(&secret, new_refresh_token).await {
                tracing::warn!(
                    "failed to persist rotated refresh_token to secret '{}' \
                     for action '{name}': {e}",
                    secret.name()
                );
            }
        }
    }
}

// ── Service-account JWT-bearer grant (RFC 7521 §5.1) ─────────────────────────

/// Owned form of the inputs for an RFC 7521 §5.1 service-account JWT-bearer
/// grant. See [`RefreshRequest`] for why we avoid `Deserialize` here.
///
/// `private_key` is sourced through [`secret_field`] (keyring reference or
/// scoped secret-store pointer).
#[derive(Debug)]
struct ServiceAccountRequest {
    client_email: String,
    private_key: String,
}

impl ServiceAccountRequest {
    async fn from_value(secrets: &ActionSecrets, auth: &Value) -> Result<Self, String> {
        Ok(Self {
            client_email: require_inline_string(auth, "client_email", "oauth_service_account")?,
            private_key: secret_field(
                secrets,
                auth,
                "private_key",
                Some("private_key_secret"),
                "oauth_service_account",
            )
            .await?,
        })
    }
}

async fn exchange_service_account_token(
    config: &ConfigService,
    secrets: &ActionSecrets,
    auth: &Value,
) -> Result<String, String> {
    let (token_uri, client) = token_endpoint(config, auth, "oauth_service_account")?;
    let req = ServiceAccountRequest::from_value(secrets, auth).await?;

    let now = chrono::Utc::now().timestamp();
    let exp = now + 3600; // 1h
    let scope = auth.get("scope").and_then(Value::as_str).unwrap_or("");
    let claims = json!({
        "iss": req.client_email,
        "scope": scope,
        "aud": token_uri,
        "iat": now,
        "exp": exp,
    });
    // RSA PEM parse + RS256 sign is a few ms of pure CPU, and only the
    // resulting token is cached — so a cache miss would otherwise stall an
    // async worker on every service-account call.
    let private_key = req.private_key.clone();
    let assertion = tokio::task::spawn_blocking(move || {
        let key = EncodingKey::from_rsa_pem(private_key.as_bytes())
            .context("service_account private_key is not valid RSA PEM")
            .map_err(|e| e.to_string())?;
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
            .map_err(|e| format!("failed to sign service-account JWT: {e}"))
    })
    .await
    .map_err(|e| format!("JWT signing task panicked: {e}"))??;

    let body = format!(
        "grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer&assertion={}",
        urlencoding(&assertion)
    );
    let resp = client
        .post(token_uri)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("oauth_service_account token endpoint POST failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!(
            "oauth_service_account token endpoint returned HTTP {status}: {text}"
        ));
    }
    let parsed: RefreshResponse = resp
        .json()
        .await
        .map_err(|e| format!("oauth_service_account response is not valid JSON: {e}"))?;
    Ok(parsed.access_token)
}

/// Minimal URL-encoder for the JWT assertion (the `assertion` value contains
/// dots which are safe; this only escapes `+`, `=`, `&`, etc.).
fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn clear() {
        cache().lock().unwrap().clear();
    }

    fn no_secrets() -> ActionSecrets {
        ActionSecrets::default()
    }

    /// A keyring reference that [`install_echo_loader`] resolves to `value`.
    fn kr(value: &str) -> Value {
        json!({ "keyring_service": "test", "keyring_account": value })
    }

    /// Resolves every keyring reference to its account name, so a test can
    /// say which credential it means without a real credential manager.
    /// Cleared when the returned guard drops, even on a failed assertion.
    fn install_echo_loader() -> impl Drop {
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                crate::secrets::set_secret_loader_for_tests(None);
            }
        }
        crate::secrets::set_secret_loader_for_tests(Some(|r| Ok(r.keyring_account.clone())));
        Guard
    }

    /// A config whose allowlist admits only `https://allowed.example`.
    fn config() -> (tempfile::TempDir, ConfigService) {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigService::open_in(dir.path()).unwrap();
        config
            .set_allowed_base_urls(vec!["https://allowed.example".into()])
            .unwrap();
        (dir, config)
    }

    async fn resolve(path: &str, name: &str, cfg: &Value) -> Result<Option<String>, String> {
        let (_dir, config) = config();
        resolve_auth(path, name, &config, cfg, &no_secrets()).await
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn no_auth_block_returns_none() {
        clear();
        assert!(resolve("/test", "a", &json!({})).await.unwrap().is_none());
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn bearer_resolves_keyring_token() {
        clear();
        let _g = install_echo_loader();
        let cfg = json!({ "auth": { "type": "bearer", "token": kr("abc123") } });
        let out = resolve("/test", "a", &cfg).await.unwrap().unwrap();
        assert_eq!(out, "Bearer abc123");
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn bearer_missing_token_errors() {
        clear();
        let cfg = json!({ "auth": { "type": "bearer" } });
        let err = resolve("/test", "bearer_missing_action", &cfg)
            .await
            .unwrap_err();
        assert!(err.contains("requires field 'token'"), "{err}");
    }

    /// Save refuses inline credentials; one that predates that check is an
    /// error at call time, never sent.
    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn inline_bearer_token_is_not_used() {
        clear();
        let cfg = json!({ "auth": { "type": "bearer", "token": "inline-value" } });
        let err = resolve("/test", "inline_action", &cfg).await.unwrap_err();
        assert!(err.contains("inline credential"), "{err}");
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn unknown_auth_type_errors_helpfully() {
        clear();
        let cfg = json!({ "auth": { "type": "magic_link" } });
        let err = resolve("/test", "a", &cfg).await.unwrap_err();
        assert!(
            err.contains("unsupported action_config.auth.type 'magic_link'"),
            "{err}"
        );
        assert!(
            err.contains("'bearer'"),
            "error should list the valid types, got: {err}"
        );
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn auth_block_missing_type_errors() {
        clear();
        let cfg = json!({ "auth": { "token": kr("x") } });
        let err = resolve("/test", "a", &cfg).await.unwrap_err();
        assert!(err.contains("missing required field 'type'"), "{err}");
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn cache_hit_avoids_second_call() {
        clear();
        let _g = install_echo_loader();
        let cfg = json!({ "auth": { "type": "bearer", "token": kr("abc123"), "ttl_secs": 60 } });
        resolve("/test", "a", &cfg).await.unwrap();
        // Second call should hit the cache and still return the same token
        // even if we mutate the config to make the bearer path fail.
        let mutated = json!({ "auth": { "type": "bearer" /* no token */ } });
        let out = resolve("/test", "a", &mutated).await.unwrap().unwrap();
        assert_eq!(out, "Bearer abc123");
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn cache_keyed_per_action() {
        clear();
        let _g = install_echo_loader();
        let cfg_a = json!({ "auth": { "type": "bearer", "token": kr("for-a"), "ttl_secs": 60 } });
        let cfg_b = json!({ "auth": { "type": "bearer", "token": kr("for-b"), "ttl_secs": 60 } });
        assert_eq!(
            resolve("/test", "a", &cfg_a).await.unwrap().unwrap(),
            "Bearer for-a"
        );
        assert_eq!(
            resolve("/test", "b", &cfg_b).await.unwrap().unwrap(),
            "Bearer for-b"
        );
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn cache_does_not_collide_across_paths_with_same_name() {
        // solx identity is (path, name), not name alone — two actions with
        // the same bare name under different paths (e.g. a "prod" and a
        // "staging" copy of the same integration, both named "webhook")
        // must not share a cached bearer token.
        clear();
        let _g = install_echo_loader();
        let cfg_prod =
            json!({ "auth": { "type": "bearer", "token": kr("prod-token"), "ttl_secs": 60 } });
        let cfg_staging =
            json!({ "auth": { "type": "bearer", "token": kr("staging-token"), "ttl_secs": 60 } });
        assert_eq!(
            resolve("/prod", "webhook", &cfg_prod)
                .await
                .unwrap()
                .unwrap(),
            "Bearer prod-token"
        );
        assert_eq!(
            resolve("/staging", "webhook", &cfg_staging)
                .await
                .unwrap()
                .unwrap(),
            "Bearer staging-token"
        );
        // Re-resolving prod (config unchanged) must still hit its own cache
        // entry, not staging's.
        assert_eq!(
            resolve("/prod", "webhook", &cfg_prod)
                .await
                .unwrap()
                .unwrap(),
            "Bearer prod-token"
        );
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn clear_auth_cache_forces_refresh() {
        clear();
        let _g = install_echo_loader();
        let cfg = json!({ "auth": { "type": "bearer", "token": kr("first") } });
        assert_eq!(
            resolve("/test", "a", &cfg).await.unwrap().unwrap(),
            "Bearer first"
        );

        // Mutate config (e.g. user rotated the bearer); clear cache; verify
        // the new token is returned.
        let cfg2 = json!({ "auth": { "type": "bearer", "token": kr("second") } });
        clear_auth_cache("/test", "a");
        assert_eq!(
            resolve("/test", "a", &cfg2).await.unwrap().unwrap(),
            "Bearer second"
        );
    }

    /// The token endpoint is held to the outbound allowlist, and refused
    /// before any credential is read: the loader below fails the test if
    /// it's reached.
    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn an_unlisted_token_uri_is_refused_before_credentials_are_read() {
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                crate::secrets::set_secret_loader_for_tests(None);
            }
        }
        crate::secrets::set_secret_loader_for_tests(Some(|r| {
            panic!("read {} before checking token_uri", r.keyring_account)
        }));
        let _g = Guard;
        for auth_type in ["oauth_refresh", "oauth_service_account"] {
            clear();
            let cfg = json!({ "auth": {
                "type": auth_type,
                "client_email": "sa@example.com",
                "client_id": kr("id"),
                "client_secret": kr("secret"),
                "refresh_token": kr("refresh"),
                "private_key": kr("pem"),
                "token_uri": "https://attacker.example/token"
            } });
            let err = resolve("/test", auth_type, &cfg).await.unwrap_err();
            assert!(err.contains("allowed_base_urls"), "{auth_type}: {err}");
        }
    }

    /// Even an existing row can't reach solx's own entries: the root token
    /// lives under the `solx` service.
    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn a_keyring_reference_cant_read_solx_credentials() {
        clear();
        let _g = install_echo_loader();
        let cfg = json!({ "auth": {
            "type": "bearer",
            "token": { "keyring_service": "solx", "keyring_account": "root:x" }
        } });
        let err = resolve("/test", "root", &cfg).await.unwrap_err();
        assert!(err.contains("solx's own"), "{err}");
    }

    #[test]
    fn urlencoding_leaves_safe_chars_alone() {
        assert_eq!(urlencoding("abc.123-XYZ_ok~bar"), "abc.123-XYZ_ok~bar");
    }

    #[test]
    fn urlencoding_escapes_unsafe_chars() {
        assert_eq!(urlencoding("a+b=c&d e"), "a%2Bb%3Dc%26d%20e");
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn oauth_authorization_code_returns_none_no_bearer() {
        clear();
        // `oauth_authorization_code` is handled by `dispatch_webhook`
        // itself — the resolver returns Ok(None) so no Bearer header is
        // injected. The dispatch path then takes over and POSTs to the
        // action's `fn_name` (the token URI) with form-encoded body.
        let cfg = json!({
            "auth": {
                "type": "oauth_authorization_code",
                "client_id_secret": "CID",
                "client_secret_secret": "CSEC"
            }
        });
        let out = resolve("/test", "a", &cfg).await.unwrap();
        assert!(
            out.is_none(),
            "oauth_authorization_code must not inject a Bearer header; got {out:?}"
        );
    }

    #[serial(auth_cache, secrets_loader)]
    #[tokio::test]
    async fn oauth_authorization_code_does_not_cache() {
        clear();
        // Unlike the other auth types, this one is intercepted upstream
        // by the dispatcher and never reaches `cache_get` / `cache_put`,
        // so resolve_auth must NOT populate the cache with a stale
        // bearer token.
        let cfg = json!({
            "auth": {
                "type": "oauth_authorization_code",
                "client_id_secret": "CID"
            }
        });
        resolve("/test", "a", &cfg).await.unwrap();
        assert!(
            cache_get(&cache_key("/test", "a")).is_none(),
            "oauth_authorization_code must not write to the auth cache"
        );
    }

    // ── scoped secret-store pointers ─────────────────────────────────────

    #[serial(auth_cache, secrets_backend)]
    #[tokio::test]
    async fn bearer_resolves_secret_pointer_with_the_actions_key() {
        clear();
        crate::secrets::set_secret_backend_for_tests(Some(
            crate::testing::support::fake_secret_backend(),
        ));
        let cfg = json!({
            "secrets": ["BEARER"],
            "auth": { "type": "bearer", "token_secret": "BEARER" }
        });
        let secrets = ActionSecrets::new(
            "action:/test/pointer",
            Some(&cfg),
            Some(crate::testing::support::TEST_KEY.to_string()),
        );
        crate::secrets::set_secret(&secrets.resolve("BEARER").unwrap(), "from-store")
            .await
            .unwrap();
        let (_dir, config) = config();
        let out = resolve_auth("/test", "pointer", &config, &cfg, &secrets).await;
        crate::secrets::set_secret_backend_for_tests(None);
        assert_eq!(out.unwrap().unwrap(), "Bearer from-store");
    }

    /// A key isn't enough: the name has to be in the action's `secrets`
    /// allowlist.
    #[serial(auth_cache, secrets_backend)]
    #[tokio::test]
    async fn secret_pointer_outside_the_allowlist_is_refused() {
        clear();
        let cfg = json!({
            "secrets": ["OTHER"],
            "auth": { "type": "bearer", "token_secret": "BEARER" }
        });
        let secrets = ActionSecrets::new(
            "action:/test/outside",
            Some(&cfg),
            Some(crate::testing::support::TEST_KEY.to_string()),
        );
        let (_dir, config) = config();
        let err = resolve_auth("/test", "outside", &config, &cfg, &secrets)
            .await
            .unwrap_err();
        assert!(
            err.contains("isn't listed in action_config.secrets"),
            "{err}"
        );
    }

    // ── optional_secret_field (oauth_authorization_code client_secret) ─────

    #[tokio::test]
    async fn optional_secret_field_returns_none_when_nothing_configured() {
        // A public/PKCE OAuth client legitimately has no client_secret —
        // this must not be an error.
        assert_eq!(
            optional_secret_field(
                &no_secrets(),
                &json!({}),
                "client_secret",
                Some("client_secret_secret"),
                "oauth_authorization_code"
            )
            .await
            .unwrap(),
            None
        );
    }

    #[serial(secrets_loader)]
    #[tokio::test]
    async fn optional_secret_field_resolves_keyring_when_present() {
        let _g = install_echo_loader();
        let auth = json!({ "client_secret": kr("shh") });
        assert_eq!(
            optional_secret_field(
                &no_secrets(),
                &auth,
                "client_secret",
                Some("client_secret_secret"),
                "oauth_authorization_code"
            )
            .await
            .unwrap(),
            Some("shh".to_string())
        );
    }

    #[tokio::test]
    async fn optional_secret_field_errors_when_pointer_configured_but_key_missing() {
        // Points at a secret-store name, but the action has no key for it —
        // a real misconfiguration that must error loudly rather than
        // silently degrading to Ok(None).
        let auth = json!({ "client_secret_secret": "MY_SECRET" });
        let err = optional_secret_field(
            &no_secrets(),
            &auth,
            "client_secret",
            Some("client_secret_secret"),
            "oauth_authorization_code",
        )
        .await
        .unwrap_err();
        assert!(err.contains("MY_SECRET"), "{err}");
    }

    #[test]
    fn refresh_storage_detects_keyring() {
        let keyring = json!({"refresh_token": {"keyring_service": "sol", "keyring_account": "x"}});
        assert!(matches!(
            RefreshStorage::from_auth(&no_secrets(), &keyring),
            Some(RefreshStorage::Keyring { .. })
        ));
    }

    #[test]
    fn refresh_storage_detects_secret_pointer_when_key_configured() {
        let cfg = json!({"secrets": ["GOOGLE_OAUTH_REFRESH_TOKEN"]});
        let secrets = ActionSecrets::new("package:p", Some(&cfg), Some("the-key".into()));
        let auth = json!({"refresh_token_secret": "GOOGLE_OAUTH_REFRESH_TOKEN"});
        assert!(matches!(
            RefreshStorage::from_auth(&secrets, &auth),
            Some(RefreshStorage::Secret(_))
        ));
    }

    #[test]
    fn refresh_storage_is_none_when_secret_pointer_has_no_key() {
        let auth = json!({"refresh_token_secret": "GOOGLE_OAUTH_REFRESH_TOKEN"});
        assert!(RefreshStorage::from_auth(&no_secrets(), &auth).is_none());
    }

    #[test]
    fn persist_rotated_refresh_flag_defaults_true() {
        // The flag is read by `exchange_refresh_token` via
        // `auth.get("persist_rotated_refresh").and_then(Value::as_bool)
        // .unwrap_or(true)`; absent / non-bool → true.
        let absent = json!({});
        assert!(absent
            .get("persist_rotated_refresh")
            .and_then(Value::as_bool)
            .unwrap_or(true));

        let string = json!({"persist_rotated_refresh": "no"});
        assert!(
            string
                .get("persist_rotated_refresh")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            "non-bool values fall through to the default-true"
        );

        let opted_out = json!({"persist_rotated_refresh": false});
        assert!(!opted_out
            .get("persist_rotated_refresh")
            .and_then(Value::as_bool)
            .unwrap_or(true));
    }
}
