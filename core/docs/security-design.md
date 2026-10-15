# Security design

Status as of 2026-10-14. This describes what solx-core enforces today, and
what it doesn't. It replaces the old `future-security-enhancements.md`,
which predated the `command_actions` and `allowed_base_urls` allowlists, the
outbound-URL gate on the `/builtin/web/*` built-ins, and moving the bearer
token into the OS credential store.

[SECURITY.md](../SECURITY.md) is the short, user-facing version. This
document is for people changing the code. It names the module that owns
each control so a change can be checked against it.

## 1. Trust model

solx runs things when asked: shell commands, outbound HTTP, WASM components,
and `.solx` scripts. Requests come from a CLI, a desktop app, an HTTP server,
or an LLM over MCP. The model rests on four assumptions:

- **A solx instance runs with the user's full OS privileges.** It is
  single-user and single-tenant. It does not sandbox its *operators*.
- **Operators are trusted. Content is not.** A human at the CLI, a holder of
  the root token, and the desktop app's Settings window can do anything. The
  controls below limit what *content* can reach: a model calling MCP tools, a
  package's WASM guest, a script, or a widget.
- **Grants live out of band.** The power to run a shell command or reach a
  host lives in `solx-config.json`, not in the actions database. Content can
  write to the database but cannot edit the config.
- **Deny by default.** An unset or empty allowlist denies everything.

## 2. Entry points and identity

### 2.1 How callers get in

| Entry point | Authenticated? | Enters through | Principal | Notes |
|---|---|---|---|---|
| `solx` CLI (local mode) | No (OS user) | `ActionManager::exec`, plus direct `save` | `None` | Can save Command/Webhook rows. Package installs depend on this. |
| `solx` CLI (remote mode) | Bearer token | `solx-client` → HTTP | Derived on the server | Only loopback URLs get the root token automatically (`credentials::is_loopback_url`). |
| `solx-mcp` over stdio | No (parent process) | `exec` | `None` | |
| `solx-server` REST | Bearer token | `exec_for` | From `require_bearer` | |
| `solx-server` `/mcp` | Bearer token | `exec_for` | From `require_bearer` via `http::request::Parts` | |
| Tauri plugin (desktop app) | No (in-process) | `exec`, direct `save` | `None` | Gated per window by Tauri capabilities (§2.4). |
| Spawned Command child | `SOLX_SERVER_TOKEN` | HTTP | `Root` today | See §3.3. |

`/health` is the only route without authentication. It reports the build
version and commit and nothing about stored data.

### 2.2 Two identities: `Caller` and `Principal`

solx tracks two separate identities. Keep them separate.

- **`Caller`** (`solx_surface::internal_actions::Caller`) is *the action that
  invoked this one*. It is `Some` only on a re-entrant hop:
  - a WASM guest's `action-exec`;
  - a `Script` action's `exec` stage;
  - a `script-exec` run, which uses the synthetic caller
    `/builtin/script/exec` and has no secrets;
  - a detached `action-start` run, whose caller is rebuilt from the calling
    action's row (`invocation.rs` `rebuild_caller`).

  The host builds the `Caller` from a row it has just read. It is never
  parsed from a request or serialized, so a client cannot spoof it. It
  carries the calling action's `action_config.secrets` behind a private
  field. Plugin handlers only ever see the secret-free `CallerInfo`.
- **`Principal`** (`solx_surface::auth::Principal`: `Root`, `Client`,
  `Action`) is *who authenticated the call chain*. It enters only through
  `ActionManager::exec_for`, from a route that has just run
  `TokenAuthority::authenticate`. It is passed explicitly, not through a
  task-local, so a `tokio::spawn` cannot silently drop it. Nested frames
  inherit it. Local entry points have no principal (`None`).

Authorization decisions must use `InternalCallCtx::external_principal()`.
It returns `None` (deny) when there is any action in the chain, when the
principal is itself an action token, or when there is no principal. Without
this check, an action could re-enter with an inherited principal and look
like an external call.

> **Status.** The principal plumbing (`exec_for`, `external_principal`,
> carrying the principal through WASM/script/`action-start`) is in the
> working tree and not yet committed. Nothing authorizes on `Principal` yet.
> It exists for the token registry's auth built-ins (§8). Every accepted
> bearer is still `Principal::Root`.

### 2.3 What identity is used for today

- **Secret scoping.** `get-secret`/`set-secret` resolve against the
  *caller's* `action_config.secrets` (§5.1).
- **Caller-only built-ins.** `console-print`, `action-cancelled`,
  `get-secret` and `set-secret` refuse to run without an action caller.
- **Nothing else.** CLI, MCP and HTTP callers are all treated as "not an
  action". There is no per-entity permission check.

### 2.4 Desktop app (Tauri capabilities)

`solx-tauri-plugin` sorts its commands into permission sets, and the host app
grants them per window:

- `default`: full manager CRUD, `actions-exec`, files, info,
  connect/disconnect.
- `packages`: list the official catalog and install or uninstall packages
  **by name**. The host resolves each name to a release zip and never
  accepts a caller-supplied URL or path.
- `config`: read and edit `solx-config.json`, and install arbitrary package
  zips. This set can edit the allowlists. It must only go to a window that
  never loads package widgets, such as the Settings window.

## 3. Authentication

### 3.1 Tokens and storage (`solx-config/src/credentials.rs`)

- **Root token.** 32 random bytes, base64, generated on first use. It is
  stored in the OS credential manager (Windows Credential Manager, macOS
  Keychain, Secret Service) under service `solx`, with account
  `root:<appdata>`. Because the account is scoped by appdata directory,
  isolated appdata dirs never share a token.
- **Migration.** A legacy `server_token` in `solx-config.json` is moved into
  the credential manager and removed from the file. This happens under the
  config file's cross-process lock, so two hosts starting at once cannot mint
  different tokens.
- **Fallback.** If `token_store: "config"` is set, or no credential manager
  is available (for example, headless Linux with no Secret Service), the
  token stays in the config file and a warning is logged.
- **Client tokens.** These are per `(appdata, server URL)` credential-manager
  entries that a client presents to a remote server.
- **Rotation.** `solx token rotate` replaces the root token. Running hosts
  keep accepting the old token until they restart. `solx token show|rotate`
  are `BinCommand`s, deliberately left out of `Commands`, so a `.solx`
  script (including a package's `install.solx`) cannot run them.
- **Comparison.** `constant_time_eq`. Only a length mismatch returns early,
  and tokens have a fixed length.

### 3.2 HTTP server (`solx-server`)

- **Bind.** `127.0.0.1` by default. The `server_host` config setting or
  `--host` can bind a LAN interface or `0.0.0.0`. Off loopback, the bearer
  check is the only gate and there is no TLS.
- **Auth.** `require_bearer` is a route layer over every data route,
  including `/mcp`. On success it stores the resulting `Principal` in the
  request extensions. On failure it returns a 401 with a JSON
  `unauthorized` body.
- **CORS.** Permissive on a loopback bind, because the bearer token is the
  gate and browser clients such as `@solx/http` need access. On a
  non-loopback bind, only the origins listed in `server_cors_origins` are
  allowed (none if the list is empty).
- **Embedded server.** `spawn_embedded`, used by the CLI so its children can
  call back, always binds loopback.

### 3.3 Credentials handed to Command actions

`exec::run_command` calls `TokenAuthority::issue_action_token` and sets
`SOLX_SERVER_URL` and `SOLX_SERVER_TOKEN` on the child. This is the only
way an action gets a token: action config has no way to set or override
environment variables. `SOLX_SERVER_URL` is set only when the process has
bound an HTTP server (`solx-server`, the CLI's embedded server, or the
desktop app with `server_embedded`). The credential is kept alive until the child exits, so a
revoking guard can be attached (§8).

With today's `SingleTokenAuthority`, the issued token is **the root token**.
Every Command action can call back with full access, and it can keep that
access after it exits.

## 4. Execution controls by action type

Execution always reads the action through `get_unmasked`, so it sees real
secret values. Reads that leave the host are masked (§5.2).

### 4.1 Command: key indirection (`solx-actions/src/exec.rs`)

- `fn_name` is a **key** into `command_actions` in `solx-config.json`, never
  a command line. The `CommandDef.command` registered under that key is what
  runs, via `cmd /C` or `sh -c`.
- An unregistered key is a hard error, including when the allowlist is
  missing entirely. The key text is never run as a fallback. So the actions
  database can only hold *pointers* to definitions a human approved.
- `CommandDef.cwd` takes precedence over `action_config.cwd`. The allowlist
  author decides where an approved command runs.
- Params arrive as JSON on stdin, so they don't appear in process listings.
- **Timeout.** The default is 300 s, overridable with
  `action_config.timeout_secs`. `kill_on_drop` kills the shell but **not its
  descendants**, so a grandchild process can outlive the timeout.
- **Environment.** The child inherits solx's own environment, plus
  `SOL_LOG_DIR`, the console loopback variables (§6), and the server
  credential (§3.3). Nothing in the action row can add a variable. The old
  `action_config.env` map is no longer applied, and a warning is logged if
  a stored row still sets anything other than `SOLX_SERVER_URL` there.

### 4.2 Webhook, and every other outbound request (`solx-actions/src/net.rs`)

All of these share a single check, `check_outbound_url`:

- a Webhook action's resolved URL;
- `/builtin/web/http-request`;
- `/builtin/web/stream/start`;
- `/builtin/web/open-url`.

The check runs before any connection is opened, so a denied request has no
side effect. The rules:

- **Schemes.** Only `http` and `https` are allowed. `file:`, `javascript:`
  and `data:` are refused outright. This matters especially for `open-url`,
  which hands the URL to the user's real browser.
- **Matching.** Entries are matched by origin, not by raw string prefix. The
  URL's scheme, host and port must equal the entry's, compared after parsing
  and normalization. An entry with no port means the scheme's default port.
  An entry like `http://host:` means any port. Only the path part is matched
  as a prefix, after `..` has been resolved. As a result,
  `https://api.example.com` does not admit `…example.com.evil.net` or
  `…example.com@evil.net`.
- **Redirects.** Every redirect hop is checked against the same list
  (`redirect_policy`), up to 10 hops.
- **Legacy key.** The old `allowed_webhook_base_urls` key is still read and
  merged with `allowed_base_urls`.
- **Auth.** Webhook auth (bearer, OAuth refresh-token, service-account JWT,
  authorization-code) is resolved in `auth.rs` from `action_config.auth`,
  which may point to keyring entries or environment variables instead of
  holding values inline (§5.3).

### 4.3 WASM: capability sandbox (`solx-actions/src/wasm/`, `solx-wasm/wit/custom-action.wit`)

- The `custom-action` world imports only `action-exec`, `artifact-read` and
  `logger`. Every sensitive operation has to go through a named action,
  where the controls in this document apply.
- WASI p2 is linked, but with an empty `WasiCtx`: no preopened directories,
  no environment variables, no inherited stdio, no sockets.
- **Preemption.** Epoch interruption makes a busy guest yield. Each
  invocation has a wall-clock ceiling of 300 s by default, which covers
  everything it invokes recursively.
- `artifact-read` goes through `FileStore`, which rejects path traversal.
  It can read **any** path in the file store (§7).

### 4.4 Script

- `Script` actions and `/builtin/script/exec` run `.solx` source through
  `ActionCommandRunner`. That runner's only side-effecting stage is `exec`
  of another action, which carries the caller and principal. Scripts have no
  direct shell, file or network access.
- These scripts are a **different runner** from the one that runs package
  `install.solx` files. Those run against the CLI's `Commands`, which can
  call `save` directly and install packages (§4.6).

### 4.5 No self-granting (`internal/mod.rs` `guard_executable_action`)

`entity-save-action` and `entity-delete-action` refuse to create, modify or
delete `command` and `webhook` rows. These built-ins are the only route to
`save` for an MCP client, a WASM guest, or an in-system script. The check
looks at both the incoming payload *and* the stored row, because `save` is a
merge-upsert. Without the stored-row check, a payload carrying only
`fn_name` could repoint an existing Command row.

Routes that bypass this guard on purpose:

- the CLI (local mode), which package installs depend on;
- HTTP `/actions/save`. A root-token holder can already run Command actions,
  and gating the route would break the CLI in remote mode;
- Tauri `actions-save`, which is in the `default` permission set (§9).

These routes can only *point* at registered `command_actions` keys and
allowed URL prefixes. They cannot create new grants.

### 4.6 Packages (`solx-packages`)

- **Declared grants.** A package's `solx-package.json` declares the
  `command_actions` and `allowed_base_urls` it needs. Install grants exactly
  those *before* running `install.solx`, and records them on the
  `InstalledPackage` row. Uninstall revokes exactly those, except entries
  that another installed package also declares. When two packages declare the
  same entry, the last write wins and a warning names both packages.
- **No review step.** Installing a package is an operator action, and
  whatever the manifest declares is granted. The manifest is the complete
  list of shell commands and hosts a package can reach, so read it first.
- **`install.solx`.** It runs with the full CLI command set: it can save any
  action type, write files, and install other packages.
- **Official catalog** (`catalog.rs`). Packages are installed by name from a
  curated list, never from a caller-supplied URL. Zips come from the latest
  `praeus/solx-packages` GitHub release, or from a local folder when
  `SOLX_PACKAGES_SOURCE` is set. Downloads are checked against the release's
  `SHA256SUMS.txt`. That catches corruption, but **not a compromised
  release**: the checksum file comes from the same place as the zips, and
  nothing is signed.
- **Zip extraction** uses `enclosed_name()` to block zip-slip, and unpacks to
  `<packages_directory>/<name>/`. Uninstall deletes only directories under
  the packages directory.

## 5. Secrets

### 5.1 Encrypted per-action secrets (`secrets.rs`, `internal/secrets.rs`)

- An action that uses `get-secret` or `set-secret` carries a `name → AES-256
  key` map in its own `action_config.secrets`.
- Values are encrypted with AES-256-GCM and stored in the OS credential
  manager.
- Both built-ins resolve against the **caller's** map, return the decrypted
  value, and never return the key. Calling either one without an action
  caller is an error.
- **Self-heal.** A blob that fails to decrypt (wrong key or corrupted data)
  is deleted and reported as "not set".

### 5.2 Masking on read (`mask.rs`)

- `ActionManager::get` and `list` redact `action_config.secrets` and
  `action_config.auth` before returning them to HTTP, MCP and Tauri callers.
- The `auth` redaction works from an allowlist. Only flow metadata (`type`,
  `auth_url`, `token_url`, `redirect_uri`, `scope`), the
  `keyring_service`/`keyring_account` pointers, and `*_env` names stay
  readable. Any new field is masked by default.
- On write, `post` runs `unmask_merge`. Any value sent back as `"***"` is
  restored from the stored row, so a get → edit → save round trip cannot
  destroy a key.
- Masking covers only `secrets` and `auth`. Anything else in
  `action_config` is readable by anyone who can `get` the action, so
  credentials belong in `secrets` or keyring references.

### 5.3 Other credential sources

- **`auth` values** can be inline strings (masked on read), keyring
  references `{keyring_service, keyring_account}`, or `*_env` names that are
  resolved from the in-process env store.
- **The env store** (`internal/env.rs`, `get-env`/`set-env`) is a namespaced,
  in-memory map. At startup it is seeded from two sources: `env_mappings`,
  which copies selected variables out of the real system environment, and
  the persisted `env_vars` from the config file. **It is not scoped to any
  caller.** Anyone who can run built-ins can read or write it, including MCP
  clients, WASM guests (through `action-exec`) and widgets. Treat any value
  mapped into it as readable by all content.

## 6. Loopback capabilities

These are bearer-style: holding the id or token is the only check.

- **Console loopback** (`solx-console/src/loopback.rs`). A single
  process-lifetime listener bound to `127.0.0.1`. Each Command invocation
  gets a random one-shot token (`SOLX_CONSOLE_TOKEN`). The token is valid
  only for that invocation's `/print` and `/cancelled`, and is deregistered
  when the invocation ends.
- **OAuth loopback** (`loopback/oauth.rs`, `internal/oauth.rs`). Started and
  stopped for each sign-in. It binds loopback only and refuses redirect hosts
  other than localhost.
- **HTTP streams and action consoles** are addressed by unguessable ids, with
  no further check on the caller.

## 7. Data access

- **File store** (`solx-files`). Absolute paths and `..` traversal out of
  the files root are refused, on every surface.
- **Tooling exclusions are how MCP access is limited** (`solx-config`
  `ExclusionPolicy`, `internal/exclude.rs`, `solx-mcp/src/server.rs`).
  They're configured in `solx-config.json` with `exclusions.tooling` (per
  entity kind: `actions`, `documents`, `types`), unioned with the legacy
  `tool_exclude`/`mcp_exclude` rules and the `solx:hidden` tag. Over MCP:
  - Excluded actions are left out of discovery, and `call_tool` refuses
    them even when the caller already knows the name.
  - Every `/builtin/*` call is **forced** into `excludeMode: "tooling"`, and
    every spelling of the flag is replaced, so a model can't pass
    `exclude_mode: "none"`. Excluded documents and types are hidden from
    built-in listings and searches, and direct get/save/delete of them is
    refused.
  - **Limit:** the forced scope applies only to `/builtin/*`. A package
    action a model is allowed to call gets its parameters unchanged. If it
    reads entities itself (over HTTP, or through `action-exec` without a
    mode), it isn't held to tooling exclusions. Exclude package actions that
    reach sensitive data.
- **User exclusions** (`exclusions.user`) hide entities from the web UI and
  REST/Tauri clients. This is a convenience, not access control. Built-ins
  called other than through MCP default to mode `none` unless the caller
  asks for one, because package install and uninstall scripts need to see
  everything.
- **`tool_destructive`** marks actions as destructive for clients that
  implement an approval step. It is advisory only and solx does not enforce
  it.
- **`SOLX_MCP_PATH_PREFIX`** sets the default scope of **discovery** only. A
  router call's own `path_prefix` overrides it, and it doesn't restrict
  `call_tool`. Use tooling exclusions for access control.

## 8. Planned: the token registry

[token-registry-plan.md](token-registry-plan.md) (phase 3, not started) adds
an optional `solx-registry` crate behind `build_authority`:

- per-invocation **action tokens**, revoked when the invocation ends through
  the `ActionCredential` guard;
- per-client **client tokens**, obtained by redeeming a one-time code, and
  individually listable and revocable.

Phases 1 and 2 are done: the `TokenAuthority` seam, credential-store storage,
and the `Principal` in request extensions. The principal plumbing in §2.2 is
the groundwork for deciding who may call the auth built-ins.

Once real `Client` and `Action` principals exist, the plan's "Follow-on"
section uses them for two more things:

- gating `command`/`webhook` saves on every route, not just the built-ins
  (gap 7);
- scoping `get-env`/`set-env` (gap 6).

## 9. Known gaps

These are accepted for now. Each one is a candidate for future hardening.

1. **No approval step at execution time.** Any row in the actions database
   runs when called. The allowlists limit *what* a row can point at, not
   *whether* it runs.
2. **One root token for everything.** Every HTTP client, MCP client and
   Command child holds the same token, and a child keeps it after exiting.
   Rotating it breaks every client. Fixed by §8.
3. **No authorization on identity.** `Principal` is carried but never
   checked. CLI, stdio MCP and Tauri are unauthenticated local entry points.
4. **No package signing.** Catalog checksums come from the same release as
   the zips. A local directory or zip is trusted unconditionally, including
   its declared grants.
5. **Unscoped artifact reads.** A WASM guest can read any path in the file
   store.
6. **Unscoped env store.** `get-env`/`set-env` are global by design, but
   they expose `env_mappings` and `env_vars` to all content (§5.3). Scoping
   is planned alongside the token registry, in that plan's "Follow-on"
   section.
7. **Command/Webhook saves are open on HTTP and Tauri.** HTTP
   `/actions/save` and Tauri `actions-save` (in the `default` set, so it
   includes widget windows) can create rows that point at existing grants.
   The guard in §4.5 doesn't apply. Gating these on `Principal` is planned
   in the token registry plan's "Follow-on" section.
8. **Command children inherit solx's environment**, and timeouts don't kill
   process trees (§4.1).
9. **Off-loopback serving has no TLS**, and the bearer token is the only
   gate (§3.2).
10. **Bearer-style loopback ids** for streams, consoles and OAuth (§6).
11. **No rate limiting or audit log** on `solx-server`. Rotation exists but
    only takes effect on restart.
12. **Package actions aren't held to tooling exclusions** (§7). Exclude any
    package action that reads sensitive entities on a model's behalf.

## 10. Rules for changing this code

- Any new path that makes a network request on a caller's behalf calls
  `net::check_outbound_url` before connecting, and uses `redirect_policy`.
- Any new path to `ActionManager::save` that content can reach goes through
  `guard_executable_action`.
- Any new re-entrant execution path passes **both** the caller and the
  principal. Dropping the caller while keeping the principal makes an action
  look like an external call.
- Any built-in that authorizes on identity uses `external_principal()` and
  treats `None` as deny, never as root.
- A handler never returns a `Caller`'s secret keys, or anything derived from
  them other than the decrypted value they protect.
- New credential fields under `action_config.auth` are masked automatically.
  Don't add them to `AUTH_VISIBLE` unless they really aren't secret.
- Anything that can edit `solx-config.json` is operator-level. Don't expose
  it to content.
