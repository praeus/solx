# Scoped tokens — design plan

Status: **In progress.** Steps 1 (the scope types, `solx_surface::scope`),
2 (plumbing: every principal, frame and actor carries its grant) and 3 (the
checks, under `scope_enforcement`, default `audit`) and 4 (packages: manifest
scopes, the own-package `exec`, `solx action scopes`, `solx scopes audit`,
and every package's declared scopes) have landed. Next is step 5, issuing
scoped client tokens, then step 6, enforcing by default. This is the
follow-on that
[token-registry-plan.md](token-registry-plan.md) calls "authorization that the
principal makes possible", and it closes the first gap in SECURITY.md's "What
is not enforced": *client and action tokens aren't scoped*.

Baseline: commit `51442d5` ("Lock keyed actions, confine package scripts and
seal the built-in catalogue"), plus the authorization consolidation that
followed it: `solx_surface::Actor` and the two `authorize_*_change` entry
points (see "Where things stand"). Read [security-design.md](security-design.md)
for the controls this plan sits on top of.

## Goal

Today every client token and every Command action's `SOLX_SERVER_TOKEN` can
do everything the root token can, apart from the auth builtins. Phase 3 of
the registry gave tokens expiry, revocation and attribution, but not
containment.

This plan gives every non-root token, and every action, a **scope set**: a
list of grants such as `read:document`, `write:action:/my/tools/*`,
`write:script` or `exec:/builtin/web/http-request`.

- **Client tokens** get their scopes when they are issued: `issue-client`, or
  a connect code whose scopes the redeemed token inherits.
- **Actions** declare theirs in `action_config.scopes`. That list acts as the
  action's own token: whatever the action calls is checked against it.
- **Command actions** turn that virtual token into a real one: the
  `SOLX_SERVER_TOKEN` a Command child gets carries exactly the action's
  declared scopes.
- **Packages** declare the scopes their actions need in the manifest, the way
  they declare `command_actions` and `allowed_base_urls` today.

## The model: who a call is checked against

Every call is checked against **one** set of scopes: that of the party
making it.

| The call is made by | It is checked against |
|---|---|
| An action (a Script or WASM frame calling something) | That action's `action_config.scopes` |
| A client token, with no action in between | The token's scopes |
| A Command child calling back over REST | Its action token, which carries the action's scopes |
| A local entry point (the CLI, the desktop app, stdio `solx-mcp`) | Nothing: unlimited |

Only the **immediate** caller matters. When a client calls action A and A
calls B, the client needs `exec` on A, and A needs `exec` on B. The client
needs nothing for B. So a package can be held to exactly what it uses: if
`solx-google` declares `["exec:/builtin/web/http-request"]`, its call to
`http-request` passes and a call to `get-env` is refused, whoever started the
chain.

This is the same precedence `Actor::builtin` already uses for `acting`
([actor.rs](../solx-surface/src/actor.rs)): the calling action first, then the
principal. Scopes follow the same path that secrets do today. An action's
secrets belong to that action, and so do its scopes.

Two rules keep this sound:

- **Only root sets scopes.** Declaring `action_config.scopes` locks the
  action the way declaring `secrets` does: only an operator (the CLI, a
  package install, the root token, the desktop app's Settings window) can
  create, change or delete an action that declares scopes
  (`guard_keyed_action`). That protects the action's code and artifact too,
  so content can't swap its own code in under someone else's grant.
- **An action with no scopes can do nothing.** It runs with an *empty* set.
  Content may still save one (with `write:action` and the kind scope), but it
  can't call any action or reach any resource until an operator grants it
  scopes. A Command action with no scopes gets a token that authenticates but
  is refused everything. This is what stops a client from escaping its own
  token: it can save a script, but the script has no scopes.

**Deliberate trade-off: confused deputy.** An action's scopes apply whoever
calls it. A client holding only `exec:/packages/solx-media/*` can run a media
action that writes documents, because an operator gave that action
`write:document:/media/*`. That is the intended semantics: the operator chose
to delegate it. A declared scope should be no wider than the action needs
(`write:document:/media/*`, not `write:document`), and the action's own
parameters must not let a caller aim it elsewhere. SECURITY.md should say so.

## Where things stand

What this plan builds on, all present at the baseline. The paths are the
baseline's: `solx-actions` has since been split into modules, so
`content.rs` is now `guard/content.rs`, `exec.rs` is `exec/command.rs` and
`exec/webhook.rs`, and `secret_config.rs` is `secrets/config.rs`. The scope
checks live in `guard/scope.rs` (`ScopeCheck`).

| Piece | Where | What it does today |
|---|---|---|
| `Principal` | `solx-surface/src/auth.rs` | `Root`, `Client { id, label }`, `Action { action_ref, invocation_id }`. `require_bearer` puts it in the request extensions. |
| `Caller` | `solx-surface/src/internal_actions.rs` | The calling action of a frame, built from its row, secrets included. Scopes go here. |
| `ExecContext` / `ExecContextInfo` | `solx-surface/src/exec_context.rs` | Carries caller, principal and exclusion mode through every frame; `enter` replaces the caller at each Script/WASM frame; `resume` rebuilds it across `action-start`. |
| `Actor` | `solx-surface/src/actor.rs` | Who is changing an action or a file: `Operator`, `Content { acting }`, `Builtin { acting }`. Built by `Actor::for_principal` (REST), `Actor::builtin(&ExecContext)` (built-ins) and the Tauri plugin's `CONTENT` constant (main window). |
| `authorize_action_change` / `authorize_file_change` | `solx-actions/src/content.rs`, reached through `solx_manager::App` | **The only authorization calls** for action saves and deletes and for file writes and deletes, on every surface. They read the stored row and run `check_action_change` (package namespace, keyed actions, locks, Command/Webhook refusal for `Builtin`) or the pinned-artifact check. Operators pass. |
| `enforce_scope` | `solx-actions/src/dispatch.rs` (`exec_as_with_inner`) | Runs on every frame and holds it to the tooling exclusions. Despite its name it has nothing to do with these scopes; rename it `enforce_exclusions`. The exec check goes beside it. |
| `exclude::guard` / `exclude::apply` | `solx-server/src/routes/exclude.rs`; the built-ins' `apply_list`/`apply_*_search` (`solx-actions/src/internal/exclude.rs`); `App::guard_excluded`; MCP `explore_tools` | Exclusion checks on every single-entity call and list narrowing on every list, search and path facet. Resource scopes are applied inside these helpers. |
| `include_paths` | `ListOptions`, `SearchQuery`, `ActionSearchQuery` in `solx-surface/src/query.rs` | Patterns combined with OR, then with AND against the rest of the query, at every SQL render site. Caller-settable today but unused outside tests; it becomes the server-set scope filter. |
| `issue_action_token` | `solx-surface/src/auth.rs` trait; `solx-registry/src/lib.rs`; called from `run_command` in `solx-actions/src/exec.rs` | Issues and releases per-invocation tokens, stored in `solx-registry.db`'s `tokens` table. |
| `guard_keyed_action` / `secret_config::normalize` | `solx-actions/src/secret_config.rs` | Refuses content creating or changing an action with `secrets`; validates `action_config` on save. `scopes` joins both. |
| Package grants | `solx-packages/src/lib.rs` (`Grants`, `InstalledPackage`) | Grants and revokes `command_actions`/`allowed_base_urls` from the manifest; records them on the installed-package row. |
| **`scope`** (step 1) | `solx-surface/src/scope.rs` | `Scope`, `ScopeSet`, `Need`, `Grant`, `PathFilter`, `Preset`, `declared_scopes`. Pure, unit-tested, not yet called. |

## Scope grammar

```
scope          := resource-scope | kind-scope | exec-scope | "all"
resource-scope := ("read" | "write") ":" resource [":" path]
resource       := "document" | "type" | "action" | "file"
kind-scope     := "write:" ("script" | "wasm" | "command" | "webhook")
exec-scope     := "exec:" path
path           := "/" [segments] ["/*"]
```

### Paths

A scope path matches exactly the way an `include_paths` pattern does
(`query::exclude_pattern_matches`): it admits an entity when it matches the
entity's **folder** or its **full reference** (`<path>/<name>`). There are
two forms:

| Scope path | Admits |
|---|---|
| `/path/name` | the entity `name` in `/path`, and every entity directly in the folder `/path/name` (not deeper) |
| `/path/*` | everything in the folder `/path` and beneath it (but not an entity *named* `path` in `/`) |
| `/*`, or no path | everything |
| `/` | the entities directly in the root folder |

So `exec:/builtin/web/http-request` admits that one action, and
`exec:/builtin/web/*` admits every web built-in, `/builtin/web/stream/start`
included. `exec:/builtin/web` admits `http-request` but not `stream/start`,
which is a folder deeper.

- **`*` only as a final `/*` segment.** `/a*`, `/*/a` and `/a/**` are
  rejected, and so are `?` and `[`. So whether one scope covers another is a
  string test (`/a/*` covers `/a/b` and `/a/b/*`; a path without `*` covers
  only itself), and a scope path is an `include_paths` pattern as it stands.
  Allowing a general trailing `*` later wouldn't change what existing scopes
  mean.
- **Case-sensitive**, like the stores' SQL `GLOB`. Dispatch checks the stored
  row's reference, so `/Builtin/...` can't alias `/builtin/...`.
- **Paths are normalised** with `normalize_path` (`//a//*` is `/a/*`) and
  must start with `/`.
- **Files.** The path is matched against the file's relative path under the
  files root, split into folder and file name (`Need::file`). A file scope
  path may not end a segment in `.`: Windows drops a trailing dot, so
  `/files/docs.` would reach `files/docs`. Windows also matches case
  insensitively, so `FILES/docs/x` names the same file as `files/docs/x`;
  that is refused by a scope on `/files/docs/*`, which fails closed.
- **Glob characters in entity paths.** `normalize_path` allows `*`, `?` and
  `[` in a segment. Such an entity can only be reached through an ancestor's
  `/*` scope.

### Other rules

- **Scopes don't imply each other.** `write:document` doesn't imply
  `read:document`, and `exec:/*` doesn't imply `write:action`. Presets cover
  the common bundles, so the grammar can stay orthogonal.
- **`all`** is the legacy "everything a client token can do today". It exists
  so the migration and the connect flow have a name for it. It never includes
  the root-only operations (see "What stays root-only").
- **Parsing is strict.** An unknown verb, resource or kind is an error at
  issue, save or install time, never a silent no-op.
- **A set is stored sorted and deduplicated**, as a JSON array of strings.
  Scopes another one covers are kept, so a listing shows what was granted.

### Kind scopes

Saving or deleting an action needs `write:action` (with a path that admits
the action) **and**, for each of the incoming and the stored `action_type`,
the matching kind scope:

| Change | Needs |
|---|---|
| Create a Script action under `/my/tools` | `write:action:/my/tools/*` and `write:script` |
| Change a Script action into a Command action | `write:action`, `write:script` and `write:command` |
| Delete a Webhook action | `write:action` and `write:webhook` |
| Edit only the description of a WASM action | `write:action` and `write:wasm` |

The stored row is checked as well as the payload, because `save` is a
merge-upsert; `guard_executable_action` reads the existing row for the same
reason. `Internal` rows need no kind scope (built-ins are sealed in the
store), and neither does a row with no `action_type`, which can't run.

Kind scopes are safe to grant because a saved action has no scopes until an
operator gives it some: content can write a script, but the script can't
call anything. `write:command` can only point an action at a key already in
the `command_actions` allowlist.

### Presets

The CLI, the desktop app and the web UI offer named bundles. They are
expanded into explicit scopes **when the token is issued**, so changing a
preset later never widens an existing token.

| Preset | Expands to |
|---|---|
| `read-only` | `read:document`, `read:type`, `read:action`, `read:file` |
| `tools` | `read:action`, `exec:/*` |
| `editor` | `read-only`, plus `write:document`, `write:type`, `write:file` |
| `full` | `all` |

## Where scopes live

- **`solx-surface/src/scope.rs`** (step 1, done): `Scope`, `ScopeSet` (parse,
  display, serde, `grants`, `covers`, `path_filter`), `Need`, `PathFilter`,
  `Grant` (`Unlimited` | `Scoped`), `Preset` and `declared_scopes`.
- **`Principal`** (step 2, done): `Client` and `Action` carry `grant: Grant`,
  and `Principal::grant()` returns it (`Unlimited` for `Root`).
- **`Caller`** (step 2, done): carries its action's grant
  (`Caller::with_grant(Grant::for_action(config))`), set by dispatch for
  Script and WASM frames and by `rebuild_caller` for a detached run. A frame
  built without one, or for an action that declares nothing, has
  `Grant::none()`. `CallerInfo` carries it too: scopes aren't secret, so
  pluggable handlers may see them.
- **`ExecContext`** (step 2, done): gains the *entry* grant, fixed where the
  chain enters: the token's for `for_principal` and `tooling(Some(..))`,
  `Unlimited` for `local()` and `tooling(None)`. `ExecContext::grant()` (and
  `ExecContextInfo::grant()`) returns the caller's grant when there is a
  caller, and the entry grant otherwise. That is "who the call is checked
  against".
- **`Actor`** (step 2, done): `Content` and `Builtin` carry `grant: &Grant`,
  and `Actor::grant()` returns it (`UNLIMITED` for `Operator`).
  `Actor::for_principal` takes it from the token, `Actor::builtin` from
  `ExecContext::grant()`, and the Tauri `CONTENT` constant is
  `Actor::LOCAL_CONTENT` (the desktop app is unlimited).
- **`solx-registry.db`** (step 2, done): `tokens.scopes TEXT` and
  `connect_codes.scopes TEXT`, holding a JSON array, added as nullable on
  open. A `NULL` row means `all` (`Grant::from_stored(None)`), so existing
  client tokens keep working; a value that no longer parses grants nothing.
  `redeem` copies a code's scopes onto its token, action tokens store the
  action's declared scopes, and `list-clients` returns each token's scopes.
- **`action_config.scopes`**: a list of scope strings, validated and stored
  canonical by `secrets::config::normalize`, beside `secrets`. Anything but a
  list of valid scope strings is refused.

## What a scope check covers

### Where the checks plug in

**Rule: no new per-surface checks.** Every surface already names an `Actor`
and calls one `authorize_*` entry point or one exclusion helper. Scopes
extend those, never the call sites.

1. **Action and file writes go through the existing entry points.** The
   resource check (`write:action`, `write:file`) and the kind check go inside
   `check_action_change` and `authorize_file_change`
   ([solx-actions/src/content.rs](../solx-actions/src/content.rs)), against
   the actor's grant. `guard_keyed_action` refuses `scopes` the way it refuses
   `secrets`. No route, built-in or Tauri command changes.
2. **Documents, types and every read go through the exclusion helpers.**
   Every single-entity read and write and every list, search and path-facet
   call already runs one: REST `exclude::guard`/`apply`/`apply_search`, the
   built-ins' `apply_list`/`apply_doc_search`/`apply_action_search`,
   `App::guard_excluded`, and MCP `explore_tools`. Each gains an `actor`
   argument and applies the exclusions and the scopes together:
   - **Single entity** (the `guard` helpers): the read or write scope for its
     resource must grant the entity (`Grant::grants`), or 403.
   - **Lists, searches and path facets** (the `apply` helpers): the query is
     narrowed through `include_paths` (see "Narrowing lists").

   This is a mechanical signature change at the existing call sites, not a
   new call beside each one.
3. **Exec is checked in one place:** dispatch (below).
4. **Refusals return 403.** Add `SolxError::Forbidden`, serialised as
   `{"kind":"forbidden", ...}`, so a client can tell "bad token" (401) from
   "not allowed" (403). Map it in `solx-client` and the JS `@solx/http`
   error handling. The existing guards keep their current errors.

Over MCP everything is an action exec, so the built-ins are where most checks
land. They reach the same entry points with `Actor::builtin`.

| Surface | Read | Write |
|---|---|---|
| `GET /docs*`, `/search`, `entity-get/list-document*`, `search-documents`, `get-field*` | `read:document` | |
| `PUT/DELETE /docs/*`, `entity-save/delete-document`, `set-field*` | | `write:document` |
| `/types*`, `/validate`, `entity-*-type*` | `read:type` | `write:type` |
| `GET /actions*`, `/actions-search`, `entity-get/list-action*`, `search-actions`, MCP `explore_tools` | `read:action` | |
| `PUT/DELETE /actions/*`, `entity-save/delete-action` | | `write:action` + kind |
| `GET /files*`, `file-get`, `file-list` | `read:file` | |
| `PUT/DELETE /files/*`, `file-put/delete/copy`, `dir-copy/delete` | | `write:file` |

`file-copy` and `dir-copy` need `read:file` on the source and `write:file` on
the destination.

### Narrowing lists

Lists, searches and path facets are narrowed with the existing
`include_paths` field on `ListOptions`, `SearchQuery` and `ActionSearchQuery`.
It becomes **server-set only**:

- **Callers can't set it.** The field gets `#[serde(skip)]`, so neither a REST
  query string, a built-in's params nor a remote `solx-client` request can
  supply it. Nothing outside tests sets it today. Remove it from:
  - the built-in `ListParams` schema (`solx-types/src/seed.rs`) and the search
    params schemas that list it;
  - the JS `@solx/surface` query type (`js/packages/surface/src/query.ts`) and
    its test in `js/packages/http/test/http.test.ts`;
  - the `includePaths=` part of `solx-server/tests/integration.rs`, which
    becomes a scope test.
- **The exclusion helper writes it** from `Grant::path_filter`:
  - `Unrestricted`: nothing is written;
  - `Only(patterns)`: the scope paths, written as they stand, since they
    already follow the `include_paths` rule;
  - `Nothing`: the call is refused with 403 **before the query runs**. An
    empty `include_paths` means *unrestricted*, so writing nothing here would
    fail open. `PathFilter` keeps the two cases apart by type.
- **Trusted Rust code may still set the field in-process.** If it did and the
  grant is limited, the helper can't intersect two OR-groups in one `Vec`, so
  it refuses the call rather than overwrite the caller's filter.
- **It narrows, never widens.** The patterns are combined with OR among
  themselves, then with AND against everything else: `pathPrefix`, the
  exclusions, the caller's filters. A caller's `pathPrefix=/elsewhere`
  returns nothing rather than escaping the scope.
- **No new SQL.** `ListOptions::to_sql` (list and path facets for every
  store), document search (`solx-docs/src/lib.rs`) and action search
  (`solx-actions/src/store.rs`) already render `include_paths`. The step 1
  tests check that `ScopeSet::grants` and the `include_paths` matcher agree.
- **Files** have no SQL list. `FileStore::list(prefix)` results are filtered
  in Rust with `Grant::grants(Need::file(..))`, in the file built-ins and REST
  `/files`.

Two limits to state in SECURITY.md:

- **Links.** Reading a document returns the references it links to, which may
  be outside the token's scope, never the linked documents' contents.
- **Path facets** report only folders that hold an admitted entity, so a scope
  never reveals the folders around it.

### Exec scopes: checked at dispatch, on every frame

`LocalActionManager::exec_as_with_inner`
([solx-actions/src/dispatch.rs](../solx-actions/src/dispatch.rs)) already
runs the tooling-exclusion check on every frame. The exec check goes right
beside it, against whoever is making this call:

```rust
let (action, secrets) = self.get_with_secrets(path, name).await?;
require(ctx.grant(), Need::Exec { path: &action.path, name: &action.name })?; // new
let params = self.enforce_exclusions(ctx.exclude_mode(), &action, params)?;
```

`ctx.grant()` is the calling action's scopes on a nested frame and the
token's on the first, so the client's call into A is checked against the
client, and A's call into B against A.

**A builtin needs both its exec scope and its resource scope.** Calling
`entity-get-document` needs `exec:/builtin/document/entity-get-document` (or
`exec:/builtin/document/*`, or `exec:/*`) and `read:document`. So `exec:/*`
really does admit every action, but data access stays governed by the
resource scopes. That keeps one grant from silently being root.

Builtins with no resource behind them (`/builtin/web/*`, `/builtin/env/*`,
`/builtin/oauth/*`, `/builtin/secrets/*`, consoles, `action-start`) are
governed by their exec path alone. `get-secret`/`set-secret` stay scoped to
the calling action as they are now; the exec scope only decides whether the
call is reachable.

**Discovery follows exec.** `explore_tools` (in
[solx-mcp/src/server.rs](../solx-mcp/src/server.rs)) drops actions the token
can't exec, so a model is never shown a tool it then gets refused. The
exclusion policy does the same thing for tooling today. `/actions-search` is
an ordinary action list, narrowed by `read:action`.

### Action tokens

`TokenAuthority::issue_action_token` gains a `scopes: &ScopeSet` argument.
`run_command` ([solx-actions/src/exec.rs](../solx-actions/src/exec.rs)) passes
the action's declared scopes, or an empty set when it declares none.
`Registry` stores them on the token row, and `authenticate` returns them on
`Principal::Action`.

`SingleTokenAuthority` (the registry feature turned off) hands every Command
child the root token, so **scopes need the registry**. Document that, and
have the single-token authority refuse to issue a credential for an action
that declares scopes, instead of handing it root. A package action's implicit
`exec:/packages/<name>/*` (see "Packages") isn't a declaration: an action
whose scopes are at most that still gets the root token, as before.

### Closing the `X-Solx-Principal: none` hole

`exec_action` in
[solx-server/src/routes/actions.rs](../solx-server/src/routes/actions.rs)
turns `X-Solx-Principal: none` into `actions.exec(...)`, which is
`ExecContext::local()`. Today that can only *drop* privileges. Once
`local()` means unlimited, **a client token sending that header would escape
its scopes.**

The same goes for `X-Solx-Principal: none` with `X-Solx-Scope: tooling`,
which reaches `exec_tooling(None)`: also a local, unlimited context.

The fix (step 2, done): the route honours the header **for the root token
only**. Only root has anything to drop: every other token is already
refused by the auth builtins, so keeping its principal costs it nothing and
keeps its scopes. `solx_server`'s `only_root_can_drop_its_principal` test
covers both header combinations.

## What stays root-only

Scopes never grant these. They remain `Principal::Root` or an operator entry
point:

- the auth builtins: creating codes, issuing, listing and revoking clients;
- locking and unlocking, changing a locked action, replacing a pinned
  artifact;
- creating, changing or deleting an action that declares `secrets` **or
  `scopes`** (`guard_keyed_action`);
- `command_actions` and `allowed_base_urls`: the allowlists stay in config. A
  scope can let a token use an allowlisted command or URL, never add one;
- writes under `/packages/<other>/`. `guard_package_namespace` still applies
  on top of scopes.

The built-ins' hard refusal of Command and Webhook rows (`Actor::Builtin` in
`check_action_change`) stays for in-process content: MCP tool calls, WASM
guests and scripts. `write:command` and `write:webhook` take effect for
`Actor::Content` on REST, which is what `solx-mcp-actions` uses. That also
closes known gap 7: REST `/actions` save of a Command row by a client token,
which is allowed today.

## Issuing scoped client tokens

- **Builtins.** `/builtin/auth/issue-client` and `create-connect-code` take
  `scopes: [string]` and `preset: string`, expanded and validated with
  `ScopeSet::expand` before anything is stored. A code stores its scopes, and
  `redeem` copies them onto the token.
- **CLI.** `solx token issue --label laptop --scope read:document --scope
  exec:/packages/solx-google/*` and `solx token connect-code --preset tools`.
  `--scope` can be repeated.
- **Granting an action scopes** (done in step 4). Operators set
  `action_config.scopes` in an install script, or with `solx action scopes
  <ref>`.
- **Desktop and web.** "Connect another device" gets a preset picker and an
  "advanced" list of scope strings.
- **Default when nothing is given: `full`** (`all`). This keeps today's
  connect flow and scripts working unchanged. A narrower default may come
  later (see "Future work").
- **Introspection.** Add `GET /auth/whoami`, which returns
  `{ kind, id, label, scopes }`, so clients and the UI can show what a token
  can do and grey out what it can't.

## Packages

### Manifest and install

- `solx-package.json` gains `scopes: [string]`: the package's ceiling, and
  the complete list a reader reviews before installing. This keeps the
  promise SECURITY.md makes for `command_actions` and `allowed_base_urls`.
- Each action's `action_config.scopes` in `install.solx` must be covered by
  the manifest's `scopes` (`Scope::covers`), or the install fails:
  `exec:/builtin/web/http-request` is covered by `exec:/builtin/web/*`.
  `guard_package_script_action`, which every `save action` in a package
  script already passes, checks each save as it runs, so the error names the
  offending action. An action left from an earlier install, whose stored
  scopes a narrower manifest no longer covers, is a warning in the install's
  output rather than a failure.
- Every package action implicitly has `exec:/packages/<name>/*`, so a package
  can always call its own actions. Its own namespace is already special in
  `guard_package_namespace`. `solx_surface::action_scopes(path, config)` is
  the one place that adds it: dispatch, `action-start` and the Command
  token all use it. `/packages/<name>` is matched exactly, like every scope
  path, so `/Packages/p/x` belongs to no package.
- `install_package` ([solx-packages/src/lib.rs](../solx-packages/src/lib.rs))
  records the canonical list on `InstalledPackage::granted_scopes` (as
  strings, so a record survives a later grammar). It's in the install
  output next to the command and URL grants. An uninstall script is held to
  the recorded list.
- An operator may still grant a package action more than its manifest lists,
  with `solx action scopes`. That's an operator decision. The next install
  that re-saves the action replaces it, and one that doesn't warns.

### Callback inventory

What packages did with `SOLX_SERVER_TOKEN` when this plan was written, found
by grepping `solx-packages` for the env var and the REST routes. The scopes
each package now declares, and the choices behind them, are in "Rollout",
step 4.

| Package | Calls | Scopes it needs |
|---|---|---|
| `solx-media` | `PUT /docs/...` (extracted media documents) | `write:document:<its output path>/*` |
| `solx-omniparse` | `PUT /docs/...`, references `/types/docs/Document` | `write:document:<output path>/*`, `read:type` |
| `solx-mcp-actions` | `PUT/DELETE /types/...`, `PUT/DELETE /actions/...` (imports MCP tools as Command actions in its own namespace) | `write:type:<its types path>/*`, `write:action:/packages/solx-mcp-actions/*`, `write:command` |
| `solx-quickjs` | `GET/PUT /files/...` (JS source in, WASM artifact out), `PUT /actions/...` (saves the built WASM action) | `build-javascript-action`: `read:file`, `write:file:<its artifact area>/*`, `write:action`, `write:wasm`. `build-javascript-file`: `read:file`, `write:file:<its staging area>/*` |
| `solx-agent` | `POST /actions/...` with `grant: [{path: "*"}]` | `exec:` for the tools it may run; see below |
| `solx-inquiry` | `POST /actions/...` | as for `solx-agent` |

**Built actions are inert until granted.** The quickjs builder saves its WASM
action with its own action token, which is content, so it can't set
`scopes`. The new action runs with no scopes until an operator grants them
(`solx action scopes ...`). The builder's own `write:action` makes it a
deputy for anyone who can exec it, but what it saves can't do anything on
its own, so that is low risk. Drop the plan's earlier `scopes` build
parameter.

**Delegating tools.** `solx-agent` runs whatever tools its model picks, so
its fixed scopes are what any caller of it gets; it was given broad scopes so
those tools keep working. `solx-inquiry` turned out to call a fixed,
read-mostly set. SECURITY.md must say that granting a client `exec` on either
grants their scopes.

**WASM and Script packages** (`solx-google`, `solx-ollama`,
`solx-livejournal`, `solx-reddit`, `solx-facebook`, ...) don't hold a token.
They make in-process nested calls (`http-request`, `get-secret`, `oauth-*`,
...), which are checked against their declared scopes. The plan was to
collect these under audit, but most of these packages need accounts to run,
so their scopes were read from each guest's source instead. Audit mode
remains the way to find what an operator's own actions need.

### Audit mode

A config key, `scope_enforcement: "off" | "audit" | "enforce"`:

- **`off`**: checks run but always pass. This is the state right after the
  code lands.
- **`audit`** (the default): a failed check is allowed but logged once per
  `(principal or calling action, need)`, naming the narrowest scope that
  would grant it. The log goes to `tracing` (target `solx::scopes`, level
  warn), which the hosts print only when asked: `RUST_LOG=solx::scopes=warn`.
  Each finding is also appended as a JSON line to
  `<appdata>/logs/scope-audit.jsonl`, which every host on the machine shares
  (rotated to `scope-audit.old.jsonl` at 4 MB). A list records what it
  would have been narrowed to, or that it would have been refused. A plain
  file rather than a table in `solx-registry.db`: findings come from every
  process, including the CLI, and the registry is an optional feature.
  `solx scopes audit [--action <ref>] [--json] [--clear]` groups the
  findings by party. For an action it prints the `solx action scopes <ref>
  --add ...` that would allow them (lists excepted: their paths are a
  choice), and the package's manifest when it's a package action. It drops
  what the action's current scopes already grant. It reads this machine's
  log only, so it's run on the solx host.
  Running an action's normal use under audit gives the exact scopes to
  grant it, or to paste into a package's manifest.
- **`enforce`**: a failed check is refused with 403.

It is read live through `ConfigService`, like the allowlists, so no restart
is needed. Because actions with no scopes can do nothing, every existing
action, package or not, needs its scopes declared before `enforce` is the
default; audit mode is how they're found.

## Rollout

1. **Scope types.** ✅ `solx-surface::scope`: grammar, matching, `covers`,
   `path_filter`, presets, `Grant` and `declared_scopes`, with unit tests. No
   behaviour change.
2. **Plumbing.** ✅
   - `Principal` gets its grant; `Caller` gets its grant from its row;
     `ExecContext` gets the entry grant and `grant()`.
   - `Actor` gets the grant; `Actor::for_principal`, `Actor::builtin` and the
     Tauri `CONTENT` constant fill it in.
   - Registry columns and migration, scopes on `issue_action_token`; the
     single-token authority issues no credential to an action that declares
     scopes.
   - The `X-Solx-Principal: none` fix.
   - Nothing checks a grant yet, so behaviour is unchanged. The
     `scope_enforcement` key arrives with the checks in step 3.
3. **Checks.** ✅
   - `scopes` validated and stored canonical in `normalize`. Declaring them
     makes an action keyed (`solx_surface::is_keyed`), so it is root-only
     to create or change (`guard_keyed_action`) and always locked.
   - Every check goes through one type, `solx_actions::ScopeCheck`, built
     from an `ExecContext`, an `Actor` or a REST `Principal`. It applies
     `scope_enforcement` (default `audit`, read live; a value it doesn't
     know enforces).
   - Action and file writes: `write:action` plus the kind scopes, and
     `write:file` on the normalised store path, inside
     `authorize_action_change` and `authorize_file_change`.
   - Exec: in dispatch on every frame, after the exclusions (so an excluded
     action still reads as missing), and up front in `start_invocation`.
     `enforce_scope` is renamed `enforce_exclusions`.
   - Built-ins: a gate in dispatch (`internal::scopes::gate`) checks
     one-entity reads, document and type writes, and refuses a list with no
     grant at all, keeping the `Forbidden` kind. A refusal raised later,
     inside a handler (an action or file write, a `dir-copy` entry), reaches
     the caller as an execution error naming the missing scope, like the
     built-ins' other refusals.
   - REST: `exclude::require_entity` on every single-entity route,
     `exclude::apply`/`apply_search` narrow every list; `/validate` needs
     `read:type` on its type; `/files` filters its listing.
   - `include_paths` is server-set: `#[serde(skip)]`, removed from the
     built-in schemas and the JS query type, written by `ScopeCheck::narrow`;
     a filter that admits nothing is refused with 403.
   - MCP discovery (`explore_tools`) is narrowed by exec scopes alone. Only
     one include group fits in a query, and what a model needs to see is
     what it may call. REST `/actions-search` follows `read:action`, like
     the other action lists.
   - `SolxError::Forbidden` (HTTP 403, `{"kind":"forbidden"}`), mapped in
     MCP tool results, the JS bindings and `@solx/surface`.
   - The desktop app needed no change: its main window is unlimited.
   - Audit findings go to `tracing` (target `solx::scopes`), once per party
     and need, and to the audit log (step 4).
4. **Packages.**
   - ✅ Manifest `scopes`, checked on each `save action` in a package script
     and recorded on `InstalledPackage::granted_scopes`. The implicit
     own-namespace exec (`action_scopes`), which single-token mode doesn't
     count as a declaration.
   - ✅ Brought forward from step 5, so one audit run serves packages and an
     operator's own actions alike: `solx action scopes <ref>` (show, or
     `--set`/`--add`/`--remove`/`--clear`; binary-only, so no package script
     can grant beyond its manifest), the audit log and `solx scopes audit`.
   - ✅ `scopes` in each `solx-packages` manifest and `install.solx`, from
     reading what each action calls (most need accounts to run, so audit
     can't reach them). Webhook actions and Command actions that never call
     back need none, and neither do `solx-git` or `solx-names`. A WASM
     module's actions share the union of what the module calls. Choices
     made:
     - The importers (facebook, livejournal, reddit) write `write:document`
       on any path, because their `path` parameter may name anywhere. Their
       images go to `write:file:/files/docs/shared/*`.
     - `solx-agent`'s loop is broad: `exec:/*`, `read:document`,
       `write:document`, `read:type`, `read:action`. SECURITY.md must say
       that exec on the agent grants these (step 6).
     - `solx-google` and `solx-inquiry` get `read:document` everywhere.
       Inquiry writes only `/inquiry/*`.
     - The rest are exact: media and omniparse write under `/media/`, and
       quickjs reads any file but writes only `/files/actions/shared/*`
       (plus `write:action` and `write:wasm` for `build-javascript-action`).
       mcp-actions writes its own namespace and `write:command`.
     - Verified by installing google, facebook, ollama, omniparse,
       mcp-actions and firefox under `enforce` (every scope parses and is
       covered), and by running ollama's actions under `enforce`.
5. **Issuing and UI.**
   - ✅ `scopes` and `preset` on `issue-client` and `create-connect-code`
     (`Registry::issue_scoped_client`, `create_scoped_connect_code`), both
     returning the resulting `scopes`; neither given means `all`. `--scope`
     (repeatable) and `--preset` on `solx token issue` and `solx token
     connect-code`. `GET /auth/whoami` (`{kind, scopes, scope_enforcement}`,
     `scopes: null` for root), for any token.
   - Desktop and web preset picker, and scopes shown in the client list.
6. **Enforce and document.**
   - Flip the default to `enforce`.
   - Update SECURITY.md (remove the first known gap, add the confused-deputy
     and delegating-tool notes, the registry requirement and the file-case
     note), `security-design.md`, `http-api.md` and the README.

## Tests

All of these exist, in `solx-surface/src/scope.rs`,
`solx-actions/tests/scope.rs`, `solx-actions/tests/command_credentials.rs`,
`solx-server/tests/scopes.rs` and `solx-packages/tests/install_locks.rs`.

- **`scope` unit tests** (step 1, done): parsing and display, strict errors,
  plain vs `/*` paths, case sensitivity, file paths, no implication, `covers`
  (and that it agrees with `grants`), presets, `path_filter` never failing
  open, and `grants` agreeing with the `include_paths` matcher on a table of
  paths.
- **REST** (`solx-server/tests/scopes.rs`, modelled on `registry.rs`), with a
  client token per scope:
  - each route is allowed with its scope and gets 403 without it;
  - lists narrow rather than fail;
  - `write:document:/a/*` can't write `/b/x`.
- **Narrowing** (per store: documents, types, actions; for list, path facets
  and search):
  - `read:document:/path/*` shows everything under `/path` and nothing at
    `/pathology`; `read:document:/path/name` shows that document and the
    folder of that name's direct members;
  - two grants (`/a/*`, `/b/*`) admit both, and nothing else;
  - `pathPrefix=/elsewhere` returns nothing, never more;
  - a caller's `includePaths` / `include_paths` in a query string, a
    built-in's params or a `solx-client` request is ignored;
  - a grant with no `read:document` gets 403 on a list, not an unrestricted
    list;
  - `file-list` and `GET /files` return only admitted files;
  - `X-Solx-Principal: none` from a client token doesn't widen its scopes.
- **MCP:**
  - `explore_tools` shows only actions the token can exec;
  - calling an un-granted action by name is refused;
  - `entity-get-document` with `exec:/*` but no `read:document` is refused.
- **Caller-based checks:**
  - an action declaring only `exec:/builtin/web/http-request` can call it and
    is refused `get-env`, whether a narrow client or the CLI started it;
  - a client holding only `exec` on that action can run it, and the action's
    nested call isn't checked against the client;
  - the same holds through `action-start`;
  - an action with no scopes can't call anything, from any entry point.
- **Action tokens** (extend `solx-actions/tests/command_credentials.rs`):
  - a Command child's token carries the declared scopes, and gets 403
    outside them;
  - with no `scopes` field, it carries none and is refused everything;
  - the single-token authority refuses to issue for an action with scopes.
- **Root-only:**
  - a client token with `write:action` and `write:script` can't save,
    change or delete an action that declares `scopes` or `secrets`;
  - it can save one with neither, which then can't call anything;
  - no scope reaches the auth builtins, lock/unlock, or another package's
    namespace;
  - `write:command` without `write:action` is refused, and so is the
    reverse.
- **Migration:** a pre-existing client token (`scopes` NULL) still works as
  `all`.
- **Packages:** an install whose action declares a scope the manifest
  doesn't cover fails; uninstall leaves nothing behind.

## Decisions

| Question | Decision |
|---|---|
| Who is a call checked against? | Its immediate caller: the calling action's declared scopes, else the token's, else nothing (local). Not the whole chain. |
| Who may set `action_config.scopes`? | Root only. Declaring scopes locks the action like `secrets`. |
| What does an action with no scopes get? | Nothing: an empty set. Content may save one; it's inert until an operator grants it scopes. |
| Is there an `"inherit"` or a chain-wide ceiling? | No. The caller-based check and root-only scopes make them unnecessary. |
| How do scope paths match? | Like `include_paths`: folder or full reference. `/path/name` names an entity (and that folder's direct members); `/path/*` is a subtree. `*` only as a final `/*`. |
| What does a client token issued with no scopes or preset get? | `full` (`all`). A narrower default may come later. |
| Does the desktop app get scopes? | No. Tauri IPC, from both windows, stays unlimited. |
| Does stdio `solx-mcp` get scopes? | Not in this plan; see "Future work". |
| How are lists narrowed? | Through `include_paths`, made server-set only and written from `Grant::path_filter`. No grant is refused with 403. |
| How does a JavaScript action built by quickjs get scopes? | An operator grants them after the build; the builder can't. |

## Future work

- **A narrower default** for client tokens issued without scopes, such as the
  `tools` preset, once the UI has a preset picker.
- **Scopes for stdio `solx-mcp`.** A config key (`mcp_scopes`, or
  `SOLX_MCP_SCOPES`) would run a stdio MCP server under the same grammar.
  That would make `SOLX_MCP_PATH_PREFIX`-style scoping real access control.
- **Delegating tools acting for their caller.** If `solx-agent` needs to be
  held to its caller's scopes rather than its own, a later design could check
  its nested calls against the intersection of the two. Not needed for the
  first cut.
- **A general trailing `*`** (`/path*`) in scope paths, if a use appears.
