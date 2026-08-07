# solx-js — Architecture

Node.js / TypeScript surface for [`solx-core`](https://github.com/...) — the lean
reimplementation of `sol`. This document is the canonical plan for the repo;
older research notes (`solx-node.md`) are superseded.

---

## 1. Goals

1. **Surface every public API of `solx-core`** in TypeScript with strict types —
   the SDK must be the obvious place to look for "what can solx do?".
2. **In-process** by default. No separate server, no extra process, no HTTP
   round-trip. `import { createSolx } from 'solx'` and you have a working
   solx instance in the same process as your script.
3. **Stay decoupled from `solx-core`.** `solx-core` (the Rust repo) must
   never depend on this repo, and this repo must never fork or copy
   `solx-core` source. Communication is via the published `solx-*` Rust
   crates, consumed through a Neon binding layer.
4. **Cross-platform prebuilt binaries** so end users do not need a Rust
   toolchain, a C++ compiler, or even `node-gyp` to `npm install solx`.
5. **Tree-shakeable subpath imports** (`solx/types`, `solx/actions`, ...)
   so bundlers can drop unused crates.

Out of scope for v1: an HTTP transport, a CLI, browser-side bindings
(solx's WASM crate is itself deferred in `solx-core`), and `solx-packages`
(it depends on `solx-scripts` over a shell boundary, which is awkward to
expose in-process — punt to v2).

---

## 2. Repository layout

One repo, one Cargo workspace, one npm workspace, one monorepo.

```
solx-js/
├── ARCHITECTURE.md              ← you are here
├── README.md
├── LICENSE
├── package.json                 ← npm workspace root
├── tsconfig.base.json           ← shared TS config (declaration, strict, ES2022)
├── Cargo.toml                   ← Rust workspace
│
├── crates/                      ← ONE binding crate (solx-bindings) whose
│   │                              internal modules mirror the solx-core
│   │                              crates. This is the idiomatic Neon
│   │                              pattern — register_module! lives in
│   │                              exactly one place.
│   └── solx-bindings/
│       ├── Cargo.toml           (cdylib, depends on all 7 solx-core crates)
│       └── src/
│           ├── lib.rs           (register_module! — the only entry point)
           ├── types.rs         (TypeManager → Arc<dyn TypeManager>)
           ├── config.rs        (ConfigService — see §3.1)
           ├── files.rs         (FileStore → Arc<dyn FileStore>)
           ├── docs.rs          (DocManager → Arc<dyn DocManager>)
           ├── actions.rs       (ActionManager → Arc<dyn ActionManager>)
│           ├── scripts.rs       (ScriptRunner binding)
│           ├── surface.rs       (DTOs, errors, path helpers — mostly
│           │                      re-exports from solx-surface, but some
│           │                      pure-function bindings like normalize_path)
│           └── async_runtime.rs (per-manager tokio runtime + cx.promise()
│                                  + deferred.settle_with plumbing)
│
├── packages/                    ← TypeScript SDK, one folder per solx crate
│   ├── sdk/                     ← the `solx` umbrella package (published)
│   ├── types/                   ← re-exports under the `solx/types` subpath
│   ├── config/
│   ├── files/
│   ├── docs/
│   ├── actions/
│   ├── scripts/
│   └── surface/                 ← shared types/errors, re-exported everywhere
│
├── platform-packages/           ← optional per-platform Neon binary packages
│   ├── solx-darwin-arm64/
│   ├── solx-darwin-x64/
│   ├── solx-linux-x64-gnu/
│   ├── solx-linux-x64-musl/
│   ├── solx-linux-arm64-gnu/
│   ├── solx-linux-arm64-musl/
│   ├── solx-win32-x64-msvc/
│   └── solx-win32-arm64-msvc/
│
└── .github/
    └── workflows/
        ├── ci.yml               ← build + test on every push
        └── release.yml          ← on tag: cross-compile all 8 targets, publish
```

### Why **one** Neon crate with modules, not seven crates

Neon's `register_module!` macro lives in **exactly one** place per
binary. You can't compose seven sub-crates' `register_module!` into a
single `.node` — each one wants to be the entry point. (You can call
`cx.export_function` from a `pub mod` of the umbrella crate to get the
same module-level namespacing as if you had separate crates.)

So the binding layer is **one** `solx-bindings` crate whose `lib.rs`
declares `pub mod types; pub mod config; ...` and then a single
`register_module!` that wires them up:

```rust
// crates/solx-bindings/src/lib.rs
pub mod async_runtime;
pub mod types;
pub mod config;
// ... etc

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    types::register(&mut cx)?;
    config::register(&mut cx)?;
    // ... etc
    Ok(())
}
```

Each submodule exposes a `register(&mut ModuleContext)` that does
`cx.export_function("open", open)` etc. This gives us:

- **One** `optionalDependencies` row per platform in the consumer's
  `package.json`.
- **One** cross-compilation matrix in CI.
- **One** `.node` file in the published tarball per platform.
- **Same internal modularity** as if we had seven crates — the
  per-crate code is isolated in its own file.

---

## 3. Per-crate mapping

| `solx-core` crate | Role | Rust binding crate | TS subpath | JS-side class |
|---|---|---|---|---|
| `solx-surface` | DTOs, errors, traits, path helpers | `solx-bindings-surface` | `solx/surface` | (types only) |
| `solx-config`  | `solx-config.json` service       | `solx-bindings-config`  | `solx/config`  | `ConfigService` |
| `solx-types`   | Type registry (libsql)           | (in `solx-bindings`)    | `solx/types`   | `TypeManager` |
| `solx-files`   | On-disk byte store               | (in `solx-bindings`)    | `solx/files`   | `FileStore` |
| `solx-docs`    | Document store + search          | (in `solx-bindings`)    | `solx/docs`    | `DocManager` |
| `solx-actions` | Action store + execution         | (in `solx-bindings`)    | `solx/actions` | `ActionManager` |
| `solx-scripts` | Shell pipeline parser/executor   | `solx-bindings-scripts` | `solx/scripts` | `ScriptRunner` |
| (skipped)      | `solx-packages`                  | —                       | —              | v2: thin wrapper |

The TS facade mirrors `solx-surface`'s `Solx` aggregate trait:

```ts
// packages/sdk/src/index.ts
export interface Solx {
  types: TypeManager;
  config: ConfigService;
  files: FileStore;
  docs: DocManager;
  actions: ActionManager;
  scripts: ScriptRunner;
}

export function createSolx(opts?: { appdata?: string }): Promise<Solx>;
```

The factory wires the concrete Neon-backed `Local*` classes the same way
`solx-cli/src/main.rs` wires its `LocalSolManager` — it opens each store
against the configured `appdata_dir`, passes the cross-store dependencies
(`TypeManager` into docs/actions; `ConfigService` into types/files/docs/
actions; etc.), and returns the aggregate.

**Naming:** the public classes are just `FileStore`, `TypeManager`,
`ConfigService`, `DocManager`, `ActionManager`, `ScriptRunner` — not
`LocalFileStore`, `LocalTypeManager`, etc. "Local" is an implementation
detail: the same class shape wraps both the in-process libsql/on-disk
impls (today, via Cargo `local` feature) and a future client-backed
binding (via `client`). Consumers should not branch on which one they
got. The Rust `Local*` impls (`LocalTypeManager`, `LocalFileStore`,
`LocalDocManager`, `LocalActionManager`, `LocalSolManager`) stay
internal — they're constructed inside the binding, never named in the
TS or umbrella surface.

### 3.1 Trait seam — preparing for a future `solx-client`

`solx-surface` already exposes `FileStore`, `TypeManager`, `DocManager`,
`ActionManager` as object-safe traits so a future `solx-client` can
implement them as HTTP proxies. The Neon binding layers hold those as
`Arc<dyn Trait>` rather than `Arc<Local*>`:

```rust
// crates/solx-bindings/src/files.rs
pub struct JsFileStore { inner: Arc<dyn FileStore> }
pub struct JsTypeManager { inner: Arc<dyn TypeManager> }
```

Construction is the only place the concrete `Local*` appears, and it's
gated by Cargo features so the swap is mechanical:

```rust
#[cfg(feature = "local")] {
    let local = LocalFileStore::new(PathBuf::from(root));
    Arc::new(local) as Arc<dyn FileStore>
}
#[cfg(not(feature = "local"))] {
    // future: construct an HTTP-backed proxy here
}
```

`Cargo.toml` exposes:

```toml
[features]
default = ["local"]
local = []
# client = ["dep:solx-client"]    # once it lands
client = []
```

The JS-side `FileStore` constructor captures the path
(`new FileStore(handle, root)`) rather than calling back into the
binding for `root()` — the trait doesn't expose `root()` and we don't
want to downcast. `ConfigService` is the canonical source for derived
paths (`typesDbPath`, `filesDir`, `docsDbPath`, `actionsDbPath`,
`logsDir`, …); other stores consume those via the `Solx` aggregate.

`ConfigService` itself doesn't have a `solx-surface` trait yet — the
binding still holds `Arc<solx_config::ConfigService>` concretely with
a `TODO` comment explaining the migration path. Adding the trait
later is a `solx-core` change; the binding swap is then the same
one-liner as for the other managers.

---

## 4. Type surface — how Rust types become TS types

`solx-core` currently has no `specta::Type` or `ts-rs` derives. We have
two options; v1 picks the simpler one:

### v1: Hand-written `types.ts` mirroring the Rust DTOs

For each `solx-surface` entity / query / error type, write a hand-authored
`interface` / `type` / `enum` in `packages/surface/src/types.ts` that
matches the serde name, field order, and `Option/Vec` shape. The shape
is small (~12 DTOs) and stable. Validation is by comment + matching
`tsc` compile against the published bindings.

Pros: zero Rust-side changes, zero codegen toolchain, types are
readable and reviewable as plain TS.

Cons: types can drift from Rust. Mitigated by snapshot tests in
`packages/surface/test/types.test.ts` that round-trip a fixture through
the Neon boundary and assert shape.

### v2: Add `specta::Type` derives to `solx-surface` + codegen

If drift becomes a maintenance burden, add `specta` to `solx-core`'s
workspace deps, derive `Type` on the DTOs, and run `specta::type_defs`
from a build script to emit `types.ts`. Out of scope for v1.

### ID and timestamp representation

| Rust | TS | Notes |
|---|---|---|
| `uuid::Uuid` | `string` (canonical hyphenated form) | Same as `sol-browser`'s convention. |
| `chrono::DateTime<Utc>` | `string` (RFC 3339 / ISO 8601) | Same as `sol-browser`. |
| `serde_json::Value` | `unknown` (or `JsonValue` discriminated union) | We do **not** type-erase into a `any`; we re-export `JsonValue` from `solx/surface` as a tagged union (`null | boolean | number | string | JsonValue[] | { [k: string]: JsonValue }`). |
| `SolxError` | `SolxError` class (extends `Error`) with `.kind` discriminator | see §6 |
| `Result<T, SolxError>` | `Promise<T>` (rejection = `SolxError`) | All manager methods are `async` so the Rust `Result` becomes a JS `Promise` rejection. |
| `Arc<dyn Trait>` | the concrete class instance; user code depends on the trait interface | e.g. `docs: LocalDocManager` but the SDK re-exports the interface as `DocManager`. |

---

## 5. The Neon binding layer

### One binding crate per solx-core crate

`crates/solx-bindings-types/Cargo.toml`:

```toml
[dependencies]
solx-types    = { path = ".../solx-core/solx-types" }   # or git/registry
solx-surface  = { path = ".../solx-core/solx-surface" }
neon          = { version = "1", features = ["napi-6"] }
```

`crates/solx-bindings-types/src/lib.rs` (sketch):

```rust
use neon::prelude::*;
use solx_surface::Result;
use solx_types::LocalTypeManager;
use std::path::PathBuf;
use std::sync::Arc;

pub struct JsTypeManager(pub Arc<LocalTypeManager>);

fn open(mut cx: FunctionContext) -> JsResult<JsBox<JsTypeManager>> {
    let path = cx.argument::<JsString>(0)?.value(&mut cx);
    let rt = tokio::runtime::Runtime::new()
        .or_else(|e| cx.throw_error(e.to_string()))?;
    let inner = rt.block_on(LocalTypeManager::open(&PathBuf::from(path)))
        .or_else(|e| cx.throw_error(format!("{:?}", e)))?;
    Ok(cx.boxed(JsTypeManager(Arc::new(inner))))
}

fn post(mut cx: FunctionContext) -> JsResult<JsPromise> {
    let mgr = cx.argument::<JsBox<JsTypeManager>>(0)?;
    let path = cx.argument::<JsString>(1)?.value(&mut cx);
    let name = cx.argument::<JsString>(2)?.value(&mut cx);
    let input_json = cx.argument::<JsString>(3)?.value(&mut cx);
    let input: solx_surface::TypeInput = serde_json::from_str(&input_json)
        .or_else(|e| cx.throw_error(e.to_string()))?;
    let rt = tokio::runtime::Runtime::new()
        .or_else(|e| cx.throw_error(e.to_string()))?;
    let mgr = mgr.0.clone();
    let channel = cx.channel();
    let (deferred, promise) = cx.promise();
    std::thread::spawn(move || {
        let result = rt.block_on(mgr.post(&path, &name, input));
        deferred.settle_with(&channel, move |mut cx| match result {
            Ok(v)  => Ok(cx.string(serde_json::to_string(&v).unwrap())),
            Err(e) => cx.throw_error(format!("{:?}", e)),
        });
    });
    Ok(promise)
}

// ... get, delete, list, resolve, validate ...

#[neon::export]
pub fn __register_types_module() { /* hook for the umbrella */ }
```

The umbrella `crates/solx-bindings/` crate composes the per-crate modules:

```rust
// crates/solx-bindings/src/lib.rs
#[neon::export]
mod surface  { pub use solx_bindings_surface::*; }
#[neon::export]
mod config   { pub use solx_bindings_config::*; }
#[neon::export]
mod types    { pub use solx_bindings_types::*; }
#[neon::export]
mod files    { pub use solx_bindings_files::*; }
#[neon::export]
mod docs     { pub use solx_bindings_docs::*; }
#[neon::export]
mod actions  { pub use solx_bindings_actions::*; }
#[neon::export]
mod scripts  { pub use solx_bindings_scripts::*; }
```

This compiles to **one** `solx.node` file that exposes seven JS modules
under one handle. The TS side uses `require('solx.node').types.open(...)`.

### Async via blocking-on-tokio

`solx-core` is `async`. Neon callbacks are sync. The pattern is:

1. Spawn a worker thread that owns a `tokio::runtime::Runtime`.
2. Use `cx.promise()` + `cx.channel()` to get a `JsPromise` and a
   `Channel` for crossing back to the JS thread.
3. `rt.block_on(future)` inside the worker, then `deferred.settle_with`
   on the channel.

This is the standard Neon async recipe; see `neon/examples/threading`.
Each `Local*` Neon crate gets its own long-lived worker thread +
runtime, created on `open()` and kept alive for the process lifetime
(neon `#[neon::main]` works too, but a per-class runtime gives us
cleaner teardown if we ever want it).

### Pulling in `solx-core` — path or git?

For local development: `solx-core = { path = "../solx-core/solx-types" }`
in each binding crate's `Cargo.toml`. This is what the v1 scaffold uses
— it lets the two repos evolve together without a release dance.

For the published `solx` npm package: a `build.rs` in each binding
crate (or in the umbrella) clones the matching `solx-core` crate at
the version pinned in `solx-core-version.txt` and points Cargo at it.
This keeps the published `solx.node` reproducible and the npm tarball
self-contained (no `path = "..."` survives into the published crate).

Git-dependency is the v2 follow-up once `solx-core` has published
versions on crates.io / a git tag convention.

---

## 6. Errors

`solx-surface::SolxError` becomes a `SolxError` class:

```ts
// packages/surface/src/error.ts
export type SolxErrorKind =
  | 'NotFound' | 'Invalid' | 'Validation' | 'Conflict'
  | 'Io'      | 'Db'      | 'Exec'       | 'Config' | 'Other';

export class SolxError extends Error {
  readonly kind: SolxErrorKind;
  readonly detail: string;
  constructor(kind: SolxErrorKind, detail: string) {
    super(`${kind}: ${detail}`);
    this.name = 'SolxError';
    this.kind = kind;
    this.detail = detail;
  }
}
```

On the Neon side, every `Result::Err(e)` is converted to a
`JsError` with the stringified discriminant in the message; the TS
loader parses the prefix and constructs a real `SolxError`. Format:

```
SolxError::NotFound: type not found: /x/y
```

The TS wrapper has a `fromNeon(e: unknown): SolxError` helper used at
every binding boundary.

---

## 7. Cross-platform binaries

Neon produces a N-API binary. N-API is **ABI-stable across Node.js
versions**, so we only need one build per `(target-triple, libc)` pair.
The `napi-rs` ecosystem calls these "napi triples":

| Triple | OS | Arch | Libc | Runner |
|---|---|---|---|---|
| `darwin-arm64`  | macOS  | arm64 | —    | `macos-14` |
| `darwin-x64`    | macOS  | x86_64 | —   | `macos-13` |
| `linux-x64-gnu`   | Linux | x86_64 | glibc | `ubuntu-22.04` |
| `linux-x64-musl`  | Linux | x86_64 | musl | `ubuntu-22.04` + `musl-tools` |
| `linux-arm64-gnu` | Linux | aarch64 | glibc | `ubuntu-22.04` + `gcc-aarch64-linux-gnu` |
| `linux-arm64-musl`| Linux | aarch64 | musl | cross-rs `aarch64-unknown-linux-musl` |
| `win32-x64-msvc`  | Windows | x86_64 | msvc | `windows-2022` |
| `win32-arm64-msvc`| Windows | aarch64 | msvc | `windows-11-arm` |

For each triple we publish a tiny npm package (`@solx/darwin-arm64`, etc.)
that contains the prebuilt `solx.node`. The umbrella `solx` package
declares them all as `optionalDependencies`; npm picks the right one at
install time.

GitHub Actions matrix in `.github/workflows/release.yml` builds all 8
on tag push; `ci.yml` builds `linux-x64-gnu` and `darwin-arm64` on PR
(fast feedback) plus `win32-x64-msvc` (most divergent target).

We use **`@neon-rs/neon` + `cargo-cp-artifact`** for the build,
**`napi-rs/cli` for `napi artifacts`** to produce platform npm packages,
and **`@npmcli/package-json` + `changesets`** for versioning. (No
`@neon-rs/cli` directly because its `npmrc` flow is opinionated; the
hand-rolled napi artifacts step is ~30 lines and easier to debug.)

---

## 8. Build, test, release

```
bun install
bun run build           # tsc on all packages; copies .node into dist
bun run test            # vitest across all packages
bun run typecheck       # tsc --noEmit

# Cut a release:
bun run version         # changesets: pick which packages bump
git tag v0.1.0 && git push --tags
# → GH Action cross-compiles 8 targets, publishes 8 platform pkgs
#   + the umbrella pkg to npm
```

`Cargo.lock` is committed; the binding crates have no third-party
release coupling, so a `cargo update -p solx-core` is the only reason
the lockfile should change.

---

## 9. What the consumer experience looks like

```ts
import { createSolx } from 'solx';

const solx = await createSolx();                 // uses ~/.praeus/solx
const types = solx.types;

// post a type
await types.post('/types/custom', 'Person', {
  schema: {
    type: 'object',
    required: ['name'],
    properties: { name: { type: 'string' } },
  },
});

// post a doc validated against it
await solx.docs.post('/research', 'note', {
  typeRef: '/types/custom/Person',
  contents: { name: 'Ada' },
  title: 'AI note',
});

// list + search
const page = await solx.docs.list({ pathPrefix: '/research', limit: 50 });
const hits = await solx.docs.search({ q: 'Ada', pathPrefix: '/research' });

// execute a Command action
const result = await solx.actions.exec('/actions', 'echo', { msg: 'hi' });
```

No HTTP, no extra process, no compile step. Single `npm install solx`.

---

## 10. Decisions log

| Decision | Choice | Why |
|---|---|---|
| N-API vs WASM | N-API (Neon) | Cheaper perf, full Node API access, no JS↔WASM marshaling tax. |
| One `.node` vs seven | One umbrella `solx.node` | Fewer platform packages, simpler `optionalDependencies`, single CI matrix. |
| One npm pkg vs seven | Umbrella `solx` with subpath exports | Tree-shakeable without 14 npm packages. |
| Specta codegen | Skip for v1, hand-write `types.ts` | 12 DTOs, stable; codegen adds a build dep to `solx-core` for marginal benefit. |
| HTTP transport | Deferred to v2 | `solx-core` has no `solx-server` yet; in-process covers the v1 use cases. |
| `solx-packages` | Deferred to v2 | Depends on shell-boundary script execution; awkward in-process. |
| Cargo dep on `solx-core` | `path` for dev, pinned-clone for publish | Lets the two repos evolve together; keeps npm tarball self-contained. |
| Versioning | changesets, single umbrella | One consumer-facing version number matches the `solx` umbrella; the platform packages are version-locked to it. |

---

## 11. References

- [solx-core docs/design-and-progress.md](https://github.com/.../solx-core/blob/main/docs/design-and-progress.md) — the upstream design this SDK implements.
- `solx-core/solx-surface/src/managers.rs` — the `Solx` aggregate trait
  we mirror as the `createSolx` factory.
- `solx-core/solx-surface/src/rpc.rs` (in the larger `sol` repo) — wire
  format we will reuse when an HTTP transport lands in v2.
- The 3-tier plan this doc replaces: `solx-node.md` (kept for
  archaeology; do not edit).
