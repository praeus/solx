# solx-server HTTP API

`solx-server` hosts the local solx managers over HTTP so several processes can
share one appdata directory. It is also the supported way to build a custom
client: everything `solx-cli`, `solx-mcp` and `solx-web` can do is reachable
through the routes below.

The surface is REST. An entity's reference lives in the URL, list and search
options ride in the query string, and the HTTP method carries the verb.

- **Base URL** — `http://127.0.0.1:8766` by default. The bind address and
  port come from `--host`/`--port`, else the config's `server_host`/
  `server_port`.
- **Auth** — every route except `/health`, `/auth/exchange` and `/connect`
  requires `Authorization: Bearer <token>`. Three kinds of token are
  accepted:
  - the **root token**, generated on first run and kept in the OS credential
    manager (`solx token show` prints it). It is accepted **from loopback
    peers only**: clients on the server's own machine use it;
  - **client tokens**, one per device, from a connect code (below) or
    `solx token issue`. They expire (90 days by default,
    `client_token_ttl_days`) and can be listed and revoked one by one;
  - **action tokens**, one per external Command invocation, handed to the
    command as `SOLX_SERVER_TOKEN` and revoked when it ends.

  A rejected token is a `401` with the usual `{kind, message}` body.
- **CORS** — on a loopback bind, permissive, so a browser page can call the
  server directly. On any other bind, only the origins listed in the config's
  `server_cors_origins` are allowed (none by default). Either way, the bearer
  token is the only real gate.

A server built without the `registry` cargo feature (it is on by default)
has only the root token, accepted from anywhere, and no `/auth/exchange` or
`/connect`.

### Serving other machines

Set `server_host` to `0.0.0.0` (or a specific interface IP), or start the
server with `--host`. If clients should use a different address than the one
bound (a `0.0.0.0` bind, a reverse proxy, a Tailscale name or IP), set
`server_public_url`; connect links use it. It takes a full URL
(`https://solx.example.com`, used as is) or a bare IP or host name
(`100.101.102.103`), which gets `http://` and `server_port`. To use a
different address for one link only, pass
`solx token connect-code --url <URL|HOST>`, or fill in "Address for the
link" in the desktop app.

Then connect each device with its own token:

1. On the server's machine, run `solx token connect-code` (or use
   "Connect another device" in the desktop app's settings). It prints an
   8-digit code such as `1234-5678`, a link
   (`<public url>/connect#code=12345678`) and the server address. The code
   works once and expires after 10 minutes (`--ttl` changes that).
2. On the other device, either open the link in a browser, run
   `solx connect <link>` (or `solx connect <server address>` and type the
   code), or enter the address and code in the desktop client's or
   solx-web's Settings. `solx connect --print` only prints the token, for an
   MCP client's configuration.

`solx token clients` lists the devices and `solx token revoke <id>` cuts one
off; the revocation applies on its next request. The server prints a warning
at startup when it isn't bound to loopback, because traffic is plain HTTP and
tokens are sent unencrypted. Run it over a private network
(Tailscale/WireGuard) or behind a TLS reverse proxy (Caddy, nginx), and don't
expose it directly to the internet.

Behind a reverse proxy **on the same machine**, every request reaches the
server from loopback, so the root token is accepted from anywhere the proxy
is. Don't give the root token to remote clients in that setup either.

### Connecting another device

| Method | Route | Body | Returns |
| --- | --- | --- | --- |
| `POST` | `/auth/exchange` | `{"code": "1234-5678"}` | `{token, client_id, expires_at, server_url}` |
| `GET` | `/connect` | — | a static HTML page |

`/auth/exchange` redeems a connect code for a client token. It needs no
bearer token. Spaces and dashes in the code are ignored. `server_url` is
`server_public_url` if set, else the address the request reached. A wrong,
expired or spent code is a `401`; the same peer's next attempt within a
second is a `429`. Every failure, from any peer and any solx process on the
host, counts against a global budget: after 10, every pending code is
invalidated and a new one has to be made.

`/connect` changes nothing. The code travels in the URL fragment, which the
browser never sends, and the page's script fills it into a form that posts
to `/auth/exchange` when the person clicks **Connect**. It then shows the
token once, to copy into a client. Link previewers and mail scanners fetch
pages without submitting forms, so they can't spend a code.

Codes and client tokens are created by four root-only actions under
`/builtin/auth`: `create-connect-code {label?, code_ttl_secs?,
token_ttl_secs?, public_url?}`, `list-clients`, `revoke-client {id}` and
`issue-client {label, token_ttl_secs?}`. They refuse every caller except the
root token (over REST or `/mcp`) and the trusted local entry points that run
them as root: `solx token …` and the desktop Settings window's `auth_exec`.
A client token, an action token, an action calling them and a local call
with no principal (`solx exec`, stdio `solx-mcp`) are all refused.

`POST /actions/{ref}` normally runs as whoever the bearer token belongs to.
A request with `X-Solx-Principal: none` runs with no principal instead, as a
local call would. `solx-client`'s `exec` always sends it (only `exec_for`
doesn't), so the CLI and the `.solx` scripts it runs can't reach the
root-only actions just because they present the loopback root token. The
header can only drop a principal, never grant one.

## Entity references in the URL

Every entity is identified by a `path` (directory-like, e.g. `/research/ai`)
and a `name` (a single segment). The URL is simply the two joined:

| Reference | URL |
| --- | --- |
| path `/research/ai`, name `note` | `/docs/research/ai/note` |
| path `/` (root), name `note` | `/docs/note` |

`/`, `\` and `:` are forbidden inside a path segment or name. Everything else
is allowed — including spaces, `%`, `#` and `?` — so **percent-encode each
segment**. A name of `100% #1` becomes `100%25%20%231`.

## Routes

### Documents

| Method | Route | Body | Returns |
| --- | --- | --- | --- |
| `GET` | `/docs` | — | `Page<Document>` |
| `GET` | `/docs/{ref}` | — | `Document` |
| `PUT` | `/docs/{ref}` | `DocumentInput` | `Document` |
| `DELETE` | `/docs/{ref}` | — | `204` |
| `GET` | `/docs-paths` | — | `Page<PathFacet>` |
| `GET` | `/search` | — | `Page<Document>` |

`PUT` is create-or-replace, so it is used for both. It always answers `200`
with the saved entity — the underlying manager does not distinguish a create
from a replace, so there is no `201`.

Query parameters for `/docs` (all optional): `pathPrefix`, `limit`, `offset`,
`filterField`, `filterValue`, `sortBy`, `sortOrder` (`asc`/`desc`),
`dateAfter`, `dateBefore` (RFC 3339). The same set applies to `/types` and
`/actions`. `filterField`'s and `sortBy`'s *values* name the underlying
column (e.g. `sortBy=created_at`) and stay snake_case — only the parameter
keys themselves are camelCase.

Query parameters for `/search`: `q`, `pathPrefix`, `typeRef`, `linkedTo`,
`limit`, `offset`, plus the two exclusion parameters below.

#### Exclusions

Every list, paths and search route on `/docs`, `/types` and `/actions` also
takes:

- `excludeMode` — `user`, `tooling` or `none`: which configured list from
  `solx-config.json`'s `exclusions` applies. **Defaults to `user`** on these
  routes, because they are what the web UI reads.
- `excludePaths` — extra patterns, repeated as needed
  (`?excludePaths=/a&excludePaths=/b*`). They add to the mode's list; they
  never remove from it.

A pattern is a glob (`*` spans `/`, `?` is one character) matched against an
entity's path **or** its full reference. `/agent/skills` hides everything
stored in `/agent/skills` and the entity `skills` in `/agent`;
`/agent/skills*` hides that folder and everything beneath it.

Exclusions are applied in SQL, so `total` is exact.

`GET`, `PUT` and `DELETE` on a single `{ref}` take `excludeMode` too, with the
same `user` default. An excluded entity reads (or deletes) as `404`, worded
exactly like a missing one, and a `PUT` to one is refused with `400`. Exec
(`POST /actions/{ref}`) is never filtered: the agent widget reaches the
built-ins that way, and passes `excludeMode: "tooling"` to them itself.

The Rust `solx-client` always sends `excludeMode=none` unless told
otherwise, so a `solx-cli` or `solx-mcp` proxying to this server behaves
exactly as it does against local managers.

Search is a top-level route rather than `/docs/search` on purpose. A static
segment beside a catch-all wins the match for every method it is registered
under, so `/docs/search` would make a document named `search` at the root
permanently unreachable.

### Types

| Method | Route | Body | Returns |
| --- | --- | --- | --- |
| `GET` | `/types` | — | `Page<TypeEntity>` |
| `GET` | `/types/{ref}` | — | `TypeEntity` |
| `PUT` | `/types/{ref}` | `TypeInput` | `TypeEntity` |
| `DELETE` | `/types/{ref}` | — | `204` |
| `GET` | `/types-paths` | — | `Page<PathFacet>` |
| `POST` | `/validate` | `{ "value": …, "typeRef": "…" }` | `204`, or `422` |

There is no `resolve` route. Resolving a type reference is just splitting it
into `(path, name)` and fetching it — which `GET /types/{ref}` already is.

### Actions

| Method | Route | Body | Returns |
| --- | --- | --- | --- |
| `GET` | `/actions` | — | `Page<Action>` |
| `GET` | `/actions/{ref}` | — | `Action` |
| `PUT` | `/actions/{ref}` | `ActionInput` | `Action` |
| `DELETE` | `/actions/{ref}` | — | `204` |
| `POST` | `/actions/{ref}` | params (JSON) | `ActionExecResult` |
| `GET` | `/actions-paths` | — | `Page<PathFacet>` |
| `GET` | `/actions-search?q=…&excludeMode=…` | — | `Page<Action>` |

`exclude_hidden=true` on `/actions-search` is the deprecated spelling of
`excludeMode=tooling`; an explicit `excludeMode` wins.

`POST` on an action's own URL executes it — RFC 9110's "resource-specific
processing of the request payload". The body is optional: a parameterless
action can be invoked with an empty `POST` and no `Content-Type`.

Secrets in an action's `actionConfig` are masked as `"***"` in every
response. Saving a masked config back does not destroy the stored secret.

An action whose `resultTypeRef` is `/builtin/types/WidgetDescriptor` renders
a UI. Its result is `{ "tag_name": …, "bin_name": …, "fields": … }` (this
result payload is the action's own opaque return value, not a generic entity
response, so it keeps the widget schema's own snake_case field names): create
the custom element `tag_name`, load its bundle from `GET /files/{bin_name}`,
and hand it `fields`. There is no widget-specific route or protocol — the
widget calls back in through the routes on this page like any other client.
See [widget-actions.md](widget-actions.md).

### Files

| Method | Route | Body | Returns |
| --- | --- | --- | --- |
| `GET` | `/files?prefix=…` | — | `{ "paths": [ … ] }` |
| `GET` | `/files/{relPath}` | — | raw bytes |
| `PUT` | `/files/{relPath}` | raw bytes | `{ "relPath": "…" }` |
| `DELETE` | `/files/{relPath}` | — | `204` |

File content is transferred as raw bytes, not base64. On `GET`, the
`Content-Type` is guessed from the extension (`application/octet-stream` when
unknown) — the file store does not persist a content type of its own; that
lives on a `FileRef` in document metadata. The request `Content-Type` on `PUT`
is ignored. Bodies are capped at 64 MiB.

A `relPath` may contain `/` and is used as-is (no path/name split). Traversal
is rejected by the file store.

### Other

| Method | Route | Notes |
| --- | --- | --- |
| `GET` | `/health` | Unauthenticated. Returns `{status, version, commit, commit_date}`. |
| `POST`/`GET`/`DELETE` | `/mcp` | MCP Streamable HTTP transport, same auth gate. |

## Errors

Every error response body is the tagged error itself:

```json
{ "kind": "not_found", "message": "document /research/ai/note" }
```

| `kind` | Status |
| --- | --- |
| `not_found` | 404 |
| `invalid` | 400 |
| `validation` | 422 |
| `conflict` | 409 |
| `io`, `db`, `exec`, `config`, `other` | 500 |

A malformed reference (a forbidden character, a `.`/`..` segment, a missing
name) is `400 invalid`, not a 404.

## Examples

```bash
TOKEN=...            # printed by `solx server start`
BASE=http://127.0.0.1:8766
AUTH="Authorization: Bearer $TOKEN"

# Save a document (create or replace).
curl -X PUT "$BASE/docs/research/ai/note" -H "$AUTH" \
     -H 'Content-Type: application/json' \
     -d '{"typeRef":"/types/docs/Document","title":"A note","contents":{"k":"v"}}'

# Read it back.
curl -H "$AUTH" "$BASE/docs/research/ai/note"

# List and search.
curl -H "$AUTH" "$BASE/docs?pathPrefix=/research&limit=20&sortBy=created_at"
curl -H "$AUTH" "$BASE/search?q=ada&typeRef=/types/docs/Document"

# Execute an action.
curl -X POST "$BASE/actions/tools/echo" -H "$AUTH" \
     -H 'Content-Type: application/json' -d '{"hello":"world"}'

# Upload and download a file, unencoded.
curl -X PUT "$BASE/files/media/pic.png" -H "$AUTH" --data-binary @pic.png
curl -H "$AUTH" "$BASE/files/media/pic.png" -o roundtrip.png

# Delete (204, empty body).
curl -X DELETE -H "$AUTH" "$BASE/docs/research/ai/note"
```

## Existing clients

You do not have to hand-roll one:

- **Rust** — `solx-client` (`RemoteTypeManager`, `RemoteDocManager`,
  `RemoteActionManager`, `RemoteFileStore`), implementing the
  `solx-surface` manager traits.
- **TypeScript / browser** — `@solx/http`'s `connectHttp(baseUrl, token)`,
  zero-dependency and browser-safe.

Both are thin wrappers over exactly the routes above, and are the best
reference for how a request is meant to be shaped.

## IPC (Tauri)

`solx-tauri-plugin` exposes the same managers to a Tauri webview over IPC,
for the solx desktop app (solx-explorer). There is no URL or token: the
host process holds the `App`, which is either local (the server build, also
served over HTTP by an embedded solx-server unless `server_embedded` is
`false`) or a `solx-client` proxy (the client build). From JS, use
`@solx/tauri`'s `connectTauri(invoke)`; commands are `plugin:solx|<name>`
and need the `solx:default` capability permission.

Each command calls the same manager method as the route beside it, and
fails with the same tagged `{kind, message}` error body. Arguments are
named (camelCase from JS); `path`/`name` are normalized exactly like a URL
reference. Exclusions behave as on the routes: options objects carry
`excludeMode`/`excludePaths`, the single-entity commands take an optional
`excludeMode` argument, and the default is `user`.

| Command | Arguments | Route |
| --- | --- | --- |
| `docs_list` / `docs_paths` | `opts?: ListOptions` | `GET /docs`, `/docs-paths` |
| `docs_get` / `docs_delete` | `path, name` | `GET`/`DELETE /docs/{ref}` |
| `docs_save` | `path, name, input` | `PUT /docs/{ref}` |
| `docs_search` | `query?: SearchQuery` | `GET /search` |
| `types_list` / `types_paths` | `opts?` | `GET /types`, `/types-paths` |
| `types_get` / `types_delete` / `types_save` | `path, name[, input]` | `/types/{ref}` |
| `types_resolve` | `typeRef` | — (`GET /types/{ref}`) |
| `types_validate` | `value, typeRef` | `POST /validate` |
| `actions_list` / `actions_paths` | `opts?` | `GET /actions`, `/actions-paths` |
| `actions_get` / `actions_delete` / `actions_save` | `path, name[, input]` | `/actions/{ref}` |
| `actions_search` | `query?: ActionSearchQuery` | `GET /actions-search` |
| `actions_exec` | `path, name, params?` | `POST /actions/{ref}` |
| `files_list` | `prefix?` | `GET /files` (returns the bare list) |
| `files_get` | `relPath` | `GET /files/{relPath}` (raw `ArrayBuffer`) |
| `files_put` | raw bytes body; `x-solx-path` header (percent-encoded) | `PUT /files/{relPath}` (returns the path) |
| `files_delete` | `relPath` | `DELETE /files/{relPath}` |
| `info` | — | stands in for `/health`: `{mode, client, version, serverUrl?, embeddedServer?}` |
| `connect` / `disconnect` | `serverUrl, credential` / — | client build only; `credential` is a connect code, a connect link (which also supplies the server) or a token. A code is redeemed at `POST /auth/exchange` first |
| `auth_exec` | `name, params?` | `POST /actions/builtin/auth/{name}` run as root; `solx:auth` only, server builds only |
| `config_get` | — | none — `{raw, effective, path, appdata, restartKeys}` for `solx-config.json`; `solx:config` only |
| `config_update` | `changes` (top-level keys; `null` removes) | none — validated write, returns `{config, changedKeys, restartRequired}`; `solx:config` only |

The `config_*` commands have no HTTP route, by design, and are not in
`solx:default`: they need the separate `solx:config` permission, which the
desktop app grants only to its Settings window. Package widgets run as JS in
the main window, and config holds the `command_actions`/`allowed_base_urls`
allowlists that solx keeps out of reach of actions, MCP and scripts. Server
builds only; a client build refuses them.

`auth_exec` is likewise its own `solx:auth` permission, granted only to the
Settings window. It runs one of the four `/builtin/auth/*` actions (and
nothing else) as root, which ordinary IPC can't: plain `actions_exec` calls
carry no principal, so the builtins refuse them, and a package widget can't
mint access for itself.
