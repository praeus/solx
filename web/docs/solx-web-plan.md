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
- **DocumentEditor ported** from sol-browser:
  - Top-level fields: name, path, title, summary, author, KB, pub_date,
    confidence, type.
  - Schema-driven `contents` editor with Fields/JSON mode toggle and
    "Show helper fields" toggle.
  - Per-field dispatch: text, number, boolean, DocRef/ActionRef
    (name-or-id with datalist), ArtifactRef (single name with letter
    preview), rich_text (round-trips as JSON), array/object as JSON.
  - CompoundDocument sections list (when type = CompoundDocument and
    there's a persisted name).
  - Preview pane calls `render document html` action and renders the
    HTML; switches to a "Contents JSON" view.
  - Replaces AddDocumentDialog (which is now removed from App.tsx).
  - Card click → edit; FAB → create.

## Validation
- web typecheck: clean
- web build: 105 modules, 320.96 kB JS / 19.43 kB CSS
- Browser smoke: click-to-edit and create flow work for documents;
  schema-driven fields render correctly (rich_text, ArtifactRef);
  preview pane surfaces backend errors gracefully.

## Resolved: the search/filter/sort rework

Three issues here shared one root cause and were fixed together.

**The casing boundary.** `@solx/surface` is camelCase; the Rust wire
format and this HTTP API are snake_case. The Bun server was handing
snake_case straight to the SDK, whose converters read `opts.pathPrefix`,
found `undefined`, and emitted nothing — so *every* multi-word option was
silently dropped in both directions. Only `q`, `limit` and `offset`
survived, because those spellings coincide. That is why `sort_by` looked
unsupported and why cards rendered with empty badges and dates.

All translation now happens in one place, `server/src/wire.ts`. Opaque
payloads (`contents`, `schema`, `action_config`, exec results) pass
through verbatim — a blanket key rewrite would corrupt user JSON.

**Composability.** `detectMode` switched to browse whenever the path
started with `/`, which it always does, so filling in a path silently
discarded the text query. Query, path scope and type now compose with AND
semantics; the mode heuristic and the Browse/Search badge are gone. See
`TAB_CAPABILITIES` in `components/SearchBar.tsx`.

**Backend parity.** `solx-actions` and `solx-types` accepted
`filter_field`/`sort_by` but ignored them, hardcoding `ORDER BY
path,name`. Both now implement them through the shared
`ListOptions::to_sql` renderer in `solx-surface`, which `solx-docs` also
uses. `/api/actions/search` was removed — it could only ever return the
unfiltered list; name search is `/api/actions?filter_field=name`.

## Known issues
- Action lookup for names with spaces (e.g. `render document html`)
  breaks at the backend router because `splitRef` on the URL splits
  on the last `/` *after* URL decoding, so the name becomes just
  `html`. Workaround: quote action names that contain spaces when
  calling `solx.actions.exec` — or fix the backend route to treat the
  encoded-name segment as opaque.
- DocumentEditor doesn't yet have a real RichTextEditor / Tiptap
  integration — rich_text fields round-trip through a textarea holding
  the ProseMirror JSON. Future work.
- DocumentEditor doesn't have a true SchemaObjectArrayEditor for nested
  array-of-object fields (e.g. BlogPostWithComments.comments). For
  now those fields fall through to JSON mode. The full
  sol-browser SchemaObjectArrayEditor (~513 lines) would need to be
  ported separately.