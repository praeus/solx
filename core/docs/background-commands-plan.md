# Background commands — design plan

Status: **In progress** (2026-10-28), on the `background-commands` branch.
Steps 1–5 and 7 are implemented (tested in
`solx-actions/tests/background.rs`), and step 6 on the `background-commands`
branch of solx-packages (`solx-package-lib/src/background.rs`, with a
"Background commands" section in its `HOWTO.md`); step 8 is not started.

## Goal

Every Command action today is one-shot. `run_command`
(`solx-actions/src/exec/command.rs`) works like this:

- It spawns the allowlisted command and writes the params to stdin as JSON.
- It closes stdin and reads one JSON result from stdout.
- When the child exits, its console loopback registration and its
  `SOLX_SERVER_TOKEN` are released.

That suits a command that does one thing and exits. It doesn't suit a
package that wraps something expensive to start or naturally long-lived: a
browser session, a model server, a file watcher. Such a package pays the
startup cost on every call and can't keep state between calls.

This plan adds **background commands**:

- A background is a long-lived child process that keeps its stdin and
  stdout open.
- It serves many action calls over them, and makes calls back into the
  host the same way.
- At most one runs per `(backgroundNamespace, backgroundName)`. The
  namespace is normally the package whose actions talk to that one
  process.
- A background starts on first use and is stopped explicitly or when the
  host terminates.

## Decisions

| Question | Decision |
|---|---|
| New `ActionType`? | No. A background action is a `Command` action whose `action_config` has a `background` object. It is still subject to the `command_actions` allowlist. |
| How many instances? | One per `(backgroundNamespace, backgroundName)` per host process. |
| `start` when already running | **Idempotent.** It returns the running instance's status. A per-key lock serializes concurrent starts. |
| Autostart from `solx-config.json`? | **No.** Any operation starts the background through its `start` action when it isn't running. |
| Shutdown | Explicit `stop`, or **every** running background when the host terminates. |
| How does the background call the host? | **Over stdio only.** solx operations, `exec` first, come back as requests and run in-process, with scopes enforced. A background gets no `SOLX_SERVER_URL`/`SOLX_SERVER_TOKEN`. |
| Restart on crash? | Not in the first version. A crash fails pending calls and removes the instance, and the next operation starts it again. |

## Where things stand

| Piece | Where | Relevance |
|---|---|---|
| One-shot Command execution | `solx-actions/src/exec/command.rs` `run_command` | Resolves the allowlist, cwd and env, then spawns `cmd /C` / `sh -c`. Background spawns reuse the resolution (see step 1). |
| Command dispatch | `solx-actions/src/dispatch.rs`, the `ActionType::Command` arm of `exec_as_with_inner` | The single place a background action branches off. |
| Per-manager runtime state | `LocalActionManager::running` (`solx-actions/src/lib.rs`) | The precedent for keeping runtime handles on the manager instead of in a process global. Tests stay isolated. |
| Nested calls with an action's identity | `LocalActionManager::exec_as` + `ExecContext` (`solx-surface/src/exec_context.rs`) | How a WASM guest calls back as its action. A background's `exec` requests go the same way. |
| Scopes | `action_scopes`, `ScopeCheck::for_exec` | Enforced by default since `0828e4f`. Background→host calls must not get around them. |
| Grace period | `ConfigService::stop_grace_secs` | Reused for how long `stop` waits before killing. |
| Package runtime | `solx-package-lib` (`solx-packages`) | Gets a `background` module for the child's side. It has no dependency on core: its env-var contract is mirrored, not shared. |
| Host shutdown hooks | solx-server `main.rs` graceful shutdown; `browser/src-tauri/src/lib.rs` `RunEvent::Exit` → `server::shutdown` | Where `stop_backgrounds()` gets called. |

## Design

### Action config

```json
"action_config": {
  "background": {
    "backgroundName": "session",
    "backgroundNamespace": "solx-firefox",
    "backgroundOperation": "start"
  }
}
```

`backgroundOperation` is one of:

- **`start`**: spawn the action's `fn_name`, resolved through
  `command_actions` exactly as a one-shot Command is. Idempotent.
- **`stop`**: send a `shutdown` request, wait `stop_grace_secs`, then kill
  the process tree and clean up. Stopping something that isn't running is a
  no-op.
- **anything else**: an operation name. The call's params are forwarded as a
  `CommandRequest` with that `op`. The action's result is the matching
  `CommandResponse`.

A package therefore declares one `start` action, an optional `stop` action,
and one action per operation, all with the same namespace and name.

### Starting on first use

An operation that finds no running instance:

1. Finds the start action: under the same package path, with matching
   `backgroundNamespace`/`backgroundName` and `backgroundOperation: "start"`.
   `list` filters by path prefix but never on `action_config`, so this lists
   the package's actions and matches in Rust. If there's no match, it's an
   error that names the missing start action.
2. Runs it through `exec_as`, as the triggering action, so the scope check
   applies to that call like any other nested call.
3. Forwards its own request once the start returns.

The background always runs with the **start action's** `fn_name` and scopes,
whichever operation arrived first. Concurrent first calls wait on the
per-key start lock, so the process is spawned once and each call is
forwarded to it.

### Wire protocol

Newline-delimited JSON on stdin (host → background) and stdout
(background → host). Each line is one message, and messages are correlated
by `id`. Both sides send requests and responses over the same pipes.

```rust
struct CommandRequest {
    id: u64,
    op: String,
    params: Value,
    /// Host → background only: the invocation this call belongs to, so
    /// console output and cancellation checks map to the right run.
    invocation_id: Option<String>,
}

struct CommandResponse {
    id: u64,
    ok: bool,
    result: Option<Value>,
    error: Option<String>,
}
```

- Each message carries a `kind` (`"request"` / `"response"`) so either side
  can tell them apart.
- Each side allocates ids for the requests it sends, so ids only need to be
  unique per direction.
- The host defines these in `solx-surface/src/background.rs`, and
  `solx-package-lib` mirrors them.
- Reserved host → background op: `shutdown`.
- Host ops the background can call, to start with:
  - `exec` `{path, name, params}`: run an action and return its result.
  - `console.print` `{level, message, data, invocation_id?}`.
  - New ops (documents, files) are extra match arms, not protocol changes.
- stderr isn't part of the protocol. A reader task drains it into the
  action's `SOL_LOG_DIR` log, so a chatty child can never fill the pipe and
  stall.

### Core: `solx-actions/src/background/`

- **`mod.rs`**:
  - `BackgroundManager`, a `Mutex<HashMap<(String, String), Arc<BackgroundHandle>>>`
    plus per-key start locks.
  - `dispatch(...)`, called from the Command arm.
  - It is a field on `LocalActionManager`, like `running`: a singleton per
    host process without being a process global.
- **`handle.rs`**: `BackgroundHandle` holds:
  - the child;
  - an mpsc sender feeding a single stdin writer task;
  - `pending: Mutex<HashMap<u64, oneshot::Sender<CommandResponse>>>`;
  - the start action's ref and scopes;
  - the status and start time.
- **`reader.rs`**: the dispatch task, a tokio task reading stdout lines.
  - A response completes its `pending` entry.
  - A request is handled on its own task, so a slow `exec` doesn't block the
    reader. An `exec` request runs `LocalActionManager::exec_as` through
    `self_ref`, with an `ExecContext` whose caller is the start action. The
    response goes back through the writer.
  - On EOF, an unreadable line or the child exiting, every pending request
    fails with an error and the handle is removed.
- **`lifecycle.rs`**:
  - `start`, which is idempotent and locked;
  - `ensure_started`, the lazy path;
  - `stop` and `stop_all`;
  - process-tree containment (below).
- **Built-ins** (optional, last step): `/builtin/background/list` and
  `/builtin/background/stop`, so the CLI and web UI can see and stop running
  instances.

### Process lifetime

`kill_on_drop` only kills the shell `run_command` spawns, not what the
shell started. A one-shot command can live with that; a long-lived
background can't. So backgrounds run contained:

- **Windows:** a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
- **POSIX:** a new process group, killed as a group.

The same containment covers host exits that skip `stop_backgrounds()`, such
as a panic or a hard kill. When the host dies, the job handle closes and
takes the background's processes with it. (On POSIX this is best effort: a
`SIGKILL`ed host leaves the group running until the next start notices.)

### One per machine

The per-key lock covers one host process, but a CLI or stdio MCP process
that isn't attached to a server starts its own backgrounds, and a
background wrapping an embedded database must not run twice against the
same files. So a start first takes an OS file lock (`LockFileEx` /
`flock`) on `<appdata>/backgrounds/<namespace>/<name>.lock`
(`background/lock.rs`):

- The OS releases it when its holder dies, so there's no stale lock.
- It's released only once the process tree is gone, not when `shutdown`
  is sent.
- On POSIX the background inherits the locked descriptor (cleared of
  `FD_CLOEXEC` in the child only), so an orphan left by a `SIGKILL`ed host
  still holds it.
- The holder writes `<name>.owner.json` (host, pid, since), since a locked
  file can't be read on Windows. A refused start names it.
- `background_lock_wait_secs` (default 0) makes a start wait for the lock
  instead of failing at once.

The embedded database's own lock stays the last line of defense.

### Scopes

This is the part with real security weight. Today the scopes a Command
child has are exactly what its token carries. A background has no token, so
the host must enforce them in-process:

- Every `exec` request is checked by `ScopeCheck` against the start
  action's `action_scopes`, exactly like a nested call from that action.
  Refusals are recorded like any other.
- Exclusions apply as for any nested call.
- A background can never do more than its start action could, whichever
  operation woke it.

### solx-package-lib: `src/background.rs`

- **`serve(handler)`**: reads requests from stdin and calls
  `handler(op, params) -> Result<Value, String>` concurrently for each.
  Responses go out through one mutex-guarded stdout writer. It returns on
  `shutdown` or end of input.
- **`host().call(op, params).await`**: a request from the background to the
  host. It has its own id counter and pending map, and shares the reader and
  writer with `serve`. Convenience wrappers: `host().exec(path, name, params)`
  and `host().print(level, message)`.
- **Logging**: the console loopback env vars are per invocation, so a
  background doesn't have them. While `serve` is running, `info`/`warn`/
  `error` send `console.print` requests tagged with the current call's
  `invocation_id`, and still mirror to stderr and the log file.
- **Stdout belongs to the protocol.** Anything a package prints to stdout
  outside these helpers corrupts the stream. The module docs need to say so
  loudly.
- **`serve_one_at_a_time(handler)`**: the same, with handlers run one after
  another in arrival order, for a package whose operations all go to a
  single-writer store. The reader keeps going meanwhile, so a handler
  waiting on a host call can't deadlock.
- **Shutdown**: on `shutdown` or end of input, `serve` refuses new requests,
  lets running handlers finish (after end of input, their pending host calls
  fail at once), and returns. A panicking handler answers with an error.

## Implementation steps

1. **Extract the shared spawn setup.** Move the allowlist, cwd, env and
   log-dir resolution out of `run_command` into a helper both paths call.
   One-shot behavior doesn't change.
2. **Protocol types.** `solx-surface/src/background.rs`, plus their mirror
   in `solx-package-lib`.
3. **Background module, core.** The manager, the handle, the reader task,
   start/stop, and process containment. Tests run against a fake background.
4. **Dispatch.** The `background` branch in the Command arm, lazy start,
   and the host-op handlers (`exec`, `console.print`) with scope checks.
5. **Shutdown wiring.** `LocalActionManager::stop_backgrounds()`, behind
   `App::shutdown()`, called from:
   - solx-server, on Ctrl-C or SIGTERM, after draining requests (capped at
     10s, so a long-lived stream can't hold it open);
   - the browser app's `RunEvent::Exit` and its `restart`, which exits
     without one;
   - solx-cli after a core command, and solx-mcp when its stdio session ends.
6. **solx-package-lib `background` module.** `serve`, `host()`, and routing
   logs to `console.print`.
7. **Built-ins and docs.** `/builtin/background/list|stop`, `http-api.md`,
   `security-design.md`, and a section in `HOWTO.md` for package authors.
8. **First user.** Convert one package (a small `solx-firefox` or
   `solx-ollama` operation) as the end-to-end proof.

Rough size: 1,500–2,200 lines including tests, split about two-thirds core,
one-fifth package-lib, the rest hosts and docs. Changes to existing code are
limited to step 1 (a mechanical extraction), the Command arm, a new
`LocalActionManager` field, and three shutdown hooks.

## Tests

- **Core, against a fake background.** The fake is a small test binary that
  speaks the protocol and can be told to echo, sleep, crash, call the host
  or ignore `shutdown`. Cases:
  - requests and responses match up, including out-of-order responses to
    concurrent requests;
  - a second `start` is a no-op that returns the running instance;
  - an operation with nothing running starts it via the start action, and
    ten concurrent first operations spawn exactly one process;
  - an operation with no start action defined returns a clear error;
  - `stop` exits cleanly on `shutdown`, and kills a child that ignores it
    after the grace period;
  - a crash fails every pending call, and the next operation starts a new
    instance;
  - `stop_backgrounds()` leaves no process behind, including a grandchild
    started by the shell.
- **Scopes:**
  - a background's `exec` inside the start action's scopes succeeds;
  - one outside them is refused and recorded;
  - starting through an operation doesn't widen the scopes.
- **solx-package-lib:** `serve` + `host().call` over in-memory duplex pipes:
  concurrent handlers, a host call made inside a handler, `shutdown`, and
  log routing.
- **End to end:**
  - Launch the browser app and call a non-start operation of the converted
    package from the CLI. The background should start, and
    `/builtin/background/list` should show it.
  - Call again: the same process should answer.
  - Quit the app and check in Task Manager / `ps` that nothing is left.
- **Regression:** `cargo test --workspace`. One-shot Command tests
  (`command_credentials.rs` and others) must pass unchanged.

## Out of scope

- Restart policies (`restart: "on-failure"`), health checks and idle
  shutdown.
- Sharing one background across host processes (CLI plus desktop app).
  Instances belong to the host that started them.
- Streaming responses or progress updates beyond `console.print`.
- A `blocking` (no tokio) variant of the package-lib module. `solx-firefox`
  is the only sync package, and it can be added when one needs it.
