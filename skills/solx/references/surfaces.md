# Surfaces: CLI, HTTP, storage, config

Four binaries ship from the same workspace and share one set of manager traits,
so any of them reaches the same data:

| Binary | What it is |
| --- | --- |
| `solx` | the CLI - local, unlimited |
| `solx-server` | HTTP over `127.0.0.1`, bearer-token gated, MCP at `/mcp` |
| `solx-mcp` | the same MCP server over stdio, with path-prefix scoping |
| `solx-client` | HTTP implementations of the manager traits (remote mode) |

The CLI is a **local, unlimited** entry point. The server and the stdio MCP
server apply token scopes.

## CLI

```
solx save <doc|action|type|file> <REFERENCE> [--json JSON] [--type TYPE_REF] [--file FILE]
solx get <doc|action|type|file> <REFERENCE>
solx delete <doc|action|type|file> <REFERENCE> [--if-exists]
solx exec <REFERENCE> [--json JSON] [--no-console]
solx list <doc|action|type|file> [--path PATH] [--limit N] [--offset N]
solx search <QUERY> [--path PATH] [--type TYPE_REF] [--linked-to REF] [--limit N] [--offset N]
solx script [--expr EXPR | --file FILE]
solx install-package <PATH> | solx uninstall-package <NAME> | solx list-packages
solx token show|rotate|connect-code|clients|revoke|issue
solx connect <TARGET> [CREDENTIAL] [--print]
solx action lock|unlock|scopes <REFERENCE> [--add SCOPE ...]
solx scopes audit
solx json <VALUE> | solx str <STRING> | solx escape <STRING> | solx random
```

Notes:

- `REFERENCE` is the full `/path/name`. For `file` it is a relative path under
  the files root.
- `save` is an upsert. `--json` overrides anything piped on stdin.
- `exec` prints one JSON document on stdout: `{action, result, success}`. The
  action's console goes to stderr, so `solx exec ... | jq` stays clean.
  `--no-console` suppresses the console stream.
- `search` searches **documents only**. Actions are searched with the
  `/builtin/action/search-actions` action.
- `delete --if-exists` treats a missing entity as success - a package's
  `uninstall.solx` runs statement by statement and aborts on the first failure,
  so without it one entity an older version never registered leaves the package
  half torn down.
- `solx json`, `solx str`, `solx escape` and `solx random` are the primitives the
  `.solx` script language is built from; they are also handy on their own.

### Passing JSON from a shell

The `--json` value must survive the shell's quote handling. In **PowerShell**,
single quotes are consumed by the shell and the inner double quotes are then
stripped as argument syntax, so `'{ "a": 1 }'` reaches the program as `{a:1}`
and fails to parse. Escape the inner quotes:

```powershell
solx exec /builtin/document/entity-save-document --json '{\"name\":\"note\"}'
```

In bash and cmd.exe, ordinary single or double quoting works as expected.

## HTTP API

Base URL `http://127.0.0.1:8766` by default. Every route except `/health`
needs `Authorization: Bearer <token>`.

| Method | Route | Body | Returns |
| --- | --- | --- | --- |
| `GET` | `/docs` | - | `Page<Document>` |
| `GET` | `/docs/{ref}` | - | `Document` |
| `PUT` | `/docs/{ref}` | `DocumentInput` | `Document` |
| `DELETE` | `/docs/{ref}` | - | `204` |
| `GET` | `/docs-paths` | - | `Page<PathFacet>` |
| `GET` | `/search` | - | `Page<Document>` |
| `GET` | `/types`, `/types/{ref}` | - | types |
| `PUT` | `/types/{ref}` | `TypeInput` | `TypeEntity` |
| `DELETE` | `/types/{ref}` | - | `204` |
| `GET` | `/types-paths` | - | `Page<PathFacet>` |
| `POST` | `/validate` | `{value, typeRef}` | `204` or `422` |
| `GET` | `/actions`, `/actions/{ref}` | - | actions |
| `PUT` | `/actions/{ref}` | `ActionInput` | `Action` |
| `DELETE` | `/actions/{ref}` | - | `204` |
| `POST` | `/actions/{ref}` | params (JSON) | `ActionExecResult` |
| `GET` | `/actions-paths` | - | `Page<PathFacet>` |
| `GET` | `/actions-search?q=...` | - | `Page<Action>` |
| `GET` | `/files?prefix=...` | - | `{paths: [...]}` |
| `GET`/`PUT`/`DELETE` | `/files/{relPath}` | raw bytes | raw bytes / `204` |
| `GET` | `/health` | - | `{status, version, commit, commit_date}` |
| `GET` | `/auth/whoami` | - | `{kind, scopes, scope_enforcement, ...}` |
| `POST`/`GET`/`DELETE` | `/mcp` | - | MCP Streamable HTTP transport |

`POST` on an action's own URL **executes** it, with the body as the params
object. A parameterless action can be invoked with an empty `POST` and no
`Content-Type`.

`{ref}` is the path and name joined by `/`: path `/research/ai`, name `note` ->
`/docs/research/ai/note`; path `/`, name `note` -> `/docs/note`. `/`, `\` and
`:` are forbidden inside a segment or name; everything else is allowed, so
percent-encode each segment.

`PUT` is create-or-replace and always answers `200` - the manager does not
distinguish create from replace.

List routes also take `excludeMode` (`user` | `tooling` | `none`, default
`user`), `excludePaths`, `pathPrefix`, `limit`, `offset`, `filterField`,
`filterValue`, `sortBy`, `sortOrder`, `dateAfter`, `dateBefore`. `sortBy` and
`filterField` values name the underlying column and stay snake_case
(`sortBy=created_at`); only the parameter keys are camelCase.

`excludeMode` defaults to `user` because those are the routes the web UI reads.
An excluded entity reads and deletes as `404` and refuses `PUT` with `400`.
`POST` (exec) is never filtered.

## Storage

Separate libsql/SQLite databases per entity kind, under
`%APPDATA%/praeus/solx` on Windows and `~/.praeus/solx` elsewhere
(`SOLX_APPDATA_DIR` overrides the root):

```
db/solx-actions.db
db/solx-docs.db
db/solx-types.db
db/solx-console.db
db/solx-registry.db
files/          # the file store (files/actions/shared, files/docs, ...)
logs/           # per-action log files and scope-audit.jsonl
solx-config.json
```

Cross-entity references are full path strings, resolved at write time.

## `solx-config.json`

| Key | Meaning |
| --- | --- |
| `command_actions` | map of allowlist key -> `{command, cwd?, env?}`; a `command` action's `fn_name` is a key **into this map** |
| `allowed_base_urls` | outbound allowlist; every webhook URL and every `/builtin/web/*` URL must match one entry |
| `exclusions` | per-mode glob lists (`user`, `tooling`) of paths hidden from lists, searches and reads |
| `installed_packages` | packages installed, with the allowlist grants each holds |
| `env_vars` | persisted environment-store variables (plaintext) |
| `server_host`, `server_tls`, `server_ca_file`, `token_store`, `scope_enforcement`, `packages_directory` | server and policy settings |

An `allowed_base_urls` entry matches on exact scheme, host and port, and a
path under the entry's. Redirects are held to the same list.

## Tokens and scopes

- The server's root token is generated on first start and kept in the OS
  credential manager, not in `solx-config.json`. `solx token show` prints it;
  `solx token rotate` replaces it. It is only accepted from the server's own
  machine.
- Client tokens are per-device and revocable: `solx token connect-code
  --label laptop` prints a code, `solx connect <url> <code>` redeems it on the
  other machine, `solx token clients` lists them, `solx token revoke <id>`
  cuts one off.
- A code gives full access unless narrowed with `--preset read-only` (or
  `tools`, `editor`) or explicit `--scope` strings such as
  `read:document:/notes/*`.
- `scope_enforcement: "audit"` in the config lets calls through while you work
  out what your actions need; `"enforce"` (the default) refuses them.
  `solx scopes audit` lists what was refused, with the `solx action scopes`
  command that would allow it.

## Serving other machines

Bind the server on the network (`server_host`), turn on TLS (`server_tls`: the
desktop app does both for you on Windows; for a standalone server run
`solx-certs init` and set it to `"auto"`), then hand each device its own
token. With TLS on, the connect link carries the server's pin
(`.../connect#code=...&pin=...`), so `solx connect '<link>'` needs no
certificate setup.

## Stale binaries

A `solx-server` copied onto the `PATH` can fall behind the source silently:
it still accepts requests and still answers plausibly, using code paths the
source no longer has. `/health` is unauthenticated and reports the build:

```sh
curl -s http://127.0.0.1:8766/health
# {"status":"ok","version":"0.1.0","commit":"364077aac950","commit_date":"..."}
git rev-parse --short=12 HEAD    # must match `commit`
```

A `-dirty` suffix means it was built from a tree with uncommitted changes. If
`commit` disagrees with `HEAD`, rebuild and redeploy before trusting anything
the server says.
