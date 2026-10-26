//! Wire types for list & search operations (pagination, filters, facets),
//! plus the shared SQL rendering for [`ListOptions`].

use serde::{Deserialize, Serialize};

use crate::error::{Result, SolxError};
use crate::path::normalize_path;

/// Sort direction for list operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

/// Which configured exclusion list a request is filtered through.
///
/// Stores never read config: a surface (HTTP route, Tauri command, builtin
/// action, MCP server) resolves the mode into concrete patterns and appends
/// them to `exclude_paths` before calling the store — see
/// `solx_config::ExclusionPolicy::apply`. The one thing a store renders from
/// the mode itself is the action store's `solx:hidden` tag check under
/// [`ExcludeMode::Tooling`], because that is a property of the row rather
/// than of its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExcludeMode {
    /// No configured exclusions; only the caller's own `exclude_paths`.
    None,
    /// What the web UI and REST/Tauri clients see.
    User,
    /// What a model sees: MCP, the agent harness, other tool catalogues.
    Tooling,
}

/// Options for a paginated `list` over an entity database.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListOptions {
    /// Restrict to entities whose path is (or is under) this prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// LIKE filter on a column (e.g. `type_ref`, `author`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_value: Option<String>,
    /// Column to sort by (e.g. `name`, `created_at`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub sort_order: SortOrder,
    /// Only rows with created_at >= this RFC 3339 timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_after: Option<String>,
    /// Only rows with created_at <= this RFC 3339 timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_before: Option<String>,
    /// Drop entities matching any of these patterns — see
    /// [`validate_exclude_pattern`] for the syntax and matching rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_paths: Vec<String>,
    /// Keep only entities matching at least one of these patterns — same
    /// syntax and matching rule as `exclude_paths`. Empty means no
    /// restriction; exclusions still apply to whatever is included.
    ///
    /// Server-set only: it is how a list is narrowed to what the caller's
    /// scopes admit (`solx_surface::scope::ScopeSet::path_filter`), so it
    /// is never read from or written to the wire, where a caller could
    /// widen it.
    #[serde(skip)]
    pub include_paths: Vec<String>,
    /// Which configured exclusion list applies. `None` means "the
    /// surface's default", which differs per surface — see [`ExcludeMode`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_mode: Option<ExcludeMode>,
}

impl ListOptions {
    pub fn limit_or(&self, default: usize) -> usize {
        self.limit.unwrap_or(default)
    }
    pub fn offset_or_zero(&self) -> usize {
        self.offset.unwrap_or(0)
    }
}

// ---------- ListSchema / ListSql ----------

/// Declares which columns a store exposes to the [`ListOptions`] knobs.
///
/// `filter_field` and `sort_by` are caller-supplied strings, so nothing from
/// the request is ever interpolated into SQL. A field is looked *up* in these
/// tables and the matched `&'static str` is what reaches the query; anything
/// unrecognized is ignored. That keeps the whitelist and the SQL in one place
/// instead of one hand-rolled `match` per store.
#[derive(Debug, Clone, Copy)]
pub struct ListSchema<'a> {
    /// Columns `filter_field` may target. Matching is a case-insensitive
    /// substring (`LIKE %value%`) on the named column.
    pub filterable: &'a [&'a str],
    /// (`sort_by` key, `ORDER BY` expression) pairs. The expression may name
    /// several comma-separated columns.
    pub sortable: &'a [(&'a str, &'a str)],
    /// `ORDER BY` expression used when `sort_by` is absent or unrecognized.
    pub default_sort: &'a str,
    /// Column bounded by `date_after`/`date_before`; `None` if unsupported.
    pub date_column: Option<&'a str>,
}

/// A rendered [`ListOptions`]: SQL fragments plus their positional binds.
///
/// Every bind is a string, which is what keeps this renderer free of any
/// database dependency — callers convert to their driver's value type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListSql {
    /// Empty, or `" WHERE …"`.
    pub where_clause: String,
    /// `" ORDER BY … ASC|DESC"`, always non-empty.
    pub order_clause: String,
    /// Positional binds for `where_clause`, in `?1..?n` order.
    pub binds: Vec<String>,
}

impl ListOptions {
    /// Render the filter and sort knobs against a store's [`ListSchema`].
    ///
    /// Fails only if `path_prefix` is not a valid path.
    pub fn to_sql(&self, schema: ListSchema<'_>) -> Result<ListSql> {
        let mut conditions: Vec<String> = Vec::new();
        let mut binds: Vec<String> = Vec::new();

        // Path prefix: the directory itself plus everything beneath it.
        if let Some(prefix) = &self.path_prefix {
            let p = normalize_path(prefix)?;
            if p == "/" {
                binds.push("/%".to_string());
                conditions.push(format!("path LIKE ?{}", binds.len()));
            } else {
                binds.push(p.clone());
                let exact = binds.len();
                binds.push(format!("{p}/%"));
                let under = binds.len();
                conditions.push(format!("(path=?{exact} OR path LIKE ?{under})"));
            }
        }

        // Date range, for stores that declare a column for it.
        if let Some(col) = schema.date_column {
            if let Some(after) = &self.date_after {
                binds.push(after.clone());
                conditions.push(format!("{col} >= ?{}", binds.len()));
            }
            if let Some(before) = &self.date_before {
                binds.push(before.clone());
                conditions.push(format!("{col} <= ?{}", binds.len()));
            }
        }

        // Exclusions.
        if let Some(cond) = exclude_conditions(&self.exclude_paths, "", &mut binds)? {
            conditions.push(cond);
        }

        // Inclusions.
        if let Some(cond) = include_conditions(&self.include_paths, "", &mut binds)? {
            conditions.push(cond);
        }

        // Substring filter on a whitelisted column.
        if let (Some(field), Some(value)) = (&self.filter_field, &self.filter_value) {
            if !value.is_empty() {
                if let Some(col) = schema.filterable.iter().find(|c| **c == field.as_str()) {
                    binds.push(format!("%{value}%"));
                    conditions.push(format!(
                        "lower(COALESCE({col},'')) LIKE lower(?{})",
                        binds.len()
                    ));
                }
            }
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let sort_expr = self
            .sort_by
            .as_deref()
            .and_then(|key| {
                schema
                    .sortable
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, sql)| *sql)
            })
            .unwrap_or(schema.default_sort);

        let dir = match self.sort_order {
            SortOrder::Desc => "DESC",
            SortOrder::Asc => "ASC",
        };

        // The direction applies to *every* column in the expression. Writing
        // `ORDER BY path,name DESC` would sort only `name` descending and
        // silently leave `path` ascending.
        let mut parts: Vec<String> = sort_expr
            .split(',')
            .map(|c| format!("{} {dir}", c.trim()))
            .collect();

        // Deterministic pagination: LIMIT/OFFSET over a non-unique sort key
        // (updated_at, category, …) can otherwise repeat or drop rows between
        // pages, so fall back to the unique (path, name) pair.
        for tie in ["path", "name"] {
            if !sort_expr.split(',').any(|c| c.trim() == tie) {
                parts.push(format!("{tie} ASC"));
            }
        }

        Ok(ListSql {
            where_clause,
            order_clause: format!(" ORDER BY {}", parts.join(", ")),
            binds,
        })
    }
}

// ---------- exclusion patterns ----------

/// Validate and canonicalize one exclusion pattern.
///
/// A pattern is a glob over a path: `*` matches any run of characters
/// (including `/`), `?` matches exactly one, everything else is literal. It
/// excludes an entity when it matches **either** the entity's folder path
/// **or** its full reference (`/path/name`). So `/agent/skills` hides
/// everything stored in that folder *and* an entity named `skills` in
/// `/agent`; `/agent/skills*` hides that folder, everything beneath it, and
/// any sibling whose path or reference starts the same way.
///
/// It must start with `/` or `*` (so `*/_private/*` works), may not contain
/// `\`, `:` or control characters, and may not have a `.` or `..` segment.
/// A trailing slash is dropped.
pub fn validate_exclude_pattern(pattern: &str) -> Result<String> {
    let p = pattern.trim();
    let bad = |why: &str| SolxError::Invalid(format!("exclude pattern '{pattern}': {why}"));
    if p.is_empty() {
        return Err(bad("empty"));
    }
    if !(p.starts_with('/') || p.starts_with('*')) {
        return Err(bad("must start with '/' or '*'"));
    }
    if let Some(c) = p
        .chars()
        .find(|c| *c == '\\' || *c == ':' || c.is_control())
    {
        return Err(bad(&format!("forbidden character {c:?}")));
    }
    if p.split('/').any(|seg| seg == "." || seg == "..") {
        return Err(bad("'.' and '..' segments are not allowed"));
    }
    let trimmed = p.trim_end_matches('/');
    Ok(if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    })
}

/// Render `patterns` as one `NOT (…)` condition per pattern, appending binds.
///
/// `col_prefix` qualifies the `path`/`name` columns (`""` or `"d."`). Each
/// pattern is bound once and referenced twice — against the path and
/// against the full reference, built the way [`crate::path::full_ref`] does
/// (no double slash at the root). SQLite's `GLOB` is used rather than
/// `LIKE`: it is case-sensitive, like [`glob_matches`], and does not treat
/// `_` as a wildcard — `LIKE '%/_private/%'` would also match `/xprivate/`.
pub fn exclude_conditions(
    patterns: &[String],
    col_prefix: &str,
    binds: &mut Vec<String>,
) -> Result<Option<String>> {
    let parts = pattern_matches_sql(patterns, col_prefix, binds)?;
    if parts.is_empty() {
        return Ok(None);
    }
    let parts: Vec<String> = parts.into_iter().map(|m| format!("NOT {m}")).collect();
    Ok(Some(parts.join(" AND ")))
}

/// Render `patterns` as one `(… OR …)` condition keeping only entities that
/// match at least one, appending binds. Same matching rule and arguments as
/// [`exclude_conditions`]; `None` when `patterns` is empty (no restriction).
///
/// The result is a self-contained group, so several include lists rendered
/// separately AND together — each can only narrow the others.
pub fn include_conditions(
    patterns: &[String],
    col_prefix: &str,
    binds: &mut Vec<String>,
) -> Result<Option<String>> {
    let parts = pattern_matches_sql(patterns, col_prefix, binds)?;
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!("({})", parts.join(" OR "))))
}

/// One `(path GLOB ?n OR full_ref GLOB ?n)` per pattern, binding each once.
fn pattern_matches_sql(
    patterns: &[String],
    col_prefix: &str,
    binds: &mut Vec<String>,
) -> Result<Vec<String>> {
    let c = col_prefix;
    let full = format!("(CASE WHEN {c}path='/' THEN '/'||{c}name ELSE {c}path||'/'||{c}name END)");
    let mut parts = Vec::with_capacity(patterns.len());
    for raw in patterns {
        let p = validate_exclude_pattern(raw)?;
        // `[` opens a character class in GLOB; `[[]` is a literal `[`.
        binds.push(p.replace('[', "[[]"));
        let n = binds.len();
        parts.push(format!("({c}path GLOB ?{n} OR {full} GLOB ?{n})"));
    }
    Ok(parts)
}

/// The Rust twin of [`exclude_conditions`], for checking one entity (direct
/// get/save/delete) without a query. An invalid pattern matches nothing.
pub fn exclude_pattern_matches(pattern: &str, path: &str, name: &str) -> bool {
    let Ok(p) = validate_exclude_pattern(pattern) else {
        return false;
    };
    if glob_matches(&p, path) {
        return true;
    }
    let full = if path == "/" {
        format!("/{name}")
    } else {
        format!("{path}/{name}")
    };
    glob_matches(&p, &full)
}

/// Whether any of `patterns` excludes the entity at `path`/`name`.
pub fn is_excluded_by(patterns: &[String], path: &str, name: &str) -> bool {
    patterns
        .iter()
        .any(|p| exclude_pattern_matches(p, path, name))
}

/// The Rust twin of [`include_conditions`]: whether the entity at
/// `path`/`name` passes `patterns`. An empty list includes everything.
pub fn is_included_by(patterns: &[String], path: &str, name: &str) -> bool {
    patterns.is_empty() || is_excluded_by(patterns, path, name)
}

/// Match a glob pattern against a path. `*` matches any sequence of
/// characters (including `/`), `?` matches any single character; every other
/// character matches literally. No external glob/regex dependency — this is
/// the classic two-pointer glob algorithm.
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t) = (usize::MAX, 0usize);

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_p = pi;
            star_t = ti;
            pi += 1;
        } else if star_p != usize::MAX {
            pi = star_p + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A page of results plus the unpaginated total.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, total: usize, limit: usize, offset: usize) -> Self {
        Page {
            items,
            total,
            limit,
            offset,
        }
    }
}

/// One row of a `paths` result: a path value in use, and how many entities
/// currently sit at exactly that path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathFacet {
    pub path: String,
    pub count: usize,
}

/// A full-text + faceted search query (documents).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    /// Free-text query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    /// Facet: restrict to a path prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    /// Facet: restrict to a type reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_ref: Option<String>,
    /// Facet: restrict to documents whose contents contain a `DocRef` to this
    /// target, given as the target's full reference (`/path/name`, as
    /// returned by `solx get doc`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// Same as [`ListOptions::exclude_paths`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_paths: Vec<String>,
    /// Same as [`ListOptions::include_paths`]: server-set only.
    #[serde(skip)]
    pub include_paths: Vec<String>,
    /// Same as [`ListOptions::exclude_mode`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_mode: Option<ExcludeMode>,
}

/// Full-text query over the action catalogue (path/name/caption/description/
/// category/phrases, via FTS5), composable with the same filters as `list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionSearchQuery {
    #[serde(flatten)]
    pub list: ListOptions,
    /// Free-text query. `None`/empty behaves exactly like a plain `list`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    /// Deprecated spelling of `exclude_mode: tooling`, still accepted so
    /// existing callers (`?exclude_hidden=true`, `excludeHidden: true`) keep
    /// getting a filtered catalogue. See [`Self::effective_mode`]; an
    /// explicit `exclude_mode` wins over it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_hidden: bool,
}

impl ActionSearchQuery {
    /// `list.exclude_mode`, falling back to `Tooling` when only the
    /// deprecated `exclude_hidden` flag was set.
    pub fn effective_mode(&self) -> Option<ExcludeMode> {
        self.list.exclude_mode.or(if self.exclude_hidden {
            Some(ExcludeMode::Tooling)
        } else {
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: ListSchema<'static> = ListSchema {
        filterable: &["name", "title", "type_ref"],
        sortable: &[
            ("name", "path,name"),
            ("path", "path,name"),
            ("updated_at", "updated_at"),
        ],
        default_sort: "path,name",
        date_column: Some("created_at"),
    };

    fn opts() -> ListOptions {
        ListOptions::default()
    }

    /// `include_paths` is how scopes narrow a list, so a caller can neither
    /// send it nor see it.
    #[test]
    fn include_paths_never_crosses_the_wire() {
        let list: ListOptions =
            serde_json::from_value(serde_json::json!({ "includePaths": ["/*"] })).unwrap();
        assert!(list.include_paths.is_empty());
        let query: SearchQuery =
            serde_json::from_value(serde_json::json!({ "includePaths": ["/*"] })).unwrap();
        assert!(query.include_paths.is_empty());
        let query: ActionSearchQuery =
            serde_json::from_value(serde_json::json!({ "includePaths": ["/*"] })).unwrap();
        assert!(query.list.include_paths.is_empty());

        let narrowed = ListOptions {
            include_paths: vec!["/a/*".into()],
            ..opts()
        };
        assert!(!serde_json::to_string(&narrowed).unwrap().contains("/a/*"));
    }

    #[test]
    fn empty_options_render_only_a_sort() {
        let sql = opts().to_sql(SCHEMA).unwrap();
        assert_eq!(sql.where_clause, "");
        assert_eq!(sql.order_clause, " ORDER BY path ASC, name ASC");
        assert!(sql.binds.is_empty());
    }

    #[test]
    fn path_prefix_matches_the_dir_and_its_descendants() {
        let sql = ListOptions {
            path_prefix: Some("/blogs".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(sql.where_clause, " WHERE (path=?1 OR path LIKE ?2)");
        assert_eq!(
            sql.binds,
            vec!["/blogs".to_string(), "/blogs/%".to_string()]
        );
    }

    #[test]
    fn root_prefix_matches_everything() {
        let sql = ListOptions {
            path_prefix: Some("/".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(sql.where_clause, " WHERE path LIKE ?1");
        assert_eq!(sql.binds, vec!["/%".to_string()]);
    }

    #[test]
    fn invalid_path_prefix_is_rejected() {
        assert!(ListOptions {
            path_prefix: Some("/a/../b".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .is_err());
    }

    #[test]
    fn filter_targets_a_whitelisted_column() {
        let sql = ListOptions {
            filter_field: Some("title".into()),
            filter_value: Some("michigan".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(
            sql.where_clause,
            " WHERE lower(COALESCE(title,'')) LIKE lower(?1)"
        );
        assert_eq!(sql.binds, vec!["%michigan%".to_string()]);
    }

    #[test]
    fn unknown_filter_field_is_ignored_not_interpolated() {
        // The guard that keeps caller-supplied identifiers out of the SQL.
        let sql = ListOptions {
            filter_field: Some("password) OR 1=1 --".into()),
            filter_value: Some("x".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(sql.where_clause, "");
        assert!(sql.binds.is_empty());
    }

    #[test]
    fn filter_value_is_bound_never_inlined() {
        let sql = ListOptions {
            filter_field: Some("name".into()),
            filter_value: Some("'; DROP TABLE docs; --".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert!(!sql.where_clause.contains("DROP"));
        assert_eq!(sql.binds, vec!["%'; DROP TABLE docs; --%".to_string()]);
    }

    #[test]
    fn empty_filter_value_is_a_no_op() {
        let sql = ListOptions {
            filter_field: Some("name".into()),
            filter_value: Some(String::new()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(sql.where_clause, "");
    }

    #[test]
    fn direction_applies_to_every_sort_column() {
        // `ORDER BY path,name DESC` would sort only `name` descending.
        let sql = ListOptions {
            sort_by: Some("name".into()),
            sort_order: SortOrder::Desc,
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(sql.order_clause, " ORDER BY path DESC, name DESC");
    }

    #[test]
    fn non_unique_sort_gets_a_stable_tiebreak() {
        let sql = ListOptions {
            sort_by: Some("updated_at".into()),
            sort_order: SortOrder::Desc,
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(
            sql.order_clause,
            " ORDER BY updated_at DESC, path ASC, name ASC"
        );
    }

    #[test]
    fn unknown_sort_key_falls_back_to_the_default() {
        let sql = ListOptions {
            sort_by: Some("; DROP TABLE".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(sql.order_clause, " ORDER BY path ASC, name ASC");
    }

    #[test]
    fn date_bounds_apply_when_the_store_declares_a_column() {
        let sql = ListOptions {
            date_after: Some("2024-01-01".into()),
            date_before: Some("2024-12-31".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(
            sql.where_clause,
            " WHERE created_at >= ?1 AND created_at <= ?2"
        );
    }

    #[test]
    fn date_bounds_are_dropped_when_unsupported() {
        let schema = ListSchema {
            date_column: None,
            ..SCHEMA
        };
        let sql = ListOptions {
            date_after: Some("2024-01-01".into()),
            ..opts()
        }
        .to_sql(schema)
        .unwrap();
        assert_eq!(sql.where_clause, "");
    }

    #[test]
    fn exclude_paths_render_as_bound_globs_over_path_and_full_ref() {
        let sql = ListOptions {
            exclude_paths: vec!["/agent/skills".into(), "/tmp*".into()],
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        let full = "(CASE WHEN path='/' THEN '/'||name ELSE path||'/'||name END)";
        assert_eq!(
            sql.where_clause,
            format!(
                " WHERE NOT (path GLOB ?1 OR {full} GLOB ?1) AND NOT (path GLOB ?2 OR {full} GLOB ?2)"
            )
        );
        assert_eq!(
            sql.binds,
            vec!["/agent/skills".to_string(), "/tmp*".to_string()]
        );
    }

    #[test]
    fn exclude_paths_compose_with_sequential_binds() {
        let sql = ListOptions {
            path_prefix: Some("/a".into()),
            date_after: Some("2024-01-01".into()),
            exclude_paths: vec!["/a/b".into()],
            filter_field: Some("name".into()),
            filter_value: Some("x".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert!(
            sql.where_clause.contains("path GLOB ?4"),
            "{}",
            sql.where_clause
        );
        assert!(
            sql.where_clause.contains("LIKE lower(?5)"),
            "{}",
            sql.where_clause
        );
        assert_eq!(sql.binds.len(), 5);
    }

    #[test]
    fn exclude_pattern_brackets_are_literal_in_glob() {
        let mut binds = Vec::new();
        let cond = exclude_conditions(&["/a[1]".into()], "d.", &mut binds)
            .unwrap()
            .unwrap();
        assert!(cond.starts_with("NOT (d.path GLOB ?1"), "{cond}");
        assert_eq!(binds, vec!["/a[[]1]".to_string()]);
    }

    #[test]
    fn exclude_pattern_is_bound_never_inlined() {
        let mut binds = Vec::new();
        let cond = exclude_conditions(&["/x' OR 1=1 --".into()], "", &mut binds)
            .unwrap()
            .unwrap();
        assert!(!cond.contains("1=1"), "{cond}");
        assert_eq!(binds, vec!["/x' OR 1=1 --".to_string()]);
    }

    #[test]
    fn invalid_exclude_patterns_are_rejected() {
        for bad in ["", "relative", "/a/../b", "/a:b", "/a\\b"] {
            assert!(validate_exclude_pattern(bad).is_err(), "{bad:?}");
        }
        assert_eq!(validate_exclude_pattern("/a/b/").unwrap(), "/a/b");
        assert_eq!(
            validate_exclude_pattern("*/_private/*").unwrap(),
            "*/_private/*"
        );
        assert!(ListOptions {
            exclude_paths: vec!["nope".into()],
            ..opts()
        }
        .to_sql(SCHEMA)
        .is_err());
    }

    #[test]
    fn rust_matcher_covers_folder_full_ref_and_globs() {
        // (pattern, path, name, excluded)
        let cases = [
            ("/agent/skills", "/agent/skills", "docs", true),
            ("/agent/skills", "/agent", "skills", true),
            ("/agent/skills", "/agent/skills/deep", "x", false),
            ("/agent/skills", "/agent", "skillsx", false),
            ("/agent/skills*", "/agent/skills/deep", "x", true),
            ("/agent/skills*", "/agent", "skillsx", true),
            ("/note", "/", "note", true),
            ("/note", "/", "other", false),
            ("*/_private/*", "/a/_private/b", "x", true),
            ("*/_private/*", "/a/_private", "x", true),
            ("*/_private/*", "/a/xprivate", "x", false),
            ("/Agent", "/agent", "x", false),
            ("/a?c", "/abc", "x", true),
        ];
        for (pat, path, name, want) in cases {
            assert_eq!(
                exclude_pattern_matches(pat, path, name),
                want,
                "{pat} vs {path} {name}"
            );
        }
    }

    #[test]
    fn include_paths_render_as_one_or_group() {
        let sql = ListOptions {
            include_paths: vec!["/notes*".into(), "/blog".into()],
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        let full = "(CASE WHEN path='/' THEN '/'||name ELSE path||'/'||name END)";
        assert_eq!(
            sql.where_clause,
            format!(
                " WHERE ((path GLOB ?1 OR {full} GLOB ?1) OR (path GLOB ?2 OR {full} GLOB ?2))"
            )
        );
        assert_eq!(sql.binds, vec!["/notes*".to_string(), "/blog".to_string()]);
    }

    #[test]
    fn include_paths_compose_after_exclusions() {
        let sql = ListOptions {
            path_prefix: Some("/a".into()),
            exclude_paths: vec!["/a/secret".into()],
            include_paths: vec!["/a/b*".into()],
            filter_field: Some("name".into()),
            filter_value: Some("x".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        let w = &sql.where_clause;
        assert!(w.contains("NOT (path GLOB ?3"), "{w}");
        assert!(w.contains("AND ((path GLOB ?4"), "{w}");
        assert!(w.contains("LIKE lower(?5)"), "{w}");
        assert_eq!(sql.binds.len(), 5);
    }

    #[test]
    fn include_patterns_are_validated_escaped_and_bound() {
        let mut binds = Vec::new();
        assert_eq!(include_conditions(&[], "", &mut binds).unwrap(), None);
        let cond = include_conditions(&["/a[1]' OR 1=1 --".into()], "d.", &mut binds)
            .unwrap()
            .unwrap();
        assert!(cond.starts_with("((d.path GLOB ?1"), "{cond}");
        assert!(!cond.contains("1=1"), "{cond}");
        assert_eq!(binds, vec!["/a[[]1]' OR 1=1 --".to_string()]);
        assert!(ListOptions {
            include_paths: vec!["nope".into()],
            ..opts()
        }
        .to_sql(SCHEMA)
        .is_err());
    }

    #[test]
    fn rust_include_matcher_agrees() {
        assert!(is_included_by(&[], "/any", "x"));
        let pats = vec!["/notes*".to_string(), "/blog".to_string()];
        assert!(is_included_by(&pats, "/notes/deep", "x"));
        assert!(is_included_by(&pats, "/blog", "x"));
        assert!(is_included_by(&pats, "/", "blog"));
        assert!(!is_included_by(&pats, "/blog/deep", "x"));
        assert!(!is_included_by(&pats, "/other", "x"));
    }

    #[test]
    fn exclude_mode_deserializes_and_folds_the_old_flag() {
        let o: ListOptions =
            serde_json::from_value(serde_json::json!({"excludeMode": "tooling"})).unwrap();
        assert_eq!(o.exclude_mode, Some(ExcludeMode::Tooling));
        let q: ActionSearchQuery =
            serde_json::from_value(serde_json::json!({"excludeHidden": true})).unwrap();
        assert_eq!(q.effective_mode(), Some(ExcludeMode::Tooling));
        let q: ActionSearchQuery = serde_json::from_value(
            serde_json::json!({"excludeHidden": true, "excludeMode": "none"}),
        )
        .unwrap();
        assert_eq!(q.effective_mode(), Some(ExcludeMode::None));
    }

    #[test]
    fn every_filter_composes_with_sequential_binds() {
        let sql = ListOptions {
            path_prefix: Some("/blogs".into()),
            date_after: Some("2024-01-01".into()),
            filter_field: Some("type_ref".into()),
            filter_value: Some("Post".into()),
            sort_by: Some("updated_at".into()),
            ..opts()
        }
        .to_sql(SCHEMA)
        .unwrap();
        assert_eq!(
            sql.where_clause,
            " WHERE (path=?1 OR path LIKE ?2) AND created_at >= ?3 \
             AND lower(COALESCE(type_ref,'')) LIKE lower(?4)"
        );
        assert_eq!(sql.binds.len(), 4);
    }
}
