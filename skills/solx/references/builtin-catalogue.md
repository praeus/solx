# The `/builtin` catalogue

Everything under `/builtin` is a native (`internal`) action. It is always
present, needs no package, and is what the agent uses for CRUD, files, HTTP,
secrets and introspection.

Each entry below gives the reference (`/builtin/<group>/<name>`), what it does
and its parameters. The parameter block is the action's `paramTypeRef` schema -
the same thing `explore_tools` prints.

The `act__` tool name is the reference with `/` replaced by `__` and an
`act__` prefix: `/builtin/document/entity-save-document` becomes
`act__builtin__document__entity-save-document`.

## Common list/search options

`entity-list-*`, `entity-list-*-paths` and `search-actions` share one option
set:

| Option | Meaning |
| --- | --- |
| `pathPrefix` | only entities under this path |
| `limit`, `offset` | pagination |
| `filterField`, `filterValue` | equality filter on a column (snake_case, e.g. `action_type`) |
| `sortBy`, `sortOrder` | sort column (snake_case) and `asc` / `desc` |
| `dateAfter`, `dateBefore` | RFC 3339 bounds |

`search-documents` takes `q`, `pathPrefix`, `typeRef`, `linkedTo`, `limit`,
`offset` instead. `linkedTo` restricts hits to documents that link to a given
full reference.

## `/builtin/action` - the catalogue itself

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `entity-list-actions` | List actions | list options |
| `entity-list-action-paths` | List distinct action paths with counts | list options |
| `entity-get-action` | Fetch one action | `path?`, `name` |
| `entity-save-action` | Create or update (upsert) an action | `path?`, `name`, `caption?`, `description?`, `capabilities?`, `phrases?`, `category?`, `paramTypeRef?`, `resultTypeRef?`, `actionType?`, `fnName?`, `binName?`, `actionConfig?`, `trusted?` |
| `entity-delete-action` | Delete an action | `path?`, `name` |
| `search-actions` | Full-text + faceted search over actions | `q?`, list options |
| `start` | Run an action detached; returns `invocation_id` at once | `path?`, `name`, `params?` |
| `poll` | Check (or long-poll) a detached invocation | `invocation_id`, `wait_secs?` (cap 60) |
| `stop` | Ask a detached invocation to stop | `invocation_id`, `force?`, `grace_secs?` |
| `cancelled` | Has this invocation been asked to stop? | - |

`start` needs a long-lived host (`solx-server` / `solx-mcp`); the CLI exits
when the action finishes, so it cannot host a detached run.

## `/builtin/document`

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `entity-list-documents` | List documents | list options |
| `entity-list-document-paths` | List distinct document paths with counts | list options |
| `entity-get-document` | Fetch one document | `path?`, `name` |
| `entity-save-document` | Create or update (upsert) a document | `path?`, `name`, `title?`, `summary?`, `typeRef?`, `contents?`, `author?`, `pubDate?`, `confidence?`, `links?` |
| `entity-delete-document` | Delete a document | `path?`, `name` |
| `get-field` | Read one field of `contents` | `path?`, `name`, `field` |
| `set-field` | Write one field of `contents` | `path?`, `name`, `field`, `value` |
| `get-field-at-path` | Read a nested field of `contents` | `doc_path?`, `name`, `path` |
| `set-field-at-path` | Write a nested field of `contents` | `doc_path?`, `name`, `path`, `value`, `create?` |
| `search-documents` | Full-text + faceted search | `q?`, `pathPrefix?`, `typeRef?`, `linkedTo?`, `limit?`, `offset?` |

`typeRef` is required when a document is created. `path` in the
`*-at-path` pair is slash-separated inside `contents` (e.g. `metadata/tags/0`);
`create: true` creates missing parents along the way.

`links` is a list of `{kind, target, field?, title?, description?}` where `kind`
is `doc_ref` or `url`.

## `/builtin/type`

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `entity-list-types` | List types | list options |
| `entity-list-type-paths` | List distinct type paths with counts | list options |
| `entity-get-type` | Fetch one type | `path?`, `name` |
| `entity-save-type` | Create or update (upsert) a type | `path?`, `name`, `description?`, `schema?`, `groups?` |
| `entity-delete-type` | Delete a type | `path?`, `name` |

`schema` is a JSON Schema document, required on create. Resolving a type
reference is just splitting `/types/x/Y` into path and name and fetching it -
there is no separate resolve route.

## `/builtin/file`

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `file-list` | List stored paths under a prefix | `prefix?` |
| `file-get` | Read bytes | `rel_path` |
| `file-put` | Write bytes | `rel_path`, `content`, `encoding?` (`utf8` \| `base64`) |
| `file-copy` | Copy a file | `source`, `dest` |
| `file-delete` | Delete a file | `rel_path` |
| `dir-copy` | Recursively copy a directory | `source`, `dest` |
| `dir-delete` | Recursively delete a directory | `rel_path` |

`rel_path` is relative to the files root and may contain `/`. Traversal is
rejected by the store.

## `/builtin/web`

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `http-request` | Issue an HTTP request | `url`, `method?`, `headers?`, `body?`, `body_encoding?`, `timeout_secs?` |
| `open-url` | Open a URL in the system browser | `url` |
| `stream/start` | Start a streaming request; returns `stream_id` + status when headers arrive | `url`, `method?`, `headers?`, `body?`, `body_encoding?`, `timeout_secs?` |
| `stream/poll` | Drain newline-delimited JSON chunks | `stream_id`, `cursor?`, `wait_secs?` |
| `stream/close` | Stop the reader task and drop its buffer | `stream_id` |

`method` defaults to `GET`; `timeout_secs` defaults to 30 (for `stream/start`
it is the *connect* timeout only - a stream has no overall timeout).

Every outbound URL must match an entry in `allowed_base_urls` in
`solx-config.json` (exact scheme, host and port, path under the entry).
Redirects are held to the same list.

## `/builtin/secrets`, `/builtin/env`

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `secrets/get-secret` | Read a secret scoped to the calling action | `name` |
| `secrets/set-secret` | Write a secret scoped to the calling action | `name`, `value` |
| `env/get-env` | Read an environment-store variable | `key`, `namespace?` |
| `env/set-env` | Write an environment-store variable | `key`, `value`, `namespace?`, `persist?` |

The default namespace is `default`, and that is where `env_mappings` entries
land. `persist: true` also writes the value into `solx-config.json` - in
plaintext, so use `set-secret` for anything sensitive.

Secrets resolve only against the *calling action's* own `action_config.secrets`
names, in that action's package scope. An action's config lists secret *names*
only; inline credentials under `actionConfig.auth` are refused on save.

## `/builtin/console`

Every action has a console: a rolling log of what it printed. The MCP server
tails it and streams it back as `notifications/progress`, so a model sees log
lines while a long call is still in flight.

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `console/list` | List known consoles, most recent first | `prefix?`, `limit?` |
| `console/read` | Read entries oldest-first from a sequence | `action_ref`, `from_seq?`, `limit?` |
| `console/tail` | Like `read`, but long-polls when nothing new | `action_ref`, `cursor?`, `limit?`, `wait_secs?` |
| `console/clear` | Drop entries from the front of a console | `action_ref`, `before_seq?` |
| `console/print` | Write an entry to the *calling action's* console | `level?`, `message?`, `data?` |
| `console/copy` | Copy one invocation's entries into the caller's console | `from_action_ref`, `invocation_id?`, `cursor?`, `limit?`, `label?` |

`print` and `copy` require an action caller - they cannot be called from the
CLI, MCP or HTTP directly. `console/read` and `console/tail` on *another*
action's console are unrestricted.

## `/builtin/background`

Background commands are one long-lived process serving many calls.

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `background/list` | List the background commands this host runs | `backgroundNamespace?` |
| `background/stop` | Stop one (shutdown, then kill after the grace period) | `backgroundNamespace`, `backgroundName` |

## `/builtin/auth` - root only

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `create-connect-code` | Mint a single-use 8-digit code another device redeems | `label?`, `code_ttl_secs?` (600), `token_ttl_secs?`, `public_url?`, `scopes?`, `preset?` |
| `issue-client` | Mint a client token directly | `label`, `token_ttl_secs?`, `scopes?`, `preset?` |
| `list-clients` | List working client tokens (never the tokens) | - |
| `revoke-client` | Revoke a client token by id | `id` |

`preset` is one of `read-only`, `tools`, `editor`, `full`. With neither
`scopes` nor `preset`, the token holds everything.

## `/builtin/oauth`

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `oauth/start` | Start a local OAuth 2.0 authorization-code loopback listener | `port?` (8765) |
| `oauth-await` | Block until the loopback for a state receives its callback | `state_value`, `timeout_secs?` |
| `oauth-stop` | Stop a loopback listener | `state_value` |

## `/builtin/test`

| Reference | Purpose | Parameters |
| --- | --- | --- |
| `test/probe` | Report the principal, caller and exclusion mode | - |
| `test/grant` | Report the scopes a call from here is checked against | - |

Useful when a call is refused and you need to know which identity and scope set
applied.
