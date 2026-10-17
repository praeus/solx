//! The `actions` table and its trigger-maintained FTS5 index, created (and
//! migrated) when [`crate::LocalActionManager::open`] runs.

use libsql::Connection;
use solx_surface::error::Result;

use crate::db::map_db;

pub(crate) const DDL: &str = "\
CREATE TABLE IF NOT EXISTS actions (\
    id TEXT PRIMARY KEY,\
    path TEXT NOT NULL,\
    name TEXT NOT NULL,\
    caption TEXT NOT NULL DEFAULT '',\
    description TEXT NOT NULL DEFAULT '',\
    capabilities TEXT NOT NULL DEFAULT '[]',\
    phrases TEXT NOT NULL DEFAULT '[]',\
    category TEXT NOT NULL DEFAULT '',\
    param_type_ref TEXT NOT NULL DEFAULT '',\
    result_type_ref TEXT NOT NULL DEFAULT '',\
    action_type TEXT NOT NULL DEFAULT '',\
    fn_name TEXT NOT NULL DEFAULT '',\
    bin_name TEXT NOT NULL DEFAULT '',\
    action_config TEXT NOT NULL DEFAULT 'null',\
    files TEXT NOT NULL DEFAULT '[]',\
    trusted INTEGER NOT NULL DEFAULT 0,\
    created_at TEXT NOT NULL,\
    updated_at TEXT NOT NULL,\
    secret_key TEXT NOT NULL DEFAULT '',\
    locked INTEGER NOT NULL DEFAULT 0,\
    pin_path TEXT NOT NULL DEFAULT '',\
    pin_sha256 TEXT NOT NULL DEFAULT '',\
    UNIQUE(path,name)\
);";

/// Columns added after the table first shipped, as `(name, definition)`.
/// `CREATE TABLE IF NOT EXISTS` leaves an older table untouched, so
/// [`migrate`] adds whichever of these it lacks.
///
/// - `secret_key`: the AES key behind an action's scoped secrets. Kept in
///   its own column, outside `action_config`, so it is never part of an
///   `Action` and nothing that reads or lists actions can return it — see
///   `crate::secret_config`. Rows from before it had their keys in
///   `action_config`; [`migrate`] moves them here.
/// - `locked`, `pin_path`, `pin_sha256`: see `crate::lock`. Older rows start
///   unlocked and unpinned.
const ADDED_COLUMNS: &[(&str, &str)] = &[
    ("secret_key", "TEXT NOT NULL DEFAULT ''"),
    ("locked", "INTEGER NOT NULL DEFAULT 0"),
    ("pin_path", "TEXT NOT NULL DEFAULT ''"),
    ("pin_sha256", "TEXT NOT NULL DEFAULT ''"),
];

/// External-content FTS5 index over the action catalogue, kept in sync with
/// `actions` purely by trigger — `save`/`delete`/`seed_builtins` never touch
/// this table directly. `id` is `TEXT PRIMARY KEY` (not `INTEGER PRIMARY
/// KEY`), so `actions` keeps SQLite's implicit `rowid`, which is what
/// `content_rowid='rowid'` links against.
const FTS_DDL: &str = "\
CREATE VIRTUAL TABLE IF NOT EXISTS actions_fts USING fts5(\
    path, name, caption, description, category, phrases, \
    content='actions', content_rowid='rowid', tokenize='porter unicode61'\
);\
CREATE TRIGGER IF NOT EXISTS actions_ai AFTER INSERT ON actions BEGIN \
  INSERT INTO actions_fts(rowid,path,name,caption,description,category,phrases)\
  VALUES (new.rowid,new.path,new.name,new.caption,new.description,new.category,new.phrases);\
END;\
CREATE TRIGGER IF NOT EXISTS actions_ad AFTER DELETE ON actions BEGIN \
  INSERT INTO actions_fts(actions_fts,rowid,path,name,caption,description,category,phrases)\
  VALUES('delete',old.rowid,old.path,old.name,old.caption,old.description,old.category,old.phrases);\
END;\
CREATE TRIGGER IF NOT EXISTS actions_au AFTER UPDATE ON actions BEGIN \
  INSERT INTO actions_fts(actions_fts,rowid,path,name,caption,description,category,phrases)\
  VALUES('delete',old.rowid,old.path,old.name,old.caption,old.description,old.category,old.phrases);\
  INSERT INTO actions_fts(rowid,path,name,caption,description,category,phrases)\
  VALUES (new.rowid,new.path,new.name,new.caption,new.description,new.category,new.phrases);\
END;";

/// Create `actions` and `actions_fts` if they don't exist yet, rebuilding
/// the index once when `actions_fts` is first created over existing rows.
pub(crate) async fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(DDL).await.map_err(map_db)?;
    for (column, definition) in ADDED_COLUMNS {
        let present = conn
            .query(
                "SELECT 1 FROM pragma_table_info('actions') WHERE name=?1",
                libsql::params![*column],
            )
            .await
            .map_err(map_db)?
            .next()
            .await
            .map_err(map_db)?
            .is_some();
        if !present {
            conn.execute(
                &format!("ALTER TABLE actions ADD COLUMN {column} {definition}"),
                (),
            )
            .await
            .map_err(map_db)?;
        }
    }
    // `actions_fts` is `content='actions'` (external content), so a bare
    // `SELECT ... FROM actions_fts` passes straight through to `actions`
    // rather than reading the index — a `WHERE rowid NOT IN (SELECT
    // rowid FROM actions_fts)` backfill would therefore always compare
    // `actions.rowid` against itself and index nothing for rows that
    // predate this table. The documented fix is `INSERT INTO
    // actions_fts(actions_fts) VALUES ('rebuild')`, run once, only the
    // first time the table is created (mirroring `solx-docs`'s
    // `created_fresh` reindex), so a normal restart doesn't pay for a
    // full rebuild every time. New rows index fine via the `FTS_DDL` triggers
    // regardless.
    let fts_existed_before: bool = {
        let mut rows = conn
            .query(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='actions_fts'",
                (),
            )
            .await
            .map_err(map_db)?;
        rows.next().await.map_err(map_db)?.is_some()
    };
    conn.execute_batch(FTS_DDL).await.map_err(map_db)?;
    if !fts_existed_before {
        conn.execute("INSERT INTO actions_fts(actions_fts) VALUES('rebuild')", ())
            .await
            .map_err(map_db)?;
    }
    migrate_secret_material(conn).await
}

/// Move secret material out of rows saved before `secret_key` existed —
/// see `crate::secret_config::scrub_legacy`. Runs at every startup, but a
/// row it has already fixed has nothing left to change.
async fn migrate_secret_material(conn: &Connection) -> Result<()> {
    let mut rows = conn
        .query(
            "SELECT id, path, name, action_config, secret_key FROM actions \
             WHERE action_config LIKE '%\"secrets\"%' OR action_config LIKE '%\"auth\"%'",
            (),
        )
        .await
        .map_err(map_db)?;
    let mut updates = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_db)? {
        let id: String = row.get(0).map_err(map_db)?;
        let path: String = row.get(1).map_err(map_db)?;
        let name: String = row.get(2).map_err(map_db)?;
        let raw: String = row.get(3).map_err(map_db)?;
        let existing_key: String = row.get(4).map_err(map_db)?;
        let Ok(mut cfg) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let out = crate::secret_config::scrub_legacy(&mut cfg);
        for note in &out.notes {
            tracing::warn!("action {path}/{name}: {note}");
        }
        if out.changed {
            // A key already in the column was written by a current save
            // and wins over anything left in the config.
            let key = if existing_key.is_empty() {
                out.key.unwrap_or_default()
            } else {
                existing_key
            };
            updates.push((id, cfg.to_string(), key));
        }
    }
    for (id, cfg, key) in updates {
        conn.execute(
            "UPDATE actions SET action_config=?2, secret_key=?3 WHERE id=?1",
            libsql::params![id, cfg, key],
        )
        .await
        .map_err(map_db)?;
    }
    Ok(())
}
