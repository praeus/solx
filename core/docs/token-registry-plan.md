# Token registry — implementation plan

Status: **Not started.** Phases 1 and 2 are done (see "Where things stand").
This document is the plan for phase 3, so it can be picked up on its own.

## Goal

Stop sharing one bearer token with everything. Today every HTTP client, every
MCP client and every external Command action holds the same root token, so:

- one leaked token is full access, and the only remedy (`solx token rotate`)
  breaks every client at once;
- a Command action that finished an hour ago still holds a working token;
- there is no way to tell, or limit, who is calling.

Phase 3 adds an optional `solx-registry` crate that issues **scoped,
expiring tokens**:

- **Action tokens.** One per external Command invocation, handed over as
  `SOLX_SERVER_TOKEN` and revoked when the invocation ends.
- **Client tokens.** One per client (a laptop, a browser, an MCP client). A
  client gets one by redeeming a **one-time code (OTC)** delivered in a URL,
  and each can be listed and revoked individually.

Without the cargo feature, solx behaves exactly as it does now.

## Where things stand (phases 1–2)

Everything phase 3 needs to plug into already exists:

| Piece | Where | What it does today |
|---|---|---|
| `TokenAuthority` trait, `Principal`, `ActionCredential` | `solx-surface/src/auth.rs` | The seam. `authenticate(bearer) -> Option<Principal>`, `issue_action_token(action_ref, invocation_id) -> Option<ActionCredential>`, `set_server_url(url)`. `ActionCredential` can carry a guard that revokes on drop. |
| `SingleTokenAuthority` | `solx-config/src/credentials.rs` | The default: every valid bearer is `Principal::Root`, and every action gets the root token. |
| `build_authority(&config)` | `solx-manager/src/lib.rs` | **The one swap point.** Phase 3 adds `#[cfg(feature = "registry")]` here. |
| `require_bearer` | `solx-server/src/auth.rs` | Calls `authenticate` and puts the `Principal` in the request extensions. Covers REST and `/mcp`. |
| Command env injection | `solx-actions/src/exec.rs` `run_command` | Calls `issue_action_token` and sets `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN` on the child. Action config can't set or override environment variables (`action_config.env` was removed). Keeps the credential alive until the child exits. |
| Root token storage | `credentials::root_token` | OS credential manager (`solx` / `root:<appdata hash>`); legacy `server_token` is migrated out of the config file. |
| Client token storage | `credentials::{client_token, stored_client_token, set_client_token, forget_client_token}` | Per `(appdata, server URL)` entries in the credential manager. The root token is offered only to loopback URLs. |
| Binary-only CLI commands | `solx-cli/src/main.rs` `BinCommand` | `solx token show|rotate`. Deliberately not in `Commands`, which `.solx` scripts (including package `install.solx`) can call. |
| Test credential store | `credentials::use_memory_store_for_tests()` | Tests call it before building an `App`. **Don't** turn this into a cargo feature: a dev-dependency feature is compiled into the regular binaries `cargo test` builds. |

Every solx-packages consumer reads only `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN`,
so issuing a different token per invocation needs **no package changes**.

## Design

### Crate and features

- New workspace member `core/solx-registry`. Dependencies: `solx-surface`,
  `solx-config`, `libsql`, `tokio`, `rand`, `sha2`, `chrono`, `serde`,
  `async-trait`, and `axum` for the exchange route.
- Features:
  - `solx-manager`: `registry = ["local", "dep:solx-registry"]`
  - `solx-server`: `registry = ["solx-manager/registry", "dep:solx-registry"]`
  - `solx-cli`: forwards `registry`
  - `browser/src-tauri`: opts in where it embeds the server.
- `build_authority` returns `solx_registry::Registry::open(...)` when the
  feature is on, otherwise `SingleTokenAuthority`.
- `App` needs a way to expose the concrete registry, for the exchange route
  and the CLI commands. Suggested: `App::registry() -> Option<Arc<Registry>>`
  behind the feature, kept next to `authority()`.

### `Registry: TokenAuthority`

**Root token.** Unchanged: the credential-manager root token, authenticating
as `Principal::Root`. Local hosts on the same machine keep using it.

**Action tokens** (in memory only):

- Map: `sha256(token) -> ActionEntry { action_ref, invocation_id, expires_at }`.
- `issue_action_token` mints a token and returns an `ActionCredential` with a
  guard whose `Drop` removes the entry.
- `expires_at` is a backstop for a leaked guard: the action's effective
  timeout plus a grace period. `run_command` already computes the timeout, so
  either pass it through `issue_action_token` (a trait change; today it takes
  only `action_ref`/`invocation_id`) or use a generous fixed ceiling.
  Detached runs (`action-start`) use `background_timeout_secs`.
- Authenticates as `Principal::Action { action_ref, invocation_id }`.

**Client tokens** (persisted):

- New `solx-registry.db` next to the other databases, with a `registry_db` path
  key (added to `RESTART_KEYS`), mirroring `actions_db`.
- Table `client_tokens(id, token_hash, label, created_at, expires_at,
  last_used_at, revoked_at)`. Only SHA-256 hashes are stored, and lookup is
  by hash, so no constant-time scan is needed.
- Authenticates as `Principal::Client { id, label }`.
- `last_used_at` writes should be throttled (e.g. at most once a minute per
  token) so every request isn't a DB write.
- Revocation must take effect on the next request. Either no cache, or a
  cache that is invalidated in-process on revoke. Remember that `solx token
  revoke` from a separate CLI process writes the DB directly, so a cache in
  the server process would go stale. Recommendation: no cache at first; the
  lookup is one indexed SQLite read.

**One-time codes** (in memory only):

- Map: `sha256(code) -> PendingCode { label, token_ttl, expires_at }`.
- 128-bit, URL-safe codes. Single use: consumed atomically on redemption.
  Default TTL 10 minutes.
- A periodic sweep drops expired codes and expired action tokens.
- Codes are lost on restart. That's acceptable, since they are short-lived.

### The OTC exchange (solx-server only)

Decision: the exchange is a **solx-server route**, not a separate listener.
A client that redeems a code must reach solx-server to use its token anyway,
so a listener the server can't serve gains nothing.

- `POST /auth/exchange`, body `{ "code": "..." }`, returns
  `{ "token", "client_id", "expires_at" }`. Mounted **outside** the bearer
  gate, next to `/health`, and only when the feature is on.
- Failed attempts are rate-limited per peer IP. This route is the one
  unauthenticated, state-changing endpoint, so it must not be brute-forceable.
- The shareable URL is `<public url>/connect#code=<code>`.
  - The code goes in the **fragment**, which browsers never send to the
    server, so it stays out of access logs, proxies and `Referer` headers.
  - `GET /connect` serves a tiny static page whose JS reads the fragment and
    POSTs it to `/auth/exchange`, or just explains what to do.
  - Native clients (CLI, Tauri) parse the fragment themselves and POST
    directly.
- `<public url>` comes from a new config key `server_public_url`, for when the
  bind address isn't what clients should use (`0.0.0.0`, a reverse proxy, a
  Tailscale name). It falls back to the bound address.
- CORS: off loopback, `/auth/exchange` follows the existing
  `server_cors_origins` policy like every other route.

### Client side

- **CLI:** `solx connect <url>`, binary-only like `solx token`. It redeems the
  code, calls `credentials::set_client_token(config, server_url, token)`, and
  sets `server_url`.
- **Desktop app (client build):** a `connect_with_code` command next to
  `connect`, with the same verification step. The Settings panel gets a "Paste
  a connect link" field.
- **Browser (`web/`):** the `/connect` page, or the Settings panel, redeems
  the code and stores the token in `localStorage` as it does today.
- **MCP clients** (e.g. VS Code's `.vscode/mcp.json` prompt): the user
  redeems a code with the CLI and pastes the token. Add `solx connect <url>
  --print` to output the token instead of saving it.

### Built-in actions

Use the plugin pattern `solx-console` uses (`solx_console::actions::plugin`
plus `seed_actions`, wired in `LocalActionManager::open`/`set_self_ref`).
Seed them only when the feature is on.

- `/builtin/auth/create-connect-code` `{ label?, code_ttl_secs?,
  token_ttl_secs? }` returns `{ code, url, expires_at }`.
- `/builtin/auth/list-clients` returns ids, labels and timestamps, never
  hashes.
- `/builtin/auth/revoke-client` `{ id }`.

Add binary-only CLI equivalents for local use:

- `solx token connect-code [--label]`
- `solx token clients`
- `solx token revoke <id>`
- additionally `solx token issue --label` to mint a client token directly.

### Who may call the auth builtins — resolved: explicit plumbing

The registry builtins must refuse anyone holding an action token, and anything
running inside an action. They need the caller's `Principal`, which used to
stop at the HTTP layer. We chose to carry it explicitly (option 2) rather than
in a task-local (option 1), because task-locals don't cross `tokio::spawn` and
`action-start` spawns detached tasks. This is now implemented:

- `ActionManager::exec_for(principal, path, name, params)` is a required trait
  method. `exec` still means "no principal" (CLI, stdio MCP, Tauri).
- REST exec takes `Extension<Principal>` from `require_bearer` and calls
  `exec_for`.
- `/mcp` needed no task-local after all. rmcp's streamable HTTP transport puts
  each request's `http::request::Parts` into `RequestContext::extensions`, so
  `call_tool` reads the `Principal` from there (`request_principal` in
  solx-mcp). Stdio has no `Parts`, so it gets `None`.
- The principal travels through `exec_as`/`exec_as_with` into `InternalCtx`
  and `InternalCallCtx.principal`. Nested frames (WASM, script, `script-exec`)
  **inherit** it for attribution.
- `action-start` moves both the principal **and** the caller into the spawned
  task. `start_invocation` rebuilds the full `Caller` from the calling
  action's row. Dropping the caller would let an action launched by Root
  re-enter as an apparently external Root call.
- Because nested frames inherit the principal, guards must check the caller
  first. They use `InternalCallCtx::external_principal()`, which fails closed:
  it returns `None` if an action is in the chain, if the principal is
  `Principal::Action`, or if there is no principal.
- Tests: `solx_actions::test_probe` (behind the `test-probe` feature, enabled
  only from dev-dependencies) echoes what a handler receives. It covers REST,
  `/mcp`, stdio, nested scripts and detached runs
  (`solx-server/tests/principal.rs`, `solx-actions/src/test_probe.rs`).

Still open: `issue_action_token` hands Command children the root token, so
they authenticate as Root until the registry issues real `Principal::Action`
tokens.

Policy:

| Caller | `create-connect-code` / `revoke-client` |
|---|---|
| `Principal::Root` | allowed |
| `Principal::Client` | allowed (open question: or only clients created with an `admin` flag?) |
| `Principal::Action`, or any in-process action `Caller` | refused |
| No principal (stdio `solx-mcp`, local CLI `exec`) | refused; local users have the binary-only `solx token ...` commands |

Refusing the no-principal case keeps a model on stdio MCP from minting access
for itself.

### Hardening to consider while here

- **Restrict the root token to loopback peers** when the registry is on. The
  middleware gets `ConnectInfo<SocketAddr>`, and a non-loopback peer must use
  a client token. Network-facing servers then never accept the all-powerful
  token from the network. Note that `axum::serve` needs
  `into_make_service_with_connect_info` for this.
- Expire client tokens by default (e.g. 90 days, configurable), and show the
  expiry in `list-clients`.
- Log redemptions and revocations to the console of the auth builtins, never
  with the token or code.

### Follow-on: authorization that the principal makes possible

These aren't part of phase 3. They become possible once real
`Principal::Client`/`Principal::Action` tokens exist, and they should reuse
`external_principal()` so they fail closed in the same way.

- **Gate Command/Webhook saves on every route** (lower priority).
  `guard_executable_action` currently covers only the
  `entity-save-action`/`entity-delete-action` built-ins. HTTP
  `/actions/save` and Tauri `actions-save` are left open because every
  token is root. With the registry, `save`/`delete` of a `command` or
  `webhook` row would be allowed only for `Principal::Root` (and possibly
  `Client` tokens marked `admin`). It would be refused for
  `Principal::Action`, for any call with an action `Caller`, and for no
  principal on a network route. That closes the remaining paths an action
  could use to self-grant, including a Command child calling back with
  `SOLX_SERVER_TOKEN`.
  - The local CLI (and so `install.solx`) still calls `save` directly with
    no principal, and must keep working.
  - Tauri needs a decision of its own. It's in-process with no token, so
    the gate there would be a narrower permission set, for example taking
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

1. **Crate skeleton and feature wiring.**
   - `Registry` with root-token auth only, passing straight through.
   - `build_authority` switch; `App::registry()`.
   - Build both with and without the feature.
2. **Action tokens.**
   - Issue, the revoke-on-drop guard, expiry, the sweep.
   - Test: the token works while the action runs, then gets 401.
3. **Client tokens and storage.**
   - `solx-registry.db` schema and migrations; hash lookup; `last_used_at`
     throttling; revoke.
4. **OTCs and the exchange route.**
   - Mint, consume and expire codes; `POST /auth/exchange`; `GET /connect`;
     rate limiting; `server_public_url`.
5. **Principal plumbing.** Choose option 1 or 2 above, after checking whether
   `/mcp` spawns.
6. **Built-in actions and CLI commands.** The three builtins with the policy
   table, plus `solx token connect-code|clients|revoke`, and `solx connect`.
7. **Clients.** Desktop `connect_with_code`, the web connect page and Settings
   field, docs (`http-api.md`, SECURITY.md, README).
8. **Optional hardening:** root token from loopback peers only, default client
   expiry.

## Tests

Run with and without the feature (`cargo test -p solx-server --features
registry`, and the default build), and call
`credentials::use_memory_store_for_tests()` in every test that builds an
`App`. Tests to write:

- **Exchange:**
  - create-connect-code → exchange → the token works and appears in
    `list-clients`;
  - revoke → 401 on the next request.
- **Codes:** reuse is rejected; expiry is rejected; a burst of wrong codes is
  rate-limited.
- **Action tokens:** valid during the invocation, 401 after it, including when
  the command fails or times out. Detached (`action-start`) runs revoke at
  completion, not when `start` returns.
- **Builtin policy** for each entry point (REST exec, `/mcp` tool call, WASM
  or script caller):
  - an action token calling `create-connect-code` is refused;
  - a client token is allowed;
  - no principal is refused.
- **Revocation across processes:** revoke from a separate CLI process and the
  running server rejects the token at once.
- **Manual:** bind the server on a LAN IP, create a code from the CLI, connect
  from a second machine using only the URL.
