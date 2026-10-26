//! Built-in action catalogue, seeded into the actions database on open.
//! Re-seeding on every startup is idempotent.
//!
//! The catalogue is the only source of `Internal` actions. `save` refuses
//! to create or change one (see `LocalActionManager::save`), seeding writes
//! every built-in locked and restores its catalogue fields over whatever
//! the row holds, and dispatch runs an `Internal` row only if it is in the
//! catalogue (`LocalActionManager::is_catalogued`). So no row can alias a
//! built-in handler under another path, where tooling exclusions keyed on
//! the built-in's path wouldn't see it, and no built-in can be repointed or
//! redescribed. [`retire_internal_aliases`] deletes the aliases saved
//! before that rule.
//!
//! Every built-in is dispatched natively (`action_type='internal'`, see
//! `crate::internal`) — there is no WASM-hosted built-in catalogue anymore.
//! Entity CRUD, search, general-purpose file-store access, document field
//! ops, an environment scratch store, HTML fetch, and scoped secrets all
//! used to be a `solx-builtin-actions` WASM component executed under
//! wasmtime's trusted `backend-action` world; that world (and the
//! component) is gone. WASM now exists solely for third-party *custom*
//! actions (`crate::wasm_host`), which reach every one of these same
//! operations recursively via `action-exec` — no separate WASM ABI needed
//! for them.
//!
//! NOTE: the `/builtin/<area>/*` reorg below moved several actions to a new
//! `path`. A row at an old path is no longer in the catalogue, so it won't
//! run; delete `db/solx-actions.db` once after upgrading past that change to
//! drop them from listings too.

use chrono::Utc;
use libsql::Connection;
use solx_surface::error::Result;
use solx_surface::internal_actions::SeedAction;
use uuid::Uuid;

use crate::db::map_db;

/// Subdivision of the builtin namespace for action operations: the
/// asynchronous `start`/`stop`/`poll`/`cancelled` alternative to `exec` (see
/// `docs/async-actions-plan.md`), plus action-entity CRUD and search.
pub const ACTION_PATH: &str = "/builtin/action";

/// Subdivision of the builtin namespace for document-entity CRUD, field
/// ops, and search.
pub const DOCUMENT_PATH: &str = "/builtin/document";

/// Subdivision of the builtin namespace for type-entity CRUD.
pub const TYPE_PATH: &str = "/builtin/type";

/// Subdivision of the builtin namespace for the general-purpose file store.
pub const FILE_PATH: &str = "/builtin/file";

/// Subdivision of the builtin namespace for the environment scratch store.
pub const ENV_PATH: &str = "/builtin/env";

/// Subdivision of the builtin namespace for per-caller-scoped secrets.
pub const SECRETS_PATH: &str = "/builtin/secrets";

/// Subdivision of the builtin namespace for the OAuth 2.0 loopback listener.
pub const OAUTH_PATH: &str = "/builtin/oauth";

/// Subdivision of the builtin namespace for everything that deals in URLs:
/// a one-shot HTTP request, opening a URL in the system browser, and (under
/// `WEB_STREAM_PATH`) host-side streaming HTTP.
pub const WEB_PATH: &str = "/builtin/web";

/// Subdivision of `WEB_PATH` for host-side streaming HTTP (`start`/`poll`/
/// `close`) — for callers with no sockets or cross-call state of their own
/// (chiefly WASM guests). See
/// `solx-packages/solx-ollama/docs/streaming-design.md`.
pub const WEB_STREAM_PATH: &str = "/builtin/web/stream";

/// Namespace for the hand-written JSON-schema types backing built-in
/// actions' `param_type_ref` (see `solx-types/src/seed.rs`). Shared by every
/// `/builtin/*` subpath — `param_type_ref` doesn't need to mirror the
/// action's own nesting.
pub const BUILTIN_TYPES_PATH: &str = "/builtin/types";

/// A nested entry under some `path` other than the flat `/builtin` root,
/// with its own dispatch key distinct from its (possibly generic) `name`.
const fn a_at(
    path: &'static str,
    name: &'static str,
    fn_name: &'static str,
    description: &'static str,
    param_type: Option<&'static str>,
) -> SeedAction {
    SeedAction {
        path,
        name,
        fn_name,
        description,
        param_type,
    }
}

/// The built-in action catalogue.
pub fn builtin_actions() -> Vec<SeedAction> {
    vec![
        // Document CRUD
        a_at(DOCUMENT_PATH, "entity-save-document", "entity-save-document", "Create or update (upsert) a document.", Some("DocumentCrudParams")),
        a_at(DOCUMENT_PATH, "entity-get-document", "entity-get-document", "Fetch a document by path+name.", Some("EntityRefParams")),
        a_at(DOCUMENT_PATH, "entity-delete-document", "entity-delete-document", "Delete a document.", Some("EntityRefParams")),
        a_at(DOCUMENT_PATH, "entity-list-documents", "entity-list-documents", "List documents, optionally filtered by path prefix.", Some("ListParams")),
        a_at(DOCUMENT_PATH, "entity-list-document-paths", "entity-list-document-paths", "List distinct document paths in use, with a count of documents at each, optionally filtered by path prefix.", Some("ListParams")),
        // Document legacy field ops (one field at a time)
        a_at(DOCUMENT_PATH, "get-field", "get-field", "Read one field from a document's contents.", Some("GetFieldParams")),
        a_at(DOCUMENT_PATH, "set-field", "set-field", "Write one field on a document's contents.", Some("SetFieldParams")),
        // Document path-style field ops (nested reads/writes via dotted path)
        a_at(DOCUMENT_PATH, "get-field-at-path", "get-field-at-path", "Read a field at a slash-separated path inside a document's contents.", Some("GetFieldAtPathParams")),
        a_at(DOCUMENT_PATH, "set-field-at-path", "set-field-at-path", "Write a field at a slash-separated path inside a document's contents (optionally creating missing parents).", Some("SetFieldAtPathParams")),
        // Document search (SQLite FTS5)
        a_at(DOCUMENT_PATH, "search-documents", "search-documents", "Full-text + faceted search over documents.", Some("SearchDocumentsParams")),
        // Action CRUD (alongside the async start/stop/poll/cancelled below)
        a_at(ACTION_PATH, "entity-save-action", "entity-save-action", "Create or update (upsert) an action.", Some("ActionCrudParams")),
        a_at(ACTION_PATH, "entity-get-action", "entity-get-action", "Fetch an action by path+name.", Some("EntityRefParams")),
        a_at(ACTION_PATH, "entity-delete-action", "entity-delete-action", "Delete an action.", Some("EntityRefParams")),
        a_at(ACTION_PATH, "entity-list-actions", "entity-list-actions", "List actions, optionally filtered by path prefix.", Some("ListParams")),
        a_at(ACTION_PATH, "entity-list-action-paths", "entity-list-action-paths", "List distinct action paths in use, with a count of actions at each, optionally filtered by path prefix.", Some("ListParams")),
        // Action search (a real FTS5 full-text index)
        a_at(ACTION_PATH, "search-actions", "search-actions", "Full-text + faceted search over actions.", Some("SearchActionsParams")),
        // Type CRUD
        a_at(TYPE_PATH, "entity-save-type", "entity-save-type", "Create or update (upsert) a type.", Some("TypeCrudParams")),
        a_at(TYPE_PATH, "entity-get-type", "entity-get-type", "Fetch a type by path+name.", Some("EntityRefParams")),
        a_at(TYPE_PATH, "entity-delete-type", "entity-delete-type", "Delete a type.", Some("EntityRefParams")),
        a_at(TYPE_PATH, "entity-list-types", "entity-list-types", "List types, optionally filtered by path prefix.", Some("ListParams")),
        a_at(TYPE_PATH, "entity-list-type-paths", "entity-list-type-paths", "List distinct type paths in use, with a count of types at each, optionally filtered by path prefix.", Some("ListParams")),
        // General-purpose file store (unrestricted rel_path access)
        a_at(FILE_PATH, "file-put", "file-put", "Write bytes to a rel-path under the files root.", Some("FilePutParams")),
        a_at(FILE_PATH, "file-get", "file-get", "Read bytes from a rel-path under the files root.", Some("FileGetParams")),
        a_at(FILE_PATH, "file-delete", "file-delete", "Delete a file at a rel-path under the files root.", Some("FileGetParams")),
        a_at(FILE_PATH, "file-list", "file-list", "List stored rel-paths under a prefix.", Some("FileListParams")),
        a_at(FILE_PATH, "file-copy", "file-copy", "Copy a file within the files root.", Some("FileCopyParams")),
        a_at(FILE_PATH, "dir-copy", "dir-copy", "Recursively copy a directory within the files root.", Some("FileCopyParams")),
        a_at(FILE_PATH, "dir-delete", "dir-delete", "Recursively delete a directory (and every file under it) within the files root.", Some("DirDeleteParams")),
        // Environment scratch store
        a_at(ENV_PATH, "get-env", "get-env", "Read a variable from the environment store, optionally from a named namespace.", Some("GetEnvParams")),
        a_at(ENV_PATH, "set-env", "set-env", "Write a variable to the environment store. In-memory by default; pass persist to also store it in solx-config.json so it survives a restart.", Some("SetEnvParams")),
        // Secrets, scoped to whichever action is currently executing
        a_at(SECRETS_PATH, "get-secret", "get-secret", "Read a secret scoped to the calling action.", Some("GetSecretParams")),
        a_at(SECRETS_PATH, "set-secret", "set-secret", "Write a secret scoped to the calling action.", Some("SetSecretParams")),
        // OAuth loopback
        a_at(OAUTH_PATH, "oauth-start", "oauth-start", "Start a local OAuth 2.0 authorization-code loopback listener.", Some("OauthStartParams")),
        a_at(OAUTH_PATH, "oauth-await", "oauth-await", "Block until the OAuth loopback for a state_value receives its callback.", Some("OauthAwaitParams")),
        a_at(OAUTH_PATH, "oauth-stop", "oauth-stop", "Stop an OAuth loopback listener.", Some("OauthStopParams")),
        // Web — a one-shot HTTP request and opening a URL in the system
        // browser; host-side streaming HTTP lives under WEB_STREAM_PATH below.
        a_at(WEB_PATH, "http-request", "http-request", "Issue an HTTP request with optional method, headers, body, and timeout.", Some("HttpRequestParams")),
        a_at(WEB_PATH, "open-url", "open-url", "Open a URL in the system browser via the platform-native handler (xdg-open / open / ShellExecuteW).", Some("OpenUrlParams")),
        // Action consoles (`/builtin/console/*`) and asynchronous actions
        // (`/builtin/action/{start,stop,poll,cancelled}`) are seeded by
        // `solx-console` itself — see `solx_console::actions::seed_actions`,
        // merged in by `LocalActionManager::open`'s call to `seed_builtins`.
        // Host-side streaming HTTP — for callers with no sockets or
        // cross-call state of their own. Unrestricted by caller, like the
        // OAuth loopback and the action consoles: access is a bearer
        // capability on the unguessable stream_id.
        a_at(WEB_STREAM_PATH, "start", "http-stream-start", "Issue a streaming HTTP request. Returns stream_id and status as soon as response headers arrive, without waiting for the body.", Some("HttpStreamStartParams")),
        a_at(WEB_STREAM_PATH, "poll", "http-stream-poll", "Drain newline-delimited JSON chunks buffered for a stream since cursor, optionally long-polling up to wait_secs for more.", Some("HttpStreamPollParams")),
        a_at(WEB_STREAM_PATH, "close", "http-stream-close", "Stop a stream's reader task and drop its buffer.", Some("HttpStreamCloseParams")),
    ]
}

/// Seed the built-in catalogue into `actions`: insert each entry locked, or,
/// where its `(path, name)` already exists, overwrite the row with the
/// catalogue's fields and lock it. Idempotent; an unchanged row keeps its
/// `id` and `updated_at`. `extra` is merged in alongside this crate's own
/// [`builtin_actions`] — entries contributed by an internal-action plugin
/// crate (e.g. `solx_console::actions::seed_actions()`), passed in here
/// rather than hard-coded into this catalogue.
pub async fn seed_builtins(conn: &Connection, extra: &[SeedAction]) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    for entry in builtin_actions().iter().chain(extra.iter()) {
        let param_type_ref = entry
            .param_type
            .map(|n| format!("{BUILTIN_TYPES_PATH}/{n}"))
            .unwrap_or_default();
        conn.execute(
            "INSERT INTO actions \
             (id,path,name,caption,description,capabilities,phrases,category,\
              param_type_ref,result_type_ref,action_type,fn_name,bin_name,\
              action_config,files,trusted,created_at,updated_at,locked,pin_path,pin_sha256) \
             VALUES (?1,?2,?3,'',?4,'[]','[]','',\
                     ?5,'','internal',?6,'',\
                     'null','[]',1,?7,?7,1,'','') \
             ON CONFLICT(path,name) DO UPDATE SET \
               caption='',description=excluded.description,capabilities='[]',phrases='[]',\
               category='',param_type_ref=excluded.param_type_ref,result_type_ref='',\
               action_type='internal',fn_name=excluded.fn_name,bin_name='',\
               action_config='null',files='[]',trusted=1,locked=1,pin_path='',pin_sha256='' \
             WHERE caption<>'' OR description<>excluded.description OR capabilities<>'[]' \
               OR phrases<>'[]' OR category<>'' OR param_type_ref<>excluded.param_type_ref \
               OR result_type_ref<>'' OR action_type<>'internal' OR fn_name<>excluded.fn_name \
               OR bin_name<>'' OR action_config<>'null' OR files<>'[]' OR trusted<>1 \
               OR locked<>1 OR pin_path<>'' OR pin_sha256<>''",
            libsql::params![
                Uuid::new_v4().to_string(),
                entry.path.to_string(),
                entry.name.to_string(),
                entry.description.to_string(),
                param_type_ref,
                entry.fn_name.to_string(),
                now.clone(),
            ],
        )
        .await
        .map_err(map_db)?;
    }
    for (path, name, fn_name) in RETIRED_BUILTINS {
        conn.execute(
            "DELETE FROM actions              WHERE path = ?1 AND name = ?2 AND action_type = 'internal' AND fn_name = ?3",
            libsql::params![*path, *name, *fn_name],
        )
        .await
        .map_err(map_db)?;
    }
    Ok(())
}

/// `("<path>/<name>", fn_name)` for each of `entries`, the form
/// `LocalActionManager::is_catalogued` looks them up in.
pub(crate) fn catalogue(entries: &[SeedAction]) -> impl Iterator<Item = (String, String)> + '_ {
    entries
        .iter()
        .map(|e| (format!("{}/{}", e.path, e.name), e.fn_name.to_string()))
}

/// Delete every `Internal` row outside `/builtin`, where the catalogue
/// never puts one: an alias saved before `save` refused them. Dispatch
/// already refuses to run one (it isn't in the catalogue); this drops it
/// from listings and from a package install's lock pass. Also clears a
/// stray `trusted` flag (see [`clear_stray_trusted`]).
pub async fn retire_internal_aliases(conn: &Connection) -> Result<()> {
    let mut rows = conn
        .query(
            "SELECT path, name FROM actions WHERE action_type='internal' \
             AND path<>'/builtin' AND path NOT LIKE '/builtin/%'",
            (),
        )
        .await
        .map_err(map_db)?;
    let mut aliases = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_db)? {
        aliases.push((
            row.get::<String>(0).map_err(map_db)?,
            row.get::<String>(1).map_err(map_db)?,
        ));
    }
    for (path, name) in aliases {
        tracing::warn!(
            "deleting internal action {path}/{name}: built-ins live only under /builtin"
        );
        conn.execute(
            "DELETE FROM actions WHERE path=?1 AND name=?2 AND action_type='internal'",
            libsql::params![path, name],
        )
        .await
        .map_err(map_db)?;
    }
    clear_stray_trusted(conn).await
}

/// Clear `trusted` on every row that isn't a built-in: one set through
/// `ActionInput.trusted` before that field was removed.
async fn clear_stray_trusted(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE actions SET trusted=0 WHERE trusted<>0 AND action_type<>'internal'",
        (),
    )
    .await
    .map_err(map_db)?;
    Ok(())
}

/// Built-ins that have been removed, as `(path, name, fn_name)`. Seeding
/// only writes what the catalogue holds, so without this an existing
/// database would keep listing them (calling one fails, as it is no longer
/// in the catalogue).
///
/// `/builtin/script/exec` (`script-exec`) ran inline script source. It was
/// removed because it let an MCP client reach excluded actions through its
/// nested `exec` stage; a saved Script action covers its use.
const RETIRED_BUILTINS: &[(&str, &str, &str)] = &[("/builtin/script", "exec", "script-exec")];

#[cfg(test)]
mod tests {
    use serde_json::json;
    use solx_surface::entities::{ActionInput, ActionType};
    use solx_surface::managers::ActionManager;
    use solx_surface::query::ListOptions;

    use super::*;
    use crate::testing::support::{setup, setup_wired};

    /// Insert an `Internal` row directly, as one saved before `save`
    /// refused them would be.
    async fn insert_internal(conn: &Connection, path: &str, name: &str, fn_name: &str) {
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO actions \
             (id,path,name,caption,description,capabilities,phrases,category,\
              param_type_ref,result_type_ref,action_type,fn_name,bin_name,\
              action_config,files,trusted,created_at,updated_at) \
             VALUES (?1,?2,?3,'','','[]','[]','','','','internal',?4,'','null','[]',0,?5,?5)",
            libsql::params![Uuid::new_v4().to_string(), path, name, fn_name, now],
        )
        .await
        .unwrap();
    }

    /// Nobody saves an `Internal` row, operator included: not a new alias,
    /// not a change to a built-in, not a retype of one.
    #[tokio::test]
    async fn internal_actions_cannot_be_saved() {
        let (_d, _c, m) = setup().await;
        let alias = ActionInput {
            action_type: Some(ActionType::Internal),
            fn_name: Some("file-put".into()),
            ..Default::default()
        };
        let caption = ActionInput {
            caption: Some("changed".into()),
            ..Default::default()
        };
        let retype = ActionInput {
            action_type: Some(ActionType::Script),
            bin_name: Some("x.solx".into()),
            ..Default::default()
        };
        for (path, name, input) in [
            ("/t", "alias", alias),
            (DOCUMENT_PATH, "entity-list-documents", caption),
            (DOCUMENT_PATH, "entity-list-documents", retype),
        ] {
            let err = m.save(path, name, input).await.unwrap_err();
            assert!(
                err.to_string().contains("built-ins"),
                "{path}/{name}: {err}"
            );
        }
    }

    /// Every built-in is locked, so content can't delete one either.
    #[tokio::test]
    async fn builtins_are_seeded_locked() {
        let (_d, _c, m) = setup_wired().await;
        assert!(
            m.get(DOCUMENT_PATH, "entity-get-document")
                .await
                .unwrap()
                .locked
        );
        let err = m
            .exec(
                ACTION_PATH,
                "entity-delete-action",
                json!({"path": DOCUMENT_PATH, "name": "entity-get-document"}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("is locked"), "{err}");
    }

    /// An alias saved before `save` refused them doesn't run, wherever it
    /// is, and one outside `/builtin` is deleted at open.
    #[tokio::test]
    async fn an_uncatalogued_internal_row_does_not_run() {
        let (_d, _c, m) = setup_wired().await;
        let conn = m.db.connect().await.unwrap();
        insert_internal(&conn, "/t", "alias", "entity-list-documents").await;
        insert_internal(&conn, "/builtin/x", "alias", "entity-list-documents").await;
        for path in ["/t", "/builtin/x"] {
            let err = m.exec(path, "alias", json!({})).await.unwrap_err();
            assert!(err.to_string().contains("not a built-in"), "{path}: {err}");
        }

        retire_internal_aliases(&conn).await.unwrap();
        assert!(m.get("/t", "alias").await.is_err());
        assert!(m.get("/builtin/x", "alias").await.is_ok());
    }

    /// Only a seeded built-in is `trusted`: a row flagged through the old
    /// `ActionInput.trusted` is cleared at open, and a re-save writes `false`.
    #[tokio::test]
    async fn only_builtins_are_trusted() {
        let (_d, _c, m) = setup().await;
        let script = ActionInput {
            action_type: Some(ActionType::Script),
            bin_name: Some("x.solx".into()),
            ..Default::default()
        };
        m.save("/t", "s", script.clone()).await.unwrap();
        let conn = m.db.connect().await.unwrap();
        conn.execute("UPDATE actions SET trusted=1 WHERE path='/t'", ())
            .await
            .unwrap();

        retire_internal_aliases(&conn).await.unwrap();
        assert!(!m.get("/t", "s").await.unwrap().trusted);
        assert!(
            m.get(DOCUMENT_PATH, "entity-get-document")
                .await
                .unwrap()
                .trusted
        );

        conn.execute("UPDATE actions SET trusted=1 WHERE path='/t'", ())
            .await
            .unwrap();
        m.save("/t", "s", script).await.unwrap();
        assert!(!m.get("/t", "s").await.unwrap().trusted);
    }

    /// A built-in repointed or redescribed before it was locked doesn't run
    /// as changed, and the next seed puts it back.
    #[tokio::test]
    async fn seeding_restores_a_tampered_builtin() {
        let (_d, _c, m) = setup_wired().await;
        let before = m.get(DOCUMENT_PATH, "entity-list-documents").await.unwrap();
        let conn = m.db.connect().await.unwrap();
        conn.execute(
            "UPDATE actions SET fn_name='entity-delete-document', description='poisoned', \
             capabilities='[\"x\"]', locked=0 WHERE path=?1 AND name='entity-list-documents'",
            libsql::params![DOCUMENT_PATH],
        )
        .await
        .unwrap();
        let err = m
            .exec(DOCUMENT_PATH, "entity-list-documents", json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a built-in"), "{err}");

        seed_builtins(&conn, &[]).await.unwrap();
        let after = m.get(DOCUMENT_PATH, "entity-list-documents").await.unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.fn_name, before.fn_name);
        assert_eq!(after.description, before.description);
        assert!(after.capabilities.is_empty());
        assert!(after.locked);
        assert!(m
            .exec(DOCUMENT_PATH, "entity-list-documents", json!({}))
            .await
            .is_ok());
    }

    /// Re-seeding an untouched catalogue changes nothing.
    #[tokio::test]
    async fn reseeding_leaves_builtins_alone() {
        let (_d, _c, m) = setup().await;
        let before = m.get(DOCUMENT_PATH, "entity-list-documents").await.unwrap();
        seed_builtins(&m.db.connect().await.unwrap(), &[])
            .await
            .unwrap();
        let after = m.get(DOCUMENT_PATH, "entity-list-documents").await.unwrap();
        assert_eq!(after.updated_at, before.updated_at);
    }

    #[tokio::test]
    async fn seeded_builtin_actions_are_discoverable() {
        let (_d, _c, m) = setup().await;
        let page = m
            .list(ListOptions {
                path_prefix: Some("/builtin".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            page.total > 0,
            "expected seeded built-in actions under /builtin"
        );
        let doc_get = page
            .items
            .iter()
            .find(|a| a.name == "entity-get-document")
            .expect("entity-get-document should be seeded");
        assert!(doc_get.trusted, "seeded built-ins should be trusted");
        assert_eq!(doc_get.action_type, Some(ActionType::Internal));

        // Every built-in is native dispatch now — there's no more WASM
        // built-in catalogue (see solx-actions/src/seed.rs's module docs).
        assert!(
            page.items
                .iter()
                .all(|a| a.action_type == Some(ActionType::Internal)),
            "every seeded /builtin action should be action_type=internal"
        );
    }
}
