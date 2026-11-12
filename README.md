# solx

**solx turns a database of actions into a live MCP/REST server.** One searchable catalogue of tools — for you and your AI agents. See [core/README.md](core/README.md) for the full introduction.

## Quick start

The desktop app — the **browser** — is the fastest way to a working solx: it wraps the web UI in a Tauri window and talks to solx over IPC, so there is no server to start alongside it.

Needs [Rust](https://tauri.app/start/prerequisites/) (stable, plus your platform's Tauri system dependencies), Node, [pnpm](https://pnpm.io) and [bun](https://bun.sh).

```sh
pnpm install --dir browser     # the Tauri CLI
bun install --cwd web/web      # the web UI the app bundles

bun --cwd browser tauri build  # → browser/src-tauri/target/release/bundle/
```

`tauri build` compiles the Rust crates it depends on — `core/` is a path
dependency of `browser/src-tauri`, so nothing needs publishing first. The
installers land in `browser/src-tauri/target/release/bundle/` (`solx_0.1.0_x64_en-US.msi`
and `solx_0.1.0_x64-setup.exe` on Windows; `.dmg`, `.deb`, `.rpm` and `.AppImage`
elsewhere).

Two other targets from the same tree:

```sh
bun --cwd browser tauri dev   # run it against a Vite dev server on :5173
cd core && cargo build --release   # the `solx` CLI and `solx-server`, no app
```

The client build — the same UI, but a thin front end for a solx-server that
already runs elsewhere — swaps in its own config and cargo features:

```sh
bun --cwd browser tauri build --config src-tauri/tauri.client.conf.json \
  --features client -- --no-default-features
```

## Packages

The action bundles — Google Workspace, Ollama, browser control, media
extraction — are **not** in this repository. They live in
[solx-packages](https://github.com/praeus/solx-packages), one directory per
package, each with a `solx-package.json` manifest and an `install.solx` script.
That repo's [HOWTO.md](https://github.com/praeus/solx-packages/blob/main/HOWTO.md)
is the authoring guide.

| Package | What it adds |
| --- | --- |
| `solx-ollama` | Chat, generate and embed with a local Ollama server. |
| `solx-mcp-actions` | Import tools from MCP servers as solx actions. |
| `solx-firefox` | A dedicated, automatable Firefox instance for browsing actions. |
| `solx-quickjs` | Builds JavaScript actions into wasm components. |
| `solx-names` | Random name generator actions. |
| `solx-agent` | A supervised agent thread over the solx action catalogue. |
| `solx-inquiry` | Ask questions answered from your solx documents. |
| `solx-git` | Clone, branch, patch and commit git repositories. |
| `solx-media` | Image, audio and video extraction via Ollama, ffmpeg and whisper. |
| `solx-omniparse` | Document parsing and OCR for extraction. |
| `solx-google` | Docs, Drive, Gmail, Tasks and Calendar actions. |
| `solx-facebook` | Facebook Pages post actions over the Graph API. |
| `solx-reddit` | Reddit extraction and browsing. |
| `solx-livejournal` | LiveJournal extraction. |

Three ways to install one:

- **The app.** Settings → Install lists the catalog and installs by name from
  the latest GitHub release of `praeus/solx-packages`, pulling in whatever a
  package requires first. Only a local desktop host can do this.
- **The CLI**, from a checkout of the packages repo or a release zip:

  ```sh
  solx install-package ../solx-packages/solx-ollama
  solx install-package solx-ollama-0.1.0.zip
  solx list-packages
  solx uninstall-package solx-ollama
  ```

  A release zip already contains the built artifact; a checkout does not, so
  build it first (`scripts/build-all.sh` in the packages repo, or
  `scripts/build-wasm.sh <package>` for a wasm one).

- **A local release folder**, while a release is still being built:
  `SOLX_PACKAGES_SOURCE=<dir of zips>` (or `github:owner/repo@tag`) points the
  catalog at it instead of GitHub.

The set of names the app will install is fixed in
[`core/solx-packages/src/catalog.rs`](core/solx-packages/src/catalog.rs) —
packages are named, never located, so nothing can install arbitrary code.

Installing a package runs its `install.solx` and grants the shell commands and
outbound hosts its manifest declares. There is no signing yet: read the
manifest first. [core/SECURITY.md](core/SECURITY.md) has the full model.

## Layout

| Folder | What it is | Builds |
| --- | --- | --- |
| [core/](core) | The Rust workspace: managers, CLI, MCP and HTTP server, Tauri plugin | `solx` (CLI), `solx-server` |
| [js/](js) | TypeScript surface: `@solx/*` packages (including the `@solx/widgets` toolkit) and the Neon bindings | npm packages |
| [web/](web) | The browser UI, talking to `solx-server` via `@solx/http`; `bun run publish-ui` uploads it so solx-server serves it at `/ui/` | static `web/web/dist` |
| [browser/](browser) | The Tauri desktop app wrapping `web/`, as a local server or a thin client | server and client installers |

The folders reference each other by relative path (`browser/src-tauri` → `core/`, `web/` → `js/`), so everything builds from this checkout alone, without publishing anything.
