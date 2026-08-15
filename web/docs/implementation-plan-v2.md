# solx-web v2 — Actions, Types, Files tabs

> Status: **Implemented (2026-08-14)**. End-to-end verified.

Implement browsing + CRUD for the three remaining entity tabs (Actions, Types, Files), with a Monaco JSON Schema editor for Types and a real exec panel for Actions.

---

## Scope

| Tab | Browsing | Editor | Special |
|---|---|---|---|
| **Actions** | List + search by path; click row → modal editor | Form: name, caption, description, category, phrases, capabilities, type selectors, action_type, exec_artifact, invoke_resource, permission, model_instructions, raw action_config JSON | Run panel — JSON params textarea, Run button, result + trace |
| **Types** | List + search by path; click row → modal editor | Form: name (immutable after create), description, groups (chips), JSON Schema (Monaco) | Pre-fill Monaco from existing schema; live parse badge |
| **Files** | Flat list with prefix filter; click row → preview/download | Modal: relPath (read-only), uploaded bytes (preview), delete button | Upload form; download via blob URL |

**Not in scope** (follow-up work): schedule editor, project scaffolding, artifact selector (entity-attached), link selector, knowledge base editor, permission editor, model editor, rich text editor, full Advanced action_config JSON editor (matching the sol-browser action_editor).

---

## Architecture

Mirror the existing Documents tab pattern. New components sit alongside the existing ones; new tabs unblock in `TabBar.tsx`; new routes mount in `server/src/index.ts`.

```
Backend (Bun)                          Frontend (React)
─────────────                          ────────────────
server/src/routes/
  actions.ts            ─────────────►  components/{ActionList,ActionEditor}.tsx
  types.ts (extended)   ─────────────►  components/{TypeList,TypeEditor}.tsx
  files.ts              ─────────────►  components/{FileList,FileEditor}.tsx
```

---

## Phase 1: Backend routes *(~3 files)*

### `server/src/routes/actions.ts`

Mirror `documents.ts` shape. Five endpoints:

| Method | Path | Calls | Notes |
|---|---|---|---|
| `GET` | `/api/actions` | `solx.actions.list(opts)` | Forwards `path_prefix`, `limit`, `offset`, `sort_by`, `sort_order`, `filter_field`, `filter_value` |
| `GET` | `/api/actions/search` | `solx.actions.list(opts)` with `filter_field=name` LIKE | Browse/search by free-text name |
| `GET` | `/api/actions/:path/:name` | `solx.actions.get(path, name)` | Same `splitRef` helper as documents |
| `POST` | `/api/actions` | `solx.actions.post(path, name, input)` | Body: `{ path, name, input: { typeRef, caption, ... } }` |
| `DELETE` | `/api/actions/:path/:name` | `solx.actions.delete(path, name)` | |
| `POST` | `/api/actions/:path/:name/exec` | `solx.actions.exec(path, name, params)` | Body: `{ params: JsonValue }` |

### `server/src/routes/types.ts` (extend existing)

| Method | Path | Calls | Notes |
|---|---|---|---|
| `GET` | `/api/types` | (existing) `solx.types.list(opts)` | |
| `GET` | `/api/types/:path/:name` | `solx.types.get(path, name)` | new |
| `POST` | `/api/types` | `solx.types.post(path, name, input)` | new — body `{ path, name, input: { description, schema, groups } }` |
| `DELETE` | `/api/types/:path/:name` | `solx.types.delete(path, name)` | new |

### `server/src/routes/files.ts` (new)

Bytes travel as base64-encoded JSON for the dev-time API:

| Method | Path | Calls | Notes |
|---|---|---|---|
| `GET` | `/api/files/list` | `solx.files.list(prefix)` | Returns `{ paths: string[] }` |
| `PUT` | `/api/files` | `solx.files.put(relPath, bytes)` | Body: `{ relPath, bytesB64 }` returns `{ relPath }` |
| `GET` | `/api/files/content` | `solx.files.get(relPath)` | Returns `{ bytesB64, size, contentTypeGuess: null }` |
| `DELETE` | `/api/files` | `solx.files.delete(relPath)` | Body: `{ relPath }` |

### `server/src/index.ts` (update)

Mount the two new routes in the `route()` dispatch.

### Refactor `splitRef()`

Extract `splitRef()` from `documents.ts` into `server/src/routes/_helpers.ts` so all three route files can use it.

---

## Phase 2: Frontend API layer *(1 file)*

### `web/src/api/api.ts` (extend)

Add fetchers:

```ts
// Actions
fetchActions(opts)         → GET /api/actions
searchActions(opts)        → GET /api/actions/search
fetchAction(path, name)    → GET /api/actions/{path}/{name}
createAction(input)        → POST /api/actions
deleteAction(path, name)   → DELETE /api/actions/{path}/{name}
execAction(path, name, params) → POST /api/actions/{path}/{name}/exec

// Types
fetchType(path, name)      → GET /api/types/{path}/{name}
createType(input)          → POST /api/types
deleteType(path, name)     → DELETE /api/types/{path}/{name}

// Files
listFiles(prefix)          → GET /api/files/list?prefix=...
getFileContent(relPath)    → GET /api/files/content?relPath=...
uploadFile(relPath, bytes) → PUT /api/files
deleteFile(relPath)        → DELETE /api/files
```

Define local DTOs: `ActionSummary`, `TypeEntityDetail`, `ActionExecResult`, `FileEntry`.

### `web/src/api/hooks.ts` (extend)

Add hooks: `useActions`, `useSearchActions`, `useAction`, `useCreateAction`, `useDeleteAction`, `useExecAction`, `useType`, `useCreateType`, `useDeleteType`, `useFilesList`, `useFileContent`, `useUploadFile`, `useDeleteFile`.

---

## Phase 3: Frontend components *(~8 files)*

### Shared building blocks

- `ChipTagInput.tsx` — for phrases/capabilities/groups/allowed_*. Enter/comma → add, Backspace-on-empty → pop, × → remove.
- `JsonSchemaEditor.tsx` — Monaco wrapper with theme detection, 2-space tabs, word-wrap, "valid JSON" badge.

### `ActionList.tsx`

Match `DocumentList.tsx` shape. Cell shows name, caption, action_type badge, category, updated_at. Delete button with confirm. Renders pagination.

### `ActionEditor.tsx` (modal)

- Form fields: name, caption, description, category, phrases (chips), capabilities (chips), param type (select), result type (select), action_type (select), exec_artifact, invoke_resource, permission, model_instructions (textarea), raw action_config JSON (collapsible).
- Run panel — JSON params textarea, Run button, result textarea, trace table.
- Save → `createAction` / `deleteAction`; refresh list on success.

### `TypeList.tsx`

Same shape as DocumentList. Cell: name, description, schema property count, updated_at.

### `TypeEditor.tsx` (modal)

- Name (read-only after creation), description, groups (chips), JSON Schema (Monaco).
- Parses Monaco on save; rejects invalid JSON.

### `FileList.tsx`

- Prefix filter input (debounced).
- Flat table of paths: each row clickable → preview/download.
- "+ Upload" button → modal with file input + relPath field.

### `FileEditor.tsx` (modal)

- Shows relPath, size, file preview (text or base64 inline image).
- Buttons: Download (browser blob), Delete.

### `App.tsx` (update)

- Enable Actions/Types/Files tabs in `TabBar`.
- Three content branches mirroring the documents branch.
- Add-fab icon switches per active tab.

### `TabBar.tsx` (update)

Mark Actions/Types/Files as `enabled: true`.

### `App.css` (extend)

Add styles for code/JSON previews, file table, action cards.

---

## Phase 4: Wiring & verification

- Mount `actions`/`files` routes in `server/src/index.ts`
- Lint, typecheck, build
- Smoke test: list, create, edit, delete for each tab via curl + browser
- Run action end-to-end

---

## Files to add

```
server/src/
  routes/
    _helpers.ts            ← splitRef + qs helpers
    actions.ts             ← new
    files.ts               ← new
  routes/types.ts          ← extend (already exists)

web/src/
  components/
    ActionList.tsx
    ActionEditor.tsx
    TypeList.tsx
    TypeEditor.tsx
    FileList.tsx
    FileEditor.tsx
    ChipTagInput.tsx
    JsonSchemaEditor.tsx

  api/api.ts               ← extend
  api/hooks.ts             ← extend
  App.tsx                  ← update
  App.css                  ← extend
  TabBar.tsx               ← update (enable 3 tabs)
```

Existing files needed: `vite.config.ts` (already has proxy), `web/src/api/api.ts` (ApiError class), `web/src/api/hooks.ts` (QueryClient).
