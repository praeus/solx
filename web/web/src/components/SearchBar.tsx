/**
 * SearchBar — the primary interaction surface.
 *
 * Design rules (a rewrite of the earlier implicit-mode version):
 *
 *   1. Controls compose. Text query, path scope and type all narrow the same
 *      result set with AND semantics. Previously a path scope silently
 *      *replaced* the text query, so you could never search within a path.
 *
 *   2. No hidden modes. There is no "type / to browse" convention and no
 *      Browse/Search badge that swaps which controls exist. Path scope is its
 *      own always-visible field.
 *
 *   3. Only honest controls. Each tab declares what the backend can actually
 *      do (see TAB_CAPABILITIES); a control is rendered only where it works.
 *
 *   4. State is legible. The summary line always spells out the active
 *      result count, the filters in effect, and the ordering — so the list on
 *      screen is never unexplained.
 */

import { type ChangeEvent, useEffect, useRef, useState } from "react";
import { useDocumentTypes } from "../api/hooks";

const DEBOUNCE_MS = 300;

export type TabId = "documents" | "actions" | "types";
export type SortBy = "name" | "updated_at";
export type SortOrder = "asc" | "desc";

/** Everything the user can dial in. One object so resets are atomic. */
export interface BrowseState {
  query: string;
  pathPrefix: string;
  /** Full type reference, e.g. "/types/docs/Post". Documents only. */
  typeRef: string;
  sortBy: SortBy;
  sortOrder: SortOrder;
}

export const EMPTY_BROWSE_STATE: BrowseState = {
  query: "",
  pathPrefix: "",
  typeRef: "",
  sortBy: "updated_at",
  sortOrder: "desc",
};

/**
 * What each tab's backing store can actually do.
 *
 *   textQuery "fulltext"  — relevance-ranked index: Tantivy for documents
 *                           (/search), FTS5 for actions (/actions-search)
 *   textQuery "substring" — case-insensitive LIKE over the store's filterable
 *                           columns, via `filter_field`/`filter_value`
 *
 * Every tab filters, sorts and paginates in the database — nothing narrows
 * in memory.
 */
export interface TabCapabilities {
  textQuery: "fulltext" | "substring";
  /** Which column a substring query targets. Unused for "fulltext". */
  filterField?: string;
  /** Search-box placeholder detail for "fulltext" — the fields `q` matches. */
  searchFields?: string;
  typeFacet: boolean;
}

export const TAB_CAPABILITIES: Record<TabId, TabCapabilities> = {
  documents: { textQuery: "fulltext", searchFields: "title, summary, contents", typeFacet: true },
  actions: { textQuery: "fulltext", searchFields: "path, name, caption, description, category", typeFacet: false },
  types: { textQuery: "substring", filterField: "name", typeFacet: false },
};

const TAB_NOUN: Record<TabId, [singular: string, plural: string]> = {
  documents: ["document", "documents"],
  actions: ["action", "actions"],
  types: ["type", "types"],
};

interface Props {
  activeTab: TabId;
  state: BrowseState;
  onChange: (next: BrowseState) => void;
  /** Result count after every active filter. */
  total: number;
  loading?: boolean;
  /**
   * True when results come back relevance-ranked from the full-text index.
   * Sort controls are hidden in that case — SearchQuery has no sort field, so
   * offering one would be a lie.
   */
  ranked?: boolean;
  /** Caveat to surface next to the summary (e.g. in-memory view truncated). */
  notice?: string | null;
}

export function SearchBar({
  activeTab,
  state,
  onChange,
  total,
  loading,
  ranked = false,
  notice,
}: Props) {
  const caps = TAB_CAPABILITIES[activeTab];

  // Text inputs are debounced; selects and buttons apply immediately.
  const [draftQuery, setDraftQuery] = useState(state.query);
  const [draftPath, setDraftPath] = useState(state.pathPrefix);

  // A ref to the live props keeps the debounce effects off the dependency
  // treadmill without going stale — the previous version omitted `query` from
  // its deps to dodge a loop, which made external resets fight the input.
  const latest = useRef({ state, onChange });
  latest.current = { state, onChange };

  // Pull external changes (tab switch, Clear all) back into the inputs.
  useEffect(() => setDraftQuery(state.query), [state.query]);
  useEffect(() => setDraftPath(state.pathPrefix), [state.pathPrefix]);

  useEffect(() => {
    if (draftQuery === latest.current.state.query) return;
    const handle = window.setTimeout(() => {
      latest.current.onChange({ ...latest.current.state, query: draftQuery });
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [draftQuery]);

  useEffect(() => {
    if (draftPath === latest.current.state.pathPrefix) return;
    const handle = window.setTimeout(() => {
      latest.current.onChange({ ...latest.current.state, pathPrefix: draftPath });
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [draftPath]);

  const { data: typesData } = useDocumentTypes();
  const documentTypes = (typesData?.items ?? [])
    .slice()
    .sort((a, b) => String(a.name ?? "").localeCompare(String(b.name ?? "")));

  const patch = (fields: Partial<BrowseState>) => onChange({ ...state, ...fields });

  const hasAnyFilter =
    state.query.trim() !== "" || state.pathPrefix.trim() !== "" || state.typeRef !== "";

  const handleTypeRefChange = (e: ChangeEvent<HTMLSelectElement>) =>
    patch({ typeRef: e.target.value });

  const [noun, nounPlural] = TAB_NOUN[activeTab];

  return (
    <div className="search-bar">
      <div className="search-bar__input-row">
        <span className="search-bar__icon" aria-hidden="true">
          ⌕
        </span>
        <input
          className="search-bar__input"
          type="text"
          value={draftQuery}
          onChange={(e) => setDraftQuery(e.target.value)}
          placeholder={
            caps.textQuery === "fulltext"
              ? `Search ${nounPlural} by ${caps.searchFields ?? "title, summary, contents"}…`
              : `Filter ${nounPlural} by ${caps.filterField ?? "name"}…`
          }
          spellCheck={false}
          aria-label={`Search ${nounPlural}`}
        />
        {draftQuery && (
          <button
            className="search-bar__clear"
            onClick={() => setDraftQuery("")}
            aria-label="Clear search text"
            title="Clear search text"
          >
            ✕
          </button>
        )}
        {loading && (
          <span className="search-bar__loading" role="status" aria-label="Loading">
            …
          </span>
        )}
      </div>

      <div className="search-bar__facets">
        <label className="search-bar__facet">
          <span className="search-bar__facet-label">Scope</span>
          <div className="search-bar__facet-with-clear">
            <input
              className="search-bar__facet-input"
              type="text"
              value={draftPath}
              onChange={(e) => setDraftPath(e.target.value)}
              placeholder="/ (everywhere)"
              spellCheck={false}
              aria-label="Limit to path"
            />
            {draftPath && (
              <button
                className="search-bar__facet-clear"
                onClick={() => setDraftPath("")}
                aria-label="Clear path scope"
                title="Clear"
              >
                ✕
              </button>
            )}
          </div>
        </label>

        {caps.typeFacet && (
          <label className="search-bar__facet">
            <span className="search-bar__facet-label">Type</span>
            <select
              className="search-bar__facet-select"
              value={state.typeRef}
              onChange={handleTypeRefChange}
            >
              <option value="">All types</option>
              {documentTypes.map((t) => {
                const ref = `${t.path ?? ""}/${t.name ?? ""}`;
                return (
                  <option key={ref} value={ref}>
                    {String(t.name ?? "")}
                  </option>
                );
              })}
            </select>
          </label>
        )}

        {/* Relevance ranking has no sort axis to offer, so the control is
            replaced by a statement of what the ordering actually is. */}
        {ranked ? (
          <span className="search-bar__facet search-bar__facet--static">
            Ranked by relevance
          </span>
        ) : (
          <label className="search-bar__facet">
            <span className="search-bar__facet-label">Sort</span>
            <select
              className="search-bar__facet-select"
              value={state.sortBy}
              onChange={(e) => patch({ sortBy: e.target.value as SortBy })}
            >
              <option value="name">Name</option>
              <option value="updated_at">Last updated</option>
            </select>
            <button
              className="search-bar__sort-btn"
              onClick={() =>
                patch({ sortOrder: state.sortOrder === "asc" ? "desc" : "asc" })
              }
              aria-label={`Sort ${state.sortOrder === "asc" ? "descending" : "ascending"}`}
              title={
                state.sortOrder === "asc"
                  ? "Ascending — click for descending"
                  : "Descending — click for ascending"
              }
            >
              {state.sortOrder === "asc" ? "↑" : "↓"}
            </button>
          </label>
        )}

        {hasAnyFilter && (
          <button
            className="search-bar__reset"
            onClick={() => onChange({ ...EMPTY_BROWSE_STATE, sortBy: state.sortBy, sortOrder: state.sortOrder })}
          >
            Clear all
          </button>
        )}
      </div>

      <div className="search-bar__summary" role="status">
        <span className="search-bar__summary-count">
          {loading && total === 0
            ? "Loading…"
            : `${total.toLocaleString()} ${total === 1 ? noun : nounPlural}`}
        </span>
        {state.query.trim() && (
          <span className="search-bar__chip">
            {caps.textQuery === "fulltext"
              ? "matching"
              : `${caps.filterField ?? "name"} contains`}{" "}
            “{state.query.trim()}”
          </span>
        )}
        {state.pathPrefix.trim() && (
          <span className="search-bar__chip">in {state.pathPrefix.trim()}</span>
        )}
        {state.typeRef && (
          <span className="search-bar__chip">
            of type {state.typeRef.replace(/^.*\//, "")}
          </span>
        )}
        <span className="search-bar__chip search-bar__chip--muted">
          {ranked
            ? "by relevance"
            : `by ${state.sortBy === "name" ? "name" : "last updated"} ${
                state.sortOrder === "asc" ? "↑" : "↓"
              }`}
        </span>
        {notice && <span className="search-bar__chip search-bar__chip--warn">{notice}</span>}
      </div>
    </div>
  );
}
