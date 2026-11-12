---
name: solx
description: Use whenever the task involves solx - the local SQLite-backed catalogue of dynamically dispatched actions served over MCP, HTTP and a CLI. Covers finding actions with the explore_tools router, running them (solx exec, POST /actions/{ref}), reading and writing documents, types and files, registering new actions, and installing packages. Trigger on "solx", "solx-server", "solx-mcp", "explore_tools", "act__", "solx exec", "action catalogue", or any request to list, search, run, create or delete solx actions, documents or types.
---

# solx

**solx turns a database of actions into a live MCP/REST server.** An action is a
*row*, not a line of tool-definition code: its `action_type` column picks the
dispatcher, and every surface - MCP, HTTP, the CLI, the web UI - serves the
same catalogue.

That shape is the whole idea, and it explains almost everything else:

- **Adding a tool is inserting a row.** There is no hand-written tool layer to
  keep in sync, and no code change to roll out.
- **The catalogue is dynamic.** Installing or uninstalling a package changes the
  set of tools at runtime, so a client never gets a large static tool list to
  wade through.
- **The database is the API.** Documents, types, actions, files and secrets are
  all reached *through* actions, so there is no separate CRUD surface to learn
  alongside them.

## The data model

| Store | Holds |
| --- | --- |
| `db/solx-actions.db` | actions - the callable catalogue |
| `db/solx-docs.db` | documents (FTS5 full-text + path-faceted search) |
| `db/solx-types.db` | types (JSON Schema) |
| `db/solx-console.db` | per-action console output |
| `db/solx-registry.db` | client tokens, connect codes, invocation records |

Separate SQLite (libsql) files, one namespace on top: every entity is addressed
as `/path/name`, e.g. `/builtin/document/entity-save-document`,
`/packages/solx-google/send-gmail-message`, `/weather/forecast`.

Data lives under `%APPDATA%/praeus/solx` (Windows) or `~/.praeus/solx`
(otherwise); `SOLX_APPDATA_DIR` overrides it. `solx-config.json` there holds the
command allowlist, the outbound-URL allowlist, installed packages and env vars.

### Action kinds

`action_type` decides what `fn_name` means:

| `action_type` | `fn_name` is | Use for |
| --- | --- | --- |
| `wasm` | an export of a `wasm32-wasip2` component | sandboxed third-party logic |
| `webhook` | the URL to call | REST APIs |
| `command` | a **key** into the `command_actions` allowlist | local binaries |
| `script` | a `.solx` script artifact | composing other actions |
| `internal` | a native handler | the `/builtin` catalogue |

`command` is the one to watch: `fn_name` is never a literal command string, it
is a pointer into `solx-config.json`'s `command_actions` map, so the database
can never hold an executable string. An unregistered key is refused, even when
the allowlist is entirely unset.

## Talking to solx from an agent

### 1. Discover - one router tool

The `solx` MCP server advertises **exactly one** tool in its `tools/list`
response: `explore_tools`. It is a router over the live database.

```jsonc
// Code Mode
await tools.solx.explore_tools({ discovery_mode: "search", search_query: "gmail send" })
// native tool list
solx_explore_tools({ discovery_mode: "search", search_query: "gmail send" })
```

- `discovery_mode: "search"` needs `search_query`. Returns **at most 10**
  candidates, each as `Tool: act__<path>__<name>` plus its description, its JSON
  schema, and a `Destructive: yes` line when the action changes or removes data.
- `discovery_mode: "list_all"` pages the whole catalogue, 10 per page. Pass the
  trailing `next_cursor: N` back as `cursor` for the next page.
- `path_prefix` narrows to a subtree - `/builtin`, `/packages/solx-google`. Use
  it when you want "just the built-ins" or "just Google".

Search is FTS-style **AND** over tokens. `save document` matches;
`save action document type entity` matches nothing, because no single row
carries all five words. Search one concept at a time and widen - `"gmail"`,
then `"gmail send"`.

A search with no hits comes back as a tool *error* whose text is
`no tools matched '<query>'`, not as an empty result. That is a normal outcome,
not a broken server.

### 2. Execute - the `act__` names are not tools

This is the part that trips people up. **`explore_tools` hands you names you
cannot call as tools.** A client's tool list comes from the MCP `tools/list`
response, and solx deliberately puts only the router there: the catalogue is
large and changes on every package install, so a flat list would mean a
`tools/list_changed` notification each time and a model sifting dozens of
static entries instead of searching on demand.

So `act__builtin__document__entity-save-document` is a *reference*, not a
callable tool name. Run it through a real surface instead.

**CLI - simplest, and local:**

```sh
solx exec /builtin/document/entity-save-document --json '{"name":"note","path":"/notes"}'
```

stdout is one JSON document: `{"action": "...", "result": {...}, "success": true}`.
The action's console streams to stderr while it runs, so stdout stays a single
clean JSON value for `| jq`. A hard failure exits non-zero with
`Error: execution error: <kind>: <message>`.

**HTTP - for a solx-server (local or remote):**

```sh
curl -s -X POST http://127.0.0.1:8766/actions/builtin/document/entity-save-document \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"note","path":"/notes"}'
```

`POST` on the action's own URL runs it, and the body *is* the params object.
Get the token with `solx token show`.

**Decoding the name.** `act__` + the path with `/` replaced by `__`:
`act__builtin__document__entity-save-document` -> `/builtin/document/entity-save-document`.
Use the `/path/name` form for the CLI and HTTP. A very long name comes back
opaque (`act__b32_<base32>`); that is the server keeping tool names under 64
characters, and it still decodes - but treat it as a signal the action should
be renamed.

### 3. Read the result

`result` is whatever the action returned. The built-ins answer with JSON
objects: `entity-list-*` and `search-*` give `{items, total, limit, offset}`;
`file-get` returns bytes as a string (base64 with `encoding: "base64"`).

Failures are **in-band**, not transport errors, so the model can read and react
to them: a handled failure gives `success: false` with the payload still
attached, and a hard error gives text like `[not_found] not found: action /nope`.

## What you can do without writing any code

| Goal | Action |
| --- | --- |
| Read/write documents | `/builtin/document/entity-get-document`, `entity-save-document`, `get-field`, `set-field`, `search-documents` |
| Define a JSON-Schema type | `/builtin/type/entity-save-type` |
| Store bytes | `/builtin/file/file-put`, `file-get`, `file-list` |
| Call an HTTP API | `/builtin/web/http-request`, `web/stream/start` |
| Store credentials | `/builtin/secrets/get-secret`, `set-secret` |
| Inspect an action's output | `/builtin/console/read`, `tail`, `list` |
| Run something long in the background | `/builtin/action/start` then `poll` / `stop` |

Full list with parameters: `references/builtin-catalogue.md`.

## Writing a new action

An action is a row. To add one:

```sh
solx save action /weather/forecast --json '{
  "actionType": "webhook",
  "fnName": "https://api.weather.gov/points/{lat},{lon}",
  "description": "Get the forecast for a coordinate pair.",
  "paramTypeRef": "/weather/ForecastParams"
}'
```

`actionType` is required on create, and `fnName` means what the table above
says. A `paramTypeRef` points at a type you saved with `solx save type`, and
gives the action a JSON Schema - that schema is what `explore_tools` shows the
model, so it is worth writing.

Watch the guardrails: `command` and `webhook` actions need an out-of-band
allowlist entry (`command_actions` / `allowed_base_urls` in `solx-config.json`),
and `entity-save-action` **refuses** to create or change `command`/`webhook`
actions for non-root callers - a guest cannot grant itself shell or outbound
HTTP. Package installs are the normal way to get one.
Details: `references/creating-actions.md`.

## Gotchas

- **Not every action is safe.** `explore_tools` labels destructive ones
  (`Destructive: yes`). Ask the user before running one.
- **A missing action is not a missing tool.** The catalogue changes under you,
  so `exec` reports `NotFound` in-band rather than pretending the tool never
  existed.
- **`command` actions may need a working directory or environment.** That lives
  in `command_actions` in `solx-config.json`, not in the row.
- **The CLI `search` command searches documents only.** Actions are searched
  with the `/builtin/action/search-actions` action.
- **Scopes.** An action runs with the scopes it declares, and one with none can
  call nothing at all. `solx scopes audit` lists refusals; `solx action scopes
  <ref> --add ...` grants them.
- **The server may be stale.** `curl -s http://127.0.0.1:8766/health` reports
  the commit the binary was built from. Compare it with
  `git rev-parse --short=12 HEAD` before trusting a server's answers - a stale
  binary still answers plausibly, using code paths the source no longer has.

## More detail

- `references/builtin-catalogue.md` - every `/builtin` action and its parameters.
- `references/surfaces.md` - CLI commands, HTTP routes, config file, storage.
- `references/creating-actions.md` - registering actions, packages, secrets, scopes.
