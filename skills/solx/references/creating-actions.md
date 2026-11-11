# Registering actions, packages, secrets and scopes

## The action row

An action is one row in `db/solx-actions.db`. These are its columns:

| Field | Meaning |
| --- | --- |
| `path`, `name` | its reference, `/path/name` |
| `actionType` | the dispatcher: `wasm`, `webhook`, `command`, `script`, `internal` |
| `fnName` | what the dispatcher runs - depends on `actionType` |
| `binName` | the artifact file name (the `.wasm` component, or the `.solx` source) |
| `description`, `caption` | what a model reads to decide to call it |
| `paramTypeRef` | the type whose JSON Schema is the action's parameters |
| `resultTypeRef` | the type describing the result |
| `phrases` | extra search terms for `explore_tools` |
| `category`, `capabilities` | grouping and metadata |
| `actionConfig` | per-action settings, including `scopes` and a `secrets` name list |
| `trusted` | whether the action is trusted |

`description` is the text `explore_tools` shows the model, so write it as
"what this does and when to use it", not as a label. `phrases` widen the search
terms beyond what the name and description contain.

### The five kinds

**`webhook`** - `fnName` is the URL. `{placeholders}` in it are filled from the
parameters. OAuth and bearer auth are resolved for you.

```sh
solx save action /weather/forecast --json '{
  "actionType": "webhook",
  "fnName": "https://api.weather.gov/points/{lat},{lon}",
  "description": "Get the forecast for a coordinate pair.",
  "paramTypeRef": "/weather/ForecastParams"
}'
```

The URL must match an entry in `allowed_base_urls`.

**`command`** - `fnName` is a **key** into `command_actions` in
`solx-config.json`, never a command string:

```jsonc
"command_actions": {
  "echo-number": { "command": "echo", "cwd": "C:\\work" }
}
```

```sh
solx save action /demo/echo --json '{"actionType":"command","fnName":"echo-number"}'
```

**`wasm`** - `fnName` is the export name of a `wasm32-wasip2` component; the
component itself is `binName`, stored in the file store under
`files/actions/shared/`. WASM guests are sandboxed by construction: the
custom-action world imports only action dispatch, artifact reads and logging -
no direct database, filesystem or system access.

**`script`** - `fnName` is the script's operation name and `binName` is a
`.solx` artifact. Scripts compose other actions.

**`internal`** - `fnName` is a native handler name. This is what `/builtin` is
made of; you rarely add one by hand.

### Types give the action a schema

`paramTypeRef` points at a type you saved separately. The type's JSON Schema
is the action's parameter contract, and it is what `explore_tools` prints.

```sh
solx save type /types/custom/Person --json '{
  "schema": {"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}
}'
solx save action /people/lookup --json '{
  "actionType":"internal","fnName":"people-lookup","paramTypeRef":"/types/custom/Person"
}'
```

A document's type is validated on save, so a type is also a data contract for
the document store.

## What a guest may and may not create

`entity-save-action` **refuses** to create, modify or delete `command` and
`webhook` actions for a non-root caller - a model, a WASM guest or a `.solx`
script cannot grant itself shell or outbound HTTP. It checks the *stored* row,
not just the payload, because `save` is a merge-upsert.

That built-in is the only route to action creation available to an MCP client,
a WASM guest or a script. The operator paths - the CLI, the root token, the
desktop app - are not confined.

Everything else is deny-by-default too:

- **Command allowlist.** A `command` action's `fnName` is a pointer into
  `command_actions` in `solx-config.json`. The database can never hold an
  executable string, only a pointer to an out-of-band, admin-approved
  definition. An unregistered key is refused, including when the allowlist is
  entirely unset.
- **Outbound allowlist.** Every outbound URL - a `webhook` action's, and any
  handed to `/builtin/web/http-request`, `/builtin/web/stream/start` or
  `/builtin/web/open-url` - must match an entry in `allowed_base_urls`: exact
  scheme, host and port, and a path under the entry's. Redirects are held to
  the same list.

## Packages

A package is a directory with a `solx-package.json` manifest (at least `name`
and `version`), an `install.solx` script and optionally an `uninstall.solx`.
(`package.json` is still read as a fallback, with a warning.)

```sh
solx install-package ./path/to/package     # a directory, or a .zip of one
solx list-packages
solx uninstall-package <name>
```

The manifest may declare:

| Key | Effect |
| --- | --- |
| `command_actions` | allowlist entries granted into the global allowlist at install, revoked at uninstall |
| `allowed_base_urls` | outbound prefixes, same lifecycle |
| `scopes` | every scope the package's actions may declare - what a reader reviews before installing |
| `unlocked_actions` | actions under `/packages/<name>/` left unlocked |

Uninstall revokes exactly what the install granted, unless another installed
package also declares the same key or prefix.

Install-time and uninstall-time scripts run as the operator, but a package
script may only save or delete actions under its own `/packages/<name>/`, may
not install or uninstall a package, and may not declare a scope its manifest
does not list. A package name must be a single path segment.

Actions a package installs are **locked**: content can run them but not change
or delete them, and a locked Script/WASM action runs only the artifact it was
locked against. `solx action lock` / `unlock` do the same by hand.

## Secrets

`get-secret` and `set-secret` resolve only against the *calling action's* own
`action_config.secrets` names, in its package's scope. Each package has one
key, generated by solx and kept in the actions database, plus its own
credential-manager entries.

`action_config` lists secret **names** only, and inline `auth` credentials are
refused on save - so nothing that reads an action can return a secret. Only
root can create or change an action that has secrets.

## Scopes

Each device's token carries scopes (`read:document:/notes/*`,
`exec:/packages/solx-google/*`, or a preset: `read-only`, `tools`, `editor`,
`full`), and each action declares the scopes it runs with. A call is checked
against its immediate caller's scopes and refused with a `403` outside them.

An action with no scopes can call nothing until an operator grants it some:

```sh
solx action scopes /packages/solx-google/send-gmail-message --add "exec:/packages/solx-google/*"
solx scopes audit
```

`scope_enforcement: "audit"` in `solx-config.json` lets calls through while you
work out what your actions need; `"enforce"` refuses them and records the
refusal.

## Locking

```sh
solx action lock /packages/solx-foo/thing     # pin the current artifact
solx action unlock /packages/solx-foo/thing
```

Locking a Script or WASM action pins its artifact, so a later swap of the file
does not change what runs. A locked action changes only through the CLI, a
package install, the root token, or the desktop app's Settings window.

## What is not enforced

Anything saved to the actions database is executable, and local entry points
(the CLI, the desktop app, the stdio MCP server) are unlimited. An action's
scopes serve anyone allowed to run it. There is no package signing.

Treat a solx instance as running with your own privileges, and scope what you
install accordingly. `core/SECURITY.md` has the full threat model.
