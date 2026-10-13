# solx-web

A browser UI for [solx-core](../solx-core) — documents, actions, types, and
files. React talking straight to `solx-server`; no backend of its own.

```
Browser (React + Vite :5173)
  └─▶ @solx/http (fetch-based, aliased to solx-js source)
      └─▶ solx-server (:8766, CORS-enabled)
```

`web/` used to route through a Bun backend that loaded solx-js's Node-native
binding and re-exposed it as REST — a second hop for what `@solx/http`
already does in one. That backend is gone.

## What's in it

**Documents** — path-prefix browsing (type `/` to start a path query),
full-text search with type and path facets, and an editor that renders fields
by their JSON Schema: rich text, images, file attachments, nested JSON, and a
type selector.

**Actions** — browse and edit actions, run them synchronously, or open the
**ActionRunner** for a detached run: `start` returns immediately, status is
long-polled to a terminal state, console output streams in a parallel `tail`
loop, and `stop` cancels — cooperatively first, force-abortable on a second
click. Closing the dialog stops watching without stopping the invocation.

**Types** — browse and edit registered types with a JSON Schema editor.

**Files** — browse the file store by prefix, preview, and edit.

All four tabs are live. Everything is filtered, sorted, and paginated in the
database rather than in the browser.

## Prerequisites

- [Bun](https://bun.sh) v1.1+
- A running `solx-server` (`cargo run -p solx-server` in `../solx-core`,
  default `http://127.0.0.1:8766`)
- A checkout of the sibling [solx-js](../solx-js) repo

`@solx/http` and `@solx/surface` aren't npm dependencies here — they're
aliased straight to their TypeScript source in `../solx-js` (see
`web/vite.config.ts`'s `resolve.alias` and `web/tsconfig.json`'s
`compilerOptions.paths`). Both are zero-dependency and browser-safe, so
there's no install or build step in that repo for solx-web's sake. The
sibling checkout has to be at `../solx-js` relative to this repo for the
aliases to resolve.

## Setup

```sh
cd web && bun install
bun run dev          # http://localhost:5173
```

Enter the server URL and bearer token in the app's Settings panel (⚙,
bottom-left); they're stored in `localStorage`. The token is `server_token`
in `solx-config.json` under your solx appdata directory — `solx-server`
generates it on first start and prints only its listen address, not the
token.

## Build

```sh
cd web && bun run build     # → web/dist/, a static site
```

No backend process to run alongside it.

## Not yet

Widgets — solx-core can serve a widget bundle and a websocket for a
frontend to mount (`/builtin/widget/*`), but solx-web doesn't mount them yet.

## Docs

[docs/](docs/) holds the design and roadmap:
[implementation-plan-v2.md](docs/implementation-plan-v2.md) is the current
one, and
[solx-web-document-editor.md](docs/solx-web-document-editor.md) covers the
schema-driven editor.

## License

MIT OR Apache-2.0.
