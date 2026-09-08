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
 * Query routing — one rule, applied per tab from TAB_CAPABILITIES:
 *
 *   Documents, no text query → /docs            (path + type facet + sort)
 *   Documents, text query    → /search           (Tantivy, path + type
 *                              facets, relevance-ranked; no sort axis)
 *   Actions, no text query   → /actions          (path + sort)
 *   Actions, text query      → /actions-search   (FTS5, path scope,
 *                              relevance-ranked; no sort axis)
 *   Types                    → /types            (path + name substring
 *                              via filter_field, + sort)
 *
 * Every query is filtered, sorted and paginated in the database. Every
 * control composes with AND semantics; none of them replaces another.
 * Files tab keeps its own prefix filter instead of the shared search bar.
 */

import { Suspense, lazy, useState } from "react";
import { TabBar, DEFAULT_TABS } from "./components/TabBar";
import {
  SearchBar,
  EMPTY_BROWSE_STATE,
  TAB_CAPABILITIES,
  type BrowseState,
  type TabId,
} from "./components/SearchBar";
import { DocumentList } from "./components/DocumentList";
// DocumentEditor is lazy-loaded because it transitively pulls in CodeMirror,
// Tiptap, and EJS — ~430 KB minified that we don't need until the user
// opens an editor. We also only mount it when `docEditor.open` is true so
// the dynamic import doesn't fire on the initial Documents-tab render.
const DocumentEditor = lazy(() =>
  import("./components/DocumentEditor").then((m) => ({ default: m.DocumentEditor })),
);
import { ActionList } from "./components/ActionList";
import { ActionEditor } from "./components/ActionEditor";
import { TypeList } from "./components/TypeList";
import { TypeEditor } from "./components/TypeEditor";
import { FileList } from "./components/FileList";
import { FileEditor } from "./components/FileEditor";
import { SettingsPanel } from "./components/SettingsPanel";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { DocumentEditorSkeleton } from "./components/DocumentEditorSkeleton";
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
import { getStoredServerUrl, isConfigured } from "./api/connection";
import "./App.css";

const PAGE_SIZE = 20;

const SEARCHABLE_TABS: TabId[] = ["documents", "actions", "types"];

function isSearchableTab(id: string): id is TabId {
  return (SEARCHABLE_TABS as string[]).includes(id);
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

  // ── Documents / Actions / Types browse state ──
  // One object, so a reset is atomic and page-resetting has a single home.
  const [browse, setBrowse] = useState<BrowseState>(EMPTY_BROWSE_STATE);
  const [page, setPage] = useState(0);

  // ── Edit dialogs ──
  const [docEditor, setDocEditor] = useState<{
    open: boolean;
    existing: { path: string; name: string } | null;
    entity: DocumentSummary | null;
  }>({ open: false, existing: null, entity: null });
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

  // Any change to a filter invalidates the current page number.
  const handleBrowseChange = (next: BrowseState) => {
    setBrowse(next);
    setPage(0);
  };

  // Filters are tab-specific — a "/blogs" scope means nothing on the Actions
  // tab — so switching tabs starts clean rather than carrying stale state.
  const handleTabChange = (tab: string) => {
    setActiveTab(tab);
    setBrowse(EMPTY_BROWSE_STATE);
    setPage(0);
  };

  // ── Fetch hooks ──
  const healthQuery = useHealth();

  const query = browse.query.trim();
  const pathPrefix = browse.pathPrefix.trim();
  const hasQuery = query.length > 0;

  // Documents: full-text when there's a query (relevance-ranked, no sort
  // axis), plain list otherwise (sortable). Path and type narrow both.
  const docsListQuery = useDocuments({
    pathPrefix: pathPrefix || undefined,
    // "type_ref" here names the solx-docs DB column filterField matches
    // against (see solx-docs::LIST_SCHEMA) — a data value, not a wire key,
    // so it stays snake_case regardless of this object's own casing.
    filterField: browse.typeRef ? "type_ref" : undefined,
    filterValue: browse.typeRef || undefined,
    sortBy: browse.sortBy,
    sortOrder: browse.sortOrder,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    enabled: activeTab === "documents" && !hasQuery,
  });
  const docsSearchQuery = useSearchDocuments({
    q: query || undefined,
    pathPrefix: pathPrefix || undefined,
    typeRef: browse.typeRef || undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    enabled: activeTab === "documents" && hasQuery,
  });

  // Actions: full-text (FTS5) when there's a query (relevance-ranked, no
  // sort axis), plain list otherwise (sortable). Path narrows both.
  const actionsListQuery = useActions({
    pathPrefix: pathPrefix || undefined,
    sortBy: browse.sortBy,
    sortOrder: browse.sortOrder,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    enabled: activeTab === "actions" && !hasQuery,
  });
  const actionsSearchQuery = useSearchActions({
    q: query || undefined,
    pathPrefix: pathPrefix || undefined,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    enabled: activeTab === "actions" && hasQuery,
  });

  // Types: substring filter on the column the tab declares, plus path scope
  // and sort — pushed down to libsql.
  const typesListQuery = useTypes({
    pathPrefix: pathPrefix || undefined,
    filterField: hasQuery ? TAB_CAPABILITIES.types.filterField : undefined,
    filterValue: hasQuery ? query : undefined,
    sortBy: browse.sortBy,
    sortOrder: browse.sortOrder,
    limit: PAGE_SIZE,
    offset: page * PAGE_SIZE,
    enabled: activeTab === "types",
  });

  // Files — owned by App so its error can be surfaced in the global banner.
  // Always fetch the root listing (cheap); the prefix filter is applied
  // inside FileList on top of this result.
  const filesQuery = useFilesList("", activeTab === "files");

  // Documents results, from whichever endpoint answered. `search` returns
  // the same `Page<Document>` shape as `list`, so no remapping is needed.
  const documents = hasQuery
    ? (docsSearchQuery.data?.items ?? [])
    : (docsListQuery.data?.items ?? []);
  const documentsTotal = hasQuery
    ? (docsSearchQuery.data?.total ?? 0)
    : (docsListQuery.data?.total ?? 0);

  // Actions results, from whichever endpoint answered. `search` returns the
  // same `Page<Action>` shape as `list`, so no remapping is needed.
  const actions = hasQuery
    ? (actionsSearchQuery.data?.items ?? [])
    : (actionsListQuery.data?.items ?? []);
  const actionsTotal = hasQuery
    ? (actionsSearchQuery.data?.total ?? 0)
    : (actionsListQuery.data?.total ?? 0);

  const showSearchBar = isSearchableTab(activeTab);

  // Per-tab view bindings for the shared search bar.
  const activeTotal =
    activeTab === "documents"
      ? documentsTotal
      : activeTab === "actions"
        ? actionsTotal
        : (typesListQuery.data?.total ?? 0);

  const activeFetching =
    activeTab === "documents"
      ? hasQuery
        ? docsSearchQuery.isFetching
        : docsListQuery.isFetching
      : activeTab === "actions"
        ? hasQuery
          ? actionsSearchQuery.isFetching
          : actionsListQuery.isFetching
        : typesListQuery.isFetching;

  // Per-tab fetch errors. We surface these in a dedicated banner so
  // the user gets immediate feedback that solx-server isn't reachable
  // — beyond what the periodic /api/health check reports.
  const tabErrors: Array<{ kind: "documents" | "actions" | "types" | "files"; message: string }> = [];
  const docsErr = (docsListQuery.error ?? docsSearchQuery.error) as Error | null;
  if (docsErr) tabErrors.push({ kind: "documents", message: docsErr.message });
  const actionErr = (actionsListQuery.error ?? actionsSearchQuery.error) as Error | null;
  if (actionErr) tabErrors.push({ kind: "actions", message: actionErr.message });
  const typeErr = typesListQuery.error as Error | null;
  if (typeErr) tabErrors.push({ kind: "types", message: typeErr.message });
  const fileErr = filesQuery.error as Error | null;
  if (fileErr) tabErrors.push({ kind: "files", message: fileErr.message });

  return (
    <div className="app">
      {!isConfigured() && (
        <div className="app-banner app-banner--warning" role="alert">
          <strong>Not connected.</strong>{" "}
          <span>Enter your solx-server URL and token to get started.</span>
          <button className="app-banner__btn" onClick={() => setSettingsOpen(true)}>
            Open settings
          </button>
        </div>
      )}
      {isConfigured() && healthQuery.data && !healthQuery.data.reachable && (
        <div className="app-banner app-banner--warning" role="alert">
          <strong>solx-server unreachable.</strong>{" "}
          <span>{getStoredServerUrl()} could not be reached. Check settings.</span>
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
              void docsListQuery.refetch();
              void docsSearchQuery.refetch();
              void actionsListQuery.refetch();
              void actionsSearchQuery.refetch();
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

      <TabBar tabs={DEFAULT_TABS} activeTab={activeTab} onTabChange={handleTabChange} />

      {showSearchBar && (
        <SearchBar
          activeTab={activeTab}
          state={browse}
          onChange={handleBrowseChange}
          total={activeTotal}
          loading={activeFetching}
          // Relevance ranking is a property of the full-text endpoint, so
          // derive it from the capability table rather than naming a tab.
          ranked={hasQuery && TAB_CAPABILITIES[activeTab].textQuery === "fulltext"}
        />
      )}

      <main className="app-main">
        <ErrorBoundary label="Entity browser">
          {activeTab === "documents" && (
            <DocumentList
              mode={hasQuery ? "search" : "browse"}
              documents={documents}
              total={documentsTotal}
              loading={hasQuery ? docsSearchQuery.isLoading : docsListQuery.isLoading}
              error={(hasQuery ? docsSearchQuery.error : docsListQuery.error) as Error | null}
              page={page}
              onPageChange={setPage}
              onCreateClick={() =>
                setDocEditor({ open: true, existing: null, entity: null })
              }
              onOpen={(doc) =>
                setDocEditor({
                  open: true,
                  existing: { path: doc.path, name: doc.name },
                  entity: doc,
                })
              }
              onRetry={() => {
                void docsListQuery.refetch();
                void docsSearchQuery.refetch();
              }}
            />
          )}

          {activeTab === "actions" && (
            <ActionList
              documents={actions}
              total={actionsTotal}
              loading={hasQuery ? actionsSearchQuery.isLoading : actionsListQuery.isLoading}
              error={(hasQuery ? actionsSearchQuery.error : actionsListQuery.error) as Error | null}
              page={page}
              onPageChange={setPage}
              onCreateClick={() =>
                setActionEditor({ open: true, existing: null, entity: null })
              }
              onOpen={(action) =>
                setActionEditor({
                  open: true,
                  existing: { path: action.path, name: action.name },
                  entity: action,
                })
              }
              onRetry={() => {
                void actionsListQuery.refetch();
                void actionsSearchQuery.refetch();
              }}
            />
          )}

          {activeTab === "types" && (
            <TypeList
              documents={typesListQuery.data?.items ?? []}
              total={typesListQuery.data?.total ?? 0}
              loading={typesListQuery.isLoading}
              error={typesListQuery.error as Error | null}
              page={page}
              onPageChange={setPage}
              onCreateClick={() =>
                setTypeEditor({ open: true, existing: null, entity: null })
              }
              onOpen={(type) =>
                setTypeEditor({
                  open: true,
                  existing: { path: type.path, name: type.name },
                  entity: type,
                })
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
          if (activeTab === "documents")
            setDocEditor({ open: true, existing: null, entity: null });
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
      {/* Only mount the editor when actually opened — this is what
          defers the lazy chunk (CodeMirror + Tiptap + EJS) until the
          user clicks a card. While the chunk loads we show a skeleton
          the same size as the editor so the layout doesn't jump. */}
      {docEditor.open && (
        <Suspense fallback={<DocumentEditorSkeleton />}>
          <DocumentEditor
            open={docEditor.open}
            onClose={() => setDocEditor({ open: false, existing: null, entity: null })}
            existing={docEditor.existing}
            existingEntity={docEditor.entity}
            defaultPath={pathPrefix || "/"}
          />
        </Suspense>
      )}

      <ActionEditor
        open={actionEditor.open}
        onClose={() => setActionEditor({ open: false, existing: null, entity: null })}
        existing={actionEditor.existing}
        existingEntity={actionEditor.entity}
        defaultPath={pathPrefix || "/"}
      />

      <TypeEditor
        open={typeEditor.open}
        onClose={() => setTypeEditor({ open: false, existing: null, entity: null })}
        existing={typeEditor.existing}
        existingEntity={typeEditor.entity}
        defaultPath={pathPrefix || "/"}
      />

      <FileEditor
        open={fileEditor.open}
        onClose={() => setFileEditor({ open: false, relPath: null })}
        relPath={fileEditor.relPath}
      />
    </div>
  );
}
