/**
 * Mirror of `solx_surface::query` (see solx-core/solx-surface/src/query.rs).
 *
 * Wire types for list + search. All filters are optional. The `Page<T>`
 * shape is the canonical pagination envelope used by every store.
 */

// ---------- SortOrder ----------

export type SortOrder = 'asc' | 'desc';
export const SortOrder = {
  Asc: 'asc',
  Desc: 'desc',
} as const;

// ---------- ExcludeMode ----------

/**
 * Which configured exclusion list (`solx-config.json` `exclusions`) a request
 * is filtered through. `'user'` is what the web UI shows, `'tooling'` what a
 * model sees, `'none'` applies only the request's own `excludePaths`.
 *
 * When omitted, the server's REST routes and the Tauri commands default to
 * `'user'`; the entity built-ins (`entity-list-documents`, ...) default to
 * `'none'`.
 */
export type ExcludeMode = 'none' | 'user' | 'tooling';

/** Per-call options for a single-entity `get`/`save`/`delete`. */
export interface EntityOptions {
  /** See {@link ExcludeMode}. An excluded entity reads as not found and cannot be written. */
  excludeMode?: ExcludeMode;
}

// ---------- ListOptions ----------

export interface ListOptions {
  /** Restrict to entities under this path. */
  pathPrefix?: string;
  /** Page size (default 100, max 1000 in the Rust impl). */
  limit?: number;
  /** Offset for pagination. */
  offset?: number;
  /** Field to filter on: typeRef | author | title | summary | name. */
  filterField?: string;
  /** Value to match (exact-string match on the Rust side). */
  filterValue?: string;
  /** Field to sort by. */
  sortBy?: string;
  /** Ascending (default) or descending. */
  sortOrder?: SortOrder;
  /** RFC 3339 timestamp; include only entities updated after. */
  dateAfter?: string;
  /** RFC 3339 timestamp; include only entities updated before. */
  dateBefore?: string;
  /**
   * Extra exclusion patterns, added to the `excludeMode` list. A pattern is a
   * glob (`*` spans `/`, `?` is one character) matched against an entity's
   * path or its full reference: `/a/b` drops entities in `/a/b` and the
   * entity `b` in `/a`; `/a/b*` also drops everything beneath.
   */
  excludePaths?: string[];
  /** See {@link ExcludeMode}. */
  excludeMode?: ExcludeMode;
}

// ---------- Page<T> ----------

export interface Page<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
}

// ---------- PathFacet ----------

/** One row of a `paths` result: a path in use, and how many entities sit there. */
export interface PathFacet {
  path: string;
  count: number;
}

// ---------- SearchQuery ----------

export interface SearchQuery {
  /** Full-text query string. */
  q?: string;
  /** Restrict to entities under this path. */
  pathPrefix?: string;
  /** Restrict to entities of this type (full ref, e.g. `/types/custom/Person`). */
  typeRef?: string;
  /** Restrict to entities that link to this target. */
  linkedTo?: string;
  /** Page size. */
  limit?: number;
  /** Offset for pagination. */
  offset?: number;
  /** See {@link ListOptions.excludePaths}. */
  excludePaths?: string[];
  /** See {@link ExcludeMode}. */
  excludeMode?: ExcludeMode;
}

// ---------- ActionSearchQuery ----------

/**
 * Full-text query over the action catalogue (path/name/caption/description/
 * category/phrases, via FTS5). Wire-flat extension of {@link ListOptions} —
 * mirrors `solx_surface::query::ActionSearchQuery`'s `#[serde(flatten)]`
 * shape, so every `ListOptions` field composes with `q` (path scope, sort,
 * date range, …). Results are `Page<Action>` (same shape as `list`), just
 * like {@link SearchQuery}'s results are `Page<Document>` — neither is a
 * separate hit/score envelope.
 */
export interface ActionSearchQuery extends ListOptions {
  /** Free-text query string. */
  q?: string;
  /** @deprecated Use `excludeMode: 'tooling'`, which wins when both are set. */
  excludeHidden?: boolean;
}
