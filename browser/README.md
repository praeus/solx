# solx-explorer

The solx desktop app: the [solx-web](../solx-web) UI in a Tauri v2 window,
talking to solx over Tauri IPC (`@solx/tauri` → `solx-tauri-plugin`) rather
than HTTP. One crate, two builds:

| Build | Cargo features | What's inside |
| --- | --- | --- |
| **server** (`solx`) | `server` (default) | All of solx-core with local managers. The UI uses them over IPC; unless `server_embedded` is `false` in `solx-config.json`, an embedded solx-server serves the same `App` on `server_host:server_port` (default `127.0.0.1:8766`) for browsers, solx-cli and desktop clients. |
| **client** (`solx-client`) | `client` | Only `solx-manager`'s remote wiring (`solx-client`), with no wasmtime/libsql/keyring. Enter a solx-server URL and token in Settings; they are verified, then stored in this app's own config dir, not the shared solx appdata. |

If the server build finds a solx host already answering on its port at
startup (a standalone `solx-server`, or a `solx-cli` session), it connects
to that host instead of opening the same databases a second time.

## Layout

This repo has no frontend of its own. It expects its siblings next to it:

```
Projects/
  solx-core/      # Rust crates, incl. solx-tauri-plugin (path deps)
  solx-js/        # @solx/tauri, @solx/http, @solx/surface (aliased by solx-web)
  solx-web/web/   # the UI: devUrl http://localhost:5173, frontendDist web/dist
  solx-explorer/  # this repo
```

## Develop and build

Needs pnpm, bun (for solx-web), and the Rust/Tauri prerequisites.

```sh
pnpm install
pnpm dev            # server build
pnpm dev:client     # client build
pnpm build          # server installer (org.praeus.solx)
pnpm build:client   # client installer (org.praeus.solx-client)
```

The client scripts merge `src-tauri/tauri.client.conf.json` (product name,
identifier, binary name) and pass `--no-default-features` through to cargo,
so both apps can be installed side by side.

## Notes

- Webview access is granted by the `solx:default` permission in
  `src-tauri/capabilities/default.json`.
- solx-web loads widget bundles and files from `blob:` URLs. `csp` is
  `null` today; any CSP you add must allow `blob:` in `script-src` and
  `img-src`.
- Command actions that call back into solx-server over HTTP (e.g.
  solx-quickjs builds) need the embedded server, so keep `server_embedded`
  on unless you know you don't use them.
