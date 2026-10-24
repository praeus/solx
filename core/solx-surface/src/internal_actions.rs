//! The seam that lets a crate outside `solx-actions` contribute internal
//! (`fn_name`-dispatched) actions to its dispatcher, without `solx-actions`
//! needing to depend on that crate's concrete types, and without that crate
//! needing to depend on `solx-actions` (which would be circular for a crate
//! `solx-actions` itself also depends on, like `solx-console`).
//!
//! Lives here — the one crate with no internal dependencies — rather than in
//! `solx-actions`, so both `solx-actions` (the dispatcher) and any plugin
//! crate (a handler provider) can depend on it without depending on each
//! other.
//!
//! [`InternalActionHandler`] is deliberately narrower than the dispatcher's
//! own full context: it gets the four manager trait objects already defined
//! in [`crate::managers`] plus the calling [`Caller`], nothing crate-specific
//! (config services, the concrete action manager, task-spawn machinery). A
//! handler that needs something beyond that (e.g. `solx-console`'s
//! `action-start`/`stop`/`poll`, which need to spawn and abort detached
//! tasks) takes it via constructor injection instead — see
//! [`ActionExecutor`].

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use crate::auth::Principal;
use crate::exec_context::ExecContextInfo;
use crate::error::Result;
use crate::managers::{ActionManager, DocManager, FileStore, TypeManager};

// ── Caller ───────────────────────────────────────────────────────────────────

/// Identity of the action that initiated an execution.
///
/// Only one caller exists today: an action invoking another action — a WASM
/// guest calling `action-exec`, or a `Script` action's `exec` stage.
/// Everything else — the CLI, the MCP server, the HTTP
/// route, `solx-client` — enters through `ActionManager::exec`/`exec_for`
/// and has no action caller, i.e. `None`.
///
/// That containment is deliberate. A `Caller` is built by the host from a
/// row it just read, never parsed from a request, never serialized, and
/// never crosses the process boundary — so unlike a client-declared
/// identity it cannot be spoofed.
///
/// ## Caller vs. principal
///
/// The *authenticated* party behind a call is a separate value, the
/// [`Principal`], which enters only through `ActionManager::exec_for` from
/// a route that just ran `TokenAuthority::authenticate`. It is plumbed
/// explicitly (never a task-local) so a `tokio::spawn` on the way — e.g. a
/// detached `action-start` run — cannot silently drop it. Nested frames
/// inherit the originating principal for attribution, so an authorization
/// check must look at the caller *first*: see
/// [`ExecContextInfo::external_principal`]. The two travel together, with
/// the exclusion mode, in a [`crate::ExecContext`].
///
/// ## What it carries, and what it must not
///
/// Only the calling action's [`ActionSecrets`], behind a private field. The
/// caller's `cwd`, `auth`, and `headers` are never in scope, so there is no
/// way for a downstream handler to reach them even by accident.
///
/// **Invariant:** a `Caller` is read-only input to secret-key resolution.
/// No internal action may return it, or any key inside it, in its result —
/// `get-secret` returns the decrypted *value*, never the key that unlocked
/// it.
#[derive(Debug, Clone)]
pub struct Caller {
    action_ref: String,
    /// Minted fresh every time this frame is built — i.e. once per
    /// `exec_as` dispatch to a `Wasm`/`Script` action, since those are the
    /// only two call sites that construct a `Caller`. Stamped onto every
    /// console entry this invocation writes, so two concurrent runs of the
    /// same action (same console) stay separable even though they share a
    /// console.
    invocation_id: String,
    secrets: ActionSecrets,
}

impl Caller {
    /// Build a caller frame for an action. `secrets` is `None` for a frame
    /// that should resolve no secrets at all.
    pub fn from_action(action_ref: impl Into<String>, secrets: Option<ActionSecrets>) -> Self {
        Caller {
            action_ref: action_ref.into(),
            invocation_id: Uuid::new_v4().to_string(),
            secrets: secrets.unwrap_or_default(),
        }
    }

    /// Like [`Self::from_action`], but stamped with a caller-chosen
    /// `invocation_id` rather than a freshly minted one — used by a
    /// detached `action-start` run, where the id has to be known *before*
    /// execution begins so `action-stop`/`action-poll` have something to
    /// address.
    pub fn with_invocation(
        action_ref: impl Into<String>,
        secrets: Option<ActionSecrets>,
        invocation_id: impl Into<String>,
    ) -> Self {
        let mut caller = Self::from_action(action_ref, secrets);
        caller.invocation_id = invocation_id.into();
        caller
    }

    /// Full reference of the calling action, e.g. `/pkg/summarize`.
    pub fn action_ref(&self) -> &str {
        &self.action_ref
    }

    /// Identifies this one run of `action_ref`, distinct from any other
    /// concurrent or subsequent run.
    pub fn invocation_id(&self) -> &str {
        &self.invocation_id
    }

    /// Secret `name`, if this action may use it.
    pub fn secret(&self, name: &str) -> Option<SecretHandle> {
        self.secrets.resolve(name)
    }

    /// The secrets this caller may resolve.
    pub fn secrets(&self) -> &ActionSecrets {
        &self.secrets
    }

    /// Strips the secrets map, keeping only what a pluggable (non-hard-coded)
    /// internal-action handler is allowed to see — see [`CallerInfo`].
    pub fn info(&self) -> CallerInfo {
        CallerInfo {
            action_ref: self.action_ref.clone(),
            invocation_id: self.invocation_id.clone(),
        }
    }
}

/// The secret scope an action belongs to: the unit that owns one AES key
/// and one namespace in the credential manager.
///
/// Every action under `/packages/<pkg>/` shares `package:<pkg>`, so a
/// package's actions read each other's secrets and an update can add one
/// that does too. Any other action is its own scope, `action:<full ref>`.
/// The prefixes keep the two kinds apart: an action at `/packages` named
/// `x` is `action:/packages/x`, not package `x`.
pub fn secret_scope(path: &str, name: &str) -> String {
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    if let (Some("packages"), Some(package)) = (segments.next(), segments.next()) {
        return format!("package:{package}");
    }
    format!("action:{}/{}", path.trim_end_matches('/'), name)
}

/// The secret names an action declares in `action_config.secrets`: a list
/// of names, or (the form older install scripts write) an object whose keys
/// are the names. Its values are ignored: keys are never taken from a
/// config.
pub fn secret_names(action_config: Option<&Value>) -> BTreeSet<String> {
    match action_config.and_then(|c| c.get("secrets")) {
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        _ => BTreeSet::new(),
    }
}

/// Whether `action_config.auth` reads a credential out of the OS credential
/// manager: any field holding a `{keyring_service, keyring_account}`
/// reference.
pub fn uses_keyring(action_config: Option<&Value>) -> bool {
    action_config
        .and_then(|c| c.get("auth"))
        .and_then(Value::as_object)
        .is_some_and(|auth| {
            auth.values()
                .any(|v| v.as_object().is_some_and(|o| o.contains_key("keyring_service")))
        })
}

/// Whether an action holds a credential: it declares scoped secrets, or its
/// auth reads a keyring entry. A keyed action is root-only to create or
/// change, and always locked, so content can't repoint what runs with it.
pub fn is_keyed(action_config: Option<&Value>) -> bool {
    !secret_names(action_config).is_empty() || uses_keyring(action_config)
}

/// The scoped-secret access of one action: the secret names it declares in
/// `action_config.secrets`, its [`secret_scope`], and that scope's AES key,
/// which solx keeps in the actions database, never in a config.
///
/// The names are an allowlist, not decoration: an action reaches only the
/// secrets it declares, even within its own scope.
#[derive(Clone, Default)]
pub struct ActionSecrets {
    scope: String,
    key: Option<String>,
    names: BTreeSet<String>,
}

impl ActionSecrets {
    /// `scope` is the action's [`secret_scope`]; `action_config` supplies the
    /// names; `key` is the scope's key.
    pub fn new(scope: impl Into<String>, action_config: Option<&Value>, key: Option<String>) -> Self {
        ActionSecrets {
            scope: scope.into(),
            key: key.filter(|k| !k.is_empty()),
            names: secret_names(action_config),
        }
    }

    /// Where secret `name` lives and the key that opens it, if the action
    /// declares that name and its scope has a key.
    pub fn resolve(&self, name: &str) -> Option<SecretHandle> {
        let key = self.key.as_ref().filter(|_| self.names.contains(name))?;
        Some(SecretHandle {
            name: name.to_string(),
            account: format!("{}/{name}", self.scope),
            key: key.clone(),
        })
    }
}

/// Never prints the key.
impl fmt::Debug for ActionSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionSecrets")
            .field("scope", &self.scope)
            .field("has_key", &self.key.is_some())
            .field("names", &self.names)
            .finish()
    }
}

/// One secret an action may use: its credential-manager account
/// (`<scope>/<name>`, so two scopes never share an entry) and the key that
/// encrypts it. Obtained only from [`ActionSecrets::resolve`].
#[derive(Clone)]
pub struct SecretHandle {
    name: String,
    account: String,
    key: String,
}

impl SecretHandle {
    /// The name the action uses for it, e.g. `GOOGLE_OAUTH_REFRESH_TOKEN`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The credential-manager account it is stored under.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// The base64 AES-256 key. Never returned to a caller.
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Never prints the key.
impl fmt::Debug for SecretHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretHandle")
            .field("account", &self.account)
            .finish()
    }
}

#[cfg(test)]
mod secret_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn packages_share_a_scope_and_everything_else_is_its_own() {
        assert_eq!(secret_scope("/packages/solx-google", "a"), "package:solx-google");
        assert_eq!(secret_scope("/packages/solx-google/_private", "b"), "package:solx-google");
        assert_eq!(secret_scope("/packages", "x"), "action:/packages/x");
        assert_eq!(secret_scope("/tools", "hook"), "action:/tools/hook");
    }

    #[test]
    fn names_come_from_a_list_or_an_objects_keys() {
        let list = json!({ "secrets": ["A", "B"] });
        let object = json!({ "secrets": { "A": "ignored", "B": "***" } });
        assert_eq!(secret_names(Some(&list)), secret_names(Some(&object)));
        assert!(secret_names(Some(&json!({}))).is_empty());
    }

    #[test]
    fn only_declared_names_resolve_and_accounts_are_scoped() {
        let cfg = json!({ "secrets": ["A"] });
        let secrets = ActionSecrets::new("package:p", Some(&cfg), Some("k".into()));
        let handle = secrets.resolve("A").unwrap();
        assert_eq!(handle.account(), "package:p/A");
        assert_eq!(handle.key(), "k");
        assert!(secrets.resolve("B").is_none());
        assert!(ActionSecrets::new("package:p", Some(&cfg), None).resolve("A").is_none());
    }
}

/// URI form, for logs and error messages: `action://pkg/summarize`.
impl fmt::Display for Caller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "action://{}", self.action_ref.trim_start_matches('/'))
    }
}

/// Non-secret identity of the calling action — everything a pluggable
/// [`InternalActionHandler`] is allowed to see about who invoked it. Unlike
/// [`Caller`], this type has no `secret` method and no secrets field to
/// add one to later by accident: the exclusion is a compile-time fact, not
/// just a documented convention.
#[derive(Debug, Clone)]
pub struct CallerInfo {
    action_ref: String,
    invocation_id: String,
}

impl CallerInfo {
    /// Full reference of the calling action, e.g. `/pkg/summarize`.
    pub fn action_ref(&self) -> &str {
        &self.action_ref
    }

    /// Identifies this one run of `action_ref`, distinct from any other
    /// concurrent or subsequent run.
    pub fn invocation_id(&self) -> &str {
        &self.invocation_id
    }
}

/// URI form, for logs and error messages: `action://pkg/summarize`.
impl fmt::Display for CallerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "action://{}", self.action_ref.trim_start_matches('/'))
    }
}

// ── Seed catalogue entries ───────────────────────────────────────────────────

/// One built-in action to seed — same shape `solx-actions`' own catalogue
/// uses, so a plugin's entries merge into the same `INSERT OR IGNORE` loop.
#[derive(Clone)]
pub struct SeedAction {
    pub path: &'static str,
    /// The entity name. `fn_name` (below) is the dispatch key the internal
    /// dispatcher matches on — for a flat `/builtin` entry the two are
    /// identical, but a nested entry (e.g. `console/print`) needs a `name`
    /// too generic to double as a global dispatch key, hence the split.
    pub name: &'static str,
    pub fn_name: &'static str,
    pub description: &'static str,
    /// Name under the builtin types namespace giving this action's
    /// `param_type_ref` a real JSON Schema, if one is seeded.
    pub param_type: Option<&'static str>,
}

// ── Handler context and trait ────────────────────────────────────────────────

/// What a pluggable internal-action handler can reach — deliberately a
/// subset of the dispatcher's own full context (no config service, no
/// concrete action-manager handle, no task-spawn machinery). A handler that
/// needs more than this takes it via constructor injection instead.
pub struct InternalCallCtx {
    pub docs: Arc<dyn DocManager>,
    pub types: Arc<dyn TypeManager>,
    pub actions: Arc<dyn ActionManager>,
    pub files: Arc<dyn FileStore>,
    /// The caller (non-secret identity only — see [`CallerInfo`]), principal
    /// and exclusion mode of this frame. A pluggable handler can never reach
    /// a secret key through it, even by accident: [`Caller`] (which can)
    /// never appears here. A handler that starts another run
    /// (`action-start`) must pass it on whole.
    pub exec: ExecContextInfo,
}

impl InternalCallCtx {
    /// See [`ExecContextInfo::external_principal`]; treat `None` as *deny*.
    pub fn external_principal(&self) -> Option<&Principal> {
        self.exec.external_principal()
    }
}

/// One internal action's implementation. Handlers are constructed with
/// whatever crate-specific state they need already baked in (dependency
/// injection) — `params`/`ctx` carry only what's genuinely per-call.
#[async_trait]
pub trait InternalActionHandler: Send + Sync {
    async fn call(
        &self,
        params: &Value,
        ctx: &InternalCallCtx,
    ) -> std::result::Result<Value, String>;
}

/// The seam `action-start`/`action-stop`/`action-poll` need into
/// `solx-actions`' own detached-task orchestration, without depending on its
/// concrete `LocalActionManager` type. Mirrors
/// `LocalActionManager::{start_invocation,stop_invocation,poll_invocation}`
/// 1:1 — see their doc comments in `solx-actions` for the full behavior
/// (long-lived-host requirement, cooperative-then-forced cancellation,
/// long-poll cadence).
///
/// `start_invocation` takes the starting call's whole [`ExecContextInfo`] so
/// the detached run keeps it across its `tokio::spawn`. Dropping the caller
/// while keeping the principal would let an action launched by root re-enter
/// as an apparently external root call (see
/// [`ExecContextInfo::external_principal`]); dropping the exclusion mode
/// would let a model detach an excluded action.
#[async_trait]
pub trait ActionExecutor: Send + Sync {
    async fn start_invocation(
        &self,
        path: &str,
        name: &str,
        params: Value,
        exec: ExecContextInfo,
    ) -> Result<Value>;
    async fn stop_invocation(
        &self,
        invocation_id: &str,
        force: bool,
        grace_secs: Option<u64>,
    ) -> Result<Value>;
    async fn poll_invocation(&self, invocation_id: &str, wait_secs: Option<u64>) -> Result<Value>;
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// A dispatcher's fn_name -> handler map plus the seed entries that back it,
/// assembled by merging one or more plugins' contributions. `solx-actions`
/// consults this only as a fallback after its own hard-coded built-ins — see
/// its `internal::run_internal`.
#[derive(Default)]
pub struct InternalActionRegistry {
    handlers: HashMap<String, Arc<dyn InternalActionHandler>>,
    seeds: Vec<SeedAction>,
}

impl InternalActionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one handler under every `fn_name` in `fn_names`.
    pub fn register(&mut self, fn_names: &[&'static str], handler: Arc<dyn InternalActionHandler>) {
        for name in fn_names {
            self.handlers.insert((*name).to_string(), handler.clone());
        }
    }

    /// Append `SeedAction` catalogue entries, independent of handler
    /// registration — mirrors how `solx-actions`' own `seed.rs` and
    /// `internal::run_internal` are two independent lists kept in sync by
    /// convention, not by construction.
    pub fn add_seeds(&mut self, seeds: Vec<SeedAction>) {
        self.seeds.extend(seeds);
    }

    pub fn get(&self, fn_name: &str) -> Option<&Arc<dyn InternalActionHandler>> {
        self.handlers.get(fn_name)
    }

    pub fn seed_actions(&self) -> &[SeedAction] {
        &self.seeds
    }

    /// Fold another plugin's registrations into this one — how
    /// `solx-actions` combines its own registry with one built by an
    /// external crate (e.g. `solx_console::actions::plugin(...)`).
    pub fn merge(&mut self, other: InternalActionRegistry) {
        self.handlers.extend(other.handlers);
        self.seeds.extend(other.seeds);
    }
}
