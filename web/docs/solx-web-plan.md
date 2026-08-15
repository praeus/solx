# solx-web plan (current state)

## Stack
- Bun backend (port 3001) → Vite (5173) → React 19 web
- `SOLX_NATIVE_BIN` env var must point at compiled `solx.node`
- Backend unwraps `solx.files.list(prefix)` (returns JSON-string) into `string[]`
- Frontend defensively normalizes `paths` so wrapper objects don't crash
- Per-tab error banner in App.tsx lifts queries and pushes errors into
  `tabErrors: Array<{kind, message}>`; banner shows
  `activeTabLabel(activeTab)` and offers a single retry button that
  refetches every failed query

## Recent changes
- Fixed empty-list bug: `detectMode(query, pathPrefix)` now treats
  path-prefix facet or empty-everything as "browse" so the user sees
  results instead of "No documents match your search."
- SearchBar takes `activeTab` and renders tab-specific facets:
  - Documents → Path, Type
  - Actions   → Path, In Type, Out Type
  - Types     → Path only
- Layout: search/path/type flow on one line; flex-wrap on small screens

## Validation
- web typecheck: clean
- web build: 105 modules, 304.91 kB JS / 16.05 kB CSS
- Browser smoke: all 4 tabs (Documents/Actions/Types/Files) render
  their lists with the path-prefix `/research` set

## Known issues
- Action `in_type_ref` / `out_type_ref` UI facets are wired to
  `SearchFacets` but not yet passed through to `/api/actions/search`
  (backend doesn't accept them yet — solx.actions.list has no
  in/out type filter)
- Many fields in the response are camelCase (`typeRef`, `actionType`,
  `updatedAt`) but the frontend interfaces use snake_case. Cards
  render with empty badges/dates as a result. Low priority.
- `listOptionsFromUrl` doesn't know about `sort_by` (only
  `sort_order`, `filter_field`, `filter_value`). Frontend sends
  `sort_by=updated_at` and it's silently dropped.