/**
 * App — the top-level shell.
 *
 * Layout:
 *   ┌──────────────────────────────────────────────┐
 *   │ TabBar                                       │
 *   ├──────────────────────────────────────────────┤
 *   │ SearchBar                                    │
 *   ├──────────────────────────────────────────────┤
 *   │ DocumentList (or search results)             │
 *   │                                              │
 *   │   ⚙ (settings, bottom-left)    + (add, BR)   │
 *   └──────────────────────────────────────────────┘
 *
 * The "query" input drives the mode:
 *   - starts with "/" → browse mode (useDocuments, with optional sort)
 *   - everything else → search mode (useSearchDocuments with facets)
 */

import { useMemo, useState } from "react";
import { TabBar, DEFAULT_TABS } from "./components/TabBar";
import {
  SearchBar,
  type SearchFacets,
  type SearchMode,
  type SortBy,
  type SortOrder,
} from "./components/SearchBar";
import { DocumentList } from "./components/DocumentList";
import { SettingsPanel } from "./components/SettingsPanel";
import { AddDocumentDialog } from "./components/AddDocumentDialog";
import { useDocuments, useHealth, useSearchDocuments } from "./api/hooks";
import "./App.css";

const PAGE_SIZE = 20;

function detectMode(query: string): SearchMode {
  return query.trim().startsWith("/") ? "browse" : "search";
}

export function App() {
  const [activeTab, setActiveTab] = useState<string>("documents");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [facets, setFacets] = useState<SearchFacets>({});
  const [sortBy, setSortBy] = useState<SortBy>("updated_at");
  const [sortOrder, setSortOrder] = useState<SortOrder>("desc");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);

  const mode = detectMode(query);
  const pathPrefix = mode === "browse" ? query.trim() : undefined;
  const searchQuery = mode === "search" ? query.trim() : "";

  const healthQuery = useHealth();

  // Reset page whenever query/facets/sort change
  const pageResetKey = JSON.stringify({ query, facets, sortBy, sortOrder });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useMemo(() => setPage(0), [pageResetKey]);

  const browseQuery = useDocuments({
    path_prefix: pathPrefix || undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    sort_by: sortBy,
    sort_order: sortOrder,
    enabled: mode === "browse",
  });

  const searchResultQuery = useSearchDocuments({
    q: searchQuery || undefined,
    path_prefix: facets.path_prefix || undefined,
    type_ref: facets.type_ref || undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    enabled: mode === "search",
  });

  // Pick the active result set based on mode
  const activeQuery = mode === "search" ? searchResultQuery : browseQuery;
  const documents = mode === "search" ? [] : (browseQuery.data?.items ?? []);
  const searchHits = mode === "search" ? (searchResultQuery.data?.hits ?? []) : undefined;
  const total = mode === "search"
    ? (searchResultQuery.data?.total ?? 0)
    : (browseQuery.data?.total ?? 0);

  return (
    <div className="app">
      {healthQuery.isError && (
        <div className="app-banner app-banner--error" role="alert">
          <strong>Cannot reach backend.</strong>{" "}
          <span>Is the Bun server running on port 3001?</span>
        </div>
      )}
      {healthQuery.data && !healthQuery.data.solxConnected && (
        <div className="app-banner app-banner--warning" role="alert">
          <strong>solx-server unreachable.</strong>{" "}
          <span>
            Backend at {String(healthQuery.data.serverUrl ?? "")} could not be reached. Check
            settings.
          </span>
          <button className="app-banner__btn" onClick={() => setSettingsOpen(true)}>
            Open settings
          </button>
        </div>
      )}

      <TabBar tabs={DEFAULT_TABS} activeTab={activeTab} onTabChange={setActiveTab} />

      <SearchBar
        query={query}
        onQueryChange={(q) => setQuery(q)}
        mode={mode}
        facets={facets}
        onFacetsChange={setFacets}
        sortBy={sortBy}
        sortOrder={sortOrder}
        onSortByChange={setSortBy}
        onSortOrderChange={setSortOrder}
        loading={activeQuery.isFetching}
      />

      <main className="app-main">
        {activeTab === "documents" ? (
          <DocumentList
            mode={mode}
            documents={mode === "search" ? (searchHits ?? []).map(toDocumentSummary) : documents}
            total={total}
            loading={activeQuery.isLoading}
            error={activeQuery.error as Error | null}
            page={page}
            onPageChange={setPage}
            onCreateClick={() => setAddOpen(true)}
            onRetry={() => {
              void browseQuery.refetch();
              void searchResultQuery.refetch();
            }}
          />
        ) : (
          <div className="app-empty-tab">
            <p>This tab is coming soon.</p>
          </div>
        )}
      </main>

      <button
        className="app-fab app-fab--left"
        onClick={() => setSettingsOpen(true)}
        aria-label="Open settings"
        title="Settings"
      >
        ⚙
      </button>
      <button
        className="app-fab app-fab--right"
        onClick={() => setAddOpen(true)}
        aria-label="Add document"
        title="Add document"
      >
        +
      </button>

      <SettingsPanel open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      <AddDocumentDialog
        open={addOpen}
        onClose={() => setAddOpen(false)}
        defaultPath={mode === "browse" ? pathPrefix : "/"}
      />
    </div>
  );
}

function toDocumentSummary(hit: {
  id: string;
  path: string;
  name: string;
  title?: string;
  summary?: string;
  type_ref?: string;
  score: number;
}): import("./api/api").DocumentSummary {
  return {
    id: hit.id,
    path: hit.path,
    name: hit.name,
    title: hit.title,
    summary: hit.summary,
    type_ref: hit.type_ref,
  };
}
