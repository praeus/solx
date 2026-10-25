# Security Policy

## Reporting a vulnerability

Please report security issues privately through GitHub's **Report a
vulnerability** button on this repository's Security tab, rather than opening
a public issue.

Include what you did, what happened, and what you expected. A proof of
concept helps but isn't required. Expect an initial response within a week.

solx is early-stage and pre-1.0, maintained by one person. There is no
embargo process, no CVE pipeline, and no backport policy — fixes land on
`main`. Only `main` is supported.

## Threat model

solx exists to execute things on request: shell commands, remote HTTP calls,
and WASM components, driven by a CLI, an HTTP server, or an LLM through MCP.
That makes the boundaries worth stating precisely.

**Assume a solx instance runs with your full user privileges.** It is not a
sandbox for untrusted operators, and it is not a multi-tenant system. The
controls below limit what *content* — a model, a package, a WASM guest — can
reach. They do not limit what a person holding your server token or your
shell can do.

### What is enforced

**Command actions cannot hold an executable string.** A `command` action's
`fn_name` is a key into `command_actions` in `solx-config.json`, not a
command line. The lookup hard-errors if the key isn't registered — including
when the allowlist is entirely unset. The actions database can therefore only
ever hold a *pointer* to an out-of-band, human-approved definition. Where a
registered definition specifies `cwd`, that wins over the invoking action's
own config: the allowlist author decides where an approved command runs.

**Outbound HTTP is prefix-allowlisted.** Every URL solx fetches on a caller's
behalf must start with an entry in `allowed_base_urls`: a `webhook` action's
resolved URL, and the URL given to `/builtin/web/http-request`,
`/builtin/web/stream/start` or `/builtin/web/open-url`. All four share one
check (`solx-actions::net::check_outbound_url`), applied before any
connection is opened — so a denied webhook leaves no trace beyond the error,
and a denied built-in makes no request at all. `open-url` additionally
refuses any scheme but `http`/`https`, since `file:`/`javascript:`/`about:`
carry no host for a prefix to constrain and it hands the URL to the user's
real browser session.

An entry is matched by origin, not as a raw string: the URL's scheme, host
and port must equal the entry's (an entry with no port means the scheme's
default; `http://host:` means any port), and only the path is a prefix
match. Both sides are compared parsed and normalised, so
`https://api.example.com` does not admit `https://api.example.com.evil.net`
or `https://api.example.com@evil.net`, and `..` cannot climb out of an
allowed path. Redirects are checked hop by hop against the same list, so an
open redirect on an allowed host is not a way out to an unlisted one.

A Webhook's OAuth `token_uri` is held to the same list, and is checked
before any credential is read, so a client secret, refresh token or signed
assertion is never sent to an unlisted host.

The `/builtin/web/*` built-ins were ungated until this check was extracted
and shared; before that, anything able to exec an action — a WASM guest, an
MCP client, a widget with a bearer token — could use `http-request` as an
unrestricted egress proxy. The former config key `allowed_webhook_base_urls`
is still read, and unioned in, so an existing config keeps working.

Both allowlists are **deny-by-default**. An empty or absent allowlist denies
everything rather than allowing everything.

**Guests cannot grant themselves execution.** `entity-save-action` and
`entity-delete-action` refuse to create, modify, or delete `command` and
`webhook` actions. That built-in is the only route to action creation
available to an MCP client, a WASM guest, or a `.solx` script, since MCP has
no separate CRUD layer. The check reads the *stored* row, not just the
incoming payload, because `save` is a merge-upsert — otherwise a payload
carrying only `fn_name` could silently repoint an existing command action.

The CLI reaches `ActionManager::save` directly and is unaffected; package
installs depend on that. The HTTP `/actions/save` route does not apply this
particular check, deliberately: anyone holding the server token can already
execute command actions, so gating it there would buy nothing and would
break the CLI in remote mode. The checks below on `/packages/`, keyed and
locked actions do apply there.

**Built-ins can't be added, aliased or edited.** Built-in actions come only
from solx's own catalogue. No one, operators included, can save a built-in
(`internal`) action, and one that isn't in the catalogue doesn't run. So no
action can run a built-in's handler under another path, where a tooling
exclusion on the built-in wouldn't apply. Every built-in is locked, and
restored from the catalogue whenever solx starts.

**Content can't write under `/packages/`.** An MCP client, a WASM guest, a
script, a client or action token over REST, and the desktop app's main
window can't save or delete an action under `/packages/`. The one exception
is a package's own action, within its own package. Otherwise content could
plant a row that a later install would adopt and lock as the package's own.

**Packages declare their grants.** A package's `solx-package.json` lists the
`command_actions` and `allowed_base_urls` it needs. Install grants
exactly those and records them on the installed-package row; uninstall
revokes exactly those, unless another installed package also declares the
same key or prefix. A collision between packages is a logged warning, not a
silent overwrite.

**Installed actions are locked.** Every action a package install leaves
under `/packages/<name>/` is locked, unless the manifest lists it under
`unlocked_actions`. A locked action can be run by anyone who can run actions,
but it can be changed, deleted, locked or unlocked only by an operator: the
`solx` CLI (`solx action lock|unlock`), a package install, the root token over
REST, or the desktop app's Settings window. An MCP client is not an operator,
even when it connects with the root token, and neither is a WASM guest, a
script, or the desktop app's main window, which hosts widgets.

Locking a Script or WASM action pins its artifact: the file it runs and the
SHA-256 of its bytes. Those bytes are checked before every run, so replacing
the file can stop a locked action from running but never makes it run
different code — which matters because the code runs with that action's
scoped secrets. Content can't write or delete a pinned file through the file
built-ins, REST `/files` (anything but the root token), or the main window.

**Secrets are scoped to the calling action.** `get-secret`/`set-secret`
resolve only against the *calling* action: a name it lists in its own
`action_config.secrets`, in its own scope. The caller is threaded host-side
and never serialized, so a client cannot spoof it.

**Each package has its own key and its own entries.** A package's actions
share one AES key, generated by solx and kept in the actions database —
never in a config, an install script or the credential manager — and
forgotten when the package is uninstalled. A standalone action has its own.
Each secret is stored in the OS credential manager under its package (or
action), so packages can't read or clobber each other's secrets even if
they use the same name.

**Action config never holds a secret.** `action_config.secrets` lists only
secret names, so no read (`get`/`list`/`search`, HTTP, MCP) can return a
key. Inline credentials in `action_config.auth` are refused — they come
from a `<field>_secret` pointer or a keyring reference. A keyring reference
can't name solx's own credential-manager entries (the root and client
tokens, the local CA, the scoped secret store), either to read one or to
write a rotated refresh token over one.

**Only root can create or change an action with credentials.** That covers
an action that declares secrets or whose `auth` has a keyring reference.
Root is the CLI, a package install, the desktop app's Settings window, or
HTTP with the root token. A client token, an MCP tool call, a WASM guest, a
script or the desktop app's main window cannot, so none of them can declare
a name in a package's scope, read a credential-manager entry, or swap the
code that runs with a key. Such an action is always locked, and a Script or
WASM one pinned: saving it locks it, it can't be unlocked while it holds
credentials, and a package reinstall leaves it locked. Rows from before this
rule are locked when solx starts.

**WASM guests are sandboxed by construction.** The `custom-action` world
imports only action dispatch, artifact reads, and logging.
There is no direct database, filesystem, network, or system access; every
sensitive operation must go through a named action, where the restrictions
above apply.

**The file store rejects traversal.** Absolute paths and `../` escapes out of
the files root are refused.

**The server binds loopback by default, and every route needs a bearer
token.** `solx-server` listens on `127.0.0.1` unless `server_host` in
`solx-config.json` (or `--host`) says otherwise. Every route except
`/health`, `/auth/exchange` and the static `/connect` page requires a bearer
token. Setting `server_host` to a LAN address or `0.0.0.0` allows non-local
access. In that case bearer tokens are the only gate, and CORS is limited to
the origins listed in `server_cors_origins`.

**Other machines get TLS when it's on, and never plain HTTP.** With
`server_tls` set, `solx-server` serves HTTPS, and plain HTTP on the same
port is accepted only from this machine; anyone else gets a 400. The desktop
app turns it on by itself for a network address on Windows
(`server_tls: "auto"`): it creates a certificate authority for this appdata
dir and renews the server certificate it signs. On Windows both keys are
non-exportable CNG keys, the authority's on the TPM when there is one. On
the TPM, a key can't leave the chip; in the software key storage provider,
non-exportability is a policy that an administrator, or malware running as
you, can get around. Elsewhere the keys are PEM files that any process
running as you can read. `solx-certs show` says which applies. A standalone
`solx-server` uses certificates from `solx-certs`, or your own PEM files.

**The root token is accepted from this machine only.** The root token is
generated on first start and kept in the OS credential manager. Requests
carrying it from any other machine are refused. Other devices each get their
own client token by redeeming a single-use connect code (`solx token
connect-code` on the host, or "Connect another device" in the desktop app's
settings). Client tokens expire after 90 days by default, and each can be
listed and revoked on its own (`solx token clients`, `solx token revoke`).
Only the root token can create codes or manage client tokens.

**Command actions get a token that dies with them.** Each external Command
invocation is handed its own token as `SOLX_SERVER_TOKEN`, revoked as soon
as the invocation ends.

**Connect codes are guarded against guessing.** A code is 8 random digits,
works once, and expires after 10 minutes. Every failed attempt, from any
address, counts against a shared budget: after 10 failures every pending code
is invalidated. The database stores only hashes of tokens and codes, but a
code's hash doesn't hide it from someone who can read `solx-registry.db`,
since there are only 10^8 codes. Such a person already has local access, and
codes are short-lived.

**MCP access follows the tooling exclusions.** `exclusions.tooling` in
`solx-config.json` (along with `tool_exclude` and the `solx:hidden` tag)
removes actions, documents and types from what a model can reach over MCP.
Excluded actions are left out of discovery, and they're refused even when
called by name. The scope covers the whole call chain, not just the tool the
model named: an excluded action is refused even when a script, a WASM guest
or `action-start` calls it on the model's behalf, and every built-in in the
chain is forced into the tooling scope, so a model can't opt out with its
own `excludeMode`.

### What is not enforced

These are known gaps, not oversights. They're accepted for now because
everything running through solx today is trusted by construction.

- **Anything saved to the actions database is executable.** There is no
  approval or confirmation step at execution time.
- **Client and action tokens aren't scoped.** A client token, or a Command
  action's own token, can do everything the root token can except create
  connect codes and manage client tokens. What they add is expiry,
  revocation and knowing who is calling.
- **A reverse proxy on the same machine hides remote callers.** Every
  request then arrives from loopback, so the root token is accepted through
  the proxy. solx doesn't support running behind one; it serves TLS itself.
- **Off-loopback TLS is opt-in outside the desktop app on Windows.** A
  standalone `solx-server`, or the desktop app elsewhere, serves plain HTTP
  to the network until `server_tls` is set.
- **solx clients don't pin the server's certificate authority yet.** They
  trust whatever their OS trust store and `server_ca_file`/`SOLX_CA_FILE`
  trust. Browsers and other HTTPS clients can only ever do that, so
  importing the solx root on a machine trusts it for every host name there.
- **Certificates can't be revoked.** Reissue the server certificate, or
  reset the certificate authority and set every device up again.
- **The in-process env store is global.** Any caller can read and write
  `get-env`/`set-env`, including the system variables mapped in through
  `env_mappings`. Don't map a secret into it.
- **No package signing or verification.** `install-package` will run the
  `install.solx` of any local directory you point it at, and grant whatever
  its manifest declares.
- **Artifact reads are unrestricted.** A WASM guest can read any path in the
  file store; there is no per-artifact permission check.
- **Loopback capabilities are bearer-style.** OAuth loopbacks, action
  consoles, and HTTP streams are reachable by anyone who knows the
  unguessable id, without a further caller check.
- **`solx-server` has no general rate limiting or audit log.** Only connect
  code exchanges are throttled. `solx token rotate` exists, but running hosts
  keep accepting the old root token until they restart.

See [docs/security-design.md](docs/security-design.md) for how each control
is implemented, the full list of known gaps, and the intended direction.

## Running solx safely

- **Keep the allowlists narrow.** Every entry in `command_actions` is a shell
  command something can trigger. Register the specific commands you need, not
  wrappers that take arbitrary arguments.
- **Re-lock after changing a locked action's artifact by hand.** A locked
  action whose file changed refuses to run until `solx action lock <ref>`
  (or a reinstall) pins the new bytes.
- **Read a package's manifest before installing it.** The `command_actions`
  and `allowed_base_urls` blocks in `solx-package.json` are a complete list
  of the shell access and outbound hosts it's asking for.
- **Use tooling exclusions to limit what a model can reach.** List anything
  a model shouldn't see or call under `exclusions.tooling` in
  `solx-config.json`, for example
  `"tooling": { "actions": ["*/_private/*"], "documents": ["/personal"] }`.
  `SOLX_MCP_PATH_PREFIX` only sets which actions a model *discovers* by
  default. It doesn't restrict what can be called, so don't rely on it for
  access control.
- **Treat every token as a credential.** The root token and each client
  token are full access to the instance, including command execution. solx
  keeps them in the OS credential manager rather than `solx-config.json`;
  `solx token show` prints the root token and `solx token rotate` replaces
  it. Don't commit tokens, and don't put them in a shared config file.
  Command actions receive their own token as `SOLX_SERVER_TOKEN` and should
  never read one from anywhere else.
- **Give each device its own token, and revoke what you don't use.** Use a
  connect code per device rather than copying the root token around, and
  revoke a device's token (`solx token revoke`) when you stop using it or
  lose it.
- **Treat a connect code like a password while it's live.** Anyone who has
  it can connect until it is used or expires. Share it over a channel you
  trust, or read it off the screen.
- **Think before exposing `solx-server` beyond loopback.** Setting
  `server_host`, or tunnelling the server, makes a shell-execution endpoint
  reachable from the network, protected only by bearer tokens. Turn on TLS
  (`server_tls`; see [docs/tls-plan.md](docs/tls-plan.md)), keep it on a
  network you trust, and set `server_cors_origins` to only the browser
  origins you use.
- **Import the solx root only where you need it.** A browser or HTTPS
  client on another device needs `solx-root.crt` in that device's trust
  store, and then trusts it for every site. solx's own clients can use
  `server_ca_file` or `SOLX_CA_FILE` instead, which affects only solx.
- **Prefer WASM for third-party logic.** The `custom-action` world is the
  only genuinely sandboxed dispatch kind.
