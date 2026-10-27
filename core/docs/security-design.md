# Security design

Status as of 2026-10-16. This describes what solx-core enforces today, and
what it doesn't. It replaces the old `future-security-enhancements.md`,
which predated the `command_actions` and `allowed_base_urls` allowlists, the
outbound-URL gate on the `/builtin/web/*` built-ins, moving the bearer
token into the OS credential store, and the token registry.

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
| `solx` CLI (remote mode) | Bearer token | `solx-client` → HTTP | `None` for `exec` (`X-Solx-Principal: none`); derived on the server for `exec_for` | Only loopback URLs get the root token automatically (`credentials::is_loopback_url`). |
| `solx token connect-code\|clients\|revoke\|issue` | No (OS user) | `exec_for(&Root, "/builtin/auth", …)` | `Root` | Trusted entry point (§3.4). Against a running host, Root comes from the loopback root token. |
| `solx-mcp` over stdio | No (parent process) | `exec` | `None` | |
| `solx-server` REST | Bearer token | `exec_for` | From `require_bearer` | Root token from loopback peers only. |
| `solx-server` `/mcp` | Bearer token | `exec_for` | From `require_bearer` via `http::request::Parts` | Same. |
| `solx-server` `/auth/exchange` | No (connect code) | `Registry::redeem` | — | Global failure budget, per-IP backoff (§3.4). |
| Tauri plugin (desktop app) | No (in-process) | `exec`, direct `save` | `None` | Gated per window by Tauri capabilities (§2.4). |
| Tauri `auth_exec` (Settings window) | No (in-process) | `exec_for(&Root, "/builtin/auth", …)` | `Root` | `solx:auth` only; four builtin names only. |
| Spawned Command child | `SOLX_SERVER_TOKEN` | HTTP | `Action` | Per-invocation token, revoked when the child ends (§3.3). |

`/health`, `/auth/exchange` and `/connect` are the only routes without
authentication. `/health` reports the build version and commit and nothing
about stored data; `/connect` is a static page.

### 2.2 Two identities: `Caller` and `Principal`

solx tracks two separate identities. Keep them separate.

- **`Caller`** (`solx_surface::internal_actions::Caller`) is *the action that
  invoked this one*. It is `Some` only on a re-entrant hop:
  - a WASM guest's `action-exec`;
  - a `Script` action's `exec` stage;
  - a detached `action-start` run, whose caller is rebuilt from the calling
    action's row (`invocation.rs` `rebuild_caller`).

  The host builds the `Caller` from a row it has just read. It is never
  parsed from a request or serialized, so a client cannot spoof it. It
  carries the calling action's `ActionSecrets` (its secret names and key)
  behind a private field. Plugin handlers only ever see the secret-free `CallerInfo`.
- **`Principal`** (`solx_surface::auth::Principal`: `Root`, `Client`,
  `Action`) is *who authenticated the call chain*. It enters only through
  `ActionManager::exec_for`, from a route that has just run
  `TokenAuthority::authenticate`. It is passed explicitly, not through a
  task-local, so a `tokio::spawn` cannot silently drop it. Nested frames
  inherit it. Local entry points have no principal (`None`).

Authorization decisions must use `InternalCallCtx::external_principal()`
(`ExecContextInfo::external_principal()`).
It returns `None` (deny) when there is any action in the chain, when the
principal is itself an action token, or when there is no principal. Without
this check, an action could re-enter with an inherited principal and look
like an external call.

With the token registry (§3.4), accepted bearers map to all three
principals: the root token to `Root`, client tokens to `Client`, and
per-invocation action tokens to `Action`.

**`ExecContext`** (`solx_surface::exec_context`) carries both, plus the
chain's `ExcludeMode` (§7), so no re-entrant path can drop one of them. It is
built only where a chain enters (`ExecContext::local`, `for_principal`,
`tooling`). The principal and exclusion mode pass to every frame unchanged.
The caller is replaced only by `ExecContext::enter`, at each frame that runs
as an action (a Script or WASM action). Handlers and detached runs get the
secret-free `ExecContextInfo`. A detached run turns it back into a full
context with `ExecContextInfo::resume`, using the caller rebuilt from its
row.

### 2.3 What identity is used for today

- **Secret scoping.** `get-secret`/`set-secret` resolve against the
  *caller's* secret names and key (§5.1).
- **Actions with secrets.** Only `Root` (or an in-process host) can create or
  change them (§5.2).
- **Caller-only built-ins.** `console-print`, `action-cancelled`,
  `get-secret` and `set-secret` refuse to run without an action caller.
- **The auth built-ins.** `/builtin/auth/*` run only when
  `external_principal()` is `Root` (§3.4).
- **Nothing else.** A `Client` or `Action` principal can do everything
  except the auth built-ins. There is no per-entity permission check.

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
- `auth`: `auth_exec`, which runs the four `/builtin/auth/*` built-ins as
  `Root` (connect codes, client tokens). Granted to the Settings window
  only, for the same reason as `config`. A unit test in the desktop app
  checks the main window's capability holds neither.

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
  check is the only gate.
- **TLS** (`server_tls`, `solx-server/src/tls.rs`; see
  [tls-plan.md](tls-plan.md)). Sources: `"auto"` (the `solx-certs`
  directory `<appdata>/certs`), PEM files, or on Windows a `CurrentUser\My`
  certificate whose CNG key signs each handshake without leaving the key
  store. One port serves both protocols: each connection's first byte is
  peeked, a TLS handshake is served over TLS, and anything else is plain
  HTTP from a loopback peer or a 400 for any other. `SOLX_SERVER_URL` for
  Command children stays `http://127.0.0.1:…`. A `server_tls` that can't be
  served stops the server from starting rather than falling back to plain
  HTTP. The desktop app writes `"auto"` for a network bind on Windows when
  `server_tls` is unset, and provisions the certificates at startup.
- **Key providers.** The CA key and the server key are non-exportable CNG
  keys on Windows: the CA's on the TPM (Microsoft Platform Crypto Provider)
  when there is one, the server's in the software key storage provider,
  since it signs every handshake. A TPM key can't leave the chip, though
  code running as the user can sign with it while it runs. A software KSP
  key's non-exportability is policy, which an administrator or malware
  running as the user can defeat. The file backend (all other platforms)
  keeps PEM keys readable by any process running as the user; the CA key is
  wrapped under a credential-manager key when there is one.
- **Auth.** `require_bearer` is a route layer over every data route,
  including `/mcp`. On success it stores the resulting `Principal` in the
  request extensions. On failure it returns a 401 with a JSON
  `unauthorized` body.
- **Root from loopback only.** With the registry on, `require_bearer`
  refuses `Principal::Root` unless the peer (`ConnectInfo<SocketAddr>`) is a
  loopback address. Without peer information it refuses (fail closed), so
  every host serves with `into_make_service_with_connect_info`. Behind a
  reverse proxy on the same machine, every request comes from loopback and
  this check can't tell them apart; that is documented rather than
  detected.
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
desktop app with `server_embedded`). The credential is kept alive until the
child exits.

With the registry, the token is a fresh **action token**
(`Principal::Action`), valid for the invocation's timeout plus 60 seconds.
`run_command` calls `TokenAuthority::release` once the child is done, on
every path after spawn (success, failure, timeout); detached runs do the
same inside their spawned task, so they release at completion, not when
`action-start` returns. For early returns and panics, the credential's
guard queues its id on drop, without any I/O, and the registry deletes
queued rows at the start of its next database call, so the token is
rejected from the next authentication in that process. A row left by a
crash, or by a process that never touches the registry again, dies at
`expires_at`.

An action token can still do everything except the auth built-ins,
including saving Command rows (gap 7). What it adds is expiry, revocation
and attribution. Without the registry, the token is the root token, which
keeps working after the action exits.

### 3.4 The token registry (`solx-registry`)

Behind the `registry` cargo feature, on by default (solx-manager,
solx-server, solx-mcp and solx-cli; the desktop app's `server` feature lists
it explicitly). `build_authority` returns a `Registry` instead of
`SingleTokenAuthority`.

- **Storage.** `solx-registry.db` in the database dir: `tokens` (client and
  action rows), `connect_codes`, and `registry_state` (the failure counter).
  Only SHA-256 hashes of tokens and codes are stored, and lookups are by
  hash, so no constant-time comparison is needed past the root token.
  Everything is in the database, not process memory, so a revocation in one
  process applies in every other one on its next request, and a sibling
  `solx-server` accepts action tokens minted by stdio `solx-mcp`.
- **Authentication.** The root token first, compared in memory with
  `constant_time_eq`. Then one indexed lookup of unrevoked, unexpired rows.
  No cache. `last_used_at` is written at most once a minute per token.
- **Client tokens** expire after `client_token_ttl_days` (90). Revoking sets
  `revoked_at`. The sweep (on open, and every 10 minutes in long-lived
  hosts) deletes expired and revoked rows and expired or spent codes.
- **Connect codes.** 8 random digits from the OS RNG (about 26.6 bits),
  single use (an atomic `UPDATE … WHERE consumed_at IS NULL AND
  expires_at > now` in an `IMMEDIATE` transaction), 10 minutes by default.
  The main threat is guessing, so there is a **global failure budget**:
  every failed redemption of a well-formed code increments a counter in the
  database, shared by every process and peer, and at 10 every pending code
  is deleted and the counter starts over. Neither a success nor a new code
  resets it. An attacker's odds are about 10 in 10^8 per batch of codes.
  `/auth/exchange` adds a one-second per-IP backoff (`429`) after a failure.
- **Code hashes hide little.** There are only 10^8 codes, so anyone who can
  read `solx-registry.db` can recover a pending one. That reader already has
  local access, and codes live 10 minutes.
- **`/connect`** is static and changes no state. The code rides in the URL
  fragment, which browsers never send, and the page's form only posts it
  when a person clicks.
- **Auth built-ins.** `/builtin/auth/{create-connect-code, list-clients,
  revoke-client, issue-client}`, registered through
  `LocalActionManager::add_plugin`. Each refuses unless
  `external_principal()` is `Root`: client tokens, action tokens, any action
  in the chain, and calls with no principal (stdio MCP, `solx exec`, plain
  Tauri IPC) are refused. The trusted ways in run them with
  `exec_for(&Principal::Root, …)`: the binary-only `solx token …` commands
  (not in `Commands`, so scripts can't reach them) and the Settings window's
  `auth_exec`. `list-clients` never returns tokens or hashes. Redemptions,
  issues and revocations are logged (never the token or code).

## 4. Execution controls by action type

Execution reads the action through `get_with_secrets`, the only path to the
action's key. Nothing that leaves the host can reach the key (§5.2).

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
  `SOL_LOG_DIR`, `SOLX_ACTION_REF` (the `/path/name` of the action being
  run, so one allowlisted command can serve many actions), the console
  loopback variables (§6), and the server credential (§3.3). Nothing in the
  action row can add a variable. The old
  `action_config.env` map is no longer applied, and a warning is logged if
  a stored row still sets anything other than `SOLX_SERVER_URL` there.

### 4.2 Webhook, and every other outbound request (`solx-actions/src/net.rs`)

All of these share a single check, `check_outbound_url`:

- a Webhook action's resolved URL;
- a Webhook's OAuth `auth.token_uri` (refresh-token and service-account);
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
  holding values inline (§5.3). `auth::token_endpoint` checks `token_uri`
  and builds its client with `redirect_policy` *before* any credential is
  resolved, since the request carries the client secret, refresh token or
  signed assertion. The authorization-code exchange posts to the Webhook's
  own URL, which is already checked.

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

- `Script` actions run `.solx` source through `ActionCommandRunner`. That
  runner's only side-effecting stage is `exec` of another action, which
  carries the script's `ExecContext`. Scripts have no direct shell,
  file or network access.
- There is no built-in for running inline script source. The former
  `/builtin/script/exec` was removed: its nested `exec` let an MCP client
  reach actions excluded from tooling. Seeding deletes its stale row.
- These scripts are a **different runner** from the one that runs package
  `install.solx` files. Those run against the CLI's `Commands`, which can
  call `save` directly and install packages (§4.6).

### 4.5 No self-granting (`content.rs` `check_action_change`)

`entity-save-action` and `entity-delete-action` authorize as
`Actor::Builtin`, which refuses to create, modify or delete `command` and
`webhook` rows. These built-ins are the only route to
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

Every other content check is shared. Each surface names its `Actor`
(`solx-surface/src/actor.rs`) and calls
`LocalActionManager::authorize_action_change` (`content.rs`; hosts reach it
through `App`), which reads the stored row and runs `check_action_change`.
The built-ins are always content, whatever the principal; HTTP is content
for everything but the root token (`Actor::for_principal`); Tauri
`actions-save`/`actions-delete` in the main window are content, and the
Settings window's `config_*` commands are the operator. For content it
refuses:

- a save or delete under `/packages/`, except by one of that package's own
  actions within its own `/packages/<name>/` (§4.6);
- a keyed action (§5.2);
- any change to a locked one (§4.5a).

### 4.5b Built-ins come only from the catalogue (`seed.rs`)

`Internal` rows run a native handler by `fn_name`. An `Internal` row saved
at another path would run a built-in under a path that tooling exclusions
(§7) don't list, and an editable built-in could be repointed or
redescribed for later MCP sessions. So:

- `LocalActionManager::save` refuses any `Internal` row, incoming or
  stored, from every caller, operators included.
- Seeding writes every built-in **locked**, and restores its catalogue
  fields over whatever the row holds, on every open.
- Dispatch runs an `Internal` row only if its path, name and `fn_name` are
  in the catalogue (`is_catalogued`).
- At open, `retire_internal_aliases` deletes `Internal` rows outside
  `/builtin`, and clears `trusted` on every row that isn't a built-in.
  `trusted` only marks a seeded built-in for the UI. It isn't on
  `ActionInput`, and `save` always writes `false`.

### 4.5a Locked actions (`lock.rs`)

A Script or WASM action runs whatever its `bin_name` resolves to, as itself —
with its scoped secrets. Locking fixes both the row and the code.

- **Columns.** `locked`, `pin_path` and `pin_sha256` on `actions`. Input can
  set only `locked` (`ActionInput.locked`); the pin is computed by
  `LocalActionManager::save` and surfaced read-only as
  `Action.artifact_pin`.
- **Pinning.** Every save of a locked `script`/`wasm` row resolves its
  artifact (shared, then owned, as dispatch does), refuses if it's missing,
  and stores its path and SHA-256. Re-saving re-pins; unlocking clears it.
  Locked `internal`/`command`/`webhook` rows freeze the row only. Every
  built-in is locked (§4.5b).
- **Keyed implies a lock.** `save` locks any keyed row (one that declares
  `secrets` or whose `auth` has a keyring reference,
  `solx_surface::is_keyed`), and refuses `locked: false` while it is keyed.
  Otherwise an unlocked keyed Script/WASM row would run whatever content
  wrote to its artifact, with its key. At open,
  `lock::lock_keyed_actions` locks and pins keyed rows saved before this
  rule; one whose artifact is missing is locked unpinned.
- **Verification.** `lock::load_artifact` reads a pinned row's artifact from
  `pin_path` only — a same-named shared file can't shadow an owned one — and
  refuses bytes whose hash differs. A locked Script/WASM row with no pin
  doesn't run.
- **Operator paths** may save, delete, lock and unlock a locked row:
  `ActionManager::save`/`delete` called directly (the CLI, package installs,
  REST with `Principal::Root`, Tauri `config_actions_save`/`_delete` in the
  `config` set). Everything else applies `guard_locked_action` (as part of
  `check_action_change`, §4.5), which refuses any change to a
  locked row and any input that sets `locked`:
  `entity-save-action`/`entity-delete-action` always (so an MCP call with
  the root token is refused), REST for any other principal, and Tauri
  `actions_save`/`actions_delete` (the main window). This keys on the entry
  path, not the principal.
- **The CLI.** `save action` is in `Commands`, so a script can run it; it
  refuses `locked`. `solx action lock|unlock` are `BinCommand`s.
- **Pinned files.** `LocalActionManager::authorize_file_change` refuses, for
  content, a write
  or delete of any locked row's `pin_path` (compared case-insensitively,
  ignoring trailing dots and spaces, as Windows opens files). It is applied
  by the file built-ins (each entry of a directory operation checked before
  any is touched), REST `PUT`/`DELETE /files` for any principal but root,
  and Tauri `files_put`/`files_delete`. It only prevents a denial of
  service: the hash check is what stops new code from running.
- **Packages.** An install unlocks the package's own unkeyed actions
  before running `install.solx` (a rebuild may write an artifact
  back over HTTP with an action token, which a pin would refuse), then locks
  everything under `/packages/<name>/` except `unlocked_actions` and records
  the refs on `InstalledPackage.locked_actions`. A failure to lock one
  action doesn't stop the rest from being locked, and then fails the
  install. A keyed action is never unlocked, and is locked
  even if `unlocked_actions` lists it (with a warning). Its artifact must be
  written by the script (`save file`, an operator path) and is re-pinned by
  the script's `save action`, so content gets no window to swap code that
  runs with the key. If `install.solx` fails, what the previous install
  locked is locked again before the error is returned.

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
  action type and write files. While it (or `uninstall.solx`) runs,
  `guard_package_script_action` refuses `save action` and `delete action`
  outside the package's own `/packages/<name>/`, and the script may not
  install or uninstall a package. A package name must be one path segment.
  Together these keep a script from saving an action in another package's
  secret scope (§5.2). Files are not confined: a script can still overwrite
  another package's pinned artifact, which stops that action running but
  never runs new code as it (§4.5a, gap 4).
- **Lock by default.** What an install leaves under `/packages/<name>/` is
  locked (§4.5a).
- **Content can't write under `/packages/`.** `guard_package_namespace`
  (§4.5) refuses content's saves and deletes there. Otherwise content could
  save a row ahead of an install, and the install's merge-upsert `save
  action` would keep the fields it doesn't set, lock the row and record it
  as the package's own. A package's own action, identified by the calling
  action or the action token's `action_ref`, may still manage its own
  `/packages/<name>/` at run time (`solx-mcp-actions` imports tools there).
  The first segment is matched case-insensitively.
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

### 5.1 Encrypted scoped secrets (`secrets.rs`, `internal/secrets.rs`)

- **Scopes.** Every action belongs to one secret scope
  (`solx_surface::secret_scope`). Every action under `/packages/<pkg>/`
  shares `package:<pkg>`; any other action is its own scope,
  `action:<full ref>`. The kind prefix keeps an action at `/packages`
  named `x` apart from package `x`.
- **One key per scope.** Each scope has one AES-256 key, generated by solx
  (`secrets::generate_key`, from the OS RNG) and kept in the `secret_keys`
  table of the actions database. It is created when the scope's first
  action that declares secrets is saved or run (`row::ensure_scope_key`),
  and deleted with the scope's last action (`row::delete_scope_key_if_unused`).
  Uninstalling a package therefore forgets its key; reinstalling upserts
  rows, so the key and the secrets it protects survive. No install script
  handles a key, and a package update can add actions that share it.
- **Names.** An action that uses `get-secret` or `set-secret` lists the
  secret names in its own `action_config.secrets`. The list is an
  allowlist even within a scope: a name the action doesn't list is
  refused.
- **Storage.** Values are encrypted with AES-256-GCM and stored in the OS
  credential manager under the account `<scope>/<name>`. Two scopes never
  share an entry, so one package can't read, overwrite, or (through
  self-heal) delete another's secret, even if both use the same name.
  Names are restricted to letters, digits, `_`, `-` and `.`, so an
  account can't be forged with a `/`.
- Both built-ins resolve against the **caller's** `ActionSecrets` (its
  scope, names and key) through a `SecretHandle`, return the decrypted
  value, and never return the key. Calling either one without an action
  caller is an error.
- **Self-heal.** A blob that fails to decrypt (wrong key or corrupted data)
  is deleted and reported as "not set". It is always the scope's own entry.
- **Why the key isn't in the credential manager.** The encrypted values
  already live there. Keeping the key in the actions database means
  anything that can read the user's credential entries still can't
  decrypt them.

### 5.2 Keeping secrets out of `action_config` (`secret_config.rs`)

- Every save runs `secret_config::normalize`, which stores `secrets` as a
  sorted list of names. The object form that older install scripts write
  (`{"NAME": $key}` or `{"NAME": "***"}`) is accepted for its keys alone.
  Its values are dropped, never stored: no key is ever taken from a
  config.
- Inline credentials at `auth.token`, `client_id`, `client_secret`,
  `refresh_token` or `private_key` are refused on save, and ignored at call
  time if an older row has one. They come from a `<field>_secret` pointer
  or a keyring reference.
- **Older databases.** Keys used to be per action: in an `actions.secret_key`
  column, and before that in `action_config.secrets`. Opening the database
  (`schema::migrate`) drops the column, rewrites old configs to name lists,
  and drops inline credentials with a logged warning
  (`secret_config::scrub_legacy`). No old key is carried over: each scope
  gets a fresh key on first use, so secrets stored before the upgrade read
  as not set and must be set again (log in again).
- So reads need no redaction: `get`, `list`, `search` and `save`'s own
  response return `action_config` as stored. `Action` has no key field, and
  only the exec path reads `secret_keys` (`get_with_secrets`).
- **Root-only.** `secret_config::guard_keyed_action` refuses a save whose
  incoming config has `secrets` or an `auth` keyring reference. It always
  applies to `entity-save-action` (MCP, WASM guests, scripts), to
  `PUT /actions/{ref}` for any principal but `Root`, and to Tauri
  `actions_save` (the main window, which hosts widgets). The CLI, package
  installs and the Settings window call `ActionManager::save` in process
  and are root. The key comes from the path, so declaring a name under
  `/packages/<pkg>/` would read that package's secret. A keyring reference
  reads (and an OAuth rotation writes) whatever entry it names. A keyed
  action is locked (§4.5a), so `guard_locked_action` refuses
  any change to it or its deletion, including repointing `bin_name`/`files`
  to run new code with the key. A root-saved action under another package's
  path would get that package's key, so a package's scripts may save and
  delete actions only under their own `/packages/<name>/` (§4.6,
  `install.solx`). The CLI run directly by the operator is not confined.

### 5.3 Other credential sources

- **`auth` credentials** are keyring references
  `{keyring_service, keyring_account}` or `<field>_secret` pointers (§5.2).
  A keyring reference may not name solx's own services, `solx` (the root
  and client tokens, the local CA) or `sol-secrets` (the scoped store).
  `secrets::check_keyring_service` refuses them on save, on every read, and
  before an OAuth rotation writes a new refresh token back. It compares
  case-insensitively and as a `.`-suffix, as Windows Credential Manager
  matches target names.
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
  - The MCP server runs every call through `ActionManager::exec_tooling`,
    which starts the chain with `ExecContext::tooling` (exclusion mode
    `ExcludeMode::Tooling`). The mode is inherited by every nested frame
    exactly like the principal: a Script's `exec`
    stage, a WASM guest's `action-exec`, and a detached `action-start` run
    (`start_invocation` refuses an excluded target before creating its
    row). A remote `solx-client` carries it as `X-Solx-Scope: tooling`,
    which only ever restricts, so the server honors it from any caller.
  - At every frame, `LocalActionManager::enforce_exclusions` (`dispatch.rs`)
    refuses an excluded action as `NotFound`, and forces every `Internal`
    action's params into `excludeMode: "tooling"`, replacing every spelling
    of the flag, so a model can't pass `exclude_mode: "none"`. This keys on
    the action type, not a `/builtin` path. An exclusion on a built-in's
    path holds because no other row can run its handler (§4.5b). Excluded
    documents and types are hidden from built-in listings and searches, and direct
    get/save/delete of them is refused.
  - `call_tool` also refuses a hidden action up front, so its error reads
    the same as an unknown tool.
  - **Limit:** the scope travels with in-process calls only. A Command or
    Webhook action that calls back into `solx-server` with its own token
    (`SOLX_SERVER_TOKEN`) starts a new, unscoped chain. Exclude such
    actions if they reach sensitive data.
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

## 8. Next: authorizing on the principal

The token registry ([token-registry-plan.md](token-registry-plan.md)) is in
place (§3.4). Now that real `Client` and `Action` principals exist, the
plan's "Follow-on" section uses them for two more things:

- gating `command`/`webhook` saves on every route, not just the built-ins
  (gap 7);
- scoping `get-env`/`set-env` (gap 6).

TLS serving is in place (§3.2), and solx clients pin the server's root
([tls-plan.md](tls-plan.md), "Pinning"). A connect link and an issued
token's `server_url` carry the pin as a URL fragment (`#pin=`), so it
arrives out of band with the credential; a pinned client checks it in the
handshake, before the code or token is sent, and trusts no other root for
that server.

## 9. Known gaps

These are accepted for now. Each one is a candidate for future hardening.

1. **No approval step at execution time.** Any row in the actions database
   runs when called. The allowlists limit *what* a row can point at, not
   *whether* it runs.
2. **Client and action tokens aren't scoped.** They expire and can be
   revoked one by one (§3.4), but each can do everything the root token
   can, apart from the auth built-ins. With the `registry` feature off, every
   client and Command child shares the root token.
3. **Little authorization on identity.** Only the auth built-ins check the
   `Principal`. CLI, stdio MCP and Tauri are unauthenticated local entry
   points.
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
   includes widget windows) can create *unlocked* rows that point at
   existing grants. The guard in §4.5 doesn't apply; a locked row can't be
   changed this way (§4.5a). Gating the rest on `Principal` is planned in
   the token registry plan's "Follow-on" section.
8. **Command children inherit solx's environment**, and timeouts don't kill
   process trees (§4.1).
9. **Off-loopback TLS is opt-in** (automatic in the desktop app on
   Windows); plain HTTP is accepted only from loopback peers (§3.2). A solx
   client given a bare `https://` address and a token, with no pin, trusts
   the built-in roots, the OS trust store and `server_ca_file`/
   `SOLX_CA_FILE`. A typed code with no pin has no trust on first use: it
   needs the root already trusted, or the pinned address.
10. **Bearer-style loopback ids** for streams, consoles and OAuth (§6).
11. **No general rate limiting or audit log** on `solx-server`. Only
    `/auth/exchange` is throttled. Root-token rotation exists but only takes
    effect on restart.
12. **The tooling scope doesn't cross a callback over HTTP** (§7). A
    Command or Webhook action that calls `solx-server` with its own token
    isn't held to tooling exclusions. Exclude any such action that reads
    sensitive entities on a model's behalf.
13. **A same-machine reverse proxy defeats root-from-loopback-only** (§3.2):
    every request reaches the server from loopback. Reverse proxies aren't
    supported; solx-server terminates TLS itself.
14. **Clients that can't pin rely on the OS trust store.** Browsers and
    third-party MCP clients need the solx root imported, and the root has
    no name constraints, so it is then trusted for every host name on that
    machine.
15. **No certificate revocation.** The certificates name no CRL or OCSP
    endpoint. A problem is handled by reissuing the server certificate or
    resetting the CA. Windows Schannel clients that insist on a revocation
    check (curl on Windows) need `--ssl-no-revoke`.

## 10. Rules for changing this code

- Any new path that makes a network request on a caller's behalf calls
  `net::check_outbound_url` before connecting, and uses `redirect_policy`.
- Any new path to `ActionManager::save`/`delete` that content can reach
  calls `authorize_action_change` with the right `Actor`: `Builtin` for a
  built-in, `Content` for a direct route (the gap-7 routes, which may point
  at Command and Webhook rows). Add new checks inside
  `check_action_change`, not at the call sites.
- New built-ins are added to the catalogue (`seed.rs` or a plugin's seed
  list), never saved as rows.
- Any new request to an endpoint taken from `action_config` (like
  `token_uri`) is checked before a credential is resolved for it.
- Any new re-entrant execution path passes the chain's `ExecContext`
  whole, and starts a new frame only with `ExecContext::enter`, never by
  building a fresh context. Dropping the caller while keeping the
  principal makes an action look like an external call; dropping the
  exclusion mode lets a model reach actions excluded from tooling. Only
  `ExcludeMode::Tooling` is enforced along the chain (`User` acts as
  `None`); `enforce_exclusions` matches it exhaustively, so enforcing `User`
  would be a deliberate change.
- Any built-in that authorizes on identity uses `external_principal()` and
  treats `None` as deny, never as root.
- Any new host that serves solx-server's router uses
  `into_make_service_with_connect_info::<SocketAddr>()`, or the root token
  is refused from every peer.
- Never log a token or connect code, and never return a token hash.
- A handler never returns a `Caller`'s secret keys, or anything derived from
  them other than the decrypted value they protect.
- A new credential field under `action_config.auth` is read through
  `auth::secret_field` and added to `auth::CREDENTIAL_FIELDS`, so it is
  never stored inline.
- Nothing outside the exec path reads `secret_keys`, and a key never
  becomes a field of `Action`. Secret-store entries are reached only
  through a `SecretHandle`, never by a bare name.
- Any new path through which content writes or deletes a file calls
  `authorize_file_change`. Only an operator path (`Actor::Operator`, or a
  direct `ActionManager`/`FileStore` call) skips the checks, and only on
  purpose.
- Anything that can edit `solx-config.json` is operator-level. Don't expose
  it to content.
