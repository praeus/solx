//! [`ActionManager`] CRUD, listing and full-text search over the actions
//! database. `action_config` is stored free of secret material, so it is
//! returned as is — see [`crate::secret_config`].

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use solx_surface::entities::{Action, ActionExecResult, ActionInput, ActionType};
use solx_surface::error::{Result, SolxError};
use solx_surface::managers::ActionManager;
use solx_surface::{ActionSecrets, ExecScope, Principal};
use solx_surface::path::{full_ref, normalize_path, validate_name};
use solx_surface::query::{
    ActionSearchQuery, ExcludeMode, ListOptions, ListSchema, ListSql, Page, PathFacet, SortOrder,
};
use uuid::Uuid;

use crate::db::map_db;
use crate::row::{action_type_to_str, get_row, get_secret_key, row_to_action, SELECT};
use crate::{lock, secret_config, LocalActionManager};

const DEFAULT_LIMIT: usize = 50;

/// Columns this store exposes to `ListOptions`. `capabilities` and `phrases`
/// are JSON arrays, so filtering them is a substring match over that text —
/// enough to find "every action tagged mcp" without a join table.
///
/// `action_config` is deliberately absent: it is free-form JSON, not a
/// column anyone should be filtering on.
const LIST_SCHEMA: ListSchema<'static> = ListSchema {
    filterable: &[
        "name",
        "caption",
        "description",
        "category",
        "capabilities",
        "phrases",
        "action_type",
        "param_type_ref",
        "result_type_ref",
        "fn_name",
        "bin_name",
    ],
    sortable: &[
        ("name", "path,name"),
        ("path", "path,name"),
        ("category", "category"),
        ("action_type", "action_type"),
        ("created_at", "created_at"),
        ("updated_at", "updated_at"),
    ],
    default_sort: "path,name",
    date_column: Some("created_at"),
};

/// Turn free-text `q` into a safe FTS5 `MATCH` expression: each whitespace-
/// separated term becomes a quoted, prefix-matched phrase (`"term"*`), ANDed
/// together (FTS5's default for space-separated terms). Quoting every term
/// keeps it immune to FTS5 query-syntax errors from special characters
/// (`"`, `-`, `:`, `(`, ...) — a bare `foo: bar` would otherwise be parsed as
/// a `foo` column filter and fail with "no such column: foo" on any column
/// name that isn't one of `actions_fts`'s. Mirrors `solx-docs`'s
/// `fts_match_query` (that crate's doc comment used to argue action search
/// terms were unlikely to need this — a normal query like `browser: firefox`
/// proved otherwise).
/// Render `opts` against [`LIST_SCHEMA`], adding the `solx:hidden` tag check
/// when the resolved mode is [`ExcludeMode::Tooling`].
///
/// Path patterns arrive already expanded in `opts.exclude_paths` (see
/// `solx_config::ExclusionPolicy::apply`); the tag is a property of the row,
/// so it is the one part of the tooling scope rendered here. `json_each`
/// compares whole array elements, so `non-solx:hidden` does not match - the
/// reason a `LIKE` over the `capabilities` JSON could not be used.
fn list_sql(opts: &ListOptions, mode: Option<ExcludeMode>) -> Result<ListSql> {
    let mut q = opts.to_sql(LIST_SCHEMA)?;
    if mode == Some(ExcludeMode::Tooling) {
        q.binds.push(solx_config::CAP_HIDDEN.to_string());
        let cond = format!(
            "NOT EXISTS (SELECT 1 FROM json_each(capabilities) WHERE value = ?{})",
            q.binds.len()
        );
        q.where_clause = if q.where_clause.is_empty() {
            format!(" WHERE {cond}")
        } else {
            format!("{} AND {cond}", q.where_clause)
        };
    }
    Ok(q)
}

fn fts_match_query(q: &str) -> String {
    q.split_whitespace()
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

impl LocalActionManager {
    /// Read a row along with its [`ActionSecrets`] — the key from the
    /// `secret_key` column, which nothing else reads.
    ///
    /// For execution only: the result must never leave the process.
    pub(crate) async fn get_with_secrets(
        &self,
        path: &str,
        name: &str,
    ) -> Result<(Action, ActionSecrets)> {
        let path = normalize_path(path)?;
        validate_name(name)?;
        let fr = full_ref(&path, name)?;
        let conn = self.db.connect().await?;
        let action = get_row(&conn, &path, name.trim())
            .await?
            .ok_or_else(|| SolxError::NotFound(format!("action {fr}")))?;
        let key = get_secret_key(&conn, &path, name.trim()).await?;
        let secrets = ActionSecrets::new(action.action_config.as_ref(), key);
        Ok((action, secrets))
    }

    /// Refuse when any of `targets` (store-relative paths about to be
    /// written or deleted) is a locked action's pinned artifact. Every
    /// surface that writes or deletes files on content's behalf calls this:
    /// the file built-ins, REST `/files` for anything but the root token,
    /// and the desktop app's main window. See `crate::lock`.
    pub async fn guard_pinned_files<'a>(
        &self,
        targets: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        let pinned = self.pinned_paths().await?;
        match lock::pinned_target(&pinned, targets) {
            Some(path) => Err(SolxError::Invalid(format!(
                "'{path}' is the artifact of a locked action; only an operator can replace \
                 or remove it (reinstall its package, or unlock the action with `solx action unlock`)"
            ))),
            None => Ok(()),
        }
    }

    /// The store path of every locked action's pinned artifact.
    async fn pinned_paths(&self) -> Result<Vec<String>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT pin_path FROM actions WHERE locked=1 AND pin_path<>''",
                (),
            )
            .await
            .map_err(map_db)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_db)? {
            out.push(row.get::<String>(0).map_err(map_db)?);
        }
        Ok(out)
    }
}

#[async_trait]
impl ActionManager for LocalActionManager {
    async fn save(&self, path: &str, name: &str, input: ActionInput) -> Result<Action> {
        let path = normalize_path(path)?;
        validate_name(name)?;
        // Checked here rather than at each surface, for the same reason
        // `guard_executable_action` is shared: the CLI, MCP, HTTP, scripts,
        // and wasm guests all arrive through this one method.
        solx_config::validate_capabilities(&input.capabilities).map_err(SolxError::Invalid)?;
        let name = name.trim().to_string();
        let conn = self.db.connect().await?;
        let existing = get_row(&conn, &path, &name).await?;

        macro_rules! merge_opt {
            ($field:ident) => {
                input
                    .$field
                    .or_else(|| existing.as_ref().and_then(|a| a.$field.clone()))
            };
        }
        macro_rules! merge_vec {
            ($field:ident) => {
                if !input.$field.is_empty() {
                    input.$field.clone()
                } else {
                    existing
                        .as_ref()
                        .map(|a| a.$field.clone())
                        .unwrap_or_default()
                }
            };
        }

        let caption = merge_opt!(caption);
        let description = merge_opt!(description);
        let category = merge_opt!(category);
        let param_type_ref = merge_opt!(param_type_ref);
        let result_type_ref = merge_opt!(result_type_ref);
        let fn_name = merge_opt!(fn_name);
        let bin_name = merge_opt!(bin_name);
        let action_type = input
            .action_type
            .or_else(|| existing.as_ref().and_then(|a| a.action_type));
        // A key supplied under `secrets` moves to its own column, and inline
        // credentials are refused, so the stored config holds no secret
        // material — see `crate::secret_config`.
        let existing_key = match existing {
            Some(_) => get_secret_key(&conn, &path, &name).await?,
            None => None,
        };
        let (action_config, secret_key) = match input.action_config {
            Some(mut incoming) => {
                let key = secret_config::normalize(&mut incoming, existing_key.as_deref())
                    .map_err(SolxError::Invalid)?;
                (Some(incoming), key)
            }
            None => (
                existing.as_ref().and_then(|a| a.action_config.clone()),
                existing_key,
            ),
        };
        let capabilities = merge_vec!(capabilities);
        let phrases = merge_vec!(phrases);
        let files = merge_vec!(files);
        let trusted = input
            .trusted
            .unwrap_or_else(|| existing.as_ref().map(|a| a.trusted).unwrap_or(false));

        let now = Utc::now();
        let now_s = now.to_rfc3339();
        let id = existing.as_ref().map(|a| a.id).unwrap_or_else(Uuid::new_v4);
        let created_at = existing.as_ref().map(|a| a.created_at).unwrap_or(now);
        // This method is the operator path: whoever reaches it may lock and
        // unlock, and content is stopped earlier, by `guard_locked_action`.
        // Every save of a locked Script/WASM action pins (or re-pins) its
        // artifact; see `crate::lock`.
        let locked = input
            .locked
            .unwrap_or_else(|| existing.as_ref().is_some_and(|a| a.locked));
        let pin = match (locked, action_type) {
            (true, Some(ActionType::Script | ActionType::Wasm)) => Some(
                lock::pin_artifact(
                    &self.files,
                    &id.to_string(),
                    bin_name.as_deref(),
                    &full_ref(&path, &name)?,
                )
                .await?,
            ),
            _ => None,
        };

        let params = libsql::params![
            id.to_string(),
            path.clone(),
            name.clone(),
            caption.clone().unwrap_or_default(),
            description.clone().unwrap_or_default(),
            serde_json::to_string(&capabilities)?,
            serde_json::to_string(&phrases)?,
            category.clone().unwrap_or_default(),
            param_type_ref.clone().unwrap_or_default(),
            result_type_ref.clone().unwrap_or_default(),
            action_type_to_str(action_type),
            fn_name.clone().unwrap_or_default(),
            bin_name.clone().unwrap_or_default(),
            serde_json::to_string(&action_config)?,
            serde_json::to_string(&files)?,
            trusted,
            created_at.to_rfc3339(),
            now_s.clone(),
            secret_key.unwrap_or_default(),
            locked,
            pin.as_ref().map(|p| p.rel_path.clone()).unwrap_or_default(),
            pin.map(|p| p.sha256).unwrap_or_default(),
        ];

        if existing.is_some() {
            conn.execute(
                "UPDATE actions SET caption=?4,description=?5,capabilities=?6,phrases=?7,category=?8,param_type_ref=?9,result_type_ref=?10,action_type=?11,fn_name=?12,bin_name=?13,action_config=?14,files=?15,trusted=?16,updated_at=?18,secret_key=?19,locked=?20,pin_path=?21,pin_sha256=?22 WHERE path=?2 AND name=?3",
                params,
            )
            .await
            .map_err(map_db)?;
        } else {
            conn.execute(
                "INSERT INTO actions (id,path,name,caption,description,capabilities,phrases,category,param_type_ref,result_type_ref,action_type,fn_name,bin_name,action_config,files,trusted,created_at,updated_at,secret_key,locked,pin_path,pin_sha256) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
                params,
            )
            .await
            .map_err(map_db)?;
        }

        get_row(&conn, &path, &name)
            .await?
            .ok_or_else(|| SolxError::Other("action vanished after write".into()))
    }

    async fn get(&self, path: &str, name: &str) -> Result<Action> {
        let path = normalize_path(path)?;
        validate_name(name)?;
        let fr = full_ref(&path, name)?;
        let conn = self.db.connect().await?;
        get_row(&conn, &path, name.trim())
            .await?
            .ok_or_else(|| SolxError::NotFound(format!("action {fr}")))
    }

    async fn delete(&self, path: &str, name: &str) -> Result<()> {
        let path = normalize_path(path)?;
        validate_name(name)?;
        let fr = full_ref(&path, name)?;
        let conn = self.db.connect().await?;
        let affected = conn
            .execute(
                "DELETE FROM actions WHERE path=?1 AND name=?2",
                libsql::params![path.clone(), name.trim().to_string()],
            )
            .await
            .map_err(map_db)?;
        if affected == 0 {
            return Err(SolxError::NotFound(format!("action {fr}")));
        }
        Ok(())
    }

    async fn list(&self, opts: ListOptions) -> Result<Page<Action>> {
        let conn = self.db.connect().await?;
        let limit = opts.limit_or(DEFAULT_LIMIT);
        let offset = opts.offset_or_zero();

        let q = list_sql(&opts, opts.exclude_mode)?;
        let binds: Vec<libsql::Value> = q.binds.iter().cloned().map(libsql::Value::from).collect();

        let total = {
            let sql = format!("SELECT COUNT(*) FROM actions{}", q.where_clause);
            let mut rows = conn.query(&sql, binds.clone()).await.map_err(map_db)?;
            rows.next()
                .await
                .map_err(map_db)?
                .map(|r| r.get::<i64>(0).unwrap_or(0))
                .unwrap_or(0) as usize
        };

        let sql = format!(
            "{SELECT}{}{} LIMIT {limit} OFFSET {offset}",
            q.where_clause, q.order_clause
        );
        let mut rows = conn.query(&sql, binds).await.map_err(map_db)?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_db)? {
            items.push(row_to_action(&row)?);
        }
        Ok(Page::new(items, total, limit, offset))
    }

    async fn paths(&self, opts: ListOptions) -> Result<Page<PathFacet>> {
        let conn = self.db.connect().await?;
        let limit = opts.limit_or(DEFAULT_LIMIT);
        let offset = opts.offset_or_zero();

        let q = list_sql(&opts, opts.exclude_mode)?;
        let binds: Vec<libsql::Value> = q.binds.iter().cloned().map(libsql::Value::from).collect();
        let dir = match opts.sort_order {
            SortOrder::Desc => "DESC",
            SortOrder::Asc => "ASC",
        };

        let total = {
            let sql = format!("SELECT COUNT(DISTINCT path) FROM actions{}", q.where_clause);
            let mut rows = conn.query(&sql, binds.clone()).await.map_err(map_db)?;
            rows.next()
                .await
                .map_err(map_db)?
                .map(|r| r.get::<i64>(0).unwrap_or(0))
                .unwrap_or(0) as usize
        };

        let sql = format!(
            "SELECT path, COUNT(*) FROM actions{} GROUP BY path ORDER BY path {dir} LIMIT {limit} OFFSET {offset}",
            q.where_clause
        );
        let mut rows = conn.query(&sql, binds).await.map_err(map_db)?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_db)? {
            let path: String = row.get(0).map_err(map_db)?;
            let count: i64 = row.get(1).map_err(map_db)?;
            items.push(PathFacet {
                path,
                count: count as usize,
            });
        }
        Ok(Page::new(items, total, limit, offset))
    }

    /// Free-text search over `path`/`name`/`caption`/`description`/
    /// `category`/`phrases`, composed with the same `path_prefix`/
    /// `filter_field`/date filters as [`Self::list`] (rendered once via
    /// `ListOptions::to_sql`, then the `MATCH` bind is appended last so none
    /// of its `?N` placeholders need renumbering). With `query.q` absent
    /// this runs the exact same query plan as `list` — no join, no ranking.
    async fn search(&self, query: ActionSearchQuery) -> Result<Page<Action>> {
        let conn = self.db.connect().await?;
        let limit = query.list.limit_or(DEFAULT_LIMIT);
        let offset = query.list.offset_or_zero();
        // Hidden-ness is decided entirely in SQL: the caller's surface has
        // already expanded the configured patterns into `exclude_paths`, and
        // `list_sql` adds the tag check, so `total` is exact.
        let q = list_sql(&query.list, query.effective_mode())?;
        let mut binds: Vec<libsql::Value> =
            q.binds.iter().cloned().map(libsql::Value::from).collect();

        let term = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let select = "SELECT a.id,a.path,a.name,a.caption,a.description,a.capabilities,\
                       a.phrases,a.category,a.param_type_ref,a.result_type_ref,a.action_type,\
                       a.fn_name,a.bin_name,a.action_config,a.files,a.trusted,a.created_at,\
                       a.updated_at,a.locked,a.pin_path,a.pin_sha256 FROM actions a";

        // `actions_fts` also has a `path` column, so a plain join would make
        // `q.where_clause`'s unqualified `path`/`created_at`/etc. ambiguous.
        // Joining through a subquery that only exposes `rowid` and FTS5's
        // built-in `rank` column (populated by `MATCH`, lower = better match)
        // keeps every other column resolvable to `actions` alone.
        let (join, where_clause, order) = match term {
            Some(t) => {
                binds.push(libsql::Value::from(fts_match_query(t)));
                let n = binds.len();
                (
                    format!(
                        " JOIN (SELECT rowid, rank FROM actions_fts WHERE actions_fts MATCH ?{n}) f ON f.rowid = a.rowid"
                    ),
                    q.where_clause.clone(),
                    " ORDER BY f.rank".to_string(),
                )
            }
            None => (
                String::new(),
                q.where_clause.clone(),
                q.order_clause.clone(),
            ),
        };

        let total = {
            let sql = format!("SELECT COUNT(*) FROM actions a{join}{where_clause}");
            let mut rows = conn.query(&sql, binds.clone()).await.map_err(map_db)?;
            rows.next()
                .await
                .map_err(map_db)?
                .map(|r| r.get::<i64>(0).unwrap_or(0))
                .unwrap_or(0) as usize
        };

        let sql = format!("{select}{join}{where_clause}{order} LIMIT {limit} OFFSET {offset}");
        let mut rows = conn.query(&sql, binds).await.map_err(map_db)?;
        let mut items = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_db)? {
            items.push(row_to_action(&row)?);
        }
        Ok(Page::new(items, total, limit, offset))
    }

    /// Entry point for every unauthenticated *external* caller — the CLI,
    /// stdio MCP, Tauri. None of them is an action and none ran
    /// `TokenAuthority::authenticate`, so both caller and principal are
    /// `None`; see [`LocalActionManager::exec_as`].
    async fn exec(&self, path: &str, name: &str, params: Value) -> Result<ActionExecResult> {
        self.exec_as(path, name, params, None, None, ExecScope::Unrestricted)
            .await
    }

    /// Entry point for authenticated routes — the HTTP REST exec and the
    /// `/mcp` transport. Not an action either, so the caller is `None`.
    async fn exec_for(
        &self,
        principal: &Principal,
        path: &str,
        name: &str,
        params: Value,
    ) -> Result<ActionExecResult> {
        self.exec_as(
            path,
            name,
            params,
            None,
            Some(principal),
            ExecScope::Unrestricted,
        )
        .await
    }

    /// Entry point for the MCP server, stdio or `/mcp`: the whole chain is
    /// held to the tooling exclusions.
    async fn exec_tooling(
        &self,
        principal: Option<&Principal>,
        path: &str,
        name: &str,
        params: Value,
    ) -> Result<ActionExecResult> {
        self.exec_as(path, name, params, None, principal, ExecScope::Tooling)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{map_db, Db};
    use crate::schema::DDL;
    use crate::test_support::*;

    /// Seed a handful of actions with distinguishable fields for list tests.
    async fn seed_list_fixtures(m: &LocalActionManager) {
        for (path, name, category, caption) in [
            ("/tools", "alpha-tool", "extraction", "Alpha the first"),
            ("/tools", "beta-tool", "search", "Beta the second"),
            ("/other", "gamma-tool", "extraction", "Gamma the third"),
        ] {
            m.save(
                path,
                name,
                ActionInput {
                    action_type: Some(ActionType::Command),
                    fn_name: Some("echo".into()),
                    category: Some(category.into()),
                    caption: Some(caption.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn list_filters_by_name_substring() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;
        let page = m
            .list(ListOptions {
                filter_field: Some("name".into()),
                filter_value: Some("ALPHA".into()), // case-insensitive
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "alpha-tool");
    }

    #[tokio::test]
    async fn list_filters_by_category_and_caption() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;

        let by_category = m
            .list(ListOptions {
                filter_field: Some("category".into()),
                filter_value: Some("extraction".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_category.total, 2);

        let by_caption = m
            .list(ListOptions {
                filter_field: Some("caption".into()),
                filter_value: Some("the third".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_caption.total, 1);
        assert_eq!(by_caption.items[0].name, "gamma-tool");
    }

    #[tokio::test]
    async fn list_filter_composes_with_path_prefix() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;
        // "extraction" matches two actions, but only one lives under /tools.
        let page = m
            .list(ListOptions {
                path_prefix: Some("/tools".into()),
                filter_field: Some("category".into()),
                filter_value: Some("extraction".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "alpha-tool");
    }

    #[tokio::test]
    async fn paths_groups_by_path_with_counts() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;

        // /tools has two actions, /other has one -- `paths` groups by path,
        // so `total` counts distinct paths, not rows.
        let page = m
            .paths(ListOptions {
                path_prefix: Some("/tools".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].path, "/tools");
        assert_eq!(page.items[0].count, 2);
    }

    #[tokio::test]
    async fn search_matches_caption_via_fts() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;
        let page = m
            .search(ActionSearchQuery {
                q: Some("Beta".into()),
                list: ListOptions::default(),
                exclude_hidden: false,
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "beta-tool");
    }

    #[tokio::test]
    async fn search_composes_q_with_path_prefix() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;
        // "extraction" matches two actions by category, but only one lives under /tools.
        let page = m
            .search(ActionSearchQuery {
                q: Some("extraction".into()),
                list: ListOptions {
                    path_prefix: Some("/tools".into()),
                    ..Default::default()
                },
                exclude_hidden: false,
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "alpha-tool");
    }

    #[tokio::test]
    async fn search_without_q_behaves_like_list() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;
        let page = m
            .search(ActionSearchQuery {
                q: None,
                list: ListOptions {
                    path_prefix: Some("/tools".into()),
                    ..Default::default()
                },
                exclude_hidden: false,
            })
            .await
            .unwrap();
        assert_eq!(page.total, 2);
    }

    /// Regression test for the backfill bug: a row inserted directly via raw
    /// SQL *before* `actions_fts` ever existed (simulating an actions.db
    /// from before this migration shipped) must still be searchable after
    /// `LocalActionManager::open` runs — i.e. the one-time `'rebuild'` on
    /// first creation actually indexes pre-existing rows, unlike the
    /// original `INSERT ... WHERE rowid NOT IN (SELECT rowid FROM
    /// actions_fts)` backfill, which — because `actions_fts` is
    /// `content='actions'` and a bare `SELECT` against it passes through to
    /// `actions` — always compared `actions.rowid` against itself and
    /// silently indexed nothing.
    #[tokio::test]
    async fn search_finds_a_row_that_predates_the_fts_index() {
        let dir = tempfile::tempdir().unwrap();
        let actions_db_path = dir.path().join("actions.db");

        // Write a row with only the plain `actions` table present — no
        // `actions_fts`, no triggers — exactly what an actions.db created
        // before this migration looks like.
        {
            let db = Db::open(&actions_db_path).await.unwrap();
            let conn = db.connect().await.unwrap();
            conn.execute_batch(DDL).await.map_err(map_db).unwrap();
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO actions (id,path,name,caption,description,capabilities,phrases,category,param_type_ref,result_type_ref,action_type,fn_name,bin_name,action_config,files,trusted,created_at,updated_at) \
                 VALUES (?1,?2,?3,?4,?5,'[]','[]','','','',?6,?7,'','null','[]',0,?8,?8)",
                libsql::params![
                    Uuid::new_v4().to_string(),
                    "/tools",
                    "old-timer",
                    "Import your old LiveJournal posts",
                    "Migrates entries from a LiveJournal export.",
                    "command",
                    "echo hi",
                    now,
                ],
            )
            .await
            .map_err(map_db)
            .unwrap();
        }

        // Now open it through the normal manager path — this is where the
        // migration (and, with it, the one-time index rebuild) runs.
        let (_cfg, m) = open_manager(dir.path(), &actions_db_path).await;

        let page = m
            .search(ActionSearchQuery {
                q: Some("livejournal".into()),
                list: ListOptions::default(),
                exclude_hidden: false,
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "old-timer");
    }

    /// A row saved before the `secret_key` column holds its key (and maybe
    /// an inline credential) in `action_config`. Read-time masking is gone,
    /// so opening the database must move both out before anything reads it.
    #[tokio::test]
    async fn opening_moves_a_legacy_key_out_of_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let actions_db_path = dir.path().join("actions.db");
        {
            let db = Db::open(&actions_db_path).await.unwrap();
            let conn = db.connect().await.unwrap();
            conn.execute_batch(DDL).await.map_err(map_db).unwrap();
            let now = Utc::now().to_rfc3339();
            let cfg = serde_json::json!({
                "secrets": { "API_TOKEN": TEST_KEY },
                "auth": { "type": "bearer", "token": "inline-token" }
            });
            conn.execute(
                "INSERT INTO actions (id,path,name,caption,description,capabilities,phrases,category,param_type_ref,result_type_ref,action_type,fn_name,bin_name,action_config,files,trusted,created_at,updated_at) \
                 VALUES (?1,'/tools','legacy','','','[]','[]','','','','webhook','https://example.com','',?2,'[]',0,?3,?3)",
                libsql::params![Uuid::new_v4().to_string(), cfg.to_string(), now],
            )
            .await
            .map_err(map_db)
            .unwrap();
        }

        let (_cfg, m) = open_manager(dir.path(), &actions_db_path).await;
        let got = serde_json::to_string(&m.get("/tools", "legacy").await.unwrap()).unwrap();
        assert!(!got.contains(TEST_KEY), "key leaked: {got}");
        assert!(!got.contains("inline-token"), "credential leaked: {got}");
        let (action, secrets) = m.get_with_secrets("/tools", "legacy").await.unwrap();
        assert_eq!(secrets.key_for("API_TOKEN"), Some(TEST_KEY));
        assert_eq!(
            action.action_config.unwrap()["secrets"]["API_TOKEN"],
            serde_json::json!("***")
        );

        // A package reinstall saves "***", which needs the moved key.
        m.save(
            "/tools",
            "legacy",
            ActionInput {
                action_config: Some(cfg_with_secret("***")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_ignores_unknown_filter_field() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;
        let all = m.list(ListOptions::default()).await.unwrap().total;
        let page = m
            .list(ListOptions {
                filter_field: Some("name) OR 1=1 --".into()),
                filter_value: Some("x".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, all);
    }

    #[tokio::test]
    async fn list_never_filters_on_action_config() {
        let (_d, _c, m) = setup().await;
        m.save(
            "/tools",
            "s",
            ActionInput {
                action_type: Some(ActionType::Command),
                fn_name: Some("echo".into()),
                action_config: Some(cfg_with_secret(TEST_KEY)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let all = m.list(ListOptions::default()).await.unwrap().total;
        // action_config is not filterable, so a probe for a secret substring
        // cannot narrow the result set and thereby confirm a guess.
        let probe = m
            .list(ListOptions {
                filter_field: Some("action_config".into()),
                filter_value: Some("super-secret".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(probe.total, all);
    }

    #[tokio::test]
    async fn list_sorts_by_name_in_both_directions() {
        let (_d, _c, m) = setup().await;
        seed_list_fixtures(&m).await;
        let names = |p: Page<Action>| -> Vec<String> {
            p.items
                .into_iter()
                .map(|a| format!("{}/{}", a.path, a.name))
                .collect()
        };
        let asc = names(
            m.list(ListOptions {
                path_prefix: Some("/tools".into()),
                sort_by: Some("name".into()),
                sort_order: solx_surface::query::SortOrder::Asc,
                ..Default::default()
            })
            .await
            .unwrap(),
        );
        let desc = names(
            m.list(ListOptions {
                path_prefix: Some("/tools".into()),
                sort_by: Some("name".into()),
                sort_order: solx_surface::query::SortOrder::Desc,
                ..Default::default()
            })
            .await
            .unwrap(),
        );
        assert_eq!(asc, vec!["/tools/alpha-tool", "/tools/beta-tool"]);
        assert_eq!(desc, vec!["/tools/beta-tool", "/tools/alpha-tool"]);
    }

    /// The key moves to its own column on save: every read shows `"***"`,
    /// and only `get_with_secrets` (execution) can reach the key.
    #[tokio::test]
    async fn the_key_is_stored_apart_from_the_config() {
        let (_d, _c, m) = setup().await;
        let input = ActionInput {
            action_type: Some(ActionType::Command),
            fn_name: Some("echo".into()),
            action_config: Some(cfg_with_secret(TEST_KEY)),
            ..Default::default()
        };
        let saved = m.save("/tools", "s", input).await.unwrap();
        let got = m.get("/tools", "s").await.unwrap();
        let page = m
            .list(ListOptions {
                path_prefix: Some("/tools".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let found = m
            .search(ActionSearchQuery {
                q: Some("s".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        for out in [
            serde_json::to_string(&saved).unwrap(),
            serde_json::to_string(&got).unwrap(),
            serde_json::to_string(&page).unwrap(),
            serde_json::to_string(&found).unwrap(),
        ] {
            assert!(!out.contains(TEST_KEY), "key leaked: {out}");
        }
        let cfg = got.action_config.as_ref().unwrap();
        assert_eq!(cfg["secrets"]["API_TOKEN"], serde_json::json!("***"));
        assert_eq!(cfg["cwd"], serde_json::json!("/work"));

        let (_, secrets) = m.get_with_secrets("/tools", "s").await.unwrap();
        assert_eq!(secrets.key_for("API_TOKEN"), Some(TEST_KEY));
        assert_eq!(secrets.key_for("UNLISTED"), None);
    }

    #[tokio::test]
    async fn search_without_the_flag_still_returns_hidden_actions() {
        // Opt-in: the CLI and admin routes inventory the registry and must
        // keep seeing everything.
        let (_d, _c, m) = setup().await;
        m.save("/tools", "visible", ActionInput::default())
            .await
            .unwrap();
        m.save(
            "/tools",
            "secret",
            ActionInput {
                capabilities: vec![solx_config::CAP_HIDDEN.into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let page = m
            .search(ActionSearchQuery {
                list: ListOptions {
                    path_prefix: Some("/tools".into()),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            page.items.len(),
            2,
            "{:?}",
            page.items.iter().map(|a| &a.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn search_with_exclude_hidden_drops_tagged_actions() {
        let (_d, _c, m) = setup().await;
        m.save("/tools", "visible", ActionInput::default())
            .await
            .unwrap();
        m.save(
            "/tools",
            "secret",
            ActionInput {
                capabilities: vec![solx_config::CAP_HIDDEN.into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let page = m
            .search(ActionSearchQuery {
                list: ListOptions {
                    path_prefix: Some("/tools".into()),
                    ..Default::default()
                },
                exclude_hidden: true,
                ..Default::default()
            })
            .await
            .unwrap();
        let names: Vec<&String> = page.items.iter().map(|a| &a.name).collect();
        assert_eq!(names, vec![&"visible".to_string()], "{names:?}");
    }

    #[tokio::test]
    async fn exclude_hidden_fills_a_page_with_an_exact_total() {
        // Hidden-ness is decided in SQL, so a limit-2 page containing one
        // hidden row still comes back full and `total` counts visible rows.
        let (_d, _c, m) = setup().await;
        m.save("/tools", "a", ActionInput::default()).await.unwrap();
        m.save(
            "/tools",
            "b",
            ActionInput {
                capabilities: vec![solx_config::CAP_HIDDEN.into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        m.save("/tools", "c", ActionInput::default()).await.unwrap();

        let page = m
            .search(ActionSearchQuery {
                list: ListOptions {
                    path_prefix: Some("/tools".into()),
                    limit: Some(2),
                    ..Default::default()
                },
                exclude_hidden: true,
                ..Default::default()
            })
            .await
            .unwrap();
        let names: Vec<&String> = page.items.iter().map(|a| &a.name).collect();
        assert_eq!(names, vec![&"a".to_string(), &"c".to_string()], "{names:?}");
        assert_eq!(page.limit, 2);
        assert_eq!(page.total, 2, "total must not count the hidden row");
    }

    #[tokio::test]
    async fn tooling_mode_hides_tagged_rows_from_list_and_paths() {
        let (_d, _c, m) = setup().await;
        m.save("/tools", "a", ActionInput::default()).await.unwrap();
        m.save(
            "/secret",
            "b",
            ActionInput {
                // A substring of the tag must not count.
                capabilities: vec![solx_config::CAP_HIDDEN.into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        m.save(
            "/tools",
            "c",
            ActionInput {
                capabilities: vec!["non-solx:hidden".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let opts = ListOptions {
            exclude_mode: Some(ExcludeMode::Tooling),
            exclude_paths: vec!["/builtin*".into()],
            ..Default::default()
        };
        let page = m.list(opts.clone()).await.unwrap();
        let names: Vec<&str> = page.items.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["a", "c"]);
        assert_eq!(page.total, 2);
        let paths = m.paths(opts).await.unwrap();
        assert_eq!(paths.items.len(), 1);
        assert_eq!(paths.items[0].path, "/tools");
        // Any other mode leaves the tag alone.
        let all = m
            .list(ListOptions {
                exclude_mode: Some(ExcludeMode::User),
                exclude_paths: vec!["/builtin*".into()],
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(all.total, 3);
    }

    #[tokio::test]
    async fn exclude_paths_filter_list_paths_and_search_exactly() {
        let (_d, _c, m) = setup().await;
        for (path, name) in [
            ("/agent/skills", "one"),
            ("/agent/skills/deep", "two"),
            ("/agent", "skills"),
            ("/agent", "keep"),
            ("/a/_private/x", "p"),
            ("/a/xprivate/x", "q"),
        ] {
            m.save(path, name, ActionInput::default()).await.unwrap();
        }
        let names = |page: Page<Action>| -> Vec<String> {
            page.items
                .iter()
                .map(|a| format!("{}/{}", a.path, a.name))
                .collect()
        };
        // Every call also drops the seeded `/builtin/*` rows.
        let list = |pats: &[&str]| ListOptions {
            exclude_paths: pats
                .iter()
                .chain(&["/builtin*"])
                .map(|p| p.to_string())
                .collect(),
            ..Default::default()
        };

        // Exact pattern: the folder itself and the entity of that full ref,
        // but not deeper folders.
        let page = m.list(list(&["/agent/skills"])).await.unwrap();
        assert_eq!(page.total, 4);
        let got = names(page);
        assert!(
            got.contains(&"/agent/skills/deep/two".to_string()),
            "{got:?}"
        );
        assert!(!got.contains(&"/agent/skills".to_string()), "{got:?}");

        // Prefix glob reaches everything beneath.
        let page = m.list(list(&["/agent/skills*"])).await.unwrap();
        assert_eq!(page.total, 3, "{:?}", names(page.clone()));

        // `_` is literal: only the real `_private` folder goes.
        let page = m.list(list(&["*/_private/*"])).await.unwrap();
        let got = names(page);
        assert!(!got.iter().any(|r| r.contains("_private")), "{got:?}");
        assert!(got.contains(&"/a/xprivate/x/q".to_string()), "{got:?}");

        // Paths facets and search see the same filter.
        let paths = m.paths(list(&["/agent*"])).await.unwrap();
        assert!(paths.items.iter().all(|f| !f.path.starts_with("/agent")));
        let page = m
            .search(ActionSearchQuery {
                list: list(&["/agent*"]),
                q: Some("one".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 0);
    }

    #[tokio::test]
    async fn include_paths_filter_list_paths_and_search_exactly() {
        let (_d, _c, m) = setup().await;
        for (path, name) in [
            ("/agent/skills", "one"),
            ("/agent/skills/deep", "two"),
            ("/agent", "skills"),
            ("/agent", "keep"),
            ("/tools", "one"),
        ] {
            m.save(path, name, ActionInput::default()).await.unwrap();
        }
        // Includes also leave out the seeded `/builtin/*` rows.
        let list = |inc: &[&str], ex: &[&str]| ListOptions {
            include_paths: inc.iter().map(|p| p.to_string()).collect(),
            exclude_paths: ex.iter().map(|p| p.to_string()).collect(),
            ..Default::default()
        };
        let refs = |page: Page<Action>| -> Vec<String> {
            let mut v: Vec<String> = page
                .items
                .iter()
                .map(|a| format!("{}/{}", a.path, a.name))
                .collect();
            v.sort();
            v
        };

        // Exact pattern: the folder and the entity of that full ref only.
        let page = m.list(list(&["/agent/skills"], &[])).await.unwrap();
        assert_eq!(
            refs(page),
            vec!["/agent/skills", "/agent/skills/one"],
        );

        // Several patterns OR together; an exclusion still wins.
        let page = m
            .list(list(&["/agent/skills*", "/tools"], &["/agent/skills/deep"]))
            .await
            .unwrap();
        assert_eq!(
            refs(page),
            vec!["/agent/skills", "/agent/skills/one", "/tools/one"],
        );

        let paths = m.paths(list(&["/agent/skills*"], &[])).await.unwrap();
        let got: Vec<&str> = paths.items.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(got, vec!["/agent", "/agent/skills", "/agent/skills/deep"]);

        let page = m
            .search(ActionSearchQuery {
                list: list(&["/tools"], &[]),
                q: Some("one".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(refs(page), vec!["/tools/one"]);
    }

    #[tokio::test]
    async fn save_rejects_an_unknown_reserved_capability() {
        let (_d, _c, m) = setup().await;
        let input = ActionInput {
            capabilities: vec!["document".into(), "solx:destructiv".into()],
            ..Default::default()
        };
        let err = m.save("/tools", "typo", input).await.unwrap_err();
        assert!(
            err.to_string().contains("solx:destructiv"),
            "a mistyped reserved tag must fail the save, not silently leave the              action ungated: {err}"
        );
    }

    #[tokio::test]
    async fn save_accepts_reserved_and_free_form_capabilities() {
        let (_d, _c, m) = setup().await;
        let input = ActionInput {
            capabilities: vec![
                "document".into(),
                solx_config::CAP_HIDDEN.into(),
                solx_config::CAP_DESTRUCTIVE.into(),
            ],
            ..Default::default()
        };
        let saved = m.save("/tools", "flagged", input).await.unwrap();
        assert!(saved
            .capabilities
            .iter()
            .any(|c| c == solx_config::CAP_HIDDEN));
        assert!(saved
            .capabilities
            .iter()
            .any(|c| c == solx_config::CAP_DESTRUCTIVE));
    }

    /// The round trip: fetch, edit an unrelated field, save the whole thing
    /// back. The `"***"` read back is the stored value, so the key stays.
    #[tokio::test]
    async fn saving_back_a_fetched_config_keeps_the_key() {
        let (_d, _c, m) = setup().await;
        m.save(
            "/tools",
            "s",
            ActionInput {
                action_type: Some(ActionType::Command),
                fn_name: Some("echo".into()),
                action_config: Some(cfg_with_secret(TEST_KEY)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let mut fetched = m.get("/tools", "s").await.unwrap().action_config.unwrap();
        fetched["cwd"] = serde_json::json!("/elsewhere");
        m.save(
            "/tools",
            "s",
            ActionInput {
                action_config: Some(fetched),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let (action, secrets) = m.get_with_secrets("/tools", "s").await.unwrap();
        assert_eq!(secrets.key_for("API_TOKEN"), Some(TEST_KEY));
        assert_eq!(
            action.action_config.unwrap()["cwd"],
            serde_json::json!("/elsewhere")
        );
    }

    /// An upsert without `action_config` keeps the key; a config without
    /// `secrets` drops it.
    #[tokio::test]
    async fn the_key_follows_the_secrets_map() {
        let (_d, _c, m) = setup().await;
        let save = |cfg: Option<serde_json::Value>| ActionInput {
            action_type: Some(ActionType::Command),
            action_config: cfg,
            ..Default::default()
        };
        m.save("/tools", "s", save(Some(cfg_with_secret(TEST_KEY))))
            .await
            .unwrap();
        m.save("/tools", "s", save(None)).await.unwrap();
        let (_, secrets) = m.get_with_secrets("/tools", "s").await.unwrap();
        assert_eq!(secrets.key_for("API_TOKEN"), Some(TEST_KEY));

        m.save(
            "/tools",
            "s",
            save(Some(serde_json::json!({ "cwd": "/w" }))),
        )
        .await
        .unwrap();
        // Re-adding the name with "***" now has no key to fall back on.
        let err = m
            .save("/tools", "s", save(Some(cfg_with_secret("***"))))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("has no key yet"), "{err}");
    }

    #[tokio::test]
    async fn saving_a_mask_without_a_key_is_rejected() {
        let (_d, _c, m) = setup().await;
        let err = m
            .save(
                "/tools",
                "s",
                ActionInput {
                    action_type: Some(ActionType::Command),
                    action_config: Some(serde_json::json!({ "secrets": { "NEW": "***" } })),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("has no key yet"), "{err}");
    }

    #[tokio::test]
    async fn saving_an_inline_credential_is_rejected() {
        let (_d, _c, m) = setup().await;
        let err = m
            .save(
                "/hooks",
                "w",
                ActionInput {
                    action_type: Some(ActionType::Webhook),
                    action_config: Some(serde_json::json!({
                        "auth": { "type": "bearer", "token": "secret-token" }
                    })),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("inline credential"), "{err}");
    }

    #[tokio::test]
    async fn save_get_delete() {
        let (_d, _c, m) = setup().await;
        let input = ActionInput {
            description: Some("echoes".into()),
            action_type: Some(ActionType::Command),
            fn_name: Some("echo".into()),
            ..Default::default()
        };
        let a = m.save("/tools", "echo", input).await.unwrap();
        assert_eq!(a.path, "/tools");
        assert_eq!(a.action_type, Some(ActionType::Command));

        assert!(m.get("/tools", "echo").await.is_ok());
        m.delete("/tools", "echo").await.unwrap();
        assert!(m.get("/tools", "echo").await.is_err());
    }
}
