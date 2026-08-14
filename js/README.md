# solx-js

TypeScript / Node.js surface for [`solx-core`](../solx-core) — in-process
Neon bindings + a tree-shakeable SDK.

```ts
import { createSolx } from 'solx';

const solx = await createSolx();                 // uses ~/.praeus/solx
await solx.types.save('/types/custom', 'Person', { schema: { /* ... */ } });
await solx.config.set('greeting', 'hello');
await solx.files.put('files/docs/x/hello.txt', new TextEncoder().encode('hi'));
```

See [docs/design-and-progress.md](./docs/design-and-progress.md) for
the full design, status, and remaining work, and
[ARCHITECTURE.md](./ARCHITECTURE.md) for the crate-by-crate detail.

## Status

**Reference implementation in place.** Three of seven crates are
end-to-end working (build, run, all tests pass):

- `solx-types`   — TypeManager (libsql)
- `solx-files`   — FileStore (on-disk)
- `solx-config`  — ConfigService (sync; opens first, drives the others)

The remaining crates (`solx-docs`, `solx-actions`, `solx-scripts`) are
scaffold-only — calling their methods throws a clear "not yet
implemented" error. The TS surface and Rust binding shape are correct;
the implementation follows crate-by-crate (see §6 of the design doc).

## Layout

```
crates/                ← Rust Neon binding (one crate, modules per solx-core crate,
                          composed into a single solx.node binary)
packages/              ← TypeScript SDK (surface, types, files, config, sdk)
platform-packages/     ← per-platform prebuilt .node packages (1 staged, 7 to go)
docs/                  ← design + status + remaining work
```

## Develop

```sh
# One-time: install deps + link workspace packages
bun install

# Build the native binary (requires Rust + a C++ toolchain)
cargo build -p solx-bindings
cp target/debug/solx_bindings.dll target/debug/solx.node
cp target/debug/solx.node platform-packages/solx-win32-x64-msvc/solx.node

# Typecheck + build the TS side
bun run typecheck
bun run build

# Run the test suite (skips if SOLX_NATIVE_BIN is not set)
SOLX_NATIVE_BIN="$(pwd)/platform-packages/solx-win32-x64-msvc/solx.node" \
SOLX_NODE_PATH="$(pwd)/platform-packages/solx-win32-x64-msvc/solx.node" \
  bun run test
```

End users install via the per-platform npm packages (no Rust needed).
