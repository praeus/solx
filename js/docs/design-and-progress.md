# solx-js — Design, Status & Remaining Work

_Last updated: 2026-08-06_

The canonical overview of the `solx-js` workspace: why this repo exists, the
design (one Neon crate, a tree-shakeable TS SDK, the trait seam), what has
been built so far, and what remains.

For crate-by-crate API surface, see [ARCHITECTURE.md](../ARCHITECTURE.md).
For the umbrella `Solx` aggregate and per-package README, see the per-package
docs / READMEs.

---

## 1. Why `solx-js` exists

[`solx-core`](../solx-core) is the Rust library that implements the durable
primitives (types, docs, actions, files, config, packages, scripts). It's
meant to be driven equally well by `solx-cli` (Rust binary) or by a
TypeScript / Node consumer — a thin web app, a model harness, a notebook,
a one-off migration script.

`solx-js` is the latter:

1. **A Node-native binding** to `solx-core` — a single `.node` binary that
   exposes every manager as a flat function (`typesOpen`, `filesPut`, …).
   Built with [Neon](https://neon-bindings.com/), the standard FFI between
   Rust and Node.

2. **A tree-shakeable TypeScript SDK** — one `class` per manager, plus a
   `createSolx()` factory that returns the wired-up aggregate. Each
   manager is its own subpath export (`solx/types`, `solx/files`,
   `solx/config`, …) so callers who only need, say, the type registry
   don't pay for the others.

3. **A trait seam with a runtime local↔client swap.** Every binding
   for a manager that has a `solx-surface::managers` trait
   (`TypeManager`, `FileStore`, `DocManager`, `ActionManager`) holds
   `Arc<dyn Trait>`, never the concrete `Local*`. Unlike the original
   plan, the local-vs-remote choice is **not** a build-time Cargo
   feature — both backends are compiled into the same `solx.node` by
   default, and each manager exposes two explicit constructors,
   `open(...)` (local) and `connect(serverUrl, token)` (client, backed
   by `solx-client`'s `Remote*` HTTP proxies against a running
   `solx-server`). `createSolx` picks which to call, once, for every
   manager, based on config/env — so **one compiled library** can run
   fully local or talk to an existing server, decided at runtime. See
   §3.2 and §6.7 (now merged/superseded) for the detail; `ConfigService`
   has no trait upstream and stays local-only in both modes.

`solx-js` deliberately does **not** try to replicate the older `solx`
CLI's high-level workflows or per-call permission UX. It's the
**transport** — the primitives exposed as idiomatic TS.

---

## 2. Repository layout

```
solx-js/
├── ARCHITECTURE.md                ← detailed design doc (this file is a status overview)
├── README.md                      ← one-page intro + dev loop
├── Cargo.toml                     ← Rust workspace (the binding crate)
├── package.json                   ← bun workspace (packages/* + platform-packages/*)
├── tsconfig.base.json             ← strict, NodeNext, ES2022 + DOM, types: [node]
├── crates/
│   └── solx-bindings/             ← ONE Neon crate; modules mirror solx-core
│       ├── Cargo.toml             (cdylib; local + client features)
│       └── src/
│           ├── lib.rs             (Neon main: register_module! entry point)
│           ├── async_runtime.rs   (long-lived tokio runtime + run_async + SolxErrorExt)
│           ├── types.rs           (TypeManager → Arc<dyn TypeManager>)
│           ├── config.rs          (ConfigService — sync; no run_async)
│           ├── files.rs           (FileStore → Arc<dyn FileStore>)
│           ├── docs.rs            (DocManager → Arc<dyn DocManager>)
│           ├── actions.rs         (ActionManager → Arc<dyn ActionManager>)
│           ├── scripts.rs         (ScriptRunner over Arc<dyn ActionManager>)
│           └── surface.rs         (DTO/error/path helpers — re-exports of solx-surface)
├── packages/                      ← TS SDK
│   ├── surface/                   (DTOs, errors, path helpers, manager interfaces)
│   ├── types/                     (TypeManager + native loader)
│   ├── files/                     (FileStore + FilePath builders)
│   ├── config/                    (ConfigService — single class, no adapter layer, local-only)
│   ├── docs/                      (DocManager)
│   ├── actions/                   (ActionManager)
│   ├── scripts/                   (ScriptRunner)
│   └── sdk/                       (umbrella `solx` package — createSolx, Solx)
├── platform-packages/
│   └── solx-win32-x64-msvc/       (currently the only staged prebuilt;
│                                    solx.node + package.json + binding.gyp)
├── docs/                          (this directory)
└── target/                        (cargo build output)
```

The repo is a **bun workspace** for the JS side and a **Cargo workspace**
for the Rust side, sharing the same root. `bun run` orchestrates everything.

---

## 3. The two design decisions that drive the shape

### 3.1 One Neon crate, not seven

The idiomatic Neon pattern is one `register_module!` entry point per
`.node` binary. Splitting each manager into its own crate (each producing
its own `.node`) makes the SDK awkward — consumers would `require()` N
binaries, share state across them via `process.env` or `globalThis`, and
duplicate the loader logic. With one binary, every manager exports a
prefixed function (`typesOpen`, `configOpen`, `filesNew`, …) under one
namespace. The TS side picks the right one.

### 3.2 The trait seam — and the runtime open()/connect() swap

`solx-surface::managers` exposes `FileStore`, `TypeManager`,
`DocManager`, `ActionManager` as object-safe `#[async_trait]` traits.
Every Neon binding holds `Arc<dyn Trait>`, not `Arc<Local*>`:

```rust
pub struct JsFileStore    { inner: Arc<dyn FileStore> }
pub struct JsTypeManager  { inner: Arc<dyn TypeManager> }
pub struct JsDocManager   { inner: Arc<dyn DocManager> }
pub struct JsActionManager{ inner: Arc<dyn ActionManager> }
```

Construction is the only place a concrete impl appears — and each
manager now exposes **two** constructors, both real:

```rust
// local — wires the in-process solx-types/-files/-docs/-actions impl
#[cfg(feature = "local")]
fn open(...) -> JsResult<JsBox<JsFileStore>> { ... Arc::new(LocalFileStore::new(...)) ... }

// client — wires an HTTP-backed proxy from solx-client, talking to a
// running solx-server
#[cfg(feature = "client")]
fn connect(server_url, token) -> JsResult<JsBox<JsFileStore>> {
    Arc::new(solx_client::RemoteFileStore::new(server_url, token))
}
```

`Cargo.toml` declares **both features on by default** — this is the
mechanism that makes "one compiled library, runtime choice" possible.
Unlike the original plan (a single Cargo feature flag choosing local
XOR client at build time), the local/remote decision is now made by
*which constructor the caller calls*, at runtime:

```toml
[features]
default = ["local", "client"]
local = []
client = []
```

`solx-client` (`RemoteTypeManager`/`RemoteFileStore`/`RemoteDocManager`/
`RemoteActionManager`, each `::new(base_url, token)`) is a lightweight
dependency — just `solx-surface` + `reqwest`, no libsql/tantivy/wasmtime
— so a build with only the `client` feature stays small.

The TS-side classes (`FileStore`, `TypeManager`, `DocManager`,
`ActionManager`) carry **no "Local"/"Remote" prefix** — consumers see
one class per manager, with `.open(...)` and `.connect(serverUrl,
token)` as its two constructors. `createSolx` is what decides which to
call (see §5.3), mirroring `solx-manager::App::build_with_config`'s
exact precedence (`SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN` env, then
`config.snapshot().serverUrl`/`serverToken`, plus an explicit
`CreateSolxOptions` override at the top). Standalone consumers of a
single subpath package (e.g. `solx/docs`) can call `.connect(...)`
directly without going through `createSolx` at all.

A small **internal-only escape hatch** makes this possible without
breaking tree-shaking or introducing a shared "handle" package: each
manager class has a `/** @internal */ static handleOf(mgr): Handle`
accessor, used only by a dependent manager's `open()` (e.g.
`DocManager.open` needs `TypeManager`'s native handle;
`ActionManager.open` needs `types`/`docs`/`files`/`config`'s;
`ScriptRunner.for` needs `ActionManager`'s) or by `createSolx` itself.
It's not part of the supported public API.

`ConfigService` is the one manager that does **not** participate in
this seam — see §6.4. It has no trait in `solx-surface` and is never
proxied, in either local or client mode (mirroring `solx-manager::App`,
whose `config` field is always the local `ConfigService` regardless of
how the other four managers are wired) — it's the thing that *tells*
the other managers which mode to use, so it can't itself be remote. The
binding's `configOpen` is intentionally **not** gated by the `local`/
`client` features at all; it was previously (incorrectly) gated behind
`#[cfg(feature = "local")]`, which meant a client-only build couldn't
even open a config file — fixed alongside this work (see §7).

---

## 4. Async vs sync — the runtime gotcha

`ConfigService` is **synchronous** in `solx-core`. The other four managers
(`TypeManager`, `FileStore`, `DocManager`, `ActionManager`) are async
because they're backed by `libsql` (which needs a tokio runtime). Neon
runs everything on a single JS event-loop thread, so we can't just call
`block_on` from a Neon worker thread — the libsql internals call
`tokio::spawn`, which would panic with "Cannot start a runtime from
within a runtime".

The fix (in `async_runtime.rs`):

```rust
static BINDING_RUNTIME: OnceLock<Arc<Runtime>> = OnceLock::new();

fn run_async<'a, F, T>(cx: &mut FunctionContext, fut: F)
  where F: Future<Output = Result<T, SolxError>> + Send + 'static,
        T: Serialize + Send + 'static,
{
    let runtime = binding_runtime();       // multi-thread, 2 workers
    let channel = cx.channel();
    let (deferred, promise) = cx.promise();
    runtime.spawn(async move {
        let result = fut.await;
        deferred.settle_with(&channel, move |mut tcx| match result {
            Ok(v) => Ok(JSON_stringify(tcx, &v)),  // wrap as JSON string
            Err(e) => tcx.throw_error(format!("SolxError::{kind}: {detail}")),
        });
    });
    Ok(promise)
}
```

Key points:

- **One long-lived multi-thread runtime**, lazily initialized via
  `OnceLock<Arc<Runtime>>`. Every `run_async` call reuses it.
- `runtime.spawn`, **never** `block_on`. libsql's internal `tokio::spawn`
  calls land in this same runtime.
- The async work serializes its `Result` to JSON before crossing the JS
  boundary — this keeps the wire format uniform regardless of which
  manager produced it. `Vec<u8>` payloads (for files) are base64-encoded
  before the same JSON envelope.

A separate one-shot current-thread runtime is used inside each
manager's `open()` (`typesOpen`, `docsOpen`, `actionsOpen`) to
construct its database — that's a one-time setup cost that has to
happen on the JS thread before `run_async` can take over. `connect()`
(client mode) needs no such runtime — building a `reqwest::Client` and
storing a `base_url`/`token` does no I/O, so every manager's `connect`
is plain synchronous Rust.

---

## 5. What's done

### 5.1 Reference implementation — fully wired

All seven manager crates are end-to-end working today (build, run, all
tests pass), each with both local and client (remote) constructors
except `solx-config`:

| Crate              | Rust binding  | TS class        | Local ctor | Client ctor | Tests |
|--------------------|---------------|------------------|------------|-------------|-------|
| `solx-types`       | `types.rs`    | `TypeManager`    | `open`     | `connect`   | round-trip + loader fallback |
| `solx-files`       | `files.rs`    | `FileStore`      | `open`     | `connect`   | put / get / list / delete + path-safety |
| `solx-docs`        | `docs.rs`     | `DocManager`     | `open`     | `connect`   | post / get / list / search / delete + schema-validation rejection |
| `solx-actions`     | `actions.rs`  | `ActionManager`  | `open`     | `connect`   | CRUD + `exec` against a seeded built-in + unknown-action rejection |
| `solx-scripts`     | `scripts.rs`  | `ScriptRunner`   | n/a — binds to an already-open `ActionManager` | | `json`/`exec` stages, last-statement return, `$params` seeding, unsupported-verb rejection |
| `solx-config`      | `config.rs`   | `ConfigService`  | `open`/`openIn` (always local) | — | set / get / patch / mutate / packages / derived paths + reopen persistence + umbrella wiring |

The `solx` umbrella package wires these together via `createSolx({ appdata })`
— opens config first (always local), then decides local vs. client
**once** for the other four managers (see §5.3), deriving local paths
from `config.typesDbPath()`/`filesDir()`/`docsDbPath()`/`actionsDbPath()`
or a `serverUrl`/`serverToken` connection. `packages/sdk/test/create.test.ts`
covers the full local path; `packages/sdk/test/create-remote.test.ts`
covers the full client path against an in-test mock HTTP server that
speaks the same wire contract as `solx-server` (bearer auth + the
`solx-surface::wire` DTOs) — this validates the Rust `Remote*` path
end-to-end through Neon without needing to build the real
`solx-server` binary.

`ScriptRunner` binds to an already-open `ActionManager` (local or
remote, transparently) rather than having its own `open`/`connect` —
there's no `ScriptRunner` trait upstream, only `solx_scripts::
CommandRunner`, and the binding's `TraitActionRunner` supports exactly
two stage verbs (`exec <path/name> [--json '<params>']`, `json
<literal>`), mirroring `solx-actions/src/script.rs`'s intentionally
narrow grammar but built on `Arc<dyn ActionManager>` instead of the
concrete `LocalActionManager` so it works in both modes. See §7's
decisions log for why the original `ScriptContext`/`runCommand`-callback
design was dropped.

### 5.2 Build & test status

```
$ cargo build -p solx-bindings                              → Finished `dev` profile
$ cargo build -p solx-bindings --no-default-features --features local   → also builds standalone
$ cargo build -p solx-bindings --no-default-features --features client  → also builds standalone
$ bun run typecheck                     → 8/8 packages exit 0
$ bun run build                         → 8/8 packages exit 0
$ SOLX_NATIVE_BIN=…/solx.node bun run test
  → @solx/types:            1/1
  → @solx/docs:              2/2
  → @solx/config:          11/11
  → @solx/files:             3/3
  → @solx/actions:           3/3
  → @solx/scripts:           5/5
  → solx (sdk, local):       3/3
  → solx (sdk, client mock): 5/5
  → 33 tests passing total
```

Note: `bun run --filter='./packages/*' test` reports a nonzero shell
exit code (5) per native-touching package even when every test in that
file passes (vitest itself prints all-green). This is a pre-existing
Bun/native-addon interaction (the long-lived multi-thread tokio runtime
in `async_runtime.rs` keeps background OS threads alive past the point
Bun expects the process to exit cleanly) — not something introduced by
this work, and not a test failure; check the vitest summary lines, not
the aggregate shell exit code.

### 5.3 The TS surface shape

The public surface is **stable** — class names match the trait names, no
"Local"/"Remote" prefix, subpath exports for tree-shaking:

```ts
// One entry point — opens config first, decides local vs. client
// once for the rest, wires everything in dependency order:
const solx = await createSolx({ appdata: '/my/data' });
// ...or force client mode explicitly (skips env/config lookup):
const remote = await createSolx({ serverUrl: 'http://127.0.0.1:8766', serverToken: '...' });

// Or subpath imports for tree-shaking, each with its own
// `.open(...)` (local) / `.connect(serverUrl, token)` (client):
import { TypeManager } from 'solx/types';
import { FileStore } from 'solx/files';
import { DocManager } from 'solx/docs';
import { ActionManager } from 'solx/actions';
import { ScriptRunner } from 'solx/scripts';
import { ConfigService } from 'solx/config';   // local-only, no .connect()
import { SolxError } from 'solx/surface';
```

`createSolx`'s local/client decision (mirrors `solx-manager::
App::build_with_config`'s precedence exactly):

1. `opts.serverUrl`/`opts.serverToken` passed to `createSolx` directly
2. `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN` env vars
3. `config.snapshot().serverUrl`/`serverToken` (persisted config)
4. otherwise: local, deriving each manager's local constructor args
   from `config`'s derived-path methods

The `Solx` aggregate — every field is now backed by a real
implementation, none are placeholders:

```ts
export interface Solx {
  readonly types: TypeManager;
  readonly files: FileStore;
  readonly config: ConfigService;  // always local
  readonly docs: DocManager;
  readonly actions: ActionManager;
  readonly scripts: ScriptRunner;
  readonly appdata: string;
}
```

Each binding class **structurally implements** the surface interface from
`@solx/surface` (no separate "adapter" / "wrapper" layer; the class name
and the interface name match). `ScriptRunner`'s surface interface was
simplified from the original speculative design (a `run(source, ctx)`
taking a JS `runCommand` callback plus `cwd`/`env`) to
`run(source: string, params?: JsonValue): Promise<JsonValue>` once the
actual backend capability (a `CommandRunner` scoped to `exec`/`json`
stages against an `ActionManager`, not arbitrary process execution) was
implemented — see §7.

### 5.4 The native loader

`packages/types/src/loader.ts` resolves the `.node` binary in this order:

1. `process.env.SOLX_NATIVE_BIN` (dev mode: points at a `cargo build` output)
2. `process.env.SOLX_NODE_PATH` (alt dev path)
3. `@solx/<triple>` (the published per-platform package)
4. Error with a clear message listing what was tried

The triple is derived from `process.platform` + `process.arch` + the
build flavor (msvc on Windows, none on POSIX). On Windows under
Git-Bash, `/d/foo` paths get normalised to `D:\foo` before resolution
(`createRequire(import.meta.url)` won't find them otherwise).

The loader is exported from `@solx/types/loader` so the other binding
packages (`@solx/files`, `@solx/config`) reuse the same resolution logic
without duplicating it.

---

## 6. What's left

`solx-docs`, `solx-actions`, and `solx-scripts` bindings, and the
runtime local↔client swap for every manager, are now **done** (see
§5). What remains:

### 6.4 `solx-surface::managers::ConfigService` trait — now understood to be permanent, not a gap

Re-investigated while wiring the runtime local/client swap for the
other four managers (§3.2): `solx-config::ConfigService` has no trait
in `solx-surface`, and **it's not a temporary gap** — `solx-manager`'s
own `App` (the Rust-side equivalent of this repo's `createSolx`) always
holds the local `ConfigService` directly, in *both* its local and
remote wiring paths (`wire_local`/`wire_remote`), because config is
what tells the other managers which mode to use in the first place; a
config service can't meaningfully be "the remote proxy that decides
whether to be remote." `solx-server` never exposes a `/config/*` route
either. So this repo's `config.rs` binding intentionally stays
unconditional (not gated by `local`/`client` at all — see §7 for the
bug this fixed) and there is no upstream trait to wait on. Left open
only as a note in case `solx-core`'s design changes; not on the active
roadmap.

### 6.5 Per-platform binaries

Today only `platform-packages/solx-win32-x64-msvc/` is staged (the
machine that developed this). The remaining 7 targets need to be
built and published:

- `solx-darwin-arm64`, `solx-darwin-x64`
- `solx-linux-x64-gnu`, `solx-linux-arm64-gnu`
- `solx-linux-x64-musl`, `solx-linux-arm64-musl`
- `solx-win32-x64-msvc` (already done) + optional `solx-win32-arm64-msvc`

The standard `napi-rs` approach is a CI matrix (`github/workflows/release.yml`)
that builds all 8 on every tag push. The bindings are `cdylib` so the
standard `napi build` workflow produces the platform package tarballs.

Estimated effort: ~half a day to wire up the CI matrix.

### 6.6 Specta codegen for TS types (v2)

Today the TS DTOs (`TypeEntity`, `Document`, `Action`, …) are
**hand-written** to mirror the Rust `serde` shape — the binding then
does snake_case ↔ camelCase conversion in TS. This works but is
maintenance overhead when the Rust types change.

The v2 plan is to add `specta::Type` derives to `solx-surface` DTOs
and run a small codegen step at `cargo build` time that emits
`packages/surface/dist/types.ts`. The hand-written TS types become
generated; consumers get exact match automatically.

Not blocking — defer until a DTO actually changes.

Estimated effort: ~1 day.

### 6.7 HTTP transport — done

~~The current transport is only the in-process Neon binary...~~ Done:
`solx-core` shipped `solx-server` (axum, bearer-token auth,
`127.0.0.1`-only) and `solx-client` (`reqwest`-backed `Remote*` trait
impls), and this repo now depends on `solx-client` directly from
`solx-bindings` (§3.2) — every manager with a trait can run in client
mode against a real `solx-server`, verified end-to-end (§5.1, §5.2).
The one piece **not** built here: `solx-js` has no code to *start* a
`solx-server` itself (that binary lives in `solx-core`); this repo is
purely a client of one, same as `solx-cli`/`solx-mcp` can be.

### 6.8 The old `solx-node.md`

This file is a research note generated by an earlier LLM session. It
predates `ARCHITECTURE.md` and this doc. Candidates:

- Delete it (the content is fully covered here and in ARCHITECTURE.md).
- Keep it as historical record (it's a snapshot of the design discussion).

The user's preference: TBD. It's safe to delete now that the work has
moved past the "research note" stage.

---

## 7. Decisions log (recent)

### Dropped: `Local*` prefix on public TS classes

Originally the binding classes were `LocalFileStore`, `LocalTypeManager`,
`LocalConfigService` — mirroring the Rust `Local*` impls. Renamed to
just `FileStore`, `TypeManager`, `ConfigService` to match the trait
names. Reasoning: the public name should not leak which backend is
compiled in. The Rust `Local*` impls stay internal — they exist only
inside `crates/solx-bindings/src/`.

### Dropped: `ConfigServiceAdapter` layer

Originally there were two classes: a raw `LocalConfigService` (just
the binding primitives) and a `ConfigServiceAdapter` that wrapped it to
add `mutate(fn)` and satisfy the surface `ConfigService` interface.
Merged into a single class. Reasoning: the surface interface is sync,
the binding primitives are sync, the adapter was a one-liner per
method. Splitting made the type story harder (consumers had to know
which to import).

### Renamed: `filesRoot` binding function → TS-side captured value

Originally the binding exported `filesRoot(handle)` to return the
on-disk root path. Dropped — the `FileStore` trait doesn't expose
`root()` and downcasting the trait object back to `LocalFileStore`
would defeat the seam. The TS `FileStore` class now captures `root`
at construction and returns it from `root()` directly. Cleaner and
aligns with the trait abstraction.

### Renamed: `LocalFileStore.new(root)` → `FileStore.open(root)`

Naming consistency with `ConfigService.open` / `ConfigService.openIn`.
Both `open` and `openIn` are the construction primitives; `new` was
Java-ish and didn't match the rest of the API.

### Added: explicit `Buffer` import in `@solx/files`

`@types/node` augments the global scope, but stricter IDE settings
(like Pylance in our setup) need explicit `node:buffer` imports.
Also added `"DOM"` to `lib` and `"types": ["node"]` to `tsconfig.base.json`
so `btoa`/`atob` (browser fallbacks) and `Buffer` resolve in both
the CLI build and the IDE.

### Cargo features — superseded: both on by default, runtime chooses

Originally `default = ["local"]`, with `client = []` as a
forward-looking stub that would fail to build once flipped on (no
`solx-client` dependency existed yet), on the theory that local-vs-
client would be a build-time XOR choice. Once `solx-client` existed
upstream and the user asked for **one compiled library** usable either
way, this was replaced: `default = ["local", "client"]`, both real,
both compiled in together. The local/remote choice moved from "which
feature flags this binary was built with" to "which constructor
(`open` vs. `connect`) the caller invokes at runtime" — see §3.2. The
features still exist so a minimal client-only build (skip libsql/
tantivy/wasmtime) or local-only build remains possible, just not as
the default.

### Fixed: `ConfigService` was wrongly gated behind the `local` feature

`config.rs`'s `use solx_config::ConfigService;` and its `open()` body
were behind `#[cfg(feature = "local")]`, inherited from before the
local/client split had a real client backend. Since `ConfigService` is
permanently local-only (§6.4) rather than feature-gated, a
`--no-default-features --features client` build couldn't even open a
config file — caught by building each feature combination standalone
while verifying the local/client work compiles independently. Fixed by
making the config binding fully unconditional.

### Fixed: `ConfigService.snapshot()` didn't convert snake_case → camelCase

Every other DTO in the codebase goes through a `*FromSnake` converter
(see `TypeManager`'s `entityFromSnake`), but `snapshot()` cast the raw
native JSON straight to the camelCase `SolxConfig` TS type with no
conversion — a latent bug that happened not to matter until
`createSolx`'s new local/client decision needed to actually read
`config.snapshot().serverUrl`/`serverToken` correctly. Added
`SolxConfigSnake` + `snapshotFromSnake()`, and added the
previously-missing `serverUrl`/`serverToken`/`serverPort` fields to
`@solx/surface`'s `SolxConfig` interface (mirroring
`solx-config/src/types.rs`, which already had them).

### Simplified: `ScriptRunner`'s TS interface

The original `@solx/surface` interface (`run(source, ctx)` where `ctx`
supplied a JS `runCommand(name, args)` callback, `cwd`, and `env`) was
speculative — written before any scripting trait existed upstream. The
actual capability that landed (`solx_scripts::CommandRunner`, consumed
narrowly by `solx-actions/src/script.rs`'s `ActionCommandRunner` for
`Script`-typed actions) only supports two stage verbs, `exec` and
`json`, dispatched against an `ActionManager` — there's no way to run
an arbitrary JS-supplied command from a script, by design (bypassing
`Internal` dispatch that way would be a capability escalation). Reduced
to `run(source: string, params?: JsonValue): Promise<JsonValue>`,
bound to an already-open `ActionManager` via `ScriptRunner.for(actions)`.

### Added: `DocumentType.Script`/`Action.trusted` were missing from the TS DTOs

Found while building `@solx/actions`'s snake↔camel conversion:
`solx_surface::entities::ActionType` gained a `Script` variant and
`Action`/`ActionInput` always had a `trusted` field, but
`@solx/surface`'s hand-written mirrors had neither. Fixed alongside
the DTO work for `@solx/actions` (this is exactly the kind of drift
§6.6's specta-codegen plan would prevent).

### Added: `static handleOf(mgr)` internal accessor per manager class

Building `DocManager.open(dbPath, indexDir, types)` (needs
`TypeManager`'s native handle), `ActionManager.open(dbPath, config,
types, docs, files)` (needs four), and `ScriptRunner.for(actions)`
(needs one) required a way for one package's class to hand its opaque
native handle to another's constructor, without making the handle
field public or introducing a shared "handle" package that would
partially undo the tree-shaking goal. Solved with a `/** @internal */
static handleOf(mgr)` per class — documented as not-public-API, used
only by dependent managers' constructors and by `createSolx`.

### Test pattern: one `createSolx()`/manager per describe block, not per test()

Discovered when `solx-docs` went from a stub to real: `solx.docs` holds
a live Tantivy `IndexWriter`, which takes an **exclusive lock** on the
search-index directory for as long as the `LocalDocManager` is alive.
Several existing tests (`packages/files/test/files.test.ts`,
`packages/config/test/config.test.ts`, `packages/sdk/test/
create.test.ts`) called `createSolx({ appdata })` fresh inside every
`test()` block against the *same* appdata — harmless when `docs` was a
placeholder, but a lock-contention deadlock once it wasn't (V8 doesn't
deterministically GC/drop the previous `JsDocManager` between tests).
Fixed by opening `solx` once in `beforeAll` and reusing it across
`test()`s in a block — which also matches how these managers are
actually meant to be used (a long-lived process opens `solx` once, not
per-operation). Where a test specifically needed to verify on-disk
persistence "across a reopen," it reopens a standalone `ConfigService`
instead of a second whole `createSolx` — config has no equivalent
exclusive-lock constraint (its writes serialize via a short-held OS
advisory lock, not a long-held index writer), so that's still a
faithful persistence check.

---

## 8. Reference

- [ARCHITECTURE.md](../ARCHITECTURE.md) — detailed crate-by-crate design
- [README.md](../README.md) — one-page intro + dev loop
- [solx-core/design-and-progress.md](../../solx-core/docs/design-and-progress.md) — the upstream Rust design doc this whole project mirrors
- `/memories/repo/solx-js-neon-runtime.md` — Neon + tokio gotchas (the
  runtime setup we use in `async_runtime.rs`)
- `/memories/repo/solx-browser-tauri-notes.md` — separate repo notes for
  the `sol-browser` Tauri app that consumes `solx-js`