# solx-web Document Editor - Features and Roadmap

_Generated 2026-08-15._ Tracks the current state of the web-based
Document editor and the gaps that remain before we reach feature parity
with the legacy sol-browser DocumentEditor (and beyond).

This document complements solx-web-plan.md (broader product/UX plan)
and design-and-progress.md in solx-core (canonical solx design
overview).

## 1. Top-level shape

The Document editor is a wide modal (`dialog--xxl`, 1080 px) with two
panes:

- **Left:** form. Top-level fields, then a schema-driven `contents`
  editor, then a Files and Data section, then any error.
- **Right:** preview. A two-tab Preview / Contents JSON switcher.
  Preview is rendered client-side from an EJS template carried in the
  type's schema (see Section 5).

Top-level fields: **name**, **path** (disabled in edit mode), **title**,
**summary**, **author**, **type**, **publication date** (YYYY-MM),
**confidence** (0-1), **files** (via `FilesField`).

KnowledgeBase is **not** carried over from sol-browser; documents are
scoped by `path` in the new system.

## 2. Top-level fields

| Field        | Wire               | Notes                                                                                                                        |
| ------------ | ------------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `name`       | top-level          | Required; immutable after creation (edit mode disables the input).                                                           |
| `path`       | top-level          | Required; must start with `/`. Immutable in edit mode.                                                                       |
| `title`      | `input.title`      | Optional display title.                                                                                                      |
| `summary`    | `input.summary`    | Optional short blurb; used by search and list views.                                                                         |
| `author`     | `input.author`     | Optional author name.                                                                                                        |
| `pub_date`   | `input.pub_date`   | Optional YYYY-MM.                                                                                                            |
| `confidence` | `input.confidence` | Optional 0-1 numeric.                                                                                                        |
| `type`       | `input.typeRef`    | Required. Backend's `typeRef` is `/packages/<pkg>/<Name>` or `/builtin/types/<Name>`; dropdown shows the bare name.       |
| `files`      | `input.files`      | `FileRef[]` (see Section 4).                                                                                                  |
| `contents`   | `input.contents`   | Schema-driven; see Section 3.                                                                                                 |

On save, the wire shape is `{ path, name, input: { type_name, title?,
..., files, contents } }`. `type_name` is mapped to
`typeRef = "/builtin/types/<Name>"` for built-in types; for
installed-package types the caller must pass the `typeRef` directly via
`createDocument({ type_ref: ... })` (not yet wired in the editor; see
Section 10).

## 3. Schema-driven contents editor

The selected type's `schema` (fetched via `useType(path, name)`) is
introspected per-property. Each property is rendered by a
per-schema-type dispatcher (`renderDocField` in
`web/src/components/DocumentEditor.tsx`):

| Schema marker                                                | Renderer                                                                                                   |
| ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `$ref` includes `DocRef` (or `allOf`/`anyOf` does)           | name-or-id input + datalist of all document names                                                           |
| `$ref` includes `ActionRef` (or `allOf`/`anyOf` does)         | name-or-id input + datalist of all action names                                                            |
| `x-sol-editor: "rich_text"` or `$ref` to `RichTextDoc`        | `RichTextField` (Tiptap)                                                                                   |
| `type: "boolean"`                                            | checkbox                                                                                                    |
| `type: "array"` or `type: "object"`                          | `JsonEditField` (CodeMirror 6 with JSON syntax highlighting + live parse badge)                             |
| `type: "number"` / `"integer"`                               | textarea (numeric coercion happens on save)                                                                |
| any other text-like                                          | textarea (with a height hint when the name contains `contents`/`summary`/`text`/`body`/`description`)      |

`JsonEditField` is reusable across the editor for any JSON input and
will be the basis for the array-of-objects editor (Section 10.2).

`RichTextField` wraps Tiptap 3 with `StarterKit` (bold/italic/code,
headings 1-3, bullet/ordered lists, blockquote, code block, hard break).
It round-trips ProseMirror JSON. The toolbar is a flat row of buttons
above the editor.

### Modes

- **Fields** (default): per-property form, schema-driven.
- **JSON**: whole-document JSON editor — `JsonEditField` again, with a
  live parse badge that gates the Save button.

### Helper fields

The "Show helper fields" toggle reveals properties that are usually
hidden:

- `x-sol-hidden: true` in the schema
- field name in `{ chapters, metadata }` (legacy helper fields from
  sol-browser)

### CompoundDocument sections

When the selected type is `CompoundDocument` and the document is
persisted, the editor queries
`/api/documents/search?type_ref=DocRef` and shows each result as a
read-only button listing `field_name` -> `target_document_name`.
Clicking is a future hook — currently the buttons are visual only.

## 4. Files and Data

### 4.1 Attached files (FilesField)

We do not carry over the legacy `ArtifactRef` system from sol-browser.
Instead, documents have a `files: FileRef[]` field on the wire:

```ts
interface FileRef {
  name: string;       // display name
  relPath: string;    // path relative to the files root
  contentType?: string;
}
```

`FilesField` lets the user do **both** of the following without leaving
the document editor:

- **Pick from existing files.** A dropdown lists every path returned
  by `useFilesList("")` (the solx-files store). Selecting an entry
  adds it to `value` as a `FileRef`.
- **Upload a new file from disk.** A file input + relPath field +
  Upload button. The bytes are PUT to `solx.files.put(relPath, bytes)`
  via the existing `/api/files` PUT endpoint, and the resulting
  `relPath` is auto-attached.

Image attachments render a 40×40 thumbnail via the new binary
`/api/files/raw?relPath=…` route. Other files just show name + path.

The default upload target is `files/docs/shared/` (mirroring the
conventional solx-files layout); callers can override with the
`uploadPrefix` prop (e.g. `"files/actions/<action>/"`).

On save, the array is sent as `input.files`. Hydration accepts either
the wire `FileRef[]` shape or the legacy `{ name }` shape via
`coerceFilesField()`.

### 4.2 Image schema fields (ImageField)

Schema fields whose `$ref` ends in `Image` or `ArtifactRef` (or whose
schema sets `x-sol-editor: "image"`) are rendered with `ImageField`
instead of a plain text input. The field stores a single relPath
string on the wire:

```jsonc
// docs.contents.icon
"icon": "files/docs/shared/icon.png"
```

`ImageField` shows:

- A 96×96 live preview loaded from `/api/files/raw?relPath=…`.
- A relPath text input (manually typed paths are accepted; the
  preview re-renders on each change).
- An "Upload image" button — opens a file picker, accepts `image/*`,
  PUTs the bytes to `solx.files.put(...)` at
  `files/docs/shared/<file.name>`, and sets the relPath to the
  normalized result.
- A small × clear button.

On hydration, legacy `{ name }` shapes are coerced to
`files/docs/shared/<name>` so old documents continue to load.

### 4.3 Inline images in rich text (RichTextField)

`@tiptap/extension-image` is registered as a Tiptap extension, so
rich-text fields gain a **🖼 toolbar button** that:

1. Opens a file picker (filter: `accept="image/*"`).
2. PUTs the chosen file to `solx.files.put(...)` at
   `files/docs/shared/<file.name>` (or wherever `uploadPrefix` is set).
3. Inserts an `<img src="/api/files/raw?relPath=…" alt="…">` node
   into the document.

The src is a real URL the browser loads directly (with the right
Content-Type), so the rich-text doc round-trips perfectly: reloading
the document re-renders the same image. CSS targets
`.rich-text-field__image` to constrain the rendered size to
`max-width: 100%; height: auto`.

### 4.4 Binary file route (`GET /api/files/raw`)

The Bun backend serves a new binary route:

```
GET /api/files/raw?relPath=files/docs/shared/icon.png
→ 200 image/png  (raw bytes, Content-Length set, Cache-Control: max-age=3600)
```

It runs `solx.files.get(relPath)` and writes the bytes back with a
Content-Type inferred from the file extension. Recognized extensions:
png/jpg/jpeg/gif/webp/svg/avif/bmp/ico/pdf/json/wasm/html/css/js/md/
txt/csv/solx. Anything else falls back to `application/octet-stream`
so the browser still offers a download.

The frontend uses `fileRawUrl(relPath) = "/api/files/raw?relPath=…"`
to build URLs for `<img src>` and Tiptap image nodes. This is a
path-only URL so the Vite dev server proxy (which forwards `/api/*`
to the Bun backend) keeps working in dev.

## 5. Preview

### Old approach (sol-browser)

`render document html` action executed server-side via the tera
template engine; templates associated with types via the type's
`x-sol-template` meta tag; the response was the rendered HTML shown
in the preview pane.

### New approach (solx-web)

Templates still live in the type's schema under `x-sol-template` (or
`x-sol-preview` alias), but rendering is **client-side** with
[`ejs`](https://ejs.co/). The preview pane is a `DocumentPreview`
component that:

1. Resolves the template from `type.schema["x-sol-template"]` (or
   `x-sol-preview`), falling back to a built-in dump that renders the
   title, summary, and contents as a `<dl>`.
2. Runs `ejs.render(template, { document, contents })` — templates
   can do `<%= contents.title %>` or `<%= document.path %>` etc.
3. Renders via `dangerouslySetInnerHTML` (the trust boundary is the
   type schema, which the user wrote themselves).

The "Contents JSON" tab shows the post-coercion `contents` map (after
schema-driven field merging) for debugging what will actually be sent
on save.

## 6. Status and what's missing

### Implemented

- [x] Top-level fields (name, path, title, summary, author, type, pub_date, confidence)
- [x] Drop `KnowledgeBase` (legacy from sol-browser)
- [x] Drop `ArtifactRef` — replaced with `FilesField` (`FileRef[]`)
- [x] Drop `LinkSelector` / `DocumentInquiryDialog` (out of scope)
- [x] Schema-driven contents editor with per-type dispatcher
- [x] `JsonEditField` (CodeMirror 6) for `array`/`object`/JSON-mode
- [x] `RichTextField` (Tiptap 3) for `rich_text` fields
- [x] `FilesField` upload-from-disk (file picker + relPath + upload)
- [x] `ImageField` for image-shaped schema fields (live preview + upload)
- [x] Inline images in `RichTextField` via `@tiptap/extension-image`
- [x] Binary `/api/files/raw` route for `<img src>` and image nodes
- [x] Fields <-> JSON mode toggle, "Show helper fields" toggle
- [x] CompoundDocument sections list
- [x] Frontend preview via EJS + `x-sol-template`
- [x] Save (create + update) via `useCreateDocument` / `useUpdateDocument`
- [x] Card click -> edit, FAB -> create
- [x] Backend URL routing fix for action names with spaces (`splitRef` decodes segments)

### Deferred

- [ ] **SchemaObjectArrayEditor** for `array-of-objects` fields (e.g.
      `BlogPostWithComments.comments`). The legacy sol-browser
      component is ~513 lines and handles recursive nesting up to a
      depth cap. The current editor falls through to `JsonEditField`
      for these fields; users can edit the JSON until the schema
      editor lands.
- [ ] **`type_ref` for installed-package types** — the editor maps a
      bare type name to `/builtin/types/<Name>` automatically. For
      types in installed packages (path `/packages/<pkg>/<Name>`) the
      `createDocument` API already accepts `type_ref`, but the editor
      needs to pass the full ref instead of just the name. Currently
      those types fall back to `/builtin/types/...` and the create
      fails.
- [ ] **Compound section navigation** — buttons render but don't open
      the target document. Trivial to wire once we decide the UX
      (new tab vs. navigate vs. modal-in-modal).
- [ ] **Inline preview refresh** — the right-pane preview re-renders
      on every keystroke (it's just an `ejs.render`). For large
      documents with heavy templates this could be debounced.
- [ ] **Per-type custom toolbar extensions for Tiptap** — we ship
      StarterKit for every `rich_text` field. Real-world schemas will
      want custom marks (e.g. `x-sol-editor: "rich_text_with_images"`)
      driving a different Tiptap extension set.
- [ ] **Code-splitting** — the build currently emits a ~1 MB chunk
      because CodeMirror + Tiptap + EJS all live in the main bundle.
      Lazy loading the document editor would cut the initial
      Documents-tab bundle by ~80 %.

### Known gaps to call out (UX)

- **The right pane hides the error banner from the global per-tab
  error banner.** If `/api/documents/<path>/<name>` 500s on hydration,
  the ErrorBoundary catches it and shows the stack — that's expected.
  But the user has no obvious way to retry; the dialog needs to be
  closed and reopened. Future: surface hydration errors inline.

## 7. Component map

```
web/src/components/
  DocumentEditor.tsx           orchestrator
  DocumentList.tsx             list view; opens DocumentEditor on card click
  JsonEditField.tsx            reusable CodeMirror JSON editor
  RichTextField.tsx            reusable Tiptap rich-text editor (now with 🖼 image button)
  FilesField.tsx               attached files picker + uploader (replaces ArtifactRef)
  ImageField.tsx               single-image schema field (live preview + upload)
  DocumentPreview.tsx          EJS-rendered preview pane
  SchemaObjectArrayEditor.tsx  placeholder; not yet wired
  ErrorBoundary.tsx            catches render-time exceptions
```

The DocumentEditor keeps all state local (no service layer) because
solx-web doesn't yet have a `documentEditorService` equivalent. The
form is fully controlled; per-field rendering is dispatched by a
single `renderDocField` helper that walks the type's schema.

## 8. Wire format

### Document detail (response)

```jsonc
{
  "id": "...",
  "path": "/test",
  "name": "hello-v7",
  "title": "Hello v7",
  "summary": "smoke test",
  "typeRef": "/packages/solx-google/GoogleDocsDoc",
  "contents": { ... },
  "author": null,
  "pubDate": "2026-08-14T06:03:48.696566100Z",
  "files": [
    { "name": "icon.png", "relPath": "files/test/hello-v7/icon.png", "contentType": "image/png" }
  ],
  "links": [],
  "createdAt": "...",
  "updatedAt": "..."
}
```

> The wire shape is camelCase (`typeRef`, `createdAt`, `updatedAt`,
> `pubDate`, `relPath`, `contentType`). The TypeScript `DocumentSummary`
> interface in `web/src/api/api.ts` is still snake_case; this is a
> known mismatch — cards currently render with empty badges/dates
> because the camelCase fields aren't read. Tracked separately.

### Create (POST /api/documents)

```jsonc
{
  "path": "/test",
  "name": "hello-v7",
  "input": {
    "typeRef": "/packages/solx-google/GoogleDocsDoc",
    "title": "Hello v7",
    "summary": "smoke test",
    "contents": { "documentId": "abc", "title": "Hello v7", "body": {...}, "revisionId": "1" },
    "files": []
  }
}
```

### Update (POST /api/documents/:path/:name)

Same body shape as create. The backend maps `type_name` to
`typeRef = /builtin/types/<Name>` automatically (see
`api.ts` `createDocument`).

## 9. Preview template examples

A `Post` type with a custom template:

```jsonc
{
  "type": "object",
  "properties": {
    "title":    { "type": "string", "title": "Title" },
    "summary":  { "type": "string", "title": "Summary" },
    "contents": { "type": "string", "title": "Body" }
  },
  "x-sol-template": "<article class=\"post\">\n  <h1><%= contents.title %></h1>\n  <% if (contents.summary) { %><p class=\"summary\"><%= contents.summary %></p><% } %>\n  <div class=\"body\"><%= contents.contents %></div>\n</article>"
}
```

The default fallback (no `x-sol-template` set) renders:

```html
<div class="doc-preview doc-preview--fallback">
  <h1>{document.title || document.name}</h1>
  <p class="doc-preview__meta">
    <code>{document.path}/{document.name}</code>
    - Type: <code>{document.type_name}</code>
  </p>
  <h2>Contents</h2>
  <dl class="doc-preview__fields">
    <dt>{key}</dt>
    <dd><pre><code>{JSON.stringify(value, null, 2)}</code></pre></dd>
  </dl>
</div>
```

## 10. Roadmap to parity (and beyond)

Roughly ordered by ROI:

1. **Fix `type_ref` for installed-package types.** Wire the editor's
   `createDocument` call to send the full `/packages/<pkg>/<Name>` ref
   when the type list reports that path, instead of always mapping to
   `/builtin/types/<Name>`. Trivial; ~10 lines.
2. **Snake/camel cleanup.** Decide whether the wire format changes
   (`typeRef` -> `type_ref`) or the frontend types do. The Rust side
   already uses snake_case for storage; the wire shape is the
   inconsistency. Smaller change: align the TS interfaces to
   camelCase.
3. **SchemaObjectArrayEditor.** Port the ~500-line component from
   sol-browser; add a dispatch in `renderDocField` for `array` whose
   `items` are objects. This unlocks `BlogPostWithComments.comments`.
4. **Compound section navigation.** Decide UX (new tab vs. modal in
   modal vs. replace dialog) and wire `onClick` on the buttons in the
   CompoundDocument sections block.
5. **Code-splitting.** `lazy(() => import("./DocumentEditor"))` from
   `DocumentList`'s card click handler; cuts initial bundle by ~80 %.
6. **Per-type custom Tiptap extensions.** Read another schema meta
   tag (`x-sol-editor-extensions?: string[]`) and load matching
   Tiptap extensions.
7. **Rich preview refresh debouncing.** Optional: wrap
   `DocumentPreview` in a 150 ms debounce when the document is large.
8. **Image previews in `FilesField`.** Detect `image/*` content types
   and render a small thumbnail.

## 11. Cross-references

- `solx-web-plan.md` — broader UX/product plan, per-tab state, error
  banner, etc.
- `implementation-plan.md` — phased build history; this editor is the
  v0.1 "Actions/Types/Files" extension of that plan.
- `../solx-core/docs/design-and-progress.md` — canonical solx design
  overview; documents/actions/types/files are the core primitives.
- `web/src/api/api.ts` — `createDocument` / `updateDocument` /
  `fetchDocument` signatures; the wire-level mapping.
- `web/src/components/DocumentEditor.tsx` — the editor itself;
  everything in this doc is implemented there.
