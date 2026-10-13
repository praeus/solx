# solx-web — Implementation Plan

> Generated 2026-08-11. This document guides the step-by-step build of the solx-web
> document browser UI. Each phase is a self-contained deliverable; phases 1.2 and 1.3
> can run in parallel.

## Status: IMPLEMENTED (v0.1)

All four phases of the plan have been built. End-to-end verified:
- React + Vite frontend on port 5173 (Vite proxy → backend)
- Bun backend on port 3001 (loadNative → solx-js SDK)
- solx-server on port 8766 (libsql + Tantivy)

Verified flows:
- Document **create** (POST → solx-server) — full UUID, timestamps, full record returned
- Document **list** (GET path_prefix)
- Document **get** (GET /api/documents/{path}/{name})
- Document **delete** (DELETE)
- Document **search** (GET q, type_ref, path_prefix)
- **Types list** (populates the facet dropdown in SearchBar)
- **Config** read/patch (server URL, token, data dir, files dir)
- **Health** endpoint (with solx connection status)

Build & test:
- `bun run typecheck` (server + web): clean
- `bun run build` (web): 84 modules, 254KB JS, 10KB CSS
- `bun run dev` (concurrent): Vite on 5173, Bun on 3001

## Implementation Notes

- The solx-js packages live in `../solx-js` and have `workspace:*` deps that don't
  resolve outside that monorepo. We vendor them into `server/vendor/` via
  `scripts/vendor-solx.mjs`, which rewrites `workspace:*` to `file:./<sibling>`.
- The vendored `solx.node` is the placeholder from the platform package. To use a
  real build, run `cargo build -p solx-bindings` in `../solx-js/crates/solx-bindings`
  and copy `target/debug/solx.node` into `server/vendor/solx-win32-x64-msvc/solx.node`.
  The vendor script preserves any existing binary larger than the placeholder.
- `SOLX_NATIVE_BIN` must be set when starting the dev script.

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  Browser (React + Vite)                             │
│  localhost:5173                                      │
│  ┌─────────┐  ┌──────────┐  ┌────────────────────┐  │
│  │ Tab Bar │  │ Doc List │  │ Settings / Add     │  │
│  │(Docs|… )│  │+ Search  │  │ (floating bubbles) │  │
│  └─────────┘  └──────────┘  └────────────────────┘  │
│        │              │              │               │
│        └────── fetch(/api/...) ──────┘               │
└───────────────────────┬─────────────────────────────┘
                        │ HTTP (Vite dev proxy)
┌───────────────────────▼─────────────────────────────┐
│  Bun Backend (solx-web/server/)                      │
│  localhost:3001                                      │
│  ┌──────────────────────────────────────────────┐   │
│  │  createSolx({ serverUrl, serverToken })       │   │
│  │    → solx.docs.list / search / get / save     │   │
│  │    → solx.types.list                          │   │
│  │    → solx.config.snapshot / patch             │   │
│  └──────────────────────────────────────────────┘   │
└───────────────────────┬─────────────────────────────┘
                        │ HTTP POST (JSON)
┌───────────────────────▼─────────────────────────────┐
│  solx-server (already running, port 8766)            │
│  /docs/*, /types/*, /files/*, /actions/*             │
└─────────────────────────────────────────────────────┘
```

**Why a Bun backend?** The `solx-js` SDK is a native Node addon (Neon bindings) —
it can't run in a browser. The Bun backend loads `solx-js`, connects to
`solx-server`, and exposes a clean REST API for the React frontend. This also
avoids CORS issues and keeps auth tokens server-side.

---

## Phase 1: Project Scaffolding

### 1.1 Monorepo root (`solx-web/`)

| File | Purpose |
|---|---|
| `package.json` | Workspace root with `workspaces: ["server", "web"]` and dev scripts |
| `tsconfig.base.json` | Shared strict TS config (ES2022, NodeNext for server, bundler for web) |
| `.gitignore` | `node_modules`, `dist`, `.env`, `*.log` |
| `README.md` | Project overview, setup instructions, dev loop |

### 1.2 Bun backend (`solx-web/server/`)

| File | Purpose |
|---|---|
| `package.json` | Deps: `solx` (workspace link to `solx-js/packages/sdk`), `@solx/surface` |
| `tsconfig.json` | Extends `../tsconfig.base.json` |
| `src/index.ts` | `Bun.serve()` HTTP server, mounts route handlers |
| `src/solx-context.ts` | `createSolx()` wrapper — reads `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN` from env |
| `src/routes/documents.ts` | Document CRUD + search endpoints |
| `src/routes/types.ts` | Type list endpoint (for doc type facet) |
| `src/routes/config.ts` | Config read/write endpoints |

### 1.3 React frontend (`solx-web/web/`)

| File | Purpose |
|---|---|
| `package.json` | Deps: `react`, `react-dom`, `@tanstack/react-query` |
| `tsconfig.json` | Extends `../tsconfig.base.json` |
| `vite.config.ts` | Vite config with proxy `/api` → `http://localhost:3001` |
| `index.html` | HTML entry point |
| `src/main.tsx` | React entry, renders `<App />` |
| `src/App.tsx` | App shell: TabBar + SearchBar + DocumentList + floating bubbles |
| `src/App.css` | App-level styles |
| `src/api/api.ts` | Typed fetch wrappers for all backend endpoints |
| `src/api/hooks.ts` | React Query hooks: `useDocuments`, `useSearchDocuments`, `useDocumentTypes`, `useConfig`, `useMutateConfig` |
| `src/components/TabBar.tsx` | Entity type tabs (Documents active; Actions/Types/Files disabled) |
| `src/components/SearchBar.tsx` | Search/browse input + path/type facet dropdowns |
| `src/components/DocumentList.tsx` | Document cards + pagination |
| `src/components/SettingsPanel.tsx` | Settings slide-out panel |
| `src/components/AddDocumentDialog.tsx` | Create document modal |

---

## Phase 2: Backend API

### 2.1 `solx-context.ts` — solx-js connection singleton

- Reads `SOLX_SERVER_URL` (default `http://127.0.0.1:8766`) and `SOLX_SERVER_TOKEN` from env
- Calls `createSolx({ serverUrl, serverToken })` on first access
- Exports `getSolx()` accessor (lazy-init, caches the promise)
- Exports `getSolxSync()` for health checks (returns null if not yet connected)
- Handles connection errors gracefully — logs, surfaces via health endpoint

### 2.2 Document routes (`server/src/routes/documents.ts`)

| Method | Path | Maps to | Notes |
|---|---|---|---|
| `GET` | `/api/documents` | `solx.docs.list(opts)` | Query params: `path_prefix`, `limit`, `offset`, `sort_by`, `sort_order`, `filter_field`, `filter_value` |
| `GET` | `/api/documents/search` | `solx.docs.search(query)` | Query params: `q`, `path_prefix`, `type_ref`, `limit`, `offset` |
| `GET` | `/api/documents/:path/:name` | `solx.docs.get(path, name)` | Path segments are URL-encoded; the `:path` param captures everything between `/api/documents/` and the last segment |
| `POST` | `/api/documents` | `solx.docs.save(path, name, input)` | Body: `{ path, name, input }` |
| `DELETE` | `/api/documents/:path/:name` | `solx.docs.delete(path, name)` | Same path/name extraction as GET |

### 2.3 Type routes (`server/src/routes/types.ts`)

| Method | Path | Maps to | Notes |
|---|---|---|---|
| `GET` | `/api/types` | `solx.types.list(opts)` | Returns `Page<TypeEntity>`; used to populate the document type facet dropdown |

### 2.4 Config routes (`server/src/routes/config.ts`)

| Method | Path | Maps to | Notes |
|---|---|---|---|
| `GET` | `/api/config` | `solx.config.snapshot()` | Returns `SolxConfig` |
| `PATCH` | `/api/config` | `solx.config.patch(patch)` | Body: partial config JSON; merges into existing config |

### 2.5 Health & error handling

- `GET /api/health` — returns `{ ok: true, serverUrl, solxConnected: boolean }`
- Global error handler maps `SolxError` variants to HTTP status codes:
  - `NotFound` → 404
  - `Invalid` → 400
  - `Validation` → 422
  - `Conflict` → 409
  - `Io`, `Db`, `Exec`, `Config`, `Other` → 500
- Error response body: `{ error: { kind, message } }`

---

## Phase 3: Frontend Core

### 3.1 Data fetching layer (`web/src/api/`)

**`api.ts`** — Typed fetch wrappers:
```typescript
// Documents
fetchDocuments(opts: ListOptions): Promise<Page<Document>>
searchDocuments(query: SearchQuery): Promise<SearchResults>
fetchDocument(path: string, name: string): Promise<Document>
createDocument(path: string, name: string, input: DocumentInput): Promise<Document>
deleteDocument(path: string, name: string): Promise<void>

// Types
fetchTypes(opts?: ListOptions): Promise<Page<TypeEntity>>

// Config
fetchConfig(): Promise<SolxConfig>
patchConfig(patch: Record<string, unknown>): Promise<void>

// Health
fetchHealth(): Promise<{ ok: boolean; serverUrl: string; solxConnected: boolean }>
```

**`hooks.ts`** — React Query hooks:
```typescript
useDocuments(opts: ListOptions)           // for path-prefix browsing
useSearchDocuments(query: SearchQuery)    // for full-text search
useDocument(path: string, name: string)   // single document detail
useDocumentTypes()                        // type facet dropdown
useConfig()                               // settings panel
useMutateConfig()                         // save settings
useCreateDocument()                       // add document dialog
useDeleteDocument()                       // delete from list
useHealth()                               // connection status banner
```

### 3.2 `TabBar.tsx`

- Renders four tabs: **Documents**, **Actions**, **Types**, **Files**
- Documents is active and clickable
- Actions, Types, Files show "Coming soon" tooltip on hover and are visually muted
- Active tab has accent underline/background
- Props: `activeTab: string`, `onTabChange: (tab: string) => void`

### 3.3 `SearchBar.tsx`

The core interaction surface:

- **Text input** (primary): placeholder "Search documents or browse by path…"
  - If text starts with `/` → **path-prefix browse mode**: shows a path indicator badge, calls `useDocuments({ path_prefix })`
  - If text doesn't start with `/` → **free-text search mode**: calls `useSearchDocuments({ q })`
  - 300ms debounce before triggering queries
  - Clear button (✕) to reset

- **Facet row** (visible when in search mode, i.e. text doesn't start with `/`):
  - **Path prefix** — text input, optional, narrows search to a path subtree
  - **Document type** — dropdown populated from `useDocumentTypes()`, shows "All types" default
  - Facets are additive: they're passed as `path_prefix` and `type_ref` in the `SearchQuery`

- **Sort controls** (visible in browse mode):
  - Sort by: Name | Last Updated
  - Direction toggle: ↑ / ↓

- Props: `mode: 'browse' | 'search'`, `query: string`, `facets: { path_prefix?, type_ref? }`, `sortBy`, `sortOrder`, callbacks for all changes

### 3.4 `DocumentList.tsx`

- **Scrollable list** of document cards
- **Each card** shows:
  - Document name (bold, primary)
  - Title (if present)
  - Type badge (small pill, e.g. "Post", "Note")
  - Summary excerpt (truncated to ~120 chars)
  - Updated date (relative or absolute)
  - Delete button (✕, with confirmation)

- **Pagination bar** (bottom):
  - ← Previous / Next →
  - "1–20 of 142" indicator
  - Page size: 20 items

- **States**:
  - **Loading**: skeleton cards (pulsing gray rectangles)
  - **Empty**: "No documents found" + "Create your first document" button
  - **Error**: "Failed to load documents" + retry button
  - **Search empty**: "No documents match your search" + suggestion to broaden

- Props: receives data from the parent (via hooks), plus callbacks for pagination, delete, and card click

### 3.5 `SettingsPanel.tsx`

- **Slide-out panel** from the left edge (or modal overlay on narrow screens)
- **Backdrop** click dismisses
- **Fields**:
  - Server URL (text input, default `http://127.0.0.1:8766`)
  - Server Token (password input, masked)
  - Data Directory (text input, read-only? or editable path)
  - Files Directory (text input)
- **Save button** — calls `patchConfig()`, shows success/error toast
- **Cancel button** — reverts changes, closes panel
- **Connection status** indicator (green dot = connected, red = disconnected)

### 3.6 `AddDocumentDialog.tsx`

- **Modal dialog** centered on screen
- **Fields**:
  - Name (required) — e.g. `my-first-post`
  - Path (required) — e.g. `/blog` (defaults to `/`)
  - Title (optional) — display title
  - Type (dropdown) — populated from `useDocumentTypes()`, defaults to "Post"
  - Summary (optional) — textarea, 3 rows
- **Create button** — calls `useCreateDocument()`, on success closes dialog and refreshes list
- **Cancel button** — closes dialog
- **Validation**: name and path are required; path must start with `/`

### 3.7 `App.tsx` — Shell layout

```
┌──────────────────────────────────────────────┐
│  TabBar                                      │
│  [Documents] [Actions] [Types] [Files]       │
├──────────────────────────────────────────────┤
│  SearchBar                                   │
│  [_____________________________] [facets]    │
├──────────────────────────────────────────────┤
│                                              │
│  DocumentList (scrollable)                   │
│  ┌──────────────────────────────────────┐    │
│  │  doc-card                             │    │
│  │  doc-card                             │    │
│  │  doc-card                             │    │
│  │  ...                                  │    │
│  └──────────────────────────────────────┘    │
│                                              │
│  Pagination: ← 1-20 of 142 →                │
│                                              │
│  ⚙ (bottom-left)              + (bottom-right)│
└──────────────────────────────────────────────┘
```

- **State** (lifted to App):
  - `activeTab: string` — currently "Documents"
  - `searchQuery: string` — the text input value
  - `searchMode: 'browse' | 'search'` — derived from whether query starts with `/`
  - `facets: { path_prefix?: string; type_ref?: string }` — search facets
  - `sortBy: 'name' | 'updated_at'`, `sortOrder: 'asc' | 'desc'`
  - `page: number` — current page (0-based)
  - `settingsOpen: boolean`, `addDialogOpen: boolean`

- **Floating bubbles**:
  - Settings (⚙) — bottom-left, fixed position, opens `SettingsPanel`
  - Add (+) — bottom-right, fixed position, opens `AddDocumentDialog`

- **Connection banner** (top, dismissible): shown when `useHealth()` reports solx-server unreachable

---

## Phase 4: Integration & Polish

### 4.1 Vite dev proxy

In `vite.config.ts`:
```typescript
export default defineConfig({
  server: {
    proxy: {
      '/api': 'http://localhost:3001',
    },
  },
});
```

### 4.2 Dev scripts

Root `package.json`:
```json
{
  "scripts": {
    "dev": "bun run --cwd server dev & bun run --cwd web dev",
    "dev:server": "bun run --cwd server dev",
    "dev:web": "bun run --cwd web dev",
    "build": "bun run --cwd web build",
    "lint": "bun run --cwd server lint && bun run --cwd web lint"
  }
}
```

Server `package.json`:
```json
{
  "scripts": {
    "dev": "bun run --watch src/index.ts",
    "lint": "tsc --noEmit"
  }
}
```

Web `package.json`:
```json
{
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "lint": "tsc --noEmit"
  }
}
```

### 4.3 Styling approach

- **CSS Modules** (`*.module.css`) for component-scoped styles
- **CSS custom properties** for theming (defined in `App.css` or `index.css`):
  ```css
  :root {
    --bg: #ffffff;
    --bg-raised: #f5f5f5;
    --text: #1a1a1a;
    --text-muted: #6b6b6b;
    --accent: #4f46e5;
    --accent-hover: #4338ca;
    --border: #e5e5e5;
    --radius: 8px;
    --shadow: 0 1px 3px rgba(0,0,0,0.1);
  }
  ```
- **Responsive**: works on desktop primarily; mobile is a stretch goal
- **Transitions**: 200ms ease for panel open/close, hover states

### 4.4 Edge cases & error handling

| Scenario | Behavior |
|---|---|
| solx-server unreachable | Red banner at top: "Cannot connect to solx-server at …" with retry |
| Empty document list | "No documents found" with "Create your first document" CTA |
| Search returns no results | "No documents match your search" with suggestion to broaden |
| Network error during fetch | React Query auto-retries (3x with exponential backoff) |
| Create document fails | Inline error message in the dialog, form stays open |
| Delete document fails | Alert with error message, list unchanged |
| Config save fails | Error toast, panel stays open |
| solx-js native addon fails to load | Backend logs error, health endpoint reports `solxConnected: false` |

---

## Verification Checklist

- [ ] **Backend smoke test**: Start solx-server, run `bun run server/src/index.ts`, hit `GET /api/health` → `{ ok: true, solxConnected: true }`
- [ ] **Document list**: `GET /api/documents?limit=5` → `Page<Document>` with items from solx-server
- [ ] **Document search**: `GET /api/documents/search?q=test` → `SearchResults` with hits
- [ ] **Browse by path**: `GET /api/documents?path_prefix=/research` → filtered results
- [ ] **Frontend dev**: `bun run dev` → Vite starts, proxy forwards `/api`, document list populates
- [ ] **Browse by path (UI)**: Type `/research` in search bar → list filters to that path prefix
- [ ] **Search (UI)**: Type `hello` → search results appear with type/path facets
- [ ] **Settings**: Open panel → change server URL → save → config persists
- [ ] **Add document**: Open dialog → fill fields → create → document appears in list
- [ ] **Delete document**: Click ✕ on a card → confirm → document removed from list
- [ ] **Pagination**: Navigate through pages, verify counts are correct
- [ ] **Connection error**: Stop solx-server → red banner appears → restart → banner dismisses

---

## Key Decisions

| Decision | Rationale |
|---|---|
| **Bun over Node.js** | User specified Bun; native `Bun.serve()`, built-in TS, fast startup |
| **React Query** | Caching, retry, loading/error states, deduplication — no need to reinvent |
| **Vite proxy over CORS** | Avoids CORS config; production mode serves static files from Bun |
| **Documents-only first** | Actions/Types/Files tabs are disabled placeholders; same pattern, add later |
| **Path + Type facets only** | `linked_to` facet deferred for simplicity |
| **Settings: connection + folders** | Minimum needed to configure solx-js; full config surface deferred |
| **CSS Modules** | Component-scoped styles, no framework dependency |

---

## Further Considerations

1. **Production build**: Vite outputs to `web/dist/`. The Bun backend should serve these
   static files in production mode. Add a `NODE_ENV=production` check in
   `server/src/index.ts` to serve `web/dist/` instead of relying on the Vite dev proxy.

2. **solx-js platform binary**: The `solx` package depends on `@solx/win32-x64-msvc`
   (or the appropriate platform package). Ensure the workspace link resolves correctly.
   On non-Windows platforms, the appropriate platform package must be installed.

3. **Document detail/edit view**: The current plan shows document cards in a list.
   Clicking a card could expand inline details or navigate to a detail view. This is
   deferred but the card `onClick` handler should be wired to accept future expansion.

4. **Actions/Types/Files tabs**: Each follows the same pattern as Documents:
   - **Actions**: list/search actions, execute button, action editor
   - **Types**: list types, view/edit JSON Schema
   - **Files**: file tree browser, upload/download, preview

5. **Authentication**: The token is configured in settings but there's no login flow.
   The backend reads it from config/env and passes it to solx-server. A proper auth
   flow (OAuth, API keys) could be added later.

---

## File Manifest

### New files to create (25 files)

```
solx-web/
├── package.json
├── tsconfig.base.json
├── .gitignore
├── README.md
├── server/
│   ├── package.json
│   ├── tsconfig.json
│   └── src/
│       ├── index.ts
│       ├── solx-context.ts
│       └── routes/
│           ├── documents.ts
│           ├── types.ts
│           └── config.ts
└── web/
    ├── package.json
    ├── tsconfig.json
    ├── vite.config.ts
    ├── index.html
    └── src/
        ├── main.tsx
        ├── App.tsx
        ├── App.css
        └── api/
        │   ├── api.ts
        │   └── hooks.ts
        └── components/
            ├── TabBar.tsx
            ├── SearchBar.tsx
            ├── DocumentList.tsx
            ├── SettingsPanel.tsx
            └── AddDocumentDialog.tsx
```

### Existing files to reference (do not modify)

| File | What to reference |
|---|---|
| `solx-js/packages/sdk/src/index.ts` | `createSolx()` API, `Solx` interface |
| `solx-js/packages/surface/src/index.ts` | DTOs: `Document`, `SearchResults`, `ListOptions`, `Page<T>`, `SolxError` |
| `solx-js/packages/docs/src/index.ts` | `DocManager` class — `list()`, `search()`, `get()`, `save()`, `delete()` |
| `solx-js/packages/config/src/index.ts` | `ConfigService` — `snapshot()`, `patch()` |
| `solx-js/packages/types/src/index.ts` | `TypeManager` — `list()` |
| `solx-core/solx-server/src/routes/docs.rs` | Server-side doc endpoints (request/response shapes) |
| `sol-browser/src/components/EntityBrowser.tsx` | UI patterns: type sidebar, entity cards, pagination, filter toolbar |
| `sol-browser/src/components/DocumentEditor.tsx` | Form fields, type selector, knowledge base selector |
| `sol-browser/src/types.ts` | `EntityTypeName`, `ENTITY_TYPES`, `ENTITY_TYPE_LABELS` |
