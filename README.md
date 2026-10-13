# solx

**solx turns a database of actions into a live MCP/REST server.** One searchable catalogue of tools — for you and your AI agents. See [core/README.md](core/README.md) for the full introduction.

## Layout

| Folder | What it is | Builds |
| --- | --- | --- |
| [core/](core) | The Rust workspace: managers, CLI, MCP and HTTP server, Tauri plugin | `solx` (CLI), `solx-server` |
| [js/](js) | TypeScript surface: `@solx/*` packages (including the `@solx/widgets` toolkit) and the Neon bindings | npm packages |
| [web/](web) | The browser UI, talking to `solx-server` via `@solx/http` | static `web/web/dist` |
| [browser/](browser) | The Tauri desktop app wrapping `web/`, as a local server or a thin client | server and client installers |

The folders reference each other by relative path (`browser/src-tauri` → `core/`, `web/` → `js/`), so everything builds from this checkout alone, without publishing anything.

These were previously the separate `solx-core`, `solx-js`, `solx-web` and `solx-explorer` repositories, plus `solx-widgets` from [solx-packages](https://github.com/praeus/solx-packages); their full history was imported here.
