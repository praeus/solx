# solx-web

A web-based document browser UI for the [solx-core](../solx-core) system. The
React frontend talks directly to `solx-server` over HTTP via `@solx/http`
(from the sibling [solx-js](../solx-js) repo) — there is no backend of its
own.

## Architecture

```
Browser (React + Vite :5173)
  └─▶ @solx/http (fetch-based, aliased straight to solx-js source)
      └─▶ solx-server (:8766, CORS-enabled)
```

`web/` used to go through a Bun backend (`server/`) that loaded solx-js's
Node-native binding and re-exposed it as a REST API — a second hop for what
`solx-client`'s Rust `Remote*` implementations already do in one. `@solx/http`
is a pure-TypeScript, browser-safe implementation of the same manager
interfaces (`solx-surface::managers`), so the frontend now calls
`solx-server` directly and that hop, and the whole `server/` app, is gone.

`@solx/http`/`@solx/surface` aren't npm dependencies of `web/` — they're
aliased straight to their TS source in the sibling `solx-js` repo (see
`web/vite.config.ts`'s `resolve.alias` and `web/tsconfig.json`'s
`compilerOptions.paths`). Both packages are zero-dependency and
browser-safe as-is, so there's no build step to keep in sync.

## Prerequisites

- [Bun](https://bun.sh) v1.1 or later
- A running `solx-server` (defaults to `http://127.0.0.1:8766`), built from
  the sibling `../solx-core` repo
- A checkout of the sibling `../solx-js` repo (only `packages/http` and
  `packages/surface` are used, via the alias above — no install/build
  needed there for `solx-web` specifically)

## Setup

```bash
cd web && bun install
```

The server URL and bearer token are entered in the app's Settings panel
(⚙, bottom-left) and stored in the browser's `localStorage` — `solx-server`
prints its token to stdout on first start.

## Development

```bash
cd web && bun run dev
```

Open <http://localhost:5173> in your browser. Make sure `solx-server` is
running first (`cargo run -p solx-server` from `solx-core`).

## Build

```bash
cd web && bun run build       # Build the React frontend to web/dist/
```

`web/dist/` is a static site — serve it from anywhere; there's no backend
process to run alongside it.

## Status

Initial implementation focuses on the **Documents** tab with:
- Path-prefix browsing (when search text starts with `/`)
- Full-text search with type/path facets
- Document create dialog
- Settings panel for server connection configuration

Actions, Types, and Files tabs are rendered as placeholders.

See `docs/implementation-plan.md` for the full design and roadmap.
