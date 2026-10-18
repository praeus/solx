//! Keeps secret material out of `action_config`, applied to every config
//! [`crate::store`] writes.
//!
//! An action's config never holds a secret, so reads need no redaction:
//!
//! * **Keys** aren't in a config at all. Each secret scope (a package, or a
//!   standalone action — see [`solx_surface::secret_scope`]) has one AES key,
//!   which solx generates and keeps in the actions database's `secret_keys`
//!   table. `action_config.secrets` is only the list of secret *names* the
//!   action may use. [`normalize`] stores it as a list and drops whatever an
//!   older install script put in the values of the object form (`$key` or
//!   `"***"`).
//! * **Credentials** (`auth.token`, `auth.client_secret`, …) are refused
//!   inline. They are resolved at call time from a `<field>_secret` pointer
//!   into the scoped secret store, or a keyring reference — see
//!   [`crate::auth::secret_field`].

use serde_json::Value;
use solx_surface::entities::{Action, ActionInput};

/// Refuse a save that would create or change an action with secrets — one
/// whose stored or incoming config declares `secrets` — for a caller that
/// isn't root.
///
/// Root is any in-process host (the CLI, a package install, Tauri) or an
/// HTTP request with the root token; those call `ActionManager::save`
/// directly. Everyone else — an MCP tool call, a WASM guest, a script, an
/// HTTP client token — must pass this check first.
///
/// It covers the whole row, not just the names. The key comes from the
/// action's scope, so declaring a name under `/packages/<pkg>/` would read
/// that package's secret, and repointing `bin_name` or `files` of an action
/// that has secrets would run new code with them.
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

/// Validate `cfg` for storage. Rewrites `secrets` as a sorted list of names
/// (accepting the older object form, whose values are ignored) and returns
/// whether the action declares any, so the caller can make sure its scope
/// has a key.
pub(crate) fn normalize(cfg: &mut Value) -> Result<bool, String> {
    reject_inline_credentials(cfg)?;
    let Some(secrets) = cfg.get_mut("secrets") else {
        return Ok(false);
    };
    let names: Vec<String> = match secrets {
        Value::Array(items) => items
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    "action_config.secrets must list secret names (strings)".to_string()
                })
            })
            .collect::<Result<_, _>>()?,
        Value::Object(map) => map.keys().cloned().collect(),
        _ => return Err("action_config.secrets must be a list of secret names".to_string()),
    };
    for name in &names {
        validate_name(name)?;
    }
    let names: std::collections::BTreeSet<String> = names.into_iter().collect();
    let declares = !names.is_empty();
    *secrets = Value::Array(names.into_iter().map(Value::String).collect());
    Ok(declares)
}

/// A secret name becomes part of a credential-manager account
/// (`<scope>/<name>`), so it may not contain a separator or anything else
/// that could make one account look like another's.
fn validate_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "action_config.secrets: '{name}' isn't a valid secret name (letters, digits, \
             '_', '-' and '.' only)"
        ))
    }
}

/// What [`scrub_legacy`] did to one row's config.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Scrubbed {
    /// The config changed and must be written back.
    pub changed: bool,
    /// Inline credentials or keys that were dropped, for a log line.
    pub notes: Vec<String>,
}

/// Strip secret material from a config stored before [`normalize`] did:
/// keys in the values of the object form of `secrets`, and inline `auth`
/// credentials. Both would otherwise be returned by every `get` and `list`.
/// The keys are discarded, not kept: each scope's key is regenerated, so
/// secrets stored under an old key read as not set. Unlike `normalize` it
/// never fails: it runs at startup over rows nobody is there to fix.
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
    if let Some(Value::Object(map)) = cfg.get("secrets") {
        let held_a_key = map.values().any(|v| v.as_str() != Some("***"));
        let names: Vec<Value> = map.keys().cloned().map(Value::String).collect();
        cfg["secrets"] = Value::Array(names);
        out.changed = true;
        if held_a_key {
            out.notes.push(
                "dropped a key stored in action_config.secrets; the scope gets a new key, \
                 so secrets stored under the old one must be set again"
                    .to_string(),
            );
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
    fn names_are_stored_as_a_sorted_list() {
        let mut cfg = json!({ "cwd": "/w", "secrets": ["B", "A", "B"] });
        assert!(normalize(&mut cfg).unwrap());
        assert_eq!(cfg, json!({ "cwd": "/w", "secrets": ["A", "B"] }));
    }

    /// The form older install scripts write: its values (a key, or "***")
    /// are dropped, never stored.
    #[test]
    fn the_object_form_keeps_only_its_names() {
        let mut cfg = json!({ "secrets": { "A": TEST_KEY, "B": "***" } });
        assert!(normalize(&mut cfg).unwrap());
        assert_eq!(cfg, json!({ "secrets": ["A", "B"] }));
        assert!(!cfg.to_string().contains(TEST_KEY));
    }

    #[test]
    fn no_names_means_no_secrets() {
        assert!(!normalize(&mut json!({ "cwd": "/w" })).unwrap());
        assert!(!normalize(&mut json!({ "secrets": [] })).unwrap());
    }

    #[test]
    fn bad_names_and_shapes_are_refused() {
        for bad in [
            json!({ "secrets": ["a/b"] }),
            json!({ "secrets": [""] }),
            json!({ "secrets": [42] }),
            json!({ "secrets": "A" }),
        ] {
            let mut cfg = bad.clone();
            assert!(normalize(&mut cfg).is_err(), "{bad}");
        }
    }

    #[test]
    fn inline_credentials_are_refused() {
        let err = normalize(&mut json!({ "auth": { "type": "bearer", "token": "t" } }))
            .unwrap_err();
        assert!(err.contains("auth.token is an inline credential"), "{err}");
    }

    #[test]
    fn pointers_and_keyring_references_are_accepted() {
        let mut cfg = json!({
            "secrets": ["TOKEN"],
            "auth": {
                "type": "oauth_refresh",
                "client_id_secret": "TOKEN",
                "refresh_token": { "keyring_service": "s", "keyring_account": "a" },
                "token_uri": "https://example.com/token"
            }
        });
        assert!(normalize(&mut cfg).is_ok());
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
        assert_eq!(out.notes.len(), 2, "{:?}", out.notes);
        assert_eq!(
            cfg,
            json!({
                "cwd": "/w",
                "secrets": ["A", "B"],
                "auth": {
                    "type": "bearer",
                    "refresh_token": { "keyring_service": "s", "keyring_account": "a" }
                }
            })
        );
        // Already in shape: nothing to do.
        assert_eq!(scrub_legacy(&mut cfg), Scrubbed::default());
    }

    /// A masked object (the shape just before this change) holds no key,
    /// so it only becomes a list.
    #[test]
    fn a_masked_object_becomes_a_list_quietly() {
        let mut cfg = json!({ "secrets": { "A": "***" } });
        let out = scrub_legacy(&mut cfg);
        assert!(out.changed);
        assert!(out.notes.is_empty(), "{:?}", out.notes);
        assert_eq!(cfg, json!({ "secrets": ["A"] }));
    }
}
