/**
 * Mirror of `solx_surface::query` (see solx-core/solx-surface/src/query.rs).
 *
 * Wire types for list + search. All filters are optional. The `Page<T>`
 * shape is the canonical pagination envelope used by every store.
 */

import type { SearchHit } from './search.js';

// ---------- SortOrder ----------

export type SortOrder = 'asc' | 'desc';
export const SortOrder = {
  Asc: 'asc',
  Desc: 'desc',
} as const;

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
}

// ---------- Page<T> ----------

export interface Page<T> {
  items: T[];
  total: number;
  limit: number;
  offset: number;
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
}

// ---------- SearchResults ----------

export interface SearchResults {
  hits: SearchHit[];
  total: number;
  limit: number;
  offset: number;
}
