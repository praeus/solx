/**
 * SearchBar — the primary interaction surface.
 *
 * Behavior:
 *   - Text starting with "/" → path-prefix browse mode (calls useDocuments)
 *   - Any other text         → free-text search mode  (calls useSearchDocuments)
 *
 * Facets are tab-specific:
 *   - documents  → Path, Type
 *   - actions    → Path, In Type, Out Type
 *   - types      → Path only (no type facet — the entities themselves are types)
 *
 * In browse mode, sort controls are shown instead of type facets.
 */

import { type ChangeEvent, useEffect, useState } from "react";
import { useDocumentTypes } from "../api/hooks";

const DEBOUNCE_MS = 300;

export type SearchMode = "browse" | "search";

export interface SearchFacets {
  path_prefix?: string;
  /** Documents: narrows search to docs of this type. */
  type_ref?: string;
  /** Actions: narrows search to actions whose param type matches. */
  in_type_ref?: string;
  /** Actions: narrows search to actions whose result type matches. */
  out_type_ref?: string;
}

export type SortBy = "name" | "updated_at";
export type SortOrder = "asc" | "desc";

interface Props {
  activeTab: "documents" | "actions" | "types";
  query: string;
  onQueryChange: (query: string) => void;
  mode: SearchMode;
  facets: SearchFacets;
  onFacetsChange: (facets: SearchFacets) => void;
  sortBy: SortBy;
  sortOrder: SortOrder;
  onSortByChange: (sortBy: SortBy) => void;
  onSortOrderChange: (order: SortOrder) => void;
  loading?: boolean;
}

export function SearchBar({
  activeTab,
  query,
  onQueryChange,
  mode,
  facets,
  onFacetsChange,
  sortBy,
  sortOrder,
  onSortByChange,
  onSortOrderChange,
  loading,
}: Props) {
  const [draftQuery, setDraftQuery] = useState(query);

  // Debounce query changes
  useEffect(() => {
    const handle = window.setTimeout(() => {
      if (draftQuery !== query) onQueryChange(draftQuery);
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
    // We intentionally do NOT include `query` in deps to avoid re-triggering
    // the debounce every time the parent updates `query` from another source.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftQuery]);

  // Sync external query updates back into the input (e.g. clear button)
  useEffect(() => {
    setDraftQuery(query);
  }, [query]);

  const handleClear = () => {
    setDraftQuery("");
    onQueryChange("");
    // Drop every facet except the active path-prefix (clearing the path
    // input is done explicitly via its own clear button).
    onFacetsChange({ path_prefix: facets.path_prefix });
  };

  const handlePathPrefixChange = (e: ChangeEvent<HTMLInputElement>) => {
    onFacetsChange({ ...facets, path_prefix: e.target.value || undefined });
  };
  const handleClearPath = () => {
    onFacetsChange({ ...facets, path_prefix: undefined });
  };
  const handleTypeRefChange = (e: ChangeEvent<HTMLSelectElement>) => {
    onFacetsChange({ ...facets, type_ref: e.target.value || undefined });
  };
  const handleInTypeRefChange = (e: ChangeEvent<HTMLSelectElement>) => {
    onFacetsChange({ ...facets, in_type_ref: e.target.value || undefined });
  };
  const handleOutTypeRefChange = (e: ChangeEvent<HTMLSelectElement>) => {
    onFacetsChange({ ...facets, out_type_ref: e.target.value || undefined });
  };

  // Types list is shared across tabs (used by Documents for its Type facet
  // and by Actions for In/Out Type).
  const { data: typesData } = useDocumentTypes();
  const documentTypes = (typesData?.items ?? []).slice().sort((a, b) =>
    String(a.name ?? "").localeCompare(String(b.name ?? "")),
  );

  return (
    <div className="search-bar">
      <div className="search-bar__input-row">
        <div className={`search-bar__mode-badge${mode === "search" ? " search-bar__mode-badge--search" : ""}`}>
          {mode === "search" ? "Search" : "Browse"}
        </div>
        <input
          className="search-bar__input"
          type="text"
          value={draftQuery}
          onChange={(e) => setDraftQuery(e.target.value)}
          placeholder={mode === "browse" ? "Type a path starting with / to browse…" : `Search ${activeTab}…`}
          spellCheck={false}
        />
        {draftQuery && (
          <button
            className="search-bar__clear"
            onClick={handleClear}
            aria-label="Clear search"
            title="Clear"
          >
            ✕
          </button>
        )}
        {loading && <span className="search-bar__loading">…</span>}
      </div>

      <div className="search-bar__facets">
        <label className="search-bar__facet">
          <span className="search-bar__facet-label">Path:</span>
          <div className="search-bar__facet-with-clear">
            <input
              className="search-bar__facet-input"
              type="text"
              value={facets.path_prefix ?? ""}
              onChange={handlePathPrefixChange}
              placeholder="/research"
              spellCheck={false}
            />
            {facets.path_prefix && (
              <button
                className="search-bar__facet-clear"
                onClick={handleClearPath}
                aria-label="Clear path filter"
                title="Clear"
              >
                ✕
              </button>
            )}
          </div>
        </label>

        {mode === "search" ? (
          <>
            {activeTab === "documents" && (
              <label className="search-bar__facet">
                <span className="search-bar__facet-label">Type:</span>
                <select
                  className="search-bar__facet-select"
                  value={facets.type_ref ?? ""}
                  onChange={handleTypeRefChange}
                >
                  <option value="">All types</option>
                  {documentTypes.map((t) => (
                    <option key={`${t.path ?? ""}/${t.name ?? ""}`} value={String(t.name ?? "")}>
                      {String(t.name ?? "")}
                    </option>
                  ))}
                </select>
              </label>
            )}

            {activeTab === "actions" && (
              <>
                <label className="search-bar__facet">
                  <span className="search-bar__facet-label">In Type:</span>
                  <select
                    className="search-bar__facet-select"
                    value={facets.in_type_ref ?? ""}
                    onChange={handleInTypeRefChange}
                  >
                    <option value="">Any</option>
                    {documentTypes.map((t) => (
                      <option key={`in-${t.path ?? ""}/${t.name ?? ""}`} value={String(t.name ?? "")}>
                        {String(t.name ?? "")}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="search-bar__facet">
                  <span className="search-bar__facet-label">Out Type:</span>
                  <select
                    className="search-bar__facet-select"
                    value={facets.out_type_ref ?? ""}
                    onChange={handleOutTypeRefChange}
                  >
                    <option value="">Any</option>
                    {documentTypes.map((t) => (
                      <option key={`out-${t.path ?? ""}/${t.name ?? ""}`} value={String(t.name ?? "")}>
                        {String(t.name ?? "")}
                      </option>
                    ))}
                  </select>
                </label>
              </>
            )}

            {/* Types tab: no type facet — the entities themselves are types. */}
          </>
        ) : (
          <>
            <label className="search-bar__facet">
              <span className="search-bar__facet-label">Sort:</span>
              <select
                className="search-bar__facet-select"
                value={sortBy}
                onChange={(e) => onSortByChange(e.target.value as SortBy)}
              >
                <option value="name">Name</option>
                <option value="updated_at">Last Updated</option>
              </select>
            </label>
            <button
              className="search-bar__sort-btn"
              onClick={() => onSortOrderChange(sortOrder === "asc" ? "desc" : "asc")}
              aria-label={`Sort ${sortOrder === "asc" ? "descending" : "ascending"}`}
              title={sortOrder === "asc" ? "Ascending — click for descending" : "Descending — click for ascending"}
            >
              {sortOrder === "asc" ? "↑" : "↓"}
            </button>
          </>
        )}
      </div>
    </div>
  );
}