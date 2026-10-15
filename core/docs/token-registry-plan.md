# Token registry — implementation plan

Status: **Ready to implement.** Phases 1 and 2 are done (see "Where things
stand"). This document is the plan for phase 3, so it can be picked up on
its own.

## Goal

Stop sharing one bearer token with everything. Today every HTTP client, every
MCP client and every external Command action holds the same root token, so:

- one leaked token is full access, and the only remedy (`solx token rotate`)
  breaks every client at once;
- a Command action that finished an hour ago still holds a working token;
- there is no way to tell, or limit, who is calling.

Phase 3 adds a `solx-registry` crate that issues **scoped, expiring tokens**:

- **Action tokens.** One per external Command invocation, handed over as
  `SOLX_SERVER_TOKEN` and revoked when the invocation ends.
- **Client tokens.** One per client (a laptop, a browser, an MCP client). A
  client gets one by redeeming a short **connect code**, and each can be
  listed and revoked individually.

The registry sits behind a cargo feature, `registry`, which is **on by
default**. This project always runs with it on. Other users of the crates can
turn it off, and then solx behaves exactly as it did before phase 3.

What action tokens buy, and what they don't: on its own, phase 3 gives
expiry, revocation and attribution. A `Principal::Action` token can still do
everything except the auth builtins, including saving Command rows.
Containment comes from the follow-on work under "Authorization that the
principal makes possible".

## Decisions

| Question | Decision |
|---|---|
| How is the registry switched on? | Cargo feature `registry`, default-on in every crate that has it. |
| Where do client tokens, action tokens and connect codes live? | All in `solx-registry.db`, not process memory. That's what makes `solx token connect-code` (a separate CLI process) and action tokens checked by a sibling server work. |
| Who may create connect codes, issue clients or revoke them? | `Principal::Root` only. Client tokens have no admin flag. |
| How do the CLI and the desktop app manage tokens? | They run the `/builtin/auth/*` builtins as Root through `exec_for`, from binary-only CLI commands and a Settings-window-only Tauri command. No new management HTTP routes. |
| Connect-code format | 8 decimal digits, shown as `1234-5678`, protected by a global failure budget. |
| One field or two for connecting? | One field that takes a code, a link or a token. |

## Where things stand (phases 1–2)

Everything phase 3 needs to plug into already exists:

| Piece | Where | What it does today |
|---|---|---|
| `TokenAuthority` trait, `Principal`, `ActionCredential` | `solx-surface/src/auth.rs` | The seam. `authenticate(bearer) -> Option<Principal>`, `issue_action_token(action_ref, invocation_id) -> Option<ActionCredential>`, `set_server_url(url)`. `ActionCredential` can carry a guard. |
| `SingleTokenAuthority` | `solx-config/src/credentials.rs` | The default: every valid bearer is `Principal::Root`, and every action gets the root token. Its server URL defaults to `sibling_server_url(config)` (where a solx-server on this appdata dir would listen) until the host reports its own bound address. |
| `build_authority(&config)` | `solx-manager/src/lib.rs` | **The one swap point.** |
| `require_bearer` | `solx-server/src/auth.rs` | Calls `authenticate` and puts the `Principal` in the request extensions. Covers REST and `/mcp`. |
| Command env injection | `solx-actions/src/exec.rs` `run_command` | Calls `issue_action_token` and sets `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN` on the child. `action_config.env` was removed, so action config can't set or override environment variables. Keeps the credential alive until the child exits. |
| Root token storage | `credentials::root_token` | OS credential manager (`solx` / `root:<appdata hash>`). A `server_token` in the config file is moved there; when the two disagree the file's copy is newer and wins. |
| Client token storage | `credentials::{client_token, stored_client_token, set_client_token, forget_client_token}` | Per `(appdata, server URL)` entries in the credential manager. The root token is offered only to loopback URLs. |
| Principal plumbing | `ActionManager::exec_for`, `InternalCallCtx::{principal, external_principal()}` | Done; see "Who may call the auth builtins". |
| Binary-only CLI commands | `solx-cli/src/main.rs` `BinCommand` | `solx token show|rotate`. Deliberately not in `Commands`, which `.solx` scripts (including package `install.solx`) can call. |
| Per-invocation loopback tokens | `solx_console::loopback::register` | The existing pattern for a token that lives exactly as long as one invocation (`Registration`, deregisters on drop). |
| Test credential store | `credentials::use_memory_store_for_tests()` | Tests call it before building an `App`. **Don't** turn this into a cargo feature: a dev-dependency feature is compiled into the regular binaries `cargo test` builds. |

Every solx-packages consumer reads only `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN`,
so issuing a different token per invocation needs **no package changes**.

## Design

### Crate and features

- New workspace member `core/solx-registry`. Dependencies: `solx-surface`,
  `solx-config`, `libsql`, `tokio`, `rand`, `base64`, `sha2` (new),
  `chrono`, `async-trait`, `serde`. No axum dependency: the routes live in
  solx-server.
- Features, all default-on:
  - `solx-manager`: `default = ["local", "registry"]`,
    `registry = ["local", "dep:solx-registry"]`.
  - `solx-server`: `default = ["registry"]`,
    `registry = ["solx-manager/registry"]`. Implying the manager feature
    keeps the two from getting out of sync.
  - `solx-cli`: `default = ["registry"]`, forwarding to both.
  - `browser/src-tauri`: **must** add `"solx-manager/registry"` and
    `"solx-server/registry"` to its `server` feature. It depends on
    solx-manager with `default-features = false`, so a default-on feature is
    silently off there otherwise.
- `build_authority` returns `Registry::open(&config).await` under
  `#[cfg(feature = "registry")]`, otherwise `SingleTokenAuthority`. It
  becomes async; `build_local` already is.
- `App::registry() -> Option<Arc<Registry>>` behind the feature, next to
  `authority()`.

### Storage: `solx-registry.db`

- `ConfigService::registry_db_path()`, a fixed file name in `db_dir()` like
  `console_db_path()`. No config key and no `RESTART_KEYS` entry.
- Connection helper copied from `solx-actions/src/db.rs` (`Db::open`/
  `connect` with `busy_timeout`), which is duplicated per crate by design.
- Tables:
  - `tokens(id TEXT PK, kind TEXT CHECK(kind IN ('client','action')),
    token_hash BLOB UNIQUE, label, action_ref, invocation_id, created_at,
    expires_at, last_used_at, revoked_at)`
  - `connect_codes(code_hash BLOB PK, label, token_ttl_secs, created_at,
    expires_at, consumed_at)`
  - `registry_state(key TEXT PK, value)`, holding the global
    `failed_exchanges` counter.
- Tokens are 32-byte random secrets (`credentials::generate_token()`). Only
  SHA-256 hashes are stored, and lookup is by hash, so no constant-time
  comparison is needed.

### `Registry: TokenAuthority`

- **Root token.** `credentials::root_token(config)`, checked in memory with
  `constant_time_eq` before any DB access. It authenticates as
  `Principal::Root`, and local hosts keep using it.
- **`authenticate`.** If the bearer isn't the root token, hash it and look
  it up:
  `… WHERE token_hash=? AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now)`.
  - No cache, so a revocation from another process takes effect on the next
    request. The lookup is one indexed SQLite read.
  - Client rows map to `Principal::Client { id, label }`, action rows to
    `Principal::Action { action_ref, invocation_id }`.
- **`last_used_at`.** An in-memory `HashMap<id, Instant>` throttles writes to
  at most one per minute per token.
- **Action tokens.**
  - `issue_action_token(action_ref, invocation_id, ttl)` inserts an `action`
    row with `expires_at = now + ttl + 60s` grace.
  - `release(&ActionCredential)` deletes it. `run_command` calls it once the
    child finishes, on every path after spawn.
  - The credential's guard is a fallback for early returns and panics. Its
    `Drop` can't await, so it spawns the same delete via
    `tokio::runtime::Handle::try_current()`.
  - Rows left by a crash, or by a CLI process that exited first, die at
    `expires_at`.
- **Server URL.** Same as `SingleTokenAuthority`: defaults to
  `sibling_server_url`, overridden by `set_server_url`. Factor out the shared
  URL slot. Because action tokens are in the DB, a sibling solx-server
  accepts tokens minted by a process that serves no HTTP itself, such as
  stdio `solx-mcp`.
- **Sweep.** On `open`, and every 10 minutes from a task the long-lived hosts
  spawn, delete expired action rows and expired or consumed codes.
- **Client tokens** expire after 90 days by default, configurable with
  `client_token_ttl_days`.
- **Methods for the builtins and the exchange route:**
  - `create_connect_code(label, code_ttl, token_ttl)`
  - `redeem(code)`: in one transaction, run
    `UPDATE connect_codes SET consumed_at=? WHERE code_hash=? AND consumed_at IS NULL AND expires_at>?`
    and insert a client token if exactly one row changed
  - `issue_client(label, ttl)`
  - `list_clients()`, which never returns hashes
  - `revoke_client(id)`

### Trait change (`solx-surface/src/auth.rs`)

- `issue_action_token(&self, action_ref, invocation_id, ttl: Duration)`.
  `run_command` passes
  `Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS))`.
  Detached runs reach `run_command` inside their spawned task with
  `background_timeout_secs`, so they release at completion with no extra
  work.
- `async fn release(&self, credential: &ActionCredential) {}`, with a no-op
  default for `SingleTokenAuthority`.
- `ActionCredential` gains an opaque `id: Option<String>`.

### Connect codes

- **Format.** 8 random decimal digits from the OS RNG, shown as `1234-5678`.
  Input ignores everything but digits, so `12345678`, `1234 5678` and
  `1234-5678` all work. Codes are meant to be read off one screen and typed
  on another.
- **Lifetime.** 10-minute TTL by default, single use (the atomic `UPDATE` in
  `redeem`). If a pending code already has the same hash, generate another.
- **Guessing is the main threat** at about 26.6 bits, and per-IP limiting
  alone doesn't stop an attacker who spreads guesses across many addresses.
  So there is a **global failure budget**:
  - every failed exchange increments `failed_exchanges` in the DB, which
    counts across processes and peers;
  - at 10 failures, every pending code is deleted and the counter resets.
    The user just generates a new code;
  - neither a successful redemption nor creating a code resets the counter,
    so an attacker can't refill the budget.

  An attacker's odds are then about 10 in 10^8 per batch of codes. A short
  per-IP delay after each failure is an extra layer.
- Hashing 8 digits doesn't hide them from anyone who can read the DB (only
  10^8 possibilities). That's acceptable, since DB readers already have local
  access and codes live 10 minutes, but SECURITY.md should say so.

### The exchange (solx-server only)

The exchange is a solx-server route, not a separate listener. A client that
redeems a code must reach solx-server to use its token anyway.

- `POST /auth/exchange` takes `{ "code": "..." }` and returns
  `{ "token", "client_id", "expires_at", "server_url" }`.
  - It's mounted **outside** the bearer gate, next to `/health`, and only
    when the feature is on.
  - `server_url` tells native clients what to store.
  - It's the one unauthenticated, state-changing endpoint, so failures count
    against the budget above.
- `GET /connect` **changes no state.** It returns a small static page:
  - the page's JavaScript reads `#code=` from the fragment, or shows an
    8-digit input when there is none, and POSTs it to `/auth/exchange`;
  - browsers never send the fragment, so the GET doesn't even carry the
    code, and the code stays out of access logs, proxies and `Referer`;
  - link previewers and mail scanners fetch with GET and don't run
    JavaScript, so they can't use a code up.
- Two ways to share a code:
  - a link, `<public url>/connect#code=12345678`;
  - the server address plus the code, typed by hand. `create-connect-code`
    returns both `url` and `display_code`.
- `<public url>` comes from a new config key `server_public_url`, for when
  the bind address isn't what clients should use (`0.0.0.0`, a reverse proxy,
  a Tailscale name). It falls back to the bound address.
- CORS: off loopback, `/auth/exchange` follows the existing
  `server_cors_origins` policy like every other route.

### Root token from loopback peers only

With the registry on, `require_bearer` rejects `Principal::Root` from a
non-loopback peer, with a 401 telling the caller to use a connect link.
Network-facing servers then never accept the all-powerful token from the
network.

- `axum::serve` needs `into_make_service_with_connect_info::<SocketAddr>()`
  for the middleware to see the peer, in `solx_server::serve` and
  `solx-server`'s `main`.
- **Caveat:** behind a reverse proxy on the same machine, every request
  arrives from loopback. Document this, and consider treating every peer as
  non-loopback when `server_public_url` is set.

### Who may call the auth builtins

The builtins (below) need the caller's `Principal`, carried explicitly rather
than in a task-local, because task-locals don't cross `tokio::spawn` and
`action-start` spawns detached tasks. This plumbing is done:

- `ActionManager::exec_for(principal, path, name, params)` is a required
  trait method. `exec` means "no principal" (CLI, stdio MCP, Tauri).
- REST exec takes `Extension<Principal>` from `require_bearer` and calls
  `exec_for`. `/mcp` reads it from the `http::request::Parts` that rmcp puts
  in `RequestContext::extensions` (`request_principal` in solx-mcp). Stdio
  has no `Parts`, so it gets `None`.
- The principal travels through `exec_as`/`exec_as_with` into `InternalCtx`
  and `InternalCallCtx.principal`. Nested frames (WASM, script,
  `script-exec`) **inherit** it for attribution.
- `action-start` moves both the principal **and** the caller into the
  spawned task. Dropping the caller would let an action launched by Root
  re-enter as an apparently external Root call.
- Guards use `InternalCallCtx::external_principal()`, which fails closed. It
  returns `None` if an action is in the chain, if the principal is
  `Principal::Action`, or if there is no principal.
- `solx_actions::test_probe` echoes what a handler receives, for tests
  (`solx-server/tests/principal.rs`).

Policy for every auth builtin:

| Caller | Allowed? |
|---|---|
| `Principal::Root` (REST or `/mcp` with the root token, which means loopback only) | yes |
| A trusted local entry point running the builtin as Root (see below) | yes |
| `Principal::Client` | no |
| `Principal::Action`, or any in-process action `Caller` | no |
| No principal (stdio `solx-mcp`, local CLI `exec`, plain Tauri IPC) | no |

Refusing the no-principal case keeps a model on stdio MCP, or package widget
code in the desktop app, from minting access for itself.

**Trusted local entry points.** The desktop UI talks to solx over Tauri IPC
with no bearer token, so its calls have no principal and the builtins refuse
them. The local CLI is in the same position. Both get a narrow way in that
runs the builtins as Root:
`exec_for(&Principal::Root, "/builtin/auth", name, params)`.

- The CLI's `solx token …` commands are binary-only (not in `Commands`), so
  scripts can't reach them.
- The desktop's `auth_exec` command is granted only to the Settings window,
  not the main window where package widgets run.

General Tauri IPC stays principal-less.

### Built-in actions

Use the plugin pattern `solx-console` uses (`plugin()` plus
`seed_actions()`). `solx-actions` can't depend on `solx-registry` without a
cycle through solx-manager, so add `LocalActionManager::add_plugin(
InternalActionRegistry)`. solx-manager calls it before `set_self_ref` when
the feature is on, and `set_self_ref` merges it the way it merges
`test_probe::plugin()`. Seeds are registered only with the feature on.

- `/builtin/auth/create-connect-code` `{ label?, code_ttl_secs?,
  token_ttl_secs? }` returns `{ display_code, url, expires_at }`.
- `/builtin/auth/list-clients` returns ids, labels and timestamps, never
  hashes.
- `/builtin/auth/revoke-client` `{ id }`.
- `/builtin/auth/issue-client` `{ label, token_ttl_secs? }` returns
  `{ token, client_id, expires_at }`, minting a client token directly.

Log redemptions, issues and revocations to the console, never with a token
or code.

### Client side

**One input takes a code, a link or a token.** A shared classifier,
`credentials::parse_connect_input(server_url, input)`, returns `Code` or
`Token` plus the effective server URL:

- a link containing `#code=` is a `Code`, and its origin becomes the server
  URL when none was given;
- input that is exactly 8 digits once spaces and dashes are removed is a
  `Code`;
- anything else non-empty is a `Token`, used as entered apart from trimming.

This can't be ambiguous: tokens are 44-character base64 and never exactly 8
digits. A TypeScript twin in `web/web/src/api/connection.ts` uses the same
test cases.

- **CLI.**
  - `solx connect <link-or-server-url> [CREDENTIAL] [--print]` is a
    binary-only `BinCommand`. It prompts for the credential when there is no
    link and no argument.
  - A code is redeemed through `/auth/exchange`; a token is used as given.
    Then it runs `credentials::set_client_token` and sets `server_url`, or
    with `--print` only prints the token (for MCP clients such as VS Code's
    `.vscode/mcp.json` prompt).
  - Management: `solx token connect-code [--label] [--ttl]`,
    `solx token clients`, `solx token revoke <id>`,
    `solx token issue --label [--ttl]`.
    - Each builds the App with `build_app()` and runs the matching builtin
      as Root.
    - With no host running, that happens in-process.
    - With a host running, the App is remote and the server derives Root
      from the loopback root token. The policy is the same either way.
- **Desktop client build.**
  - The existing `connect(server_url, token)` becomes
    `connect(server_url, credential)`: classify the input, redeem a code if
    it is one, then run the unchanged verify-and-store steps.
  - There is no second command. The UI has a server-address field and a
    "Connect code or token" field.
- **Desktop host build.** Settings gets "Connect another device", backed by
  `auth_exec`: it shows the code large as `1234-5678` with the server address
  and a copyable link, counts down to expiry, and lists and revokes clients.
- **Browser (`web/`).** The same single field in Settings. A code is redeemed
  first; either way the token is stored in `localStorage` as it is today. The
  `/connect` page does the same for a link.

### Follow-on: authorization that the principal makes possible

These aren't part of phase 3. They become possible once real
`Principal::Client`/`Principal::Action` tokens exist, and they should reuse
`external_principal()` so they fail closed in the same way.

- **Gate Command/Webhook saves on every route** (lower priority).
  - `guard_executable_action` currently covers only the
    `entity-save-action`/`entity-delete-action` built-ins. HTTP
    `/actions/save` and Tauri `actions-save` were left open because every
    token was root.
  - With the registry, `save`/`delete` of a `command` or `webhook` row would
    be allowed only for `Principal::Root`. It would be refused for
    `Principal::Action`, for any call with an action `Caller`, and for no
    principal on a network route.
  - That closes the remaining paths an action could use to grant itself
    access, including a Command child calling back with
    `SOLX_SERVER_TOKEN`.
  - The local CLI (and so `install.solx`) still calls `save` directly with
    no principal, and must keep working.
  - Tauri needs a decision of its own. It's in-process with no token, so the
    gate there would be a narrower permission set, for example taking
    `actions-save` for Command/Webhook out of `default`, rather than a
    principal check.
- **Scope `get-env`/`set-env`.** The env store is a global, process-wide
  map. That's by design, but it also holds system variables copied in
  through `env_mappings`. Options:
  - refuse `set-env` to `Principal::Action` and to action callers outside
    the namespace they own;
  - make `env_mappings`-seeded entries read-only;
  - let a namespace declare which principals may read it.

  Decide once there are principals to scope by. Until then, treat anything
  in the store as readable by all content.

## Implementation steps

1. **Crate skeleton, DB and features.**
   - A `Registry` with root-token auth only.
   - The `build_authority` switch and `App::registry()`.
   - Feature wiring, including `browser/src-tauri`.
   - Build with default features and with
     `--no-default-features --features local`.
2. **Trait change and action tokens.**
   - `ttl` and `release`, wired through `run_command`.
   - The guard fallback and the sweep.
3. **Client tokens and connect codes.**
   - Issue, redeem, list and revoke.
   - The failure budget, throttled `last_used_at`, and default expiry.
4. **Server routes.**
   - `/auth/exchange`, `/connect` and `server_public_url`.
   - Per-IP delay and loopback-only root.
5. **Builtins and CLI.**
   - `add_plugin`, the four builtins with the policy check.
   - `solx token connect-code|clients|revoke|issue`, `solx connect`, and
     `parse_connect_input`.
6. **Clients and docs.**
   - Desktop `connect(server_url, credential)`, `auth_exec` with its
     Settings-only capability, and the "Connect another device" section.
   - The web single field and the `/connect` page.
   - `http-api.md`, `SECURITY.md`, `security-design.md`, README.

## Tests

Run `cargo test --workspace` (registry on by default), plus build checks
with it off: `cargo check -p solx-server --no-default-features` and
`cargo check -p solx-manager --no-default-features --features local`. Every
test that builds an `App` or opens a `Registry` calls
`credentials::use_memory_store_for_tests()`.

- **Registry unit tests:**
  - issue, authenticate, release;
  - expiry, revoke, `last_used_at` throttling, the sweep.
- **Codes:**
  - reuse is rejected; expiry is rejected;
  - `1234 5678` and `1234-5678` both redeem;
  - 10 wrong codes, even from different peers, invalidate the pending code,
    so the right code then fails;
  - a successful redemption doesn't reset the budget.
- **Exchange** (`solx-server/tests/registry.rs`, modelled on `principal.rs`):
  - create-connect-code → exchange → the token works and appears in
    `list-clients` → revoke → 401 on the next request;
  - `GET /connect`, with or without a fragment, leaves the DB unchanged;
  - the root token from a non-loopback peer is refused.
- **Action tokens** (extend `solx-actions/tests/command_credentials.rs`):
  - the child's token authenticates as `Principal::Action` while it runs,
    and gets a 401 after it exits;
  - the same holds when the command fails or times out;
  - detached (`action-start`) runs release at completion, not when `start`
    returns.
- **Cross-process:**
  - two `Registry` instances on one appdata dir: a revoke in one is rejected
    by the other at once;
  - an action token issued by one is accepted by the other (the
    sibling-server case).
- **Builtin policy**, for each entry point:
  - Root over REST and `/mcp` is allowed;
  - a Client token, an Action token, a nested script caller and no principal
    (stdio, plain Tauri `actions-exec`) are refused.
- **Trusted entry points:**
  - `solx token connect-code` works with no host running and against a
    running host;
  - `auth_exec` rejects names outside its list;
  - the main window's capability doesn't grant `auth_exec`.
- **Classifier** (Rust and TS):
  - `12345678`, `1234-5678` and ` 1234 5678 ` are codes;
  - a `/connect#code=…` link is a code, and its origin fills in the server
    URL;
  - a 44-character base64 string is a token;
  - 7 digits are treated as a token and fail verification with a clear
    error.
- **Manual:**
  - bind the server on a LAN IP and run `solx token connect-code` on the
    host;
  - on a second machine, connect with `solx connect` and with the link in a
    browser;
  - check that the root token is refused from the second machine.
