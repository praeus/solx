# solx-web

A web-based document browser UI for the [solx-core](../solx-core) system. Built on
the [solx-js](../solx-js) TypeScript SDK and the running `solx-server`.

## Architecture

```
Browser (React + Vite :5173)
  └─▶ Vite dev proxy (/api)
      └─▶ Bun backend (:3001)
          └─▶ solx-js SDK (createSolx)
              └─▶ solx-server (:8766)
```

- **`web/`** — React + Vite frontend. Document browser UI with tabbed entity
  views, search, facets, settings panel, and create dialog.
- **`server/`** — Bun HTTP backend. Loads `solx-js`, connects to `solx-server`,
  exposes a clean REST API for the frontend.

## Prerequisites

- [Bun](https://bun.sh) v1.1 or later
- A running `solx-server` (defaults to `http://127.0.0.1:8766`)

## Setup

```bash
# Install root tooling (concurrently, typescript)
bun install

# Vendor the solx-js packages from ../solx-js and install them
bun run vendor
cd server && bun install
cd ../web && bun install
```

> The first time, or whenever the native `solx.node` changes, build it from
> `../solx-js/crates/solx-bindings` and copy the output to
> `server/vendor/solx-win32-x64-msvc/solx.node`. The server expects
> `SOLX_NATIVE_BIN` to be set to that file when it boots.

# Configure solx-server connection (optional, defaults shown)
export SOLX_SERVER_URL=http://127.0.0.1:8766
export SOLX_SERVER_TOKEN=<your-token>
```

## Development

```bash
# bash / Git Bash
SOLX_NATIVE_BIN="$(pwd)/server/vendor/solx-win32-x64-msvc/solx.node" bun run dev
```

```powershell
# PowerShell
$env:SOLX_NATIVE_BIN = "$pwd\server\vendor\solx-win32-x64-msvc\solx.node"; bun run dev
```

Open <http://localhost:5173> in your browser.

## Build

```bash
bun run build       # Build the React frontend
```

The Bun backend serves the built frontend from `web/dist/` in production mode.

## Status

Initial implementation focuses on the **Documents** tab with:
- Path-prefix browsing (when search text starts with `/`)
- Full-text search with type/path facets
- Document create dialog
- Settings panel for server connection configuration

Actions, Types, and Files tabs are rendered as placeholders.

See `docs/implementation-plan.md` for the full design and roadmap.
