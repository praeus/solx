/**
 * App — the top-level shell.
 *
 * Layout:
 *   ┌──────────────────────────────────────────────┐
 *   │ TabBar                                       │
 *   ├──────────────────────────────────────────────┤
 *   │ SearchBar (Documents only)                   │
 *   ├──────────────────────────────────────────────┤
 *   │ Active tab content (Documents / Actions / …) │
 *   │                                              │
 *   │   ⚙ (settings, bottom-left)  + (add, BR)     │
 *   └──────────────────────────────────────────────┘
 *
 * Documents tab:
 *   - "query" input drives the mode:
 *       - starts with "/" → browse mode (useDocuments, with optional sort)
 *       - everything else → search mode (useSearchDocuments with facets)
 *
 * Actions / Types / Files tabs:
 *   - Either browse by path or search by name (mirrors the logic for documents).
 *   - Type- and Action-specific facets are omitted in this PR.
 *   - Files tab uses a prefix filter instead of a search bar.
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
import { ActionList } from "./components/ActionList";
import { ActionEditor } from "./components/ActionEditor";
import { TypeList } from "./components/TypeList";
import { TypeEditor } from "./components/TypeEditor";
import { FileList } from "./components/FileList";
import { FileEditor } from "./components/FileEditor";
import { SettingsPanel } from "./components/SettingsPanel";
import { AddDocumentDialog } from "./components/AddDocumentDialog";
import { ErrorBoundary } from "./components/ErrorBoundary";
import type {
  ActionSummary,
  DocumentSummary,
  TypeEntity,
} from "./api/api";
import {
  useActions,
  useDocuments,
  useFilesList,
  useHealth,
  useSearchActions,
  useSearchDocuments,
  useTypes,
} from "./api/hooks";
import "./App.css";

const PAGE_SIZE = 20;

function detectMode(query: string, pathPrefix: string): SearchMode {
  // Browse mode is triggered by either the main query OR the path-prefix
  // facet starting with "/". When neither is set we still default to browse
  // (with no prefix) so the user sees "everything" instead of an empty list.
  if (query.trim().startsWith("/")) return "browse";
  if (pathPrefix.trim().startsWith("/")) return "browse";
  if (query.trim().length > 0) return "search";
  return "browse";
}

function activeTabLabel(id: string): string {
  switch (id) {
    case "documents": return "Documents";
    case "actions":   return "Actions";
    case "types":     return "Types";
    case "files":     return "Files";
    default:          return id;
  }
}

export function App() {
  const [activeTab, setActiveTab] = useState<string>("documents");

  // ── Documents / Actions / Types search state ──
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [facets, setFacets] = useState<SearchFacets>({});
  const [sortBy, setSortBy] = useState<SortBy>("updated_at");
  const [sortOrder, setSortOrder] = useState<SortOrder>("desc");

  // ── Edit dialogs ──
  const [addOpen, setAddOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [actionEditor, setActionEditor] = useState<{
    open: boolean;
    existing: { path: string; name: string } | null;
    entity: ActionSummary | null;
  }>({ open: false, existing: null, entity: null });
  const [typeEditor, setTypeEditor] = useState<{
    open: boolean;
    existing: { path: string; name: string } | null;
    entity: TypeEntity | null;
  }>({ open: false, existing: null, entity: null });
  const [fileEditor, setFileEditor] = useState<{
    open: boolean;
    relPath: string | null;
  }>({ open: false, relPath: null });

  // ── Fetch hooks ──
  const healthQuery = useHealth();

  const pathPrefix = facets.path_prefix?.trim() ?? "";
  const mode = detectMode(query, pathPrefix);

  // Documents
  // Browse mode runs when:
  //   - the user typed "/foo" in the search box, OR
  //   - the path-prefix facet is "/foo", OR
  //   - both query and path are empty (show everything).
  const browseQuery = useDocuments({
    path_prefix: mode === "browse" ? pathPrefix || undefined : undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    sort_by: sortBy,
    sort_order: sortOrder,
    enabled: activeTab === "documents" && mode === "browse",
  });
  const searchResultQuery = useSearchDocuments({
    q: mode === "search" ? query.trim() || undefined : undefined,
    path_prefix: facets.path_prefix || undefined,
    type_ref: facets.type_ref || undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    enabled: activeTab === "documents" && mode === "search",
  });

  // Actions
  const actionsQuery = useActions({
    path_prefix: mode === "browse" ? pathPrefix || undefined : undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    sort_by: sortBy,
    sort_order: sortOrder,
    enabled: activeTab === "actions" && mode === "browse",
  });
  const searchActionsQuery = useSearchActions({
    q: mode === "search" ? query.trim() || undefined : undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    enabled: activeTab === "actions" && mode === "search",
  });

  // Types
  const typesListQuery = useTypes({
    path_prefix: mode === "browse" ? pathPrefix || undefined : undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    sort_by: sortBy,
    sort_order: sortOrder,
    enabled: activeTab === "types" && mode === "browse",
  });

  // Files — owned by App so its error can be surfaced in the global banner.
  // Always fetch the root listing (cheap); the prefix filter is applied
  // inside FileList on top of this result.
  const filesQuery = useFilesList("", activeTab === "files");

  // Reset page whenever query/facets/sort change
  const pageResetKey = JSON.stringify({ tab: activeTab, query, facets, sortBy, sortOrder });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useMemo(() => setPage(0), [pageResetKey]);

  // Active query per tab
  const documents = browseQuery.data?.items ?? [];
  const searchHits = searchResultQuery.data?.hits ?? [];
  const documentsTotal = mode === "search"
    ? (searchResultQuery.data?.total ?? 0)
    : (browseQuery.data?.total ?? 0);
  const actionsList = actionsQuery.data?.items ?? [];
  const searchActionsList = searchActionsQuery.data?.items ?? [];
  const actionsTotal = mode === "search"
    ? (searchActionsQuery.data?.total ?? 0)
    : (actionsQuery.data?.total ?? 0);
  const typesList = typesListQuery.data?.items ?? [];
  const typesTotal = typesListQuery.data?.total ?? 0;

  const isSearch = mode === "search";
  const showSearchBar = activeTab === "documents" || activeTab === "actions" || activeTab === "types";

  // Per-tab fetch errors. We surface these in a dedicated banner so
  // the user gets immediate feedback that solx-server isn't reachable
  // — beyond what the periodic /api/health check reports.
  const tabErrors: Array<{ kind: "documents" | "actions" | "types" | "files"; message: string }> = [];
  const browseErr = (browseQuery.error ?? searchResultQuery.error) as Error | null;
  if (browseErr) tabErrors.push({ kind: "documents", message: browseErr.message });
  const actionErr = (actionsQuery.error ?? searchActionsQuery.error) as Error | null;
  if (actionErr) tabErrors.push({ kind: "actions", message: actionErr.message });
  const typeErr = typesListQuery.error as Error | null;
  if (typeErr) tabErrors.push({ kind: "types", message: typeErr.message });
  const fileErr = filesQuery.error as Error | null;
  if (fileErr) tabErrors.push({ kind: "files", message: fileErr.message });

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
      {tabErrors.length > 0 && (
        <div className="app-banner app-banner--warning" role="alert">
          <strong>{activeTabLabel(activeTab)} could not be loaded.</strong>{" "}
          <span>
            {tabErrors.length === 1
              ? tabErrors[0].message
              : `${tabErrors.length} tabs have errors. See settings → connection, then retry.`}
          </span>
          <button
            className="app-banner__btn"
            onClick={() => {
              void browseQuery.refetch();
              void searchResultQuery.refetch();
              void actionsQuery.refetch();
              void searchActionsQuery.refetch();
              void typesListQuery.refetch();
              void filesQuery.refetch();
            }}
          >
            Retry all
          </button>
          <button className="app-banner__btn" onClick={() => setSettingsOpen(true)}>
            Open settings
          </button>
        </div>
      )}

      <TabBar tabs={DEFAULT_TABS} activeTab={activeTab} onTabChange={setActiveTab} />

      {showSearchBar && (
        <SearchBar
          activeTab={(activeTab === "documents" || activeTab === "actions" || activeTab === "types")
            ? activeTab
            : "documents"}
          query={query}
          onQueryChange={(q) => setQuery(q)}
          mode={mode}
          facets={facets}
          onFacetsChange={setFacets}
          sortBy={sortBy}
          sortOrder={sortOrder}
          onSortByChange={setSortBy}
          onSortOrderChange={setSortOrder}
          loading={
            activeTab === "documents"
              ? (isSearch ? searchResultQuery.isFetching : browseQuery.isFetching)
              : activeTab === "actions"
                ? (isSearch ? searchActionsQuery.isFetching : actionsQuery.isFetching)
                : typesListQuery.isFetching
          }
        />
      )}

      <main className="app-main">
        <ErrorBoundary label="Entity browser">
          {activeTab === "documents" && (
            <DocumentList
              mode={mode}
              documents={isSearch ? searchHits.map(toDocumentSummary) : documents}
              total={documentsTotal}
              loading={
                isSearch
                  ? searchResultQuery.isLoading
                  : browseQuery.isLoading
              }
              error={(isSearch ? searchResultQuery.error : browseQuery.error) as Error | null}
              page={page}
              onPageChange={setPage}
              onCreateClick={() => setAddOpen(true)}
              onRetry={() => {
                void browseQuery.refetch();
                void searchResultQuery.refetch();
              }}
            />
          )}

          {activeTab === "actions" && (
            <ActionList
              documents={isSearch ? searchActionsList : actionsList}
              total={actionsTotal}
              loading={isSearch ? searchActionsQuery.isLoading : actionsQuery.isLoading}
              error={(isSearch ? searchActionsQuery.error : actionsQuery.error) as Error | null}
              page={page}
              onPageChange={setPage}
              onCreateClick={() =>
                setActionEditor({ open: true, existing: null, entity: null })
              }
              onRetry={() => {
                void actionsQuery.refetch();
                void searchActionsQuery.refetch();
              }}
            />
          )}

          {activeTab === "types" && (
            <TypeList
              documents={typesList}
              total={typesTotal}
              loading={typesListQuery.isLoading}
              error={typesListQuery.error as Error | null}
              page={page}
              onPageChange={setPage}
              onCreateClick={() =>
                setTypeEditor({ open: true, existing: null, entity: null })
              }
              onRetry={() => void typesListQuery.refetch()}
            />
          )}

          {activeTab === "files" && (
            <FileList
              onFileOpen={(relPath) => setFileEditor({ open: true, relPath })}
              onCreateClick={() => setFileEditor({ open: true, relPath: null })}
              filesQuery={filesQuery}
            />
          )}
        </ErrorBoundary>
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
        onClick={() => {
          if (activeTab === "documents") setAddOpen(true);
          else if (activeTab === "actions")
            setActionEditor({ open: true, existing: null, entity: null });
          else if (activeTab === "types")
            setTypeEditor({ open: true, existing: null, entity: null });
          else if (activeTab === "files") setFileEditor({ open: true, relPath: null });
        }}
        aria-label={
          activeTab === "documents"
            ? "Add document"
            : activeTab === "actions"
              ? "Add action"
              : activeTab === "types"
                ? "Add type"
                : "Upload file"
        }
        title={
          activeTab === "documents"
            ? "Add document"
            : activeTab === "actions"
              ? "Add action"
              : activeTab === "types"
                ? "Add type"
                : "Upload file"
        }
      >
        +
      </button>

      <SettingsPanel open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      <AddDocumentDialog
        open={addOpen}
        onClose={() => setAddOpen(false)}
        defaultPath={mode === "browse" ? pathPrefix || "/" : "/"}
      />

      <ActionEditor
        open={actionEditor.open}
        onClose={() => setActionEditor({ open: false, existing: null, entity: null })}
        existing={actionEditor.existing}
        existingEntity={actionEditor.entity}
        defaultPath={mode === "browse" ? pathPrefix || "/" : "/"}
      />

      <TypeEditor
        open={typeEditor.open}
        onClose={() => setTypeEditor({ open: false, existing: null, entity: null })}
        existing={typeEditor.existing}
        existingEntity={typeEditor.entity}
        defaultPath={mode === "browse" ? pathPrefix || "/" : "/"}
      />

      <FileEditor
        open={fileEditor.open}
        onClose={() => setFileEditor({ open: false, relPath: null })}
        relPath={fileEditor.relPath}
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
}): DocumentSummary {
  return {
    id: hit.id,
    path: hit.path,
    name: hit.name,
    title: hit.title,
    summary: hit.summary,
    type_ref: hit.type_ref,
  };
}
