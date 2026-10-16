//! Typed view of `solx-config.json`. The writer path edits the file as a raw
//! `serde_json::Value` so unknown fields survive; this struct is only used to
//! read a convenient typed snapshot.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use solx_surface::entities::{Action, ActionType};
use solx_surface::error::SolxError;
use solx_surface::path::{full_ref, normalize_path};
use solx_surface::query::{
    is_excluded_by, validate_exclude_pattern, ExcludeMode, ListOptions, SearchQuery,
};

/// A single allowed shell command for `Command`-type actions. A Command
/// action's `fn_name` is never the literal command to run — it's a key into
/// `SolxConfig.command_actions`, resolved here at execution time. See
/// `docs/next-steps.md` §1.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandDef {
    /// The literal shell command string that actually runs.
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Working directory override. Takes priority over the action's own
    /// `action_config.cwd` when both are set — the allowlist author, not the
    /// action, decides where an approved command runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// A record of an installed package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub installed_at: String,
    /// `command_actions` keys this package's manifest declared and had
    /// granted into the global allowlist at install time. `#[serde(default)]`
    /// so a record from before this field existed just reads as empty —
    /// reinstalling repopulates it. `uninstall_package` uses this to know
    /// exactly what to revoke, rather than re-reading (possibly stale)
    /// `package.json` off disk. See `docs/next-steps.md` §1.
    #[serde(default)]
    pub granted_commands: Vec<String>,
    /// `allowed_base_urls` prefixes this package's manifest declared and had
    /// granted at install time. Same defaulting/rationale as
    /// `granted_commands`.
    ///
    /// The `alias` is load-bearing: rows written before the rename carry
    /// `granted_webhook_prefixes`, and without it they would read as empty —
    /// so `uninstall_package` would silently revoke nothing.
    #[serde(default, alias = "granted_webhook_prefixes")]
    pub granted_base_urls: Vec<String>,
}

/// Typed snapshot of the config. All fields are optional so a partial or
/// hand-edited file still parses; defaults are supplied by the accessors on
/// [`crate::ConfigService`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SolxConfig {
    /// Root directory for the entity databases. Defaults to `<appdata>/db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_directory: Option<String>,
    /// Root directory for stored files. Defaults to `<appdata>/files`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files_directory: Option<String>,
    /// Directory packages installed from a zip are unpacked into. Defaults
    /// to `<appdata>/packages`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packages_directory: Option<String>,
    /// Directory for model files. Defaults to `<appdata>/models`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models_directory: Option<String>,
    /// Directory for the Tantivy search indexes. Defaults to `<appdata>/search_index`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_index_dir: Option<String>,

    /// Filename for the documents DB (within `data_directory`). Default `solx-docs.db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_db: Option<String>,
    /// Filename for the actions DB. Default `solx-actions.db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions_db: Option<String>,
    /// Filename for the types DB. Default `solx-types.db`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types_db: Option<String>,

    /// Installed package registry.
    #[serde(default)]
    pub installed_packages: Vec<InstalledPackage>,

    /// Environment-store allowlist: guest key -> system/process env var
    /// name to read. Only keys listed here are ever visible via the
    /// `get-env` built-in action — callers never see the raw process
    /// environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_mappings: Option<HashMap<String, String>>,

    /// Persisted environment-store variables, `namespace -> key -> value`.
    /// Written by the `set-env` built-in when called with `persist: true`,
    /// and loaded back into the store at startup, so a variable survives a
    /// restart. Distinct from `env_mappings`, which is a read-only allowlist
    /// over the *system* environment and is never written here.
    ///
    /// **Plaintext.** Anything sensitive belongs in `get-secret`/`set-secret`,
    /// which are encrypted and held in the OS credential manager. Delete a
    /// variable by removing it from this map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_vars: Option<HashMap<String, HashMap<String, String>>>,

    /// Base URL of a remote `solx-server` to proxy all manager calls to
    /// (e.g. `"http://127.0.0.1:8766"`). `None` (default) means local mode
    /// — every existing user's exact current behavior, no server needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    /// Legacy plaintext home of the bearer token. Normally empty: the token
    /// lives in the OS credential manager (see `solx_config::credentials`),
    /// and a value found here is moved there and removed on first use. Only
    /// written when `token_store` is `"config"`, or as a fallback when no
    /// credential manager is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_token: Option<String>,
    /// Where bearer tokens are kept: `"keyring"` (default, the OS credential
    /// manager) or `"config"` (plaintext `server_token` in this file, e.g.
    /// for a headless machine or CI with no credential manager).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_store: Option<String>,
    /// Port `solx-server` binds on `127.0.0.1`. Server-side only —
    /// independent of `server_url`, since the machine running `solx-server`
    /// won't normally have its own config's `server_url` set at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_port: Option<u16>,
    /// Address `solx-server` binds on. Server-side only. Defaults to
    /// `127.0.0.1`; set to e.g. `"0.0.0.0"` (or a LAN/Tailscale IP) to serve
    /// other machines. Once it's off loopback, the bearer token is the only
    /// gate and traffic is plaintext HTTP, so put it on a private network
    /// or behind a TLS reverse proxy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_host: Option<String>,
    /// Browser origins allowed by CORS when `server_host` is not loopback
    /// (e.g. `["https://solx.example.com"]`). On loopback, CORS stays wide
    /// open. Off loopback, `None` means no cross-origin browser access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_cors_origins: Option<Vec<String>>,
    /// The address other machines use to reach this `solx-server`, when it
    /// isn't the bind address: a `0.0.0.0` bind, a reverse proxy, a
    /// Tailscale name or IP. Connect links (`<url>/connect#code=…`) and the
    /// `server_url` returned by `/auth/exchange` use it. A full URL is used
    /// as is; a bare IP or host name gets `http://` and `server_port`.
    /// Defaults to the bound address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_public_url: Option<String>,
    /// Lifetime of a client token issued by the token registry (connect
    /// codes, `solx token issue`), in days. Defaults to 90.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_token_ttl_days: Option<i64>,
    /// Whether a host that runs the local managers in-process (e.g. the
    /// solx desktop app's server build) also exposes them over HTTP, as an
    /// embedded `solx-server` on `server_host`/`server_port`. Defaults to
    /// `true`. Has no effect on the standalone `solx-server` binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_embedded: Option<bool>,

    /// Ring-buffer cap on entries retained per action console. Oldest
    /// entries are evicted once a console exceeds this. Defaults to 5000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_max_entries: Option<i64>,
    /// A console whose most recent write is older than this many days is
    /// dropped entirely by the startup sweep. Defaults to 7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_ttl_days: Option<i64>,

    /// Wall-clock ceiling for a detached (`action-start`) invocation with no
    /// `action_config.timeout_secs` of its own. Defaults to 86400 (24h) —
    /// deliberately much larger than `exec`'s 300s default, since a detached
    /// run is expected to outlive any one caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_timeout_secs: Option<u64>,
    /// How long `action-stop` waits for cooperative exit (the running
    /// action noticing `cancel_requested` and returning on its own) before
    /// force-aborting the task. Defaults to 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_grace_secs: Option<u64>,
    /// A terminal invocation row older than this many days is dropped by
    /// the startup sweep. Defaults to 7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_ttl_days: Option<i64>,

    /// A streaming HTTP request (`http_stream/start`) whose buffer has gone
    /// this many seconds without a `http_stream/poll` self-terminates, so an
    /// abandoned generation doesn't pin a connection open forever. Defaults
    /// to 120.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_stream_idle_ttl_secs: Option<u64>,
    /// Max bytes buffered per stream before the oldest chunks are dropped to
    /// make room (counted in the poll response's `dropped`). Defaults to
    /// 8388608 (8 MiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_stream_max_buffer_bytes: Option<u64>,

    /// Allowlist of shell commands `Command`-type actions may run, keyed by
    /// an opaque name. A Command action's `fn_name` is that key, resolved
    /// against this map — never a literal command string. **Deny-by-default**:
    /// unset or empty means no Command action can execute at all.
    ///
    /// ```json
    /// "command_actions": {
    ///   "compress-pdf": {
    ///     "command": "gs -dBATCH -dNOPAUSE -sDEVICE=pdfwrite -sOutputFile=out.pdf in.pdf",
    ///     "description": "Compress a PDF file",
    ///     "cwd": "~/scripts"
    ///   }
    /// }
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_actions: Option<HashMap<String, CommandDef>>,

    /// Allowlist of URL prefixes any action may send an outbound HTTP
    /// request to. A request URL must match at least one listed prefix: the
    /// same scheme, host and port (an entry without a port means the default
    /// one; `http://host:` means any), and a path starting with the entry's
    /// path. Redirects are held to the same list. See `solx-actions::net`.
    /// **Deny-by-default**: unset or empty means no outbound request is
    /// permitted at all.
    ///
    /// This governs `Webhook`-type actions *and* the `/builtin/web/*`
    /// built-ins (`http-request`, `http-stream-start`, `open-url`) — which is
    /// why it is no longer named for webhooks.
    ///
    /// ```json
    /// "allowed_base_urls": ["https://hooks.example.com"]
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_base_urls: Option<Vec<String>>,

    /// Former name of [`Self::allowed_base_urls`], still read so an existing
    /// `solx-config.json` keeps working. Unioned with the current field by
    /// `merged_base_urls`, exactly as `mcp_exclude` is unioned into
    /// `tool_exclude`. Nothing writes it: the mutators fold it into
    /// `allowed_base_urls` and drop it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_webhook_base_urls: Option<Vec<String>>,

    /// Denylist of action paths (and optionally specific action names at
    /// those paths) hidden from the tool catalogue a model sees. A rule's
    /// `path` is a glob pattern matched against the action's full path
    /// (leading slash); `*` matches any characters including `/`, `?`
    /// matches a single character. When `actions` is unset or empty, every
    /// action under the matched path is hidden; otherwise only the named
    /// actions are.
    ///
    /// ```json
    /// "tool_exclude": [
    ///   { "path": "*/_private/*" },
    ///   { "path": "/packages/solx-google", "actions": ["search"] }
    /// ]
    /// ```
    ///
    /// One of *three* inputs to the hidden decision — see [`ToolPolicy`],
    /// which unions this with [`SolxConfig::mcp_exclude`] and the
    /// [`CAP_HIDDEN`] tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_exclude: Option<Vec<ToolRule>>,

    /// Former name of [`SolxConfig::tool_exclude`], still read so an existing
    /// `solx-config.json` keeps working untouched. The rules are unioned, not
    /// overridden, so a file carrying both keys behaves as the sum of the two.
    ///
    /// Deliberately a second field rather than `#[serde(alias)]` on
    /// `tool_exclude`: serde rejects a struct that carries *both* names as a
    /// duplicate field, and [`crate::ConfigService::snapshot`] swallows a
    /// deserialization error into `SolxConfig::default()` — so adding the new
    /// key beside an existing old one would silently blank the entire config,
    /// command allowlists and all, rather than merging the two lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_exclude: Option<Vec<ToolRule>>,

    /// Action paths flagged as destructive: a model may call them, but a
    /// caller that implements an approval step should stop and ask first.
    /// Same rule shape as [`SolxConfig::tool_exclude`].
    ///
    /// ```json
    /// "tool_destructive": [
    ///   { "path": "/builtin/document", "actions": ["entity-delete-document"] }
    /// ]
    /// ```
    ///
    /// Also one of several inputs — see [`ToolPolicy`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_destructive: Option<Vec<ToolRule>>,

    /// Per-scope exclusion patterns for documents, types and actions. See
    /// [`Exclusions`] and [`ExclusionPolicy`].
    ///
    /// ```json
    /// "exclusions": {
    ///   "user":    { "documents": ["/agent/skills", "/agent/sessions"] },
    ///   "tooling": { "actions": ["*/_private/*"] }
    /// }
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusions: Option<Exclusions>,
}

/// The two independent exclusion scopes. Each is a set of patterns (see
/// `solx_surface::query::validate_exclude_pattern`) per entity kind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Exclusions {
    /// Hidden from the web UI and REST/Tauri clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<ExclusionLists>,
    /// Hidden from model-facing callers: MCP, the agent harness, and any
    /// builtin call made with `excludeMode: "tooling"`. For actions this is
    /// unioned with `tool_exclude`/`mcp_exclude` and the `solx:hidden` tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooling: Option<ExclusionLists>,
}

/// Exclusion patterns for one scope, by entity kind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExclusionLists {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documents: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
}

impl ExclusionLists {
    fn for_kind(&self, kind: EntityKind) -> &[String] {
        match kind {
            EntityKind::Document => &self.documents,
            EntityKind::Type => &self.types,
            EntityKind::Action => &self.actions,
        }
    }
}

/// Which entity store an exclusion applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Document,
    Type,
    Action,
}

impl EntityKind {
    /// The noun the stores use in their `NotFound` messages, so an excluded
    /// entity reads exactly like a missing one.
    pub fn label(self) -> &'static str {
        match self {
            EntityKind::Document => "document",
            EntityKind::Type => "type",
            EntityKind::Action => "action",
        }
    }
}

/// Resolved exclusion patterns for every (scope, kind) pair, built once from
/// a config snapshot and then applied to many requests or entities.
///
/// Same reasoning as [`ToolPolicy`] for being a struct: one snapshot, one
/// place the decision is made. Exclusions only ever *add* - `apply` appends
/// to whatever `exclude_paths` the caller already sent.
#[derive(Debug, Clone, Default)]
pub struct ExclusionPolicy {
    user: ExclusionLists,
    tooling: ExclusionLists,
}

impl ExclusionPolicy {
    /// Build from a snapshot. Tooling action patterns include the legacy
    /// `tool_exclude`/`mcp_exclude` rules, converted by
    /// [`ToolRule::to_patterns`].
    pub fn from_config(cfg: &SolxConfig) -> Self {
        let ex = cfg.exclusions.clone().unwrap_or_default();
        let user = ex.user.unwrap_or_default();
        let mut tooling = ex.tooling.unwrap_or_default();
        for rule in cfg
            .tool_exclude
            .iter()
            .flatten()
            .chain(cfg.mcp_exclude.iter().flatten())
        {
            tooling.actions.extend(rule.to_patterns());
        }
        ExclusionPolicy { user, tooling }
    }

    /// The configured patterns for `kind` under `mode`.
    pub fn patterns(&self, kind: EntityKind, mode: ExcludeMode) -> &[String] {
        match mode {
            ExcludeMode::None => &[],
            ExcludeMode::User => self.user.for_kind(kind),
            ExcludeMode::Tooling => self.tooling.for_kind(kind),
        }
    }

    /// Resolve `opts.exclude_mode` (falling back to `default`), record the
    /// resolved mode back on `opts`, and append its patterns to
    /// `opts.exclude_paths`. Returns the resolved mode.
    ///
    /// The mode is written back rather than cleared so the action store can
    /// still render the `solx:hidden` tag check for `Tooling`.
    pub fn apply(
        &self,
        kind: EntityKind,
        opts: &mut ListOptions,
        default: ExcludeMode,
    ) -> ExcludeMode {
        let mode = opts.exclude_mode.unwrap_or(default);
        opts.exclude_mode = Some(mode);
        opts.exclude_paths
            .extend(self.patterns(kind, mode).iter().cloned());
        mode
    }

    /// [`Self::apply`] for a document [`SearchQuery`].
    pub fn apply_search(&self, query: &mut SearchQuery, default: ExcludeMode) -> ExcludeMode {
        let mode = query.exclude_mode.unwrap_or(default);
        query.exclude_mode = Some(mode);
        query
            .exclude_paths
            .extend(self.patterns(EntityKind::Document, mode).iter().cloned());
        mode
    }

    /// Whether the entity at `path`/`name` is excluded under `mode`, by
    /// path alone. For actions under `Tooling` the row's `solx:hidden` tag
    /// also counts - use [`Self::is_action_excluded`] when the row is at hand.
    pub fn is_excluded(&self, kind: EntityKind, mode: ExcludeMode, path: &str, name: &str) -> bool {
        is_excluded_by(self.patterns(kind, mode), path, name)
    }

    /// [`Self::is_excluded`] for an action row, including the tag.
    pub fn is_action_excluded(&self, mode: ExcludeMode, action: &Action) -> bool {
        (mode == ExcludeMode::Tooling && has_tag(action, CAP_HIDDEN))
            || self.is_excluded(EntityKind::Action, mode, &action.path, &action.name)
    }

    /// Refuse direct access to an excluded entity.
    ///
    /// A read or delete reports `NotFound`, worded exactly as the store
    /// words a genuinely missing entity: a caller that could tell the two
    /// apart could enumerate what is excluded. A write reports `Invalid`
    /// instead - "not found" would be nonsense for a create, and refusing
    /// to write somewhere reveals nothing about what is already there.
    ///
    /// `path`/`name` are normalized first; one that fails normalization
    /// passes through so the store can report the real problem.
    pub fn guard(
        &self,
        kind: EntityKind,
        mode: ExcludeMode,
        path: &str,
        name: &str,
        write: bool,
    ) -> Result<(), SolxError> {
        let (Ok(path), Ok(fr)) = (normalize_path(path), full_ref(path, name)) else {
            return Ok(());
        };
        if !self.is_excluded(kind, mode, &path, name.trim()) {
            return Ok(());
        }
        Err(excluded_error(kind, &fr, write))
    }

    /// [`Self::guard`] for an action row already read, so the
    /// `solx:hidden` tag counts under `Tooling`.
    pub fn guard_action(
        &self,
        mode: ExcludeMode,
        action: &Action,
        write: bool,
    ) -> Result<(), SolxError> {
        if !self.is_action_excluded(mode, action) {
            return Ok(());
        }
        let fr = full_ref(&action.path, &action.name).unwrap_or_default();
        Err(excluded_error(EntityKind::Action, &fr, write))
    }
}

fn excluded_error(kind: EntityKind, full_ref: &str, write: bool) -> SolxError {
    if write {
        SolxError::Invalid(format!("{} {full_ref} is excluded", kind.label()))
    } else {
        SolxError::NotFound(format!("{} {full_ref}", kind.label()))
    }
}

/// Reserved capability tag marking an action hidden from model-facing tool
/// catalogues. Namespaced so a typo is visible as an unrecognized tag rather
/// than silently unhiding the action, and so it cannot collide with a
/// descriptive capability.
pub const CAP_HIDDEN: &str = "solx:hidden";

/// Reserved capability tag marking an action destructive. See [`CAP_HIDDEN`]
/// for why it is namespaced.
pub const CAP_DESTRUCTIVE: &str = "solx:destructive";

/// A single path/name rule, used by [`SolxConfig::tool_exclude`] and
/// [`SolxConfig::tool_destructive`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolRule {
    /// Glob pattern matched against the action's full path (leading slash).
    pub path: String,
    /// When set and non-empty, only these action names at the matched path
    /// match; otherwise every action under the path does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<String>>,
}

/// Former name of [`ToolRule`]. The struct now backs `tool_exclude` and
/// `tool_destructive`, so the `McpExclude` name no longer describes it.
pub type McpExcludeRule = ToolRule;

impl ToolRule {
    /// This rule as exclusion patterns: `[path]` when it names no actions,
    /// otherwise `path/<action>` for each one.
    ///
    /// Exclusion patterns match a path *or* a full reference, so
    /// `path/<action>` also covers a folder of that name under `path` - a
    /// slightly wider net than [`Self::matches`], accepted so hidden-ness can
    /// be decided in SQL.
    ///
    /// A pattern that fails `validate_exclude_pattern` is dropped. That keeps
    /// behaviour identical rather than loosening it: such a rule (no leading
    /// `/` or `*`, a `:`, a `..` segment) could never match a normalized
    /// path, and left in it would fail every tooling query instead.
    pub fn to_patterns(&self) -> Vec<String> {
        let patterns = match &self.actions {
            Some(names) if !names.is_empty() => {
                let base = self.path.trim_end_matches('/');
                names.iter().map(|n| format!("{base}/{n}")).collect()
            }
            _ => vec![self.path.clone()],
        };
        patterns
            .into_iter()
            .filter(|p| validate_exclude_pattern(p).is_ok())
            .collect()
    }

    /// Whether this rule matches the action at `path`/`name`.
    pub fn matches(&self, path: &str, name: &str) -> bool {
        if !glob_matches(&self.path, path) {
            return false;
        }
        match &self.actions {
            Some(names) if !names.is_empty() => names.iter().any(|n| n == name),
            _ => true,
        }
    }
}

/// Resolved hidden/destructive policy, built once from a config snapshot and
/// then applied to many actions.
///
/// Two reasons this is a struct rather than a pair of `ConfigService`
/// methods. It is the single place both decisions are made, so a caller
/// cannot consult one input and forget the other; and `ConfigService::snapshot`
/// deserializes the whole config on every call, which a per-action check
/// inside a catalogue loop would pay hundreds of times.
///
/// **Both decisions union their inputs, and nothing subtracts.** Config rules
/// are a floor that a row cannot lower: anything that can call
/// `entity-save-action` can rewrite a row's `capabilities` (the executable-
/// action guard protects Command and Webhook *rows*, not the tags on a Script
/// or Wasm one), so a tag alone would be a flag the flagged party can remove.
#[derive(Debug, Clone, Default)]
pub struct ToolPolicy {
    /// Tooling-scope action exclusion patterns - the same list
    /// [`ExclusionPolicy`] renders into SQL, so the catalogue and the
    /// call-time check cannot disagree.
    hidden: Vec<String>,
    destructive: Vec<ToolRule>,
}

impl ToolPolicy {
    pub fn new(hidden: Vec<ToolRule>, destructive: Vec<ToolRule>) -> Self {
        ToolPolicy {
            hidden: hidden.iter().flat_map(ToolRule::to_patterns).collect(),
            destructive,
        }
    }

    /// Build from already-resolved tooling action patterns.
    pub fn from_patterns(hidden: Vec<String>, destructive: Vec<ToolRule>) -> Self {
        ToolPolicy {
            hidden,
            destructive,
        }
    }

    /// Hidden = tooling action exclusions (`exclusions.tooling.actions` ∪
    /// `tool_exclude` ∪ `mcp_exclude`) ∪ the [`CAP_HIDDEN`] tag.
    ///
    /// A hidden action is omitted from the catalogue *and* refused when
    /// called by name — hiding a tool a client has already learned the name
    /// of is not, on its own, a control.
    pub fn is_hidden(&self, action: &Action) -> bool {
        has_tag(action, CAP_HIDDEN) || is_excluded_by(&self.hidden, &action.path, &action.name)
    }

    /// Destructive = config rules ∪ the [`CAP_DESTRUCTIVE`] tag ∪ every
    /// Command and Webhook row, unconditionally.
    ///
    /// That last clause is the one to keep even if the rest changes: a shell
    /// action is destructive by construction, and must never depend on
    /// someone having remembered to tag it.
    pub fn is_destructive(&self, action: &Action) -> bool {
        matches!(
            action.action_type,
            Some(ActionType::Command) | Some(ActionType::Webhook)
        ) || has_tag(action, CAP_DESTRUCTIVE)
            || self
                .destructive
                .iter()
                .any(|r| r.matches(&action.path, &action.name))
    }
}

/// Prefix owned by solx for capability tags that carry meaning to the
/// runtime rather than describing what an action does.
pub const RESERVED_CAP_PREFIX: &str = "solx:";

/// Every reserved capability tag the runtime understands.
pub const RESERVED_CAPS: &[&str] = &[CAP_HIDDEN, CAP_DESTRUCTIVE];

/// Reject a `solx:`-prefixed capability that isn't one the runtime knows.
///
/// This is what makes the namespace worth having. Without it, `solx:destructiv`
/// is merely an unrecognized tag and the action silently stays ungated — the
/// exact failure a typo in a bare `destructive` would cause. Unprefixed tags
/// are the free-form descriptive vocabulary and are never checked.
pub fn validate_capabilities(capabilities: &[String]) -> Result<(), String> {
    for cap in capabilities {
        if cap.starts_with(RESERVED_CAP_PREFIX) && !RESERVED_CAPS.contains(&cap.as_str()) {
            return Err(format!(
                "unknown reserved capability '{cap}': the '{RESERVED_CAP_PREFIX}' prefix is \
                 reserved, and the recognized tags are {}",
                RESERVED_CAPS.join(", ")
            ));
        }
    }
    Ok(())
}

/// Exact match on one capability entry.
///
/// Deliberately not the SQL `LIKE` filter `capabilities` supports: a
/// substring test would make `non-destructive` match `solx:destructive`.
fn has_tag(action: &Action, tag: &str) -> bool {
    action.capabilities.iter().any(|c| c == tag)
}

/// Moved to `solx-surface` so the SQL exclusion renderer and this crate's
/// rule matching share one implementation; re-exported for existing callers.
pub use solx_surface::query::glob_matches;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_star_matches_across_slashes() {
        assert!(glob_matches("*/_private/*", "/a/_private/b"));
        assert!(glob_matches("*/_private/*", "/_private/b"));
        assert!(glob_matches("*/_private/*", "/x/y/_private/z"));
        assert!(!glob_matches("*/_private/*", "/a/internal/b"));
        assert!(!glob_matches("*/_private/*", "/a/_private"));
    }

    #[test]
    fn glob_question_matches_single_char() {
        assert!(glob_matches(
            "/packages/solx-?oogle",
            "/packages/solx-google"
        ));
        assert!(!glob_matches(
            "/packages/solx-?oogle",
            "/packages/solx-gooogle"
        ));
    }

    #[test]
    fn glob_literal_and_anchoring() {
        assert!(glob_matches("/builtin", "/builtin"));
        assert!(!glob_matches("/builtin", "/builtin/console"));
        assert!(glob_matches("/builtin/*", "/builtin/console"));
        assert!(glob_matches("*", "/anything/at/all"));
    }

    /// Built through serde rather than a struct literal: `Action` has no
    /// `Default`, and this way the helper survives the struct gaining fields.
    fn action(path: &str, name: &str, caps: &[&str], ty: Option<ActionType>) -> Action {
        serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "path": path,
            "name": name,
            "capabilities": caps,
            "actionType": ty,
        }))
        .expect("test action must deserialize")
    }

    #[test]
    fn tag_hides_an_action_no_config_rule_covers() {
        let policy = ToolPolicy::default();
        assert!(policy.is_hidden(&action("/packages/x", "y", &[CAP_HIDDEN], None)));
        assert!(!policy.is_hidden(&action("/packages/x", "y", &["hidden"], None)));
    }

    #[test]
    fn config_rule_hides_an_untagged_action() {
        let policy = ToolPolicy::new(
            vec![ToolRule {
                path: "/packages/x".into(),
                actions: None,
            }],
            vec![],
        );
        assert!(policy.is_hidden(&action("/packages/x", "y", &[], None)));
        assert!(!policy.is_hidden(&action("/packages/z", "y", &[], None)));
    }

    #[test]
    fn removing_the_tag_cannot_unhide_what_config_hid() {
        // The whole reason config is a floor rather than the only input:
        // a row's capabilities are writable by anything that can call
        // entity-save-action.
        let policy = ToolPolicy::new(
            vec![ToolRule {
                path: "/packages/x".into(),
                actions: None,
            }],
            vec![],
        );
        assert!(policy.is_hidden(&action("/packages/x", "y", &["totally-safe"], None)));
    }

    #[test]
    fn command_and_webhook_are_destructive_untagged() {
        let policy = ToolPolicy::default();
        assert!(policy.is_destructive(&action("/p", "a", &[], Some(ActionType::Command))));
        assert!(policy.is_destructive(&action("/p", "a", &[], Some(ActionType::Webhook))));
        assert!(!policy.is_destructive(&action("/p", "a", &[], Some(ActionType::Wasm))));
        assert!(!policy.is_destructive(&action("/p", "a", &[], None)));
    }

    #[test]
    fn destructive_unions_tag_and_config() {
        let policy = ToolPolicy::new(
            vec![],
            vec![ToolRule {
                path: "/builtin/document".into(),
                actions: Some(vec!["entity-delete-document".into()]),
            }],
        );
        assert!(policy.is_destructive(&action(
            "/builtin/document",
            "entity-delete-document",
            &[],
            Some(ActionType::Internal)
        )));
        assert!(!policy.is_destructive(&action(
            "/builtin/document",
            "entity-get-document",
            &[],
            Some(ActionType::Internal)
        )));
        assert!(policy.is_destructive(&action(
            "/packages/x",
            "wipe",
            &[CAP_DESTRUCTIVE],
            Some(ActionType::Wasm)
        )));
    }

    #[test]
    fn reserved_prefix_rejects_a_typo_but_allows_free_form_tags() {
        assert!(validate_capabilities(&["document".into(), CAP_HIDDEN.into()]).is_ok());
        // The whole point of the namespace: this is an error, not a silently
        // ungated action.
        let err = validate_capabilities(&["solx:destructiv".into()]).unwrap_err();
        assert!(err.contains("solx:destructiv"), "{err}");
        assert!(err.contains(CAP_DESTRUCTIVE), "{err}");
        // Unprefixed tags are free-form and never checked.
        assert!(validate_capabilities(&["destructive".into(), "hidden".into()]).is_ok());
    }

    #[test]
    fn a_tag_is_matched_exactly_not_as_a_substring() {
        let policy = ToolPolicy::default();
        assert!(!policy.is_destructive(&action(
            "/p",
            "a",
            &["non-solx:destructive"],
            Some(ActionType::Wasm)
        )));
        assert!(!policy.is_hidden(&action("/p", "a", &["solx:hidden-ish"], None)));
    }

    #[test]
    fn hidden_and_destructive_are_independent() {
        let policy = ToolPolicy::default();
        let a = action("/p", "a", &[CAP_DESTRUCTIVE], Some(ActionType::Wasm));
        assert!(policy.is_destructive(&a));
        assert!(
            !policy.is_hidden(&a),
            "destructive must stay callable, just gated"
        );
    }

    #[test]
    fn rule_path_only_excludes_everything_under_path() {
        let rule = ToolRule {
            path: "*/_private/*".into(),
            actions: None,
        };
        assert!(rule.matches("/a/_private/b", "anything"));
        assert!(!rule.matches("/a/public/b", "anything"));
    }

    #[test]
    fn rule_with_actions_excludes_only_named() {
        let rule = ToolRule {
            path: "/packages/solx-google".into(),
            actions: Some(vec!["search".into()]),
        };
        assert!(rule.matches("/packages/solx-google", "search"));
        assert!(!rule.matches("/packages/solx-google", "list"));
        assert!(!rule.matches("/packages/solx-other", "search"));
    }

    #[test]
    fn rule_converts_to_exclusion_patterns() {
        let whole = ToolRule {
            path: "*/_private/*".into(),
            actions: None,
        };
        assert_eq!(whole.to_patterns(), vec!["*/_private/*".to_string()]);
        // A rule that could never match is dropped rather than left to fail
        // every query.
        let dead = ToolRule {
            path: "packages/no-slash".into(),
            actions: None,
        };
        assert!(dead.to_patterns().is_empty());
        let named = ToolRule {
            path: "/builtin/document".into(),
            actions: Some(vec!["a".into(), "b".into()]),
        };
        assert_eq!(
            named.to_patterns(),
            vec![
                "/builtin/document/a".to_string(),
                "/builtin/document/b".to_string()
            ]
        );
    }

    #[test]
    fn exclusion_policy_scopes_are_independent() {
        let cfg: SolxConfig = serde_json::from_value(serde_json::json!({
            "exclusions": {
                "user": { "documents": ["/agent/skills"] },
                "tooling": { "documents": ["/secret"] }
            },
            "tool_exclude": [{ "path": "/packages/x" }]
        }))
        .unwrap();
        let p = ExclusionPolicy::from_config(&cfg);
        let d = EntityKind::Document;
        assert!(p.is_excluded(d, ExcludeMode::User, "/agent/skills", "s"));
        assert!(!p.is_excluded(d, ExcludeMode::Tooling, "/agent/skills", "s"));
        assert!(p.is_excluded(d, ExcludeMode::Tooling, "/secret", "s"));
        assert!(!p.is_excluded(d, ExcludeMode::None, "/secret", "s"));
        // Legacy tool_exclude folds into tooling actions only.
        let a = EntityKind::Action;
        assert!(p.is_excluded(a, ExcludeMode::Tooling, "/packages/x", "y"));
        assert!(!p.is_excluded(a, ExcludeMode::User, "/packages/x", "y"));
        let tagged = action("/ok", "y", &[CAP_HIDDEN], None);
        assert!(p.is_action_excluded(ExcludeMode::Tooling, &tagged));
        assert!(!p.is_action_excluded(ExcludeMode::User, &tagged));
    }

    #[test]
    fn guard_reads_like_not_found_and_refuses_writes() {
        let cfg: SolxConfig = serde_json::from_value(serde_json::json!({
            "exclusions": { "user": { "documents": ["/agent/skills"] } }
        }))
        .unwrap();
        let p = ExclusionPolicy::from_config(&cfg);
        let d = EntityKind::Document;
        // Unnormalized input still matches.
        let err = p
            .guard(d, ExcludeMode::User, "agent/skills/", "x", false)
            .unwrap_err();
        assert!(
            matches!(err, SolxError::NotFound(ref m) if m == "document /agent/skills/x"),
            "{err:?}"
        );
        let err = p
            .guard(d, ExcludeMode::User, "/agent", "skills", true)
            .unwrap_err();
        assert!(matches!(err, SolxError::Invalid(_)), "{err:?}");
        assert!(p
            .guard(d, ExcludeMode::None, "/agent/skills", "x", false)
            .is_ok());
        assert!(p.guard(d, ExcludeMode::User, "/notes", "x", false).is_ok());
    }

    #[test]
    fn exclusion_policy_apply_resolves_default_and_appends() {
        let cfg: SolxConfig = serde_json::from_value(serde_json::json!({
            "exclusions": { "user": { "types": ["/t"] } }
        }))
        .unwrap();
        let p = ExclusionPolicy::from_config(&cfg);
        let mut o = ListOptions {
            exclude_paths: vec!["/mine".into()],
            ..Default::default()
        };
        assert_eq!(
            p.apply(EntityKind::Type, &mut o, ExcludeMode::User),
            ExcludeMode::User
        );
        assert_eq!(o.exclude_mode, Some(ExcludeMode::User));
        assert_eq!(o.exclude_paths, vec!["/mine".to_string(), "/t".to_string()]);

        let mut o = ListOptions {
            exclude_mode: Some(ExcludeMode::None),
            ..Default::default()
        };
        p.apply(EntityKind::Type, &mut o, ExcludeMode::User);
        assert!(o.exclude_paths.is_empty());
    }

    #[test]
    fn rule_with_empty_actions_behaves_like_path_only() {
        let rule = ToolRule {
            path: "/x".into(),
            actions: Some(vec![]),
        };
        assert!(rule.matches("/x", "anything"));
    }
}
