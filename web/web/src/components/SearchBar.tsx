/**
 * SearchBar — the primary interaction surface.
 *
 * Behavior:
 *   - Text starting with "/" → path-prefix browse mode (calls useDocuments)
 *   - Any other text         → free-text search mode  (calls useSearchDocuments)
 *
 * In search mode, optional facets narrow the query:
 *   - path_prefix
 *   - type_ref   (populated from useDocumentTypes)
 *
 * In browse mode, sort controls are shown instead of facets.
 */

import { type ChangeEvent, useEffect, useState } from "react";
import { useDocumentTypes } from "../api/hooks";

const DEBOUNCE_MS = 300;

export type SearchMode = "browse" | "search";

export interface SearchFacets {
  path_prefix?: string;
  type_ref?: string;
}

export type SortBy = "name" | "updated_at";
export type SortOrder = "asc" | "desc";

interface Props {
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
    onFacetsChange({});
  };

  const handlePathPrefixChange = (e: ChangeEvent<HTMLInputElement>) => {
    onFacetsChange({ ...facets, path_prefix: e.target.value || undefined });
  };

  const handleTypeRefChange = (e: ChangeEvent<HTMLSelectElement>) => {
    onFacetsChange({ ...facets, type_ref: e.target.value || undefined });
  };

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
          placeholder={mode === "browse" ? "Type a path starting with / to browse…" : "Search documents…"}
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

      {mode === "search" ? (
        <div className="search-bar__facets">
          <label className="search-bar__facet">
            <span className="search-bar__facet-label">Path:</span>
            <input
              className="search-bar__facet-input"
              type="text"
              value={facets.path_prefix ?? ""}
              onChange={handlePathPrefixChange}
              placeholder="/research"
              spellCheck={false}
            />
          </label>

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
        </div>
      ) : (
        <div className="search-bar__facets">
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
        </div>
      )}
    </div>
  );
}
