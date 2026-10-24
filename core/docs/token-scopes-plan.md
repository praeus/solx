# Scoped tokens — design plan

Status: **Proposed** (2026-10-24). Nothing here is implemented yet. This is
the follow-on that [token-registry-plan.md](token-registry-plan.md) calls
"authorization that the principal makes possible", and it closes the first
gap in SECURITY.md's "What is not enforced": *client and action tokens
aren't scoped*.

## Goal

Today every client token and every Command action's `SOLX_SERVER_TOKEN` can
do everything the root token can, apart from the auth builtins. Phase 3
gave tokens expiry, revocation and attribution, but not containment.

This plan gives every non-root token a **scope set**: a list of grants such
as `read:document`, `write:action`, `write:command` or
`exec:/packages/solx-google`. Every route and every action dispatch checks
the call against that set.

- **Client tokens** get their scopes when they are issued: `issue-client`,
  or a connect code whose scopes the redeemed token inherits.
- **Action tokens** get the scopes the action declares in
  `action_config.scopes`. A Command child calling back can do exactly what
  its action asked for, and nothing more. An action that declares
  `"inherit"` instead gets its caller's scopes.
- **Packages** declare the scopes their actions need, the same way they
  declare `command_actions` and `allowed_base_urls` today. Install grants
  them and reports them.

## Scope grammar

```
scope    := resource-scope | kind-scope | exec-scope | "all"
resource-scope := ("read" | "write") ":" resource [":" path]
resource := "document" | "type" | "action" | "file"
kind-scope     := "write:" ("script" | "wasm" | "command" | "webhook")
exec-scope     := "exec:" path
```

- **Paths are segment-prefix matches**, normalised with
  `solx_surface::path::normalize_path`. `exec:/builtin/web` admits
  `/builtin/web/http-request` and `/builtin/web/stream/start` but not
  `/builtin/webby/x`. An exact action reference such as
  `exec:/builtin/web/http-request` admits only that action. `exec:/` admits
  every action.
- **A resource scope with no path means `/`.** `read:document` is
  `read:document:/`. `write:document:/research` limits writes to that
  subtree. For `file`, the path is the rel-path prefix under the files root.
- **Scopes don't imply each other.** `write:document` doesn't imply
  `read:document`, and `exec:/` doesn't imply `write:action`. Presets (below)
  cover the common bundles, so the grammar can stay orthogonal.
- **`all`** is the legacy "everything a client token can do today". It
  exists so the migration and the connect flow have a name for it. It never
  includes the root-only operations (see "What stays root-only").
- Parsing is strict. An unknown verb, resource or kind is an error at
  issue, save or install time, never a silent no-op.

### Kind scopes

Saving or deleting an action needs `write:action` (with a path that covers
the action) **and**, for each of the incoming and the stored `action_type`,
the matching kind scope:

| Change | Needs |
|---|---|
| Create a Script action under `/my/tools` | `write:action:/my/tools` and `write:script` |
| Change a Script action into a Command action | `write:action`, `write:script` and `write:command` |
| Delete a Webhook action | `write:action` and `write:webhook` |
| Edit only the description of a WASM action | `write:action` and `write:wasm` |

The stored row is checked as well as the payload, because `save` is a
merge-upsert. This is the same reason `guard_executable_action` reads the
existing row.

`write:command` is less dangerous than it sounds: a Command `fn_name` can
only name a key that is already in the `command_actions` allowlist. What it
grants is the ability to *point a new action at* an approved command.
`write:script` and `write:wasm` are safe because of the exec ceiling (below):
a script that a token wrote runs with no more than that token's scopes,
since any scopes it declares must be covered by that token's own.

### Presets

The CLI, the desktop app and the web UI offer named bundles. They are
expanded into explicit scopes **when the token is issued**, so changing a
preset later never widens an existing token.

| Preset | Expands to |
|---|---|
| `read-only` | `read:document`, `read:type`, `read:action`, `read:file` |
| `tools` | `read:action`, `exec:/` |
| `editor` | `read-only`, plus `write:document`, `write:type`, `write:file` |
| `full` | `all` |

## What a scope check covers

### Resource scopes: one guard, called from REST and from the builtins

Add a guard next to `exclude::guard`/`exclude::apply`
([solx-server/src/routes/exclude.rs](../solx-server/src/routes/exclude.rs)):

- `scope::require(scopes, Need::Read(Resource::Document, path))` for
  single-entity routes. It returns 403 `{"kind":"forbidden", ...}`, which
  needs a new `SolxError::Forbidden` so a client can tell "bad token" (401)
  from "not allowed" (403).
- `scope::narrow(scopes, Resource::Document, &mut opts)` for list, search
  and path facets. It clamps `opts.path_prefix` to the granted paths, the
  same way exclusions are applied, so a list returns only what the token can
  read instead of failing.

The same calls go inside the entity builtins in
[solx-actions/src/internal/entity.rs](../solx-actions/src/internal/entity.rs)
and `doc_fields.rs`, reading the scopes from `InternalCallCtx`. Over MCP
everything is an action exec, so the builtins are where most checks land.

| Surface | Read | Write |
|---|---|---|
| `GET /docs*`, `/search`, `entity-get/list-document*`, `search-documents`, `get-field*` | `read:document` | |
| `PUT/DELETE /docs/*`, `entity-save/delete-document`, `set-field*` | | `write:document` |
| `/types*`, `/validate`, `entity-*-type*` | `read:type` | `write:type` |
| `GET /actions*`, `/actions-search`, `entity-get/list-action*`, `search-actions`, MCP `explore_tools` | `read:action` | |
| `PUT/DELETE /actions/*`, `entity-save/delete-action` | | `write:action` + kind |
| `GET /files*`, `file-get`, `file-list` | `read:file` | |
| `PUT/DELETE /files/*`, `file-put/delete/copy`, `dir-copy/delete` | | `write:file` |

`file-copy` and `dir-copy` need `read:file` on the source and `write:file`
on the destination.

### Exec scopes: checked at dispatch, on every frame

`LocalActionManager::exec_as_with_inner`
([solx-actions/src/dispatch.rs](../solx-actions/src/dispatch.rs)) already
runs `enforce_scope` on every frame for the tooling exclusions. The exec
check goes right beside it:

```rust
let action_ref = full_ref(&action.path, &action.name)?;
ctx.ceiling().require_exec(&action_ref)?;            // new
let params = self.enforce_scope(ctx.exclude_mode(), &action, params)?;
```

Because it runs on every frame, a Script, WASM or `action-start` call is
checked too, not just the action the client named. That is the same reason
`enforce_scope` runs on every frame.

**A builtin needs both its exec scope and its resource scope.** For example,
calling `entity-get-document` over MCP needs
`exec:/builtin/document/entity-get-document` (or a prefix of it) and
`read:document`. So `exec:/` really does admit every action, but data access
stays governed by the resource scopes. That keeps one grant (`exec:/`) from
silently being root.

Builtins with no resource behind them (`/builtin/web/*`, `/builtin/env/*`,
`/builtin/oauth/*`, `/builtin/secrets/*`, consoles, `action-start`) are
governed by their exec path alone. `get-secret`/`set-secret` stay scoped to
the calling action as they are now. The exec scope only decides whether the
call is reachable.

**Discovery follows exec.** `explore_tools` (in
[solx-mcp/src/server.rs](../solx-mcp/src/server.rs)) and `/actions-search`
drop actions the token can't exec, so a model is never shown a tool that it
then gets refused. The exclusion policy does the same thing for tooling
today.

### The ceiling: what a call chain may do

`ExecContext`
([solx-surface/src/exec_context.rs](../solx-surface/src/exec_context.rs))
gains a fourth fact, `ceiling: Ceiling`. Like the others it is fixed at
entry and carried through every path that re-enters, which is the point of
the bundle.

- **At entry.**
  - `for_principal(Client)`: the token's scopes.
  - `for_principal(Action)`: the scopes stored on the action token (see
    "Action tokens").
  - `Root` and `local()`: `Ceiling::Unlimited`. The desktop app (Tauri IPC,
    from either the main or the Settings window) stays `local()` with no
    ceiling. That is a decision, not a gap. Stdio `solx-mcp` also stays
    `local()` for now (see "Future work").
- **On `enter(frame)`**, by the frame's `action_config.scopes`:

  | `action_config.scopes` | Ceiling inside the frame |
  |---|---|
  | A list of scopes | Exactly that list. The action runs with what it asked for, whoever calls it, like a setuid binary. |
  | `"inherit"` | The caller's ceiling, unchanged. This is for delegating tools, such as an agent loop or a builder that saves actions for its caller. |
  | Absent | Inherited during the rollout. **Empty** after rollout step 4, so an action that never declared anything can't call anything. |

- **Through `action-start`.** `ExecContextInfo::resume` carries the ceiling
  across the `tokio::spawn`, the same way it carries the principal and the
  exclusion mode.

### Declaring scopes is delegation

A fixed list only stays sound if nobody can declare more than they already
hold. Otherwise content could save an action that declares `exec:/` and run
it to escalate. So declaring scopes is a **delegation**, checked against the
writer's own ceiling:

- **Declaring or changing.** A save that sets or changes
  `action_config.scopes` to a list needs every scope in it to be covered by
  the caller's ceiling. Root and local entry points are unlimited, so they
  can declare anything.
- **Modifying or deleting.** Any save or delete of an action that *already*
  declares a list needs the caller's ceiling to cover the stored list,
  whatever else the change touches. Otherwise a narrower writer could swap
  in different code to run under someone else's grant.
- **`"inherit"` needs no extra right.** It can never exceed whoever runs it.
- **Declaring scopes doesn't lock the action.** The covers-check does the
  job a lock would, and it lets a builder rebuild the action it created.
  Actions with `secrets` keep their own root-only rule and lock, unchanged.

The check runs wherever an action is saved or deleted for content: REST
`/actions`, `entity-save-action` and `entity-delete-action`, alongside
`guard_content_action_change`
([solx-actions/src/content.rs](../solx-actions/src/content.rs)). It needs
`ScopeSet::covers(&ScopeSet)`, the same subset-by-coverage test the package
manifest check uses.

A declared list is a *ceiling*, not only a grant. A package action with
`scopes: ["exec:/builtin/web/http-request"]` that tries `get-env` is
refused **even when the CLI started the chain**. That makes a package's
declared list complete and testable, and it is what the audit mode below is
for.

**Deliberate trade-off: confused deputy.** A client holding only
`exec:/packages/solx-media` can run a media action that writes documents,
because the action declared `write:document`. That is the intended
semantics: whoever declared the list already held it and chose to delegate
it. But a declared scope
should be no wider than the action needs (`write:document:/media`, not
`write:document`), and the action's own parameters must not let a caller
aim it anywhere. SECURITY.md should say so.

### Action tokens

`TokenAuthority::issue_action_token` gains a `scopes: &ScopeSet` argument.
`run_command`
([solx-actions/src/exec.rs](../solx-actions/src/exec.rs)) passes the
ceiling the action's frame gets by the table above. `Registry` stores the
scopes on the token row.

- **A declared list.** The token carries exactly that list.
- **`"inherit"`.** The token carries the caller's ceiling. Started by a
  scoped client, it gets that client's scopes. Started locally (the CLI or
  the desktop app), it gets `Unlimited`, which the registry stores as `all`.
- **Absent.** During the rollout this behaves like `"inherit"`. A client can
  no longer reach near-root through a Command, but a local start still gets
  `all`, as today. From rollout step 4 the token carries no scopes, so it
  authenticates but can't do anything.

### Closing the `X-Solx-Principal: none` hole

`exec_action` in
[solx-server/src/routes/actions.rs](../solx-server/src/routes/actions.rs)
turns `X-Solx-Principal: none` into `actions.exec(...)`, which is
`ExecContext::local()`. Today that can only *drop* privileges, because no
principal is refused by the auth builtins. Once the ceiling exists, `local()`
means `Unlimited`, so **a client token sending that header would escape its
scopes.**

The fix: the ceiling is derived from the authenticated token, not from the
principal, and the header drops only the principal (attribution and
auth-builtin access). Add `ExecContext::anonymous(ceiling)`, and make the
REST route call `exec_ceiled(ceiling, ...)` instead of `exec`. Test this
explicitly.

## Where scopes live

- **`Principal`** (`solx-surface/src/auth.rs`): `Client` and `Action` gain
  `scopes: Arc<ScopeSet>`. `Root` stays scope-less and means everything.
  `Principal` is compared with `==` in several routes
  (`principal != Principal::Root`), which still works.
- **`solx-surface/src/scope.rs`** (new): `Scope`, `ScopeSet` (parse,
  display, serde as a list of strings), `Need`, `Ceiling`, and the
  `require*`/`narrow` helpers. It has no I/O, so it is unit-testable on its
  own.
- **`solx-registry.db`**: `tokens.scopes TEXT` and `connect_codes.scopes
  TEXT`, holding a JSON array.
  - The migration adds both columns as nullable. A `NULL` row means `all`,
    so existing client tokens keep working.
  - `list-clients` returns each token's scopes.
- **`action_config.scopes`**: either a sorted list of scope strings or the
  string `"inherit"`. It is normalised and validated in
  `secret_config::normalize` alongside `secrets` (rename the module to
  `config_guard` or similar while there).

## What stays root-only

Scopes never grant these. They remain `Principal::Root` or an operator entry
point:

- the auth builtins: creating codes, issuing, listing and revoking clients;
- locking and unlocking, changing a locked action, replacing a pinned
  artifact;
- creating or changing an action that declares `secrets`
  (`guard_keyed_action`). Declared `scopes` are not root-only; they follow
  the delegation rule in "Declaring scopes is delegation";
- `command_actions` and `allowed_base_urls`: the allowlists stay in config.
  A scope can let a token use an allowlisted command or URL, never add one;
- writes under `/packages/<other>/`. `guard_package_namespace` still
  applies on top of scopes.

`entity-save-action`'s hard refusal of Command and Webhook rows
(`guard_executable_action`) stays for in-process content: MCP tool calls,
WASM guests and scripts. `write:command` and `write:webhook` take effect on
REST, which is what `solx-mcp-actions` uses. That also closes known gap 7:
REST `/actions` save of a Command row by a client token, which is allowed
today.

## Issuing scoped client tokens

- **Builtins.** `/builtin/auth/issue-client` and `create-connect-code` take
  `scopes: [string]` and `preset: string`. Both are expanded and validated
  before anything is stored. A code stores its scopes, and `redeem` copies
  them onto the token.
- **CLI.** `solx token issue --label laptop --scope read:document --scope
  exec:/packages/solx-google` and `solx token connect-code --preset tools`.
  `--scope` can be repeated.
- **Desktop and web.** "Connect another device" gets a preset picker and an
  "advanced" list of scope strings.
- **Default when nothing is given: `full`** (`all`). This keeps today's
  connect flow and scripts working unchanged. A narrower default list may
  come later (see "Future work").
- **Introspection.** Add `GET /auth/whoami`, which returns
  `{ kind, id, label, scopes }`, so clients and the UI can show what a token
  can do and grey out what it can't. This is cheaper than probing for 403s.

## Packages

### Manifest and install

- `solx-package.json` gains `scopes: [string]`: the package's ceiling, and
  the complete list a reader reviews before installing. This keeps the
  promise SECURITY.md makes for `command_actions` and `allowed_base_urls`.
- Each action's `action_config.scopes` list in `install.solx` must be a
  subset of the manifest's `scopes`, or the install fails. Use
  subset-by-coverage: `exec:/builtin/web/http-request` is covered by
  `exec:/builtin/web`. An action that declares `"inherit"` passes this check
  and is listed as such in the install output, so a reader can see which
  actions run with their caller's authority.
- Every package action implicitly has `exec:/packages/<name>`, so a package
  can always call its own actions. Its own namespace is already special in
  `guard_package_namespace`.
- `install_package`
  ([solx-packages/src/lib.rs](../solx-packages/src/lib.rs)) records the
  granted scopes on `InstalledPackage`. Install output lists them next to
  the command and URL grants.

### Callback inventory

These are what packages do today with `SOLX_SERVER_TOKEN`, found by grepping
`solx-packages` for the env var and the REST routes. Everything that
reaches the server through `solx-package-lib::server::ServerConfig` or a
`serverConfig.ts` is listed.

| Package | Calls | Scopes it needs |
|---|---|---|
| `solx-media` | `PUT /docs/...` (extracted media documents) | `write:document:<its output path>` |
| `solx-omniparse` | `PUT /docs/...`, references `/types/docs/Document` | `write:document:<output path>`, `read:type` |
| `solx-mcp-actions` | `PUT/DELETE /types/...`, `PUT/DELETE /actions/...` (imports MCP tools as Command actions in its own namespace) | `write:type:<its types path>`, `write:action:/packages/solx-mcp-actions`, `write:command` |
| `solx-quickjs` | `GET/PUT /files/...` (JS source in, WASM artifact out), `PUT /actions/...` (saves the built WASM action) | `build-javascript-action`: `"inherit"`, so the caller needs `read:file`, `write:file`, `write:action` and `write:wasm` for the target. `build-javascript-file`: `read:file`, `write:file:<its staging area>` |
| `solx-agent` | `POST /actions/...` with `grant: [{path: "*"}]` | `"inherit"`: the agent loop can call whatever its caller can |
| `solx-inquiry` | `POST /actions/...` | `"inherit"`, as for `solx-agent` |

**JavaScript actions take their scopes as parameters.**
`build-javascript-action` gains a `scopes` parameter, added to
`QuickjsBuildParams`. It writes that parameter onto the WASM action it
saves, as `action_config.scopes`. Because the builder declares `"inherit"`,
its token carries the caller's ceiling, so the server's covers-check is
made against the caller:

- a scoped client can't give the new JavaScript action more than the client
  holds;
- from the CLI or the desktop app, the caller is unlimited, so it can give
  the action anything;
- with no `scopes` parameter, the builder saves no `scopes` field, which
  means an empty ceiling after rollout step 4.

Rebuilding the same action needs the caller's ceiling to cover the scopes
the action already declares, by the modify rule.

**WASM and Script packages** (`solx-google`, `solx-ollama`,
`solx-livejournal`, `solx-reddit`, `solx-facebook`, ...) don't hold a token.
They make in-process nested calls (`http-request`, `get-secret`,
`oauth-*`, ...). Under the ceiling they need declared exec scopes for every
builtin they call. Those calls are too spread out to inventory by grep, so
**the audit mode produces this list** (below).

### Audit mode

A config key, `scope_enforcement: "off" | "audit" | "enforce"`:

- **`off`**: checks run but always pass. This is the state right after the
  code lands.
- **`audit`**: a failed check is allowed but logged once per
  `(principal or declared action, need)`. The log goes to `tracing`, and
  `solx scopes audit` reads a small table in `solx-registry.db` that holds
  the same records. Running each package's `verify.solx` and normal use
  under audit gives the exact scope list to paste into each manifest.
- **`enforce`**: a failed check is refused with 403.

It is read live through `ConfigService`, like the allowlists, so no restart
is needed.

## Rollout

1. **Scope types.** Add `solx-surface::scope`: parsing, matching, presets
   and `Ceiling`, with unit tests. No behaviour change.
2. **Plumbing.**
   - `Principal` gets scopes, `ExecContext` gets a ceiling, and
     `ExecContextInfo` carries it through `resume`.
   - Registry columns and migration, scopes on `issue_action_token`.
   - The `X-Solx-Principal: none` fix.
   - Everything runs with `scope_enforcement = "off"`.
3. **Checks.**
   - Resource guards in REST routes and entity builtins, the exec check in
     dispatch, kind checks on action save and delete, discovery filtering,
     `SolxError::Forbidden`.
   - The delegation check on `action_config.scopes` (covers-check on
     declare, modify and delete), and `"inherit"`.
   - Default the config to `audit`.
4. **Packages.**
   - Manifest `scopes`, the subset check, and recording on
     `InstalledPackage`.
   - Run every package under audit, then add `scopes` to each manifest and
     `install.solx`: a list for fixed-purpose actions, `"inherit"` for the
     delegating ones (quickjs's builder, `solx-agent`, `solx-inquiry`).
   - Add the `scopes` parameter to `build-javascript-action`.
   - Once every package declares its scopes, an action with no `scopes`
     field gets an **empty** ceiling instead of inheriting.
5. **Issuing and UI.**
   - `--scope`/`--preset` on the CLI, builtin parameters, `whoami`.
   - Desktop and web preset picker, and scopes shown in the client list.
6. **Enforce and document.**
   - Flip the default to `enforce`.
   - Update SECURITY.md (remove the first known gap, add the confused-deputy
     note), `security-design.md`, `http-api.md` and the README.

## Tests

- **`scope` unit tests**:
  - segment-prefix matching (`/builtin/web` vs `/builtin/webby`), `exec:/`,
    exact refs, normalisation (`//a/`, case of the first segment);
  - strict parsing errors, preset expansion, subset-by-coverage for the
    manifest check.
- **REST** (`solx-server/tests/scopes.rs`, modelled on `registry.rs`), with
  a client token per scope:
  - each route is allowed with its scope and gets 403 without it;
  - lists narrow rather than fail;
  - `write:document:/a` can't write `/b`;
  - `X-Solx-Principal: none` from a client token doesn't widen its scopes.
- **MCP:**
  - `explore_tools` shows only actions the token can exec;
  - calling an un-granted action by name is refused;
  - `entity-get-document` with `exec:/` but no `read:document` is refused.
- **Ceiling:**
  - a script written by a scoped token can't call outside the token's
    scopes, including via `action-start`;
  - a locked action that declares scopes runs with them under a narrower
    client;
  - a declared ceiling refuses an undeclared builtin even from the CLI.
- **Action tokens** (extend `solx-actions/tests/command_credentials.rs`):
  - a Command child's token carries the declared list;
  - with `"inherit"`, it carries the caller's ceiling exactly;
  - with no `scopes` field, it inherits during the rollout and is empty after
    step 4;
  - it gets 403 outside its scopes.
- **Delegation:**
  - a client token with `write:action` and `write:wasm` but not `exec:/`
    can't save an action declaring `exec:/`, and can save one declaring a
    scope it holds;
  - a narrower writer can't modify or delete an action whose declared list
    exceeds its ceiling, even to change only the description;
  - declaring `"inherit"` needs no extra scope;
  - declaring scopes doesn't lock the action, so its creator can save it
    again.
- **quickjs builder:**
  - `build-javascript-action` called by a scoped client can't give the JS
    action scopes that client lacks;
  - called from the CLI, it can give it any scopes;
  - with no `scopes` parameter, the saved action has no `scopes` field.
- **Root-only:**
  - a client token with `write:action` can't save an action that declares
    `secrets`;
  - no scope reaches the auth builtins, lock/unlock, or another package's
    namespace;
  - `write:command` without `write:action` is refused, and so is the
    reverse.
- **Migration:** a pre-existing client token (`scopes` NULL) still works as
  `all`.
- **Packages:** an install whose action declares a scope missing from the
  manifest fails; uninstall leaves nothing behind.

## Decisions

| Question | Decision |
|---|---|
| What does a client token issued with no scopes or preset get? | `full` (`all`). A narrower default list may come later. |
| Does the desktop app get a ceiling? | No. Tauri IPC, from both the main and the Settings window, stays `ExecContext::local()`, unlimited. |
| Does stdio `solx-mcp` get a ceiling? | Not in this plan; see "Future work". |
| How does a JavaScript action get its scopes? | As a `scopes` parameter to `build-javascript-action`, which declares `"inherit"`, so the covers-check is made against its caller. |
| Who may declare `action_config.scopes`? | Anyone whose ceiling covers the declared list (delegation). It doesn't lock the action. `secrets` stay root-only. |
| How do builtins with path parameters narrow? | The exec scope only makes the builtin reachable. The resource scope narrows it: `exec:/builtin/file/file-get` plus `read:file:/x` can read only under `/x`. |

## Future work

- **A narrower default** for client tokens issued without scopes, such as
  the `tools` preset, once the UI has a preset picker.
- **A ceiling for stdio `solx-mcp`.** A config key (`mcp_scopes`, or
  `SOLX_MCP_SCOPES`) would run a stdio MCP server under the same grammar.
  That would make `SOLX_MCP_PATH_PREFIX`-style scoping real access control.
