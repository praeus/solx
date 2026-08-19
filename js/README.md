# solx-js

TypeScript surface for [solx-core](../solx-core). Two ways in, one interface:
in-process Neon bindings for Node, and a zero-dependency `fetch` client for
the browser. Both implement the same manager contract as the Rust
`solx-surface` traits.

```ts
import { createSolx } from 'solx';

const solx = await createSolx();                 // uses ~/.praeus/solx
await solx.types.save('/types/custom', 'Person', { schema: { /* ... */ } });
await solx.docs.save('/research/ai', 'note', { type: '/types/custom/Person', contents: { name: 'Ada' } });
const result = await solx.actions.exec('/builtin/document', 'search_documents', { query: 'Ada' });
```

## Local and remote are the same call

Every manager except `config` has two constructors — `open()` (in-process,
libsql + Tantivy through Neon) and `connect(serverUrl, token)` (HTTP against
`solx-server`). `createSolx` picks one for all of them, once, using the same
precedence as `solx-manager::App` in Rust:

1. `opts.serverUrl` / `opts.serverToken`
2. `SOLX_SERVER_URL` / `SOLX_SERVER_TOKEN`
3. `serverUrl` / `serverToken` from `solx-config.json`

`config` is always local — it's what tells the other managers which mode to
use, so it can't itself be the proxy that decides whether to be remote.
Nothing above the constructor changes between modes.

## Packages

| Package | What it is |
|---|---|
| `solx`          | The umbrella. `createSolx` plus re-exports of everything below, with subpath exports (`solx/types`, `solx/docs`, …) for tree-shaking. |
| `@solx/surface` | Shared DTOs, `SolxError`, path helpers. Zero-dependency and browser-safe. |
| `@solx/types`   | Type registry — JSON-schema types by path, validation. |
| `@solx/files`   | On-disk byte store. |
| `@solx/docs`    | Document store with full-text and faceted search. |
| `@solx/actions` | Action store and execution. |
| `@solx/scripts` | Script runner, bound to an already-open `ActionManager`. |
| `@solx/config`  | Local config service (sync; always local). |
| `@solx/http`    | Pure-`fetch` client for `solx-server`. Zero-dependency, browser-safe, no Neon — this is what a web frontend uses. |

`@solx/http` is the reason [solx-web](../solx-web) needs no backend of its
own: it implements the same manager interfaces in plain TypeScript, so a
browser talks to `solx-server` directly.

## Status

Every manager is end-to-end working — build, run, tests pass — in both local
and client mode. `@solx/scripts` binds to an already-open `ActionManager`
rather than having its own constructors, since there's no `ScriptRunner`
trait upstream.

Not done yet:

- **Per-platform binaries.** Only `solx-win32-x64-msvc` is staged. The other
  seven targets (darwin arm64/x64, linux x64/arm64 × gnu/musl, win32-arm64)
  need a CI matrix before this is installable without a Rust toolchain.
- **Nothing is published to npm.** All packages are `private: true` at
  version `0.0.0`.
- **TS DTOs are hand-written** to mirror the Rust `serde` shape, with
  snake_case ↔ camelCase conversion in TS. Specta codegen is the planned
  replacement; deferred until a DTO actually changes.

## Layout

```
crates/solx-bindings/   Rust Neon binding — one crate, a module per solx-core
                        crate, composed into a single solx.node
packages/               TypeScript SDK (see the table above)
platform-packages/      Per-platform prebuilt .node packages (1 of 8 staged)
docs/                   Design, status, and remaining work
```

## Develop

```sh
bun install

# Build the native binary (requires Rust + a C++ toolchain)
cargo build -p solx-bindings
cp target/debug/solx_bindings.dll target/debug/solx.node
cp target/debug/solx.node platform-packages/solx-win32-x64-msvc/solx.node

bun run typecheck
bun run build

# Tests skip themselves if SOLX_NATIVE_BIN is unset
SOLX_NATIVE_BIN="$(pwd)/platform-packages/solx-win32-x64-msvc/solx.node" \
SOLX_NODE_PATH="$(pwd)/platform-packages/solx-win32-x64-msvc/solx.node" \
  bun run test
```

The binding also builds standalone in either mode:
`cargo build -p solx-bindings --no-default-features --features local` (or
`--features client`).

The remote-mode tests run against an in-test mock HTTP server speaking the
same wire contract as `solx-server` — bearer auth plus the
`solx-surface::wire` DTOs — so the `Remote*` path is verified end-to-end
through Neon without building the real server binary.

## Docs

[docs/design-and-progress.md](docs/design-and-progress.md) is the full
design, status, and decisions log.
[ARCHITECTURE.md](ARCHITECTURE.md) has the crate-by-crate detail.

## License

MIT OR Apache-2.0.
