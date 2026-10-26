//! Minimal libsql connection helper (duplicated per entity crate by design).

use std::path::Path;
use std::sync::Arc;

use libsql::{Builder, Connection, Database};
use solx_surface::error::{Result, SolxError};

pub fn map_db<E: std::fmt::Display>(e: E) -> SolxError {
    SolxError::Db(e.to_string())
}

#[derive(Clone)]
pub struct Db {
    db: Arc<Database>,
}

impl Db {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Builder::new_local(path).build().await.map_err(map_db)?;
        Ok(Db { db: Arc::new(db) })
    }

    pub async fn connect(&self) -> Result<Connection> {
        let conn = self.db.connect().map_err(map_db)?;
        let mut rows = conn
            .query("PRAGMA busy_timeout=10000", ())
            .await
            .map_err(map_db)?;
        while rows.next().await.map_err(map_db)?.is_some() {}
        Ok(conn)
    }
}

pub async fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tokens (
             id            TEXT PRIMARY KEY,
             kind          TEXT NOT NULL CHECK(kind IN ('client','action')),
             token_hash    BLOB NOT NULL UNIQUE,
             label         TEXT,
             action_ref    TEXT,
             invocation_id TEXT,
             created_at    INTEGER NOT NULL,
             expires_at    INTEGER,
             last_used_at  INTEGER,
             revoked_at    INTEGER
         );
         CREATE TABLE IF NOT EXISTS connect_codes (
             code_hash      BLOB PRIMARY KEY,
             label          TEXT,
             token_ttl_secs INTEGER,
             created_at     INTEGER NOT NULL,
             expires_at     INTEGER NOT NULL,
             consumed_at    INTEGER
         );
         CREATE TABLE IF NOT EXISTS registry_state (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )
    .await
    .map_err(map_db)?;
    // Added after the first release. `scopes` is a JSON array of scope
    // strings; `NULL` is a row from before scopes existed, which keeps
    // working as `all`.
    for table in ["tokens", "connect_codes"] {
        if !has_column(conn, table, "scopes").await? {
            conn.execute(&format!("ALTER TABLE {table} ADD COLUMN scopes TEXT"), ())
                .await
                .map_err(map_db)?;
        }
    }
    Ok(())
}

async fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(conn
        .query(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name=?2",
            libsql::params![table.to_string(), column.to_string()],
        )
        .await
        .map_err(map_db)?
        .next()
        .await
        .map_err(map_db)?
        .is_some())
}
