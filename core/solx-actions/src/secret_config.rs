//! Keeps secret material out of `action_config`, applied to every config
//! [`crate::store`] writes.
//!
//! An action's config never holds a secret, so reads need no redaction:
//!
//! * **The AES key** for the action's scoped secrets (see [`crate::secrets`])
//!   lives in the actions table's `secret_key` column, never in an
//!   [`Action`](solx_surface::entities::Action). `action_config.secrets`
//!   keeps only the secret *names* the action may use, each mapped to the
//!   literal [`MASK`]. A save may still carry a real key there — that is how
//!   an install script sets one — and [`normalize`] lifts it into the column.
//! * **Credentials** (`auth.token`, `auth.client_secret`, …) are refused
//!   inline. They are resolved at call time from a `<field>_secret` pointer
//!   into the scoped secret store, or a keyring reference — see
//!   [`crate::auth::secret_field`].
//!
//! Because `"***"` is the stored value itself, the `get` → edit → `save`
//! round trip keeps working without any merge: the echoed `"***"` is
//! written back as is, and the column keeps its key.

use serde_json::Value;
use solx_surface::entities::{Action, ActionInput};

/// Stands in for the key in `action_config.secrets`.
pub const MASK: &str = "***";

/// Refuse a save that would create or change a *keyed* action — one whose
/// stored or incoming config declares `secrets` — for a caller that isn't
/// root.
///
/// Root is any in-process host (the CLI, a package install, Tauri) or an
/// HTTP request with the root token; those call `ActionManager::save`
/// directly. Everyone else — an MCP tool call, a WASM guest, a script, an
/// HTTP client token — must pass this check first.
///
/// It covers the whole row, not just the key. Adding a name to a keyed
/// action's `secrets` would extend its key to another package's secrets
/// (and a failed decrypt clears the entry), and repointing its `bin_name`
/// or `files` would run new code with the key.
pub fn guard_keyed_action(
    existing: Option<&Action>,
    incoming: &ActionInput,
    reference: &str,
) -> Result<(), String> {
    let declares_secrets = |cfg: Option<&Value>| cfg.is_some_and(|c| c.get("secrets").is_some());
    if declares_secrets(existing.and_then(|a| a.action_config.as_ref()))
        || declares_secrets(incoming.action_config.as_ref())
    {
        return Err(format!(
            "{reference} has scoped secrets; only root (the solx CLI, a package install, \
             or the root token) can create or modify it"
        ));
    }
    Ok(())
}

/// Validate `cfg` for storage and work out the action's key.
///
/// Rewrites every value under `secrets` to [`MASK`] and returns the key to
/// store in the `secret_key` column: the one supplied in `cfg`, else
/// `existing_key` when every value is already the mask, else `None` when
/// the config declares no secrets at all.
pub(crate) fn normalize(
    cfg: &mut Value,
    existing_key: Option<&str>,
) -> Result<Option<String>, String> {
    reject_inline_credentials(cfg)?;
    let Some(secrets) = cfg.get_mut("secrets") else {
        return Ok(None);
    };
    let map = secrets.as_object_mut().ok_or_else(|| {
        "action_config.secrets must be an object mapping secret names to a key".to_string()
    })?;
    if map.is_empty() {
        return Ok(None);
    }

    let mut supplied: Option<&str> = None;
    for (name, value) in map.iter() {
        let key = value.as_str().ok_or_else(|| {
            format!("action_config.secrets.{name} must be a string (a base64 key, or \"{MASK}\")")
        })?;
        if key == MASK {
            continue;
        }
        match supplied {
            None => supplied = Some(key),
            Some(k) if k == key => {}
            Some(_) => {
                return Err(
                    "action_config.secrets holds more than one key; an action has a single \
                     key, so give every secret name the same one"
                        .to_string(),
                )
            }
        }
    }

    let key = match supplied {
        Some(k) => {
            crate::secrets::decode_key(k).map_err(|e| format!("action_config.secrets: {e}"))?;
            k.to_string()
        }
        None => match existing_key {
            Some(k) => k.to_string(),
            None => {
                let name = map.keys().next().map(String::as_str).unwrap_or_default();
                return Err(format!(
                    "action_config.secrets.{name} is \"{MASK}\" but this action has no key \
                     yet; supply one (e.g. from `random 32`)"
                ));
            }
        },
    };
    for value in map.values_mut() {
        *value = Value::String(MASK.to_string());
    }
    Ok(Some(key))
}

/// What [`scrub_legacy`] did to one row's config.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Scrubbed {
    /// The config changed and must be written back.
    pub changed: bool,
    /// A key lifted out of `secrets`, for the `secret_key` column.
    pub key: Option<String>,
    /// Problems worth a log line: inline credentials dropped, keys that
    /// were malformed or disagreed.
    pub notes: Vec<String>,
}

/// Bring a config stored before [`normalize`] existed into its shape.
///
/// Such a row keeps its real key(s) under `secrets` and may hold inline
/// credentials under `auth`. With read-time masking gone, every `get` and
/// `list` would return both. So the first valid key moves to the column,
/// every `secrets` value becomes [`MASK`], and inline credentials (which
/// execution no longer uses) are dropped. Unlike `normalize` it never
/// fails: it runs at startup over rows nobody is there to fix.
pub(crate) fn scrub_legacy(cfg: &mut Value) -> Scrubbed {
    let mut out = Scrubbed::default();
    if let Some(auth) = cfg.get_mut("auth").and_then(Value::as_object_mut) {
        for field in crate::auth::CREDENTIAL_FIELDS {
            if matches!(auth.get(*field), Some(Value::String(s)) if !s.is_empty()) {
                auth.remove(*field);
                out.changed = true;
                out.notes.push(format!(
                    "dropped inline auth.{field}; point '{field}_secret' at a scoped secret \
                     or use a keyring reference"
                ));
            }
        }
    }
    if let Some(map) = cfg.get_mut("secrets").and_then(Value::as_object_mut) {
        for value in map.values_mut() {
            if value.as_str() == Some(MASK) {
                continue;
            }
            match value.as_str() {
                Some(k) if crate::secrets::decode_key(k).is_ok() => match &out.key {
                    None => out.key = Some(k.to_string()),
                    Some(kept) if kept == k => {}
                    Some(_) => out.notes.push(
                        "secrets held more than one key; kept the first, so secrets stored \
                         under the others will read as not set"
                            .to_string(),
                    ),
                },
                _ => out.notes.push("dropped a malformed key under secrets".to_string()),
            }
            *value = Value::String(MASK.to_string());
            out.changed = true;
        }
    }
    out
}

/// A string at any of [`crate::auth::CREDENTIAL_FIELDS`] is an inline
/// credential. A keyring reference (an object) is a pointer and is fine.
fn reject_inline_credentials(cfg: &Value) -> Result<(), String> {
    let Some(auth) = cfg.get("auth").and_then(Value::as_object) else {
        return Ok(());
    };
    for field in crate::auth::CREDENTIAL_FIELDS {
        if matches!(auth.get(*field), Some(Value::String(s)) if !s.is_empty()) {
            return Err(format!(
                "action_config.auth.{field} is an inline credential, which actions don't \
                 store; point '{field}_secret' at a scoped secret, or give a keyring \
                 reference ({{\"keyring_service\": …, \"keyring_account\": …}})"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TEST_KEY;
    use serde_json::json;

    #[test]
    fn supplied_key_is_lifted_out_and_masked() {
        let mut cfg = json!({ "cwd": "/w", "secrets": { "A": TEST_KEY, "B": TEST_KEY } });
        let key = normalize(&mut cfg, None).unwrap();
        assert_eq!(key.as_deref(), Some(TEST_KEY));
        assert_eq!(
            cfg,
            json!({ "cwd": "/w", "secrets": { "A": MASK, "B": MASK } })
        );
    }

    #[test]
    fn mask_keeps_the_existing_key() {
        let mut cfg = json!({ "secrets": { "A": MASK } });
        assert_eq!(
            normalize(&mut cfg, Some("old")).unwrap().as_deref(),
            Some("old")
        );
    }

    #[test]
    fn a_new_key_replaces_the_existing_one() {
        let mut cfg = json!({ "secrets": { "A": TEST_KEY, "B": MASK } });
        assert_eq!(
            normalize(&mut cfg, Some("old")).unwrap().as_deref(),
            Some(TEST_KEY)
        );
        assert_eq!(cfg["secrets"]["B"], json!(MASK));
    }

    #[test]
    fn mask_without_any_key_is_refused() {
        let err = normalize(&mut json!({ "secrets": { "A": MASK } }), None).unwrap_err();
        assert!(err.contains("has no key yet"), "{err}");
    }

    #[test]
    fn differing_keys_are_refused() {
        let other = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";
        let err = normalize(
            &mut json!({ "secrets": { "A": TEST_KEY, "B": other } }),
            None,
        )
        .unwrap_err();
        assert!(err.contains("more than one key"), "{err}");
    }

    #[test]
    fn malformed_key_is_refused() {
        let err = normalize(&mut json!({ "secrets": { "A": "c2hvcnQ=" } }), None).unwrap_err();
        assert!(err.contains("must decode to 32 bytes"), "{err}");
    }

    #[test]
    fn no_secrets_means_no_key() {
        assert_eq!(
            normalize(&mut json!({ "cwd": "/w" }), Some("old")).unwrap(),
            None
        );
        assert_eq!(
            normalize(&mut json!({ "secrets": {} }), Some("old")).unwrap(),
            None
        );
    }

    #[test]
    fn inline_credentials_are_refused() {
        let err = normalize(
            &mut json!({ "auth": { "type": "bearer", "token": "t" } }),
            None,
        )
        .unwrap_err();
        assert!(err.contains("auth.token is an inline credential"), "{err}");
    }

    #[test]
    fn legacy_configs_are_scrubbed() {
        let mut cfg = json!({
            "cwd": "/w",
            "secrets": { "A": TEST_KEY, "B": TEST_KEY },
            "auth": {
                "type": "bearer",
                "token": "inline",
                "refresh_token": { "keyring_service": "s", "keyring_account": "a" }
            }
        });
        let out = scrub_legacy(&mut cfg);
        assert!(out.changed);
        assert_eq!(out.key.as_deref(), Some(TEST_KEY));
        assert_eq!(out.notes.len(), 1, "{:?}", out.notes);
        assert_eq!(
            cfg,
            json!({
                "cwd": "/w",
                "secrets": { "A": MASK, "B": MASK },
                "auth": {
                    "type": "bearer",
                    "refresh_token": { "keyring_service": "s", "keyring_account": "a" }
                }
            })
        );
        // Already in shape: nothing to do.
        assert_eq!(scrub_legacy(&mut cfg), Scrubbed::default());
    }

    #[test]
    fn legacy_scrub_masks_even_bad_keys() {
        let other = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";
        let mut cfg = json!({ "secrets": { "A": TEST_KEY, "B": other, "C": "short" } });
        let out = scrub_legacy(&mut cfg);
        assert_eq!(out.key.as_deref(), Some(TEST_KEY));
        assert_eq!(out.notes.len(), 2, "{:?}", out.notes);
        assert_eq!(cfg, json!({ "secrets": { "A": MASK, "B": MASK, "C": MASK } }));
    }

    #[test]
    fn pointers_and_keyring_references_are_accepted() {
        let mut cfg = json!({
            "secrets": { "TOKEN": TEST_KEY },
            "auth": {
                "type": "oauth_refresh",
                "client_id_secret": "TOKEN",
                "refresh_token": { "keyring_service": "s", "keyring_account": "a" },
                "token_uri": "https://example.com/token"
            }
        });
        assert!(normalize(&mut cfg, None).is_ok());
    }
}
