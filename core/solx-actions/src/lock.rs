//! Locked actions and their pinned artifacts.
//!
//! A Script or WASM action runs whatever bytes its `bin_name` resolves to,
//! with its own identity — and so its own scoped secrets. Content that can
//! write the file store (`file-put`, `file-copy`, `dir-copy`) or repoint the
//! row (`entity-save-action`) could otherwise swap in its own code and read
//! those secrets. Locking closes both:
//!
//! * **The row.** A locked action can be saved, deleted, locked or unlocked
//!   only through an operator path: the `solx` CLI, a package install, the
//!   root token over REST, or the desktop app's Settings window. Everything
//!   content can reach (every `/builtin/entity/*` call, whatever principal
//!   the chain carries; REST with a client or action token; the desktop
//!   app's main window, which hosts widgets) applies
//!   [`guard_locked_action`]. Content may not set `locked` at all.
//! * **The artifact.** Saving a locked Script/WASM action pins its artifact:
//!   the store path it resolved to and the SHA-256 of its bytes, in columns
//!   no input can write. Dispatch reads only the pinned path and refuses
//!   bytes that no longer match ([`load_artifact`]), so an overwrite can
//!   stop a locked action running but never run new code as it. The file
//!   built-ins also refuse to write or delete a pinned path
//!   ([`pinned_target`]), so the overwrite doesn't happen in the first
//!   place.
//!
//! Every operator save of a locked action re-pins it, which is how a
//! reinstall (or `solx action lock`) picks up a new artifact.
//!
//! A keyed action (scoped secrets, or an `auth` keyring reference — see
//! [`solx_surface::is_keyed`]) is always locked: `save` locks it and
//! refuses to unlock it while it is keyed, and [`lock_keyed_actions`]
//! locks rows saved before that rule. Unlocked, its artifact or URL would
//! be content's to rewrite, and the rewrite would run with its credential.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use solx_surface::entities::{Action, ActionInput, ArtifactPin};
use solx_surface::error::{Result, SolxError};
use solx_surface::managers::FileStore;

/// Refuse, for content, any save or delete of a locked action, and any
/// input that sets `locked` (content may neither lock nor unlock).
///
/// `existing` is the stored row, so a merge-upsert that omits `locked` is
/// still caught; `incoming` is `None` for a delete.
pub fn guard_locked_action(
    existing: Option<&Action>,
    incoming: Option<&ActionInput>,
    reference: &str,
) -> std::result::Result<(), String> {
    if existing.is_some_and(|a| a.locked) {
        return Err(format!(
            "{reference} is locked; only an operator (the solx CLI, a package install, \
             the root token, or the desktop app's Settings window) can change or delete it"
        ));
    }
    if incoming.is_some_and(|i| i.locked.is_some()) {
        return Err(format!(
            "cannot lock or unlock {reference} from here; use `solx action lock` or \
             `solx action unlock`"
        ));
    }
    Ok(())
}

/// Lowercase hex SHA-256.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Find an action's artifact the way dispatch always has: the shared
/// location first, then the action's own. Returns the store path it was
/// found at and its bytes.
pub(crate) async fn resolve_artifact(
    files: &Arc<dyn FileStore>,
    action_id: &str,
    bin_name: &str,
    kind: &str,
) -> Result<(String, Vec<u8>)> {
    let shared = solx_files::shared_action_file_path(bin_name);
    if let Ok(bytes) = files.get(&shared).await {
        return Ok((shared, bytes));
    }
    let owned = solx_files::action_file_path(action_id, bin_name);
    match files.get(&owned).await {
        Ok(bytes) => Ok((owned, bytes)),
        Err(_) => Err(SolxError::Exec(format!(
            "{kind} artifact '{bin_name}' not found (tried '{shared}' and '{owned}')"
        ))),
    }
}

/// The pin for an action being saved locked: its artifact's path and hash.
pub(crate) async fn pin_artifact(
    files: &Arc<dyn FileStore>,
    action_id: &str,
    bin_name: Option<&str>,
    reference: &str,
) -> Result<ArtifactPin> {
    let bin_name = bin_name.ok_or_else(|| {
        SolxError::Invalid(format!(
            "cannot lock {reference}: it has no bin_name (artifact)"
        ))
    })?;
    let (rel_path, bytes) = resolve_artifact(files, action_id, bin_name, "locked")
        .await
        .map_err(|e| SolxError::Invalid(format!("cannot lock {reference}: {e}")))?;
    Ok(ArtifactPin {
        rel_path,
        sha256: sha256_hex(&bytes),
    })
}

/// An action's artifact bytes, for execution. A pinned action reads only
/// its pinned path — never re-resolving, so a same-named shared file can't
/// shadow an owned one — and refuses bytes that don't match the pin.
pub(crate) async fn load_artifact(
    files: &Arc<dyn FileStore>,
    action: &Action,
    reference: &str,
    bin_name: &str,
    kind: &str,
) -> Result<Vec<u8>> {
    let Some(pin) = &action.artifact_pin else {
        // Only a row locked before its artifact existed (see
        // [`lock_keyed_actions`]): running whatever resolves would defeat
        // the lock.
        if action.locked {
            return Err(SolxError::Exec(format!(
                "locked action {reference} has no pinned {kind} artifact; lock it again with \
                 `solx action lock` (or reinstall its package) once '{bin_name}' is in place"
            )));
        }
        return resolve_artifact(files, &action.id.to_string(), bin_name, kind)
            .await
            .map(|(_, bytes)| bytes);
    };
    let bytes = files.get(&pin.rel_path).await.map_err(|_| {
        SolxError::Exec(format!(
            "{kind} artifact for locked action {reference} is missing from '{}'",
            pin.rel_path
        ))
    })?;
    if sha256_hex(&bytes) != pin.sha256 {
        return Err(SolxError::Exec(format!(
            "{kind} artifact for {reference} ('{}') changed since it was locked; \
             re-lock it with `solx action lock` (or reinstall its package) if the change is intended",
            pin.rel_path
        )));
    }
    Ok(bytes)
}

/// Lock every keyed action that isn't locked yet, pinning a Script/WASM
/// action's artifact. `save` keeps that true from now on; this catches rows
/// saved before it did. Runs at every open, but finds nothing once they are
/// locked.
///
/// A Script/WASM action whose artifact is missing is still locked, unpinned,
/// which [`load_artifact`] refuses to run until an operator locks it again.
pub(crate) async fn lock_keyed_actions(
    conn: &libsql::Connection,
    files: &Arc<dyn FileStore>,
) -> Result<()> {
    use solx_surface::entities::ActionType;

    let mut rows = conn
        .query(
            &format!(
                "{} WHERE locked=0 AND (action_config LIKE '%\"secrets\"%' \
                 OR action_config LIKE '%\"keyring_service\"%')",
                crate::row::SELECT
            ),
            (),
        )
        .await
        .map_err(crate::db::map_db)?;
    let mut keyed = Vec::new();
    while let Some(row) = rows.next().await.map_err(crate::db::map_db)? {
        let action = crate::row::row_to_action(&row)?;
        if solx_surface::is_keyed(action.action_config.as_ref()) {
            keyed.push(action);
        }
    }
    for action in keyed {
        let reference = solx_surface::path::full_ref(&action.path, &action.name)?;
        let pin = match action.action_type {
            Some(ActionType::Script | ActionType::Wasm) => {
                match pin_artifact(
                    files,
                    &action.id.to_string(),
                    action.bin_name.as_deref(),
                    &reference,
                )
                .await
                {
                    Ok(pin) => Some(pin),
                    Err(e) => {
                        tracing::warn!(
                            "{e}; locked without a pin, so it won't run until re-locked"
                        );
                        None
                    }
                }
            }
            _ => None,
        };
        tracing::info!(
            "locked {reference}: it has scoped secrets, declared scopes or a keyring reference"
        );
        conn.execute(
            "UPDATE actions SET locked=1, pin_path=?2, pin_sha256=?3 WHERE id=?1",
            libsql::params![
                action.id.to_string(),
                pin.as_ref().map(|p| p.rel_path.clone()).unwrap_or_default(),
                pin.map(|p| p.sha256).unwrap_or_default()
            ],
        )
        .await
        .map_err(crate::db::map_db)?;
    }
    Ok(())
}

/// One path in a form for comparing: what Windows would open is the same
/// for differently-cased names and ignores trailing dots and spaces in a
/// segment, so `Login.SOLX.` and `login.solx` must compare equal. On a
/// case-sensitive filesystem that only ever refuses more, never less.
fn comparable(path: &str) -> String {
    path.split('/')
        .map(|seg| seg.trim_end_matches(['.', ' ']).to_lowercase())
        .collect::<Vec<_>>()
        .join("/")
}

/// The first of `targets` (store-relative paths about to be written or
/// deleted) that is a pinned artifact, if any. A target the store would
/// refuse anyway is skipped; the store reports it.
pub(crate) fn pinned_target<'a>(
    pinned: &[String],
    targets: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let pinned: Vec<String> = pinned.iter().map(|p| comparable(p)).collect();
    targets.into_iter().find_map(|target| {
        let normalized = solx_files::normalize_rel_path(target).ok()?;
        pinned
            .contains(&comparable(&normalized))
            .then_some(normalized)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locked() -> Action {
        serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "path": "/p",
            "name": "a",
            "locked": true,
        }))
        .unwrap()
    }

    #[test]
    fn content_can_neither_touch_a_locked_row_nor_set_the_flag() {
        let unlocked = Action {
            locked: false,
            ..locked()
        };
        assert!(guard_locked_action(Some(&locked()), None, "/p/a").is_err());
        assert!(
            guard_locked_action(Some(&locked()), Some(&ActionInput::default()), "/p/a").is_err()
        );
        for flag in [true, false] {
            let input = ActionInput {
                locked: Some(flag),
                ..Default::default()
            };
            assert!(guard_locked_action(Some(&unlocked), Some(&input), "/p/a").is_err());
            assert!(guard_locked_action(None, Some(&input), "/p/a").is_err());
        }
        assert!(
            guard_locked_action(Some(&unlocked), Some(&ActionInput::default()), "/p/a").is_ok()
        );
        assert!(guard_locked_action(None, None, "/p/a").is_ok());
    }

    #[test]
    fn pinned_paths_match_as_the_store_and_windows_would_open_them() {
        let pinned = vec!["files/actions/shared/login.solx".to_string()];
        let hit = |t: &str| pinned_target(&pinned, [t]).is_some();
        assert!(hit("files/actions/shared/login.solx"));
        assert!(hit("files//actions\\shared/./login.solx"));
        assert!(hit("files/Actions/shared/LOGIN.solx."));
        assert!(!hit("files/actions/shared/login.solx2"));
        assert!(!hit("files/actions/shared"));
        assert!(pinned_target(&pinned, ["a.txt", "files/actions/shared/login.solx"]).is_some());
    }
}

/// End to end through dispatch: what content can and can't do to a locked
/// action, and what the operator path (`ActionManager::save`) does.
#[cfg(test)]
mod lifecycle_tests {
    use std::sync::Arc;

    use serde_json::{json, Value};
    use solx_surface::entities::{ActionInput, ActionType};
    use solx_surface::managers::ActionManager;
    use solx_surface::{ExecContext, Principal};

    use crate::seed::{ACTION_PATH, FILE_PATH};
    use crate::testing::support::setup_wired;
    use crate::LocalActionManager;

    const BIN: &str = "locked.solx";

    fn shared() -> String {
        solx_files::shared_action_file_path(BIN)
    }

    async fn put(m: &LocalActionManager, rel: &str, source: &str) {
        m.files.put(rel, source.as_bytes().to_vec()).await.unwrap();
    }

    /// Operator: a Script action running `json 42`, saved locked.
    async fn locked_script(m: &LocalActionManager) {
        put(m, &shared(), "json 42").await;
        m.save(
            "/p",
            "s",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some(BIN.into()),
                locked: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    async fn run(m: &LocalActionManager) -> solx_surface::Result<Value> {
        m.exec("/p", "s", json!({})).await.map(|r| r.result)
    }

    async fn builtin(m: &Arc<LocalActionManager>, path: &str, name: &str, params: Value) -> String {
        m.exec(path, name, params).await.unwrap_err().to_string()
    }

    #[tokio::test]
    async fn locking_pins_the_artifact_and_needs_one() {
        let (_d, _cfg, m) = setup_wired().await;
        locked_script(&m).await;
        let action = m.get("/p", "s").await.unwrap();
        assert!(action.locked);
        let pin = action.artifact_pin.expect("pinned");
        assert_eq!(pin.rel_path, shared());
        assert_eq!(pin.sha256, super::sha256_hex(b"json 42"));
        assert_eq!(run(&m).await.unwrap(), json!(42));

        let err = m
            .save(
                "/p",
                "missing",
                ActionInput {
                    action_type: Some(ActionType::Script),
                    bin_name: Some("nope.solx".into()),
                    locked: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot lock"), "{err}");
    }

    #[tokio::test]
    async fn a_changed_artifact_is_refused_and_content_cant_change_it() {
        let (_d, _cfg, m) = setup_wired().await;
        locked_script(&m).await;

        // Content: every file built-in that could replace or remove it.
        let evil = json!({"rel_path": shared(), "content": "json 666"});
        for (name, params) in [
            ("file-put", evil),
            ("file-delete", json!({"rel_path": shared()})),
            (
                "file-copy",
                json!({"source": shared(), "dest": "files/actions/SHARED/Locked.solx."}),
            ),
            ("dir-delete", json!({"rel_path": "files/actions"})),
        ] {
            let err = builtin(&m, FILE_PATH, name, params).await;
            assert!(err.contains("artifact of a locked action"), "{name}: {err}");
        }
        put(&m, "files/staging/other.solx", "json 666").await;
        put(&m, &format!("files/staging/{BIN}"), "json 666").await;
        let err = builtin(
            &m,
            FILE_PATH,
            "dir-copy",
            json!({"source": "files/staging", "dest": "files/actions/shared"}),
        )
        .await;
        assert!(
            err.contains("artifact of a locked action"),
            "dir-copy: {err}"
        );
        // A refused directory operation touched nothing.
        assert!(m
            .files
            .get("files/actions/shared/other.solx")
            .await
            .is_err());
        assert_eq!(run(&m).await.unwrap(), json!(42));

        // Anything that gets past the built-ins (a direct disk edit) stops
        // the action rather than running as it.
        put(&m, &shared(), "json 666").await;
        let err = run(&m).await.unwrap_err();
        assert!(
            err.to_string().contains("changed since it was locked"),
            "{err}"
        );

        // The operator re-locks, which re-pins.
        m.save(
            "/p",
            "s",
            ActionInput {
                locked: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(run(&m).await.unwrap(), json!(666));
    }

    #[tokio::test]
    async fn a_shared_file_cant_shadow_a_pinned_owned_artifact() {
        let (_d, _cfg, m) = setup_wired().await;
        let action = m
            .save(
                "/p",
                "s",
                ActionInput {
                    action_type: Some(ActionType::Script),
                    bin_name: Some(BIN.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let owned = solx_files::action_file_path(&action.id.to_string(), BIN);
        put(&m, &owned, "json 42").await;
        m.save(
            "/p",
            "s",
            ActionInput {
                locked: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            m.get("/p", "s")
                .await
                .unwrap()
                .artifact_pin
                .unwrap()
                .rel_path,
            owned
        );

        // Unpinned, the shared location would win.
        put(&m, &shared(), "json 666").await;
        assert_eq!(run(&m).await.unwrap(), json!(42));
    }

    #[tokio::test]
    async fn content_cant_change_delete_lock_or_unlock_whatever_its_principal() {
        let (_d, _cfg, m) = setup_wired().await;
        locked_script(&m).await;
        put(&m, "files/actions/shared/evil.solx", "json 666").await;

        let repoint = json!({"path": "/p", "name": "s", "binName": "evil.solx"});
        let unlock = json!({"path": "/p", "name": "s", "locked": false});
        let delete = json!({"path": "/p", "name": "s"});
        for (name, params) in [
            ("entity-save-action", repoint),
            ("entity-save-action", unlock),
            ("entity-delete-action", delete),
        ] {
            // Unauthenticated, as the root token, and in the tooling scope.
            let local = m.exec(ACTION_PATH, name, params.clone()).await.unwrap_err();
            let root = m
                .exec_for(&Principal::Root, ACTION_PATH, name, params.clone())
                .await
                .unwrap_err();
            let tooling = m
                .exec_tooling(Some(&Principal::Root), ACTION_PATH, name, params)
                .await
                .unwrap_err();
            for err in [local, root, tooling] {
                assert!(err.to_string().contains("is locked"), "{name}: {err}");
            }
        }

        // Nor lock an unlocked action.
        m.save(
            "/p",
            "open",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some(BIN.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let err = builtin(
            &m,
            ACTION_PATH,
            "entity-save-action",
            json!({"path": "/p", "name": "open", "locked": true}),
        )
        .await;
        assert!(err.contains("cannot lock or unlock"), "{err}");

        let action = m.get("/p", "s").await.unwrap();
        assert!(action.locked);
        assert_eq!(action.bin_name.as_deref(), Some(BIN));
        assert_eq!(run(&m).await.unwrap(), json!(42));
        assert_eq!(
            m.exec_as("/p", "s", json!({}), &ExecContext::tooling(None))
                .await
                .unwrap()
                .result,
            json!(42)
        );
    }

    fn keyed_script() -> ActionInput {
        ActionInput {
            action_type: Some(ActionType::Script),
            bin_name: Some(BIN.into()),
            action_config: Some(json!({ "secrets": ["TOKEN"] })),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn an_action_with_secrets_is_always_locked_and_pinned() {
        let (_d, _cfg, m) = setup_wired().await;
        put(&m, &shared(), "json 42").await;
        // Saved without asking for a lock.
        let action = m.save("/p", "s", keyed_script()).await.unwrap();
        assert!(action.locked);
        assert_eq!(action.artifact_pin.expect("pinned").rel_path, shared());

        let err = m
            .save(
                "/p",
                "s",
                ActionInput {
                    locked: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot unlock"), "{err}");

        // So content can neither swap its code nor delete it (and its key).
        let err = builtin(
            &m,
            FILE_PATH,
            "file-put",
            json!({"rel_path": shared(), "content": "json 666"}),
        )
        .await;
        assert!(err.contains("artifact of a locked action"), "{err}");
        let err = builtin(
            &m,
            ACTION_PATH,
            "entity-delete-action",
            json!({"path": "/p", "name": "s"}),
        )
        .await;
        assert!(err.contains("is locked"), "{err}");

        // Dropping its secrets is what lets an operator unlock it.
        let action = m
            .save(
                "/p",
                "s",
                ActionInput {
                    action_config: Some(json!({})),
                    locked: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!action.locked);
    }

    #[tokio::test]
    async fn an_action_with_secrets_needs_its_artifact_to_save() {
        let (_d, _cfg, m) = setup_wired().await;
        let err = m.save("/p", "s", keyed_script()).await.unwrap_err();
        assert!(err.to_string().contains("cannot lock"), "{err}");
    }

    /// A row saved before secrets implied a lock is locked when the store
    /// opens; one whose artifact is missing is locked unpinned and won't run.
    #[tokio::test]
    async fn opening_locks_keyed_rows_left_unlocked() {
        let (_d, _cfg, m) = setup_wired().await;
        put(&m, &shared(), "json 42").await;
        m.save("/p", "s", keyed_script()).await.unwrap();
        m.save("/p", "gone", keyed_script()).await.unwrap();
        let conn = m.db.connect().await.unwrap();
        conn.execute(
            "UPDATE actions SET locked=0, pin_path='', pin_sha256='' WHERE path='/p'",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "UPDATE actions SET bin_name='nope.solx' WHERE name='gone'",
            (),
        )
        .await
        .unwrap();

        super::lock_keyed_actions(&conn, &m.files).await.unwrap();

        let action = m.get("/p", "s").await.unwrap();
        assert!(action.locked);
        assert_eq!(
            action.artifact_pin.expect("pinned").sha256,
            super::sha256_hex(b"json 42")
        );
        assert_eq!(run(&m).await.unwrap(), json!(42));

        let gone = m.get("/p", "gone").await.unwrap();
        assert!(gone.locked && gone.artifact_pin.is_none());
        // Even with a file now at the name it would have resolved.
        put(&m, "files/actions/shared/nope.solx", "json 666").await;
        let err = m.exec("/p", "gone", json!({})).await.unwrap_err();
        assert!(err.to_string().contains("no pinned"), "{err}");
    }

    #[tokio::test]
    async fn unlocking_clears_the_pin_and_frees_the_artifact() {
        let (_d, _cfg, m) = setup_wired().await;
        locked_script(&m).await;
        m.save(
            "/p",
            "s",
            ActionInput {
                locked: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let action = m.get("/p", "s").await.unwrap();
        assert!(!action.locked);
        assert!(action.artifact_pin.is_none());

        m.exec(
            FILE_PATH,
            "file-put",
            json!({"rel_path": shared(), "content": "json 7"}),
        )
        .await
        .unwrap();
        assert_eq!(run(&m).await.unwrap(), json!(7));
    }
}
