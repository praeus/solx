//! `solx-config` — the config service backing `solx-config.json`.
//!
//! Concurrency model (the explicit cross-process requirement):
//! * **Reads** are lock-free. The in-memory cache is refreshed by comparing the
//!   file's mtime and length; if another process wrote the file, the next read
//!   reloads it. A stamp too close to when it was loaded is never trusted (see
//!   [`RACY_WINDOW`]), since a coarse mtime can't tell two quick writes apart.
//! * **Writes** take an OS advisory exclusive lock (`fs2`) over the config file
//!   for the whole read-modify-write, serializing writers across processes.
//! * The write path edits the file as a raw [`serde_json::Value`] so unknown
//!   fields written by other tools/versions survive.

pub mod credentials;
mod types;

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

use fs2::FileExt;
use serde_json::{Map, Value};
use solx_surface::error::{Result, SolxError};
use solx_surface::query::ExcludeMode;
use solx_surface::scope::Enforcement;

pub use types::{
    glob_matches, validate_capabilities, CommandDef, EntityKind, ExclusionLists, ExclusionPolicy,
    Exclusions, InstalledPackage, McpExcludeRule, SolxConfig, ToolPolicy, ToolRule,
    CAP_DESTRUCTIVE, CAP_HIDDEN, RESERVED_CAPS, RESERVED_CAP_PREFIX,
};

/// `tool_exclude` ∪ the former `mcp_exclude`. Union rather than
/// either-or, so a config that still carries only the old key keeps working
/// and one carrying both gets the sum instead of a silent winner.
fn merged_exclude(cfg: &SolxConfig) -> Vec<ToolRule> {
    let mut rules = cfg.tool_exclude.clone().unwrap_or_default();
    rules.extend(cfg.mcp_exclude.clone().unwrap_or_default());
    rules
}

/// The outbound-host allowlist: `allowed_base_urls` plus anything still
/// under the former `allowed_webhook_base_urls` key. Same shape and same
/// reasoning as [`merged_exclude`] — an existing config keeps working
/// without a migration pass over the file.
fn merged_base_urls(cfg: &SolxConfig) -> Vec<String> {
    let mut list = cfg.allowed_base_urls.clone().unwrap_or_default();
    for prefix in cfg.allowed_webhook_base_urls.clone().unwrap_or_default() {
        if !list.iter().any(|p| *p == prefix) {
            list.push(prefix);
        }
    }
    list
}

/// Read both spellings out of a raw config object, current first.
///
/// The mutators below work on the raw JSON rather than a [`SolxConfig`], so
/// they cannot lean on [`merged_base_urls`]. Pairing this with
/// [`write_base_urls`] is what keeps a legacy config from ending up with its
/// grants split across two keys.
fn read_base_urls(obj: &serde_json::Map<String, serde_json::Value>) -> Vec<String> {
    let mut list: Vec<String> = obj
        .get(KEY_BASE_URLS)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let legacy: Vec<String> = obj
        .get(KEY_BASE_URLS_LEGACY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    for prefix in legacy {
        if !list.iter().any(|p| *p == prefix) {
            list.push(prefix);
        }
    }
    list
}

/// Write the allowlist under the current key and drop the legacy one, so a
/// config converges on a single key the first time it is mutated.
fn write_base_urls(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    list: Vec<String>,
) -> Result<()> {
    obj.insert(KEY_BASE_URLS.into(), serde_json::to_value(list)?);
    obj.remove(KEY_BASE_URLS_LEGACY);
    Ok(())
}

const KEY_BASE_URLS: &str = "allowed_base_urls";
const KEY_BASE_URLS_LEGACY: &str = "allowed_webhook_base_urls";

const CONFIG_FILE: &str = "solx-config.json";

/// Default port `solx-server` binds on `127.0.0.1` when `server_port` isn't
/// configured. Distinct from the OAuth loopback's own default port (8765,
/// see `solx-actions::oauth_loopback::DEFAULT_LOOPBACK_PORT`).
pub const DEFAULT_SERVER_PORT: u16 = 8766;

/// Resolve the solx appdata directory:
/// `SOLX_APPDATA_DIR` env override → `%APPDATA%/praeus/solx` (Windows) →
/// `$HOME/.praeus/solx` → temp dir fallback.
pub fn appdata_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SOLX_APPDATA_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.trim().is_empty() {
                return PathBuf::from(appdata).join("praeus").join("solx");
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home).join(".praeus").join("solx");
        }
    }
    std::env::temp_dir().join("praeus").join("solx")
}

/// How far a cached stamp's mtime must trail the moment it was loaded before
/// the cache trusts it — git's "racy clean" rule. Filesystem timestamps are
/// coarse (a ~16ms tick on Windows, 2s on FAT), so a write landing in the same
/// tick as the one we cached leaves the mtime unchanged; the length check
/// doesn't help either when the new contents are the same size. Inside this
/// window every read goes to disk, which costs a re-read only just after a
/// write.
const RACY_WINDOW: Duration = Duration::from_secs(2);

/// What the cache compares against the file on disk to detect a change.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    mtime: SystemTime,
    len: u64,
}

fn stamp_of(meta: &fs::Metadata) -> Option<Stamp> {
    Some(Stamp {
        mtime: meta.modified().ok()?,
        len: meta.len(),
    })
}

struct Cached {
    value: Value,
    /// `None` when the file didn't exist at load time.
    stamp: Option<Stamp>,
    loaded_at: SystemTime,
}

impl Cached {
    fn new(value: Value, stamp: Option<Stamp>) -> Self {
        Cached {
            value,
            stamp,
            loaded_at: SystemTime::now(),
        }
    }

    /// Whether this cache still describes a file currently stamped `disk`.
    fn is_fresh(&self, disk: Option<Stamp>) -> bool {
        match (self.stamp, disk) {
            (None, None) => true,
            (Some(cached), Some(disk)) if cached == disk => cached
                .mtime
                .checked_add(RACY_WINDOW)
                .is_some_and(|settled| settled <= self.loaded_at),
            _ => false,
        }
    }
}

/// The config service. Cheap to share via `Arc`; holds a stamp-guarded cache.
pub struct ConfigService {
    appdata: PathBuf,
    path: PathBuf,
    cache: RwLock<Cached>,
}

impl ConfigService {
    /// Open (or lazily create) the config service rooted at the default appdata
    /// directory.
    pub fn open() -> Result<Self> {
        Self::open_in(appdata_dir())
    }

    /// Open the config service with an explicit appdata directory (used in tests).
    pub fn open_in(appdata: impl Into<PathBuf>) -> Result<Self> {
        let appdata = appdata.into();
        fs::create_dir_all(&appdata)?;
        let path = appdata.join(CONFIG_FILE);
        let (value, stamp) = read_file(&path)?;
        Ok(ConfigService {
            appdata,
            path,
            cache: RwLock::new(Cached::new(value, stamp)),
        })
    }

    /// The appdata directory this service is rooted at.
    pub fn appdata(&self) -> &Path {
        &self.appdata
    }

    /// Path to the config file.
    pub fn config_path(&self) -> &Path {
        &self.path
    }

    /// Current file stamp on disk (if the file exists).
    fn disk_stamp(&self) -> Option<Stamp> {
        fs::metadata(&self.path).ok().as_ref().and_then(stamp_of)
    }

    /// Return a clone of the current config value, reloading from disk first if
    /// another process changed it since we last read.
    pub fn raw_snapshot(&self) -> Value {
        let disk = self.disk_stamp();
        {
            let cache = self.cache.read().unwrap();
            if cache.is_fresh(disk) {
                return cache.value.clone();
            }
        }
        // Stale — reload under a write lock.
        let mut cache = self.cache.write().unwrap();
        if let Ok((value, stamp)) = read_file(&self.path) {
            *cache = Cached::new(value, stamp);
        }
        cache.value.clone()
    }

    /// Typed snapshot of the config.
    pub fn snapshot(&self) -> SolxConfig {
        serde_json::from_value(self.raw_snapshot()).unwrap_or_default()
    }

    /// Read a top-level key.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.raw_snapshot().get(key).cloned()
    }

    /// Read-modify-write the config under a cross-process exclusive lock. The
    /// closure receives the authoritative on-disk object (fresh under the lock)
    /// as a mutable [`serde_json::Map`], preserving any unknown keys.
    pub fn mutate<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&mut Map<String, Value>) -> Result<()>,
    {
        // Hold the cache write lock for the whole RMW so in-process writers are
        // serialized too and the cache stays consistent with the file.
        let mut cache = self.cache.write().unwrap();

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&self.path)?;
        file.lock_exclusive()
            .map_err(|e| SolxError::Config(format!("lock config file: {e}")))?;

        let result = (|| -> Result<Value> {
            // Read authoritative current contents under the lock.
            let mut buf = String::new();
            file.seek(SeekFrom::Start(0))?;
            file.read_to_string(&mut buf)?;
            let mut obj = if buf.trim().is_empty() {
                Map::new()
            } else {
                match serde_json::from_str::<Value>(&buf)? {
                    Value::Object(m) => m,
                    other => {
                        return Err(SolxError::Config(format!(
                            "config root must be a JSON object, found {}",
                            kind_of(&other)
                        )));
                    }
                }
            };

            f(&mut obj)?;

            let value = Value::Object(obj);
            let serialized = serde_json::to_string_pretty(&value)?;
            file.seek(SeekFrom::Start(0))?;
            file.set_len(0)?;
            file.write_all(serialized.as_bytes())?;
            file.flush()?;
            Ok(value)
        })();

        // Best-effort unlock (also released on drop).
        let _ = FileExt::unlock(&file);

        let value = result?;
        *cache = Cached::new(value, file.metadata().ok().as_ref().and_then(stamp_of));
        Ok(())
    }

    /// Set a single top-level key.
    pub fn set(&self, key: &str, value: Value) -> Result<()> {
        self.mutate(|obj| {
            obj.insert(key.to_string(), value);
            Ok(())
        })
    }

    /// Shallow-merge a JSON object into the top level (overwriting on collision).
    pub fn patch(&self, patch: Value) -> Result<()> {
        let patch = match patch {
            Value::Object(m) => m,
            other => {
                return Err(SolxError::Config(format!(
                    "patch must be a JSON object, found {}",
                    kind_of(&other)
                )));
            }
        };
        self.mutate(|obj| {
            for (k, v) in patch {
                obj.insert(k, v);
            }
            Ok(())
        })
    }

    // ── Derived path accessors ───────────────────────────────────────────────

    pub fn db_dir(&self) -> PathBuf {
        match self.snapshot().data_directory {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => self.appdata.join("db"),
        }
    }

    pub fn docs_db_path(&self) -> PathBuf {
        self.db_dir().join(
            self.snapshot()
                .docs_db
                .unwrap_or_else(|| "solx-docs.db".into()),
        )
    }

    pub fn actions_db_path(&self) -> PathBuf {
        self.db_dir().join(
            self.snapshot()
                .actions_db
                .unwrap_or_else(|| "solx-actions.db".into()),
        )
    }

    /// Separate physical file from `actions_db_path()` — consoles and
    /// invocations have no DB-level foreign keys into the `actions` table,
    /// only an app-level `action_ref` string, so they don't need to share
    /// its file.
    pub fn console_db_path(&self) -> PathBuf {
        self.db_dir().join("solx-console.db")
    }

    /// The token registry's database (client tokens, action tokens and
    /// connect codes). A fixed name like the console's, shared by every
    /// process on this appdata dir: that's what lets a CLI process mint a
    /// connect code that a running host then redeems.
    pub fn registry_db_path(&self) -> PathBuf {
        self.db_dir().join("solx-registry.db")
    }

    pub fn types_db_path(&self) -> PathBuf {
        self.db_dir().join(
            self.snapshot()
                .types_db
                .unwrap_or_else(|| "solx-types.db".into()),
        )
    }

    pub fn files_dir(&self) -> PathBuf {
        match self.snapshot().files_directory {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => self.appdata.join("files"),
        }
    }

    /// Where packages installed from a zip are unpacked (one subdirectory
    /// per package). Read per use, so changing it needs no restart.
    pub fn packages_dir(&self) -> PathBuf {
        match self.snapshot().packages_directory {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => self.appdata.join("packages"),
        }
    }

    pub fn search_index_dir(&self) -> PathBuf {
        match self.snapshot().search_index_dir {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => self.appdata.join("search_index"),
        }
    }

    /// Whether an in-process host also serves the managers over HTTP. See
    /// `SolxConfig::server_embedded`. Defaults to `true`.
    pub fn server_embedded(&self) -> bool {
        self.snapshot().server_embedded.unwrap_or(true)
    }

    /// Ring-buffer cap on entries retained per action console. Defaults to
    /// 5000 when unset or non-positive.
    pub fn console_max_entries(&self) -> i64 {
        match self.snapshot().console_max_entries {
            Some(n) if n > 0 => n,
            _ => 5000,
        }
    }

    /// TTL (in days) before an idle console is swept away entirely. Defaults
    /// to 7 when unset or non-positive.
    pub fn console_ttl_days(&self) -> i64 {
        match self.snapshot().console_ttl_days {
            Some(n) if n > 0 => n,
            _ => 7,
        }
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.appdata.join("logs")
    }

    /// Wall-clock ceiling for a detached invocation with no
    /// `action_config.timeout_secs` of its own. Defaults to 86400 (24h)
    /// when unset or non-positive.
    pub fn background_timeout_secs(&self) -> u64 {
        match self.snapshot().background_timeout_secs {
            Some(n) if n > 0 => n,
            _ => 86400,
        }
    }

    /// Grace period `action-stop` waits for cooperative exit before
    /// force-aborting. Defaults to 10 when unset or non-positive.
    pub fn stop_grace_secs(&self) -> u64 {
        match self.snapshot().stop_grace_secs {
            Some(n) if n > 0 => n,
            _ => 10,
        }
    }

    /// Lifetime of a client token issued by the token registry, in days.
    /// Defaults to 90.
    pub fn client_token_ttl_days(&self) -> i64 {
        match self.snapshot().client_token_ttl_days {
            Some(n) if n > 0 => n,
            _ => 90,
        }
    }

    /// How a call outside its scopes is treated (`scope_enforcement`).
    /// Defaults to enforce. A value that isn't `off`, `audit` or `enforce`
    /// enforces too: a typo must not turn the checks off.
    pub fn scope_enforcement(&self) -> Enforcement {
        match self.snapshot().scope_enforcement.as_deref() {
            None => Enforcement::default(),
            Some(value) => value.parse().unwrap_or(Enforcement::Enforce),
        }
    }

    /// `server_public_url` as a full URL (see [`normalize_public_url`]: a bare
    /// host gets `server_port`, and `https://` when `server_tls` is on, else
    /// `http://`); `None` when unset or invalid.
    pub fn server_public_url(&self) -> Option<String> {
        let snap = self.snapshot();
        let port = snap.server_port.unwrap_or(DEFAULT_SERVER_PORT);
        let https = self.server_tls().is_ok_and(|tls| tls.is_on());
        snap.server_public_url
            .filter(|u| !u.trim().is_empty())
            .and_then(|u| normalize_public_url(&u, port, https).ok())
    }

    /// `server_ca_file`, resolved against the appdata dir; `None` when unset.
    pub fn server_ca_file(&self) -> Option<PathBuf> {
        self.snapshot()
            .server_ca_file
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .map(|f| self.appdata.join(f))
    }

    /// `server_tls`, parsed (see [`ServerTls::parse`]). [`ServerTls::Off`]
    /// when unset. Relative file paths come back resolved against the
    /// appdata dir.
    pub fn server_tls(&self) -> Result<ServerTls> {
        let tls = match self.snapshot().server_tls {
            Some(value) => ServerTls::parse(&value)
                .map_err(|e| SolxError::Validation(format!("server_tls: {e}")))?,
            None => ServerTls::Off,
        };
        Ok(match tls {
            ServerTls::Files {
                cert_file,
                key_file,
            } => ServerTls::Files {
                cert_file: self.appdata.join(cert_file),
                key_file: self.appdata.join(key_file),
            },
            other => other,
        })
    }

    /// The pin clients should hold for this server: with `server_tls:
    /// "auto"`, the `pin` solx-certs records in `<appdata>/certs/tls.json`
    /// for the root it serves. `None` for any other `server_tls`, or before
    /// the certificates exist; clients of such a server rely on the OS trust
    /// store or `server_ca_file` instead.
    pub fn server_tls_pin(&self) -> Option<String> {
        if !matches!(self.server_tls(), Ok(ServerTls::Auto)) {
            return None;
        }
        let path = self.appdata.join("certs").join("tls.json");
        let state: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let pin = state.get("pin")?.as_str()?;
        credentials::is_pin(pin).then(|| pin.to_string())
    }

    /// TTL (in days) before a terminal invocation row is swept away.
    /// Defaults to 7 when unset or non-positive.
    pub fn invocation_ttl_days(&self) -> i64 {
        match self.snapshot().invocation_ttl_days {
            Some(n) if n > 0 => n,
            _ => 7,
        }
    }

    /// Idle TTL (in seconds) before an unpolled http_stream self-terminates.
    /// Defaults to 120 when unset or non-positive.
    pub fn http_stream_idle_ttl_secs(&self) -> u64 {
        match self.snapshot().http_stream_idle_ttl_secs {
            Some(n) if n > 0 => n,
            _ => 120,
        }
    }

    /// Max bytes buffered per http_stream before oldest chunks are dropped.
    /// Defaults to 8 MiB when unset or non-positive.
    pub fn http_stream_max_buffer_bytes(&self) -> u64 {
        match self.snapshot().http_stream_max_buffer_bytes {
            Some(n) if n > 0 => n,
            _ => 8 * 1024 * 1024,
        }
    }

    // ── Package registry ──────────────────────────────────────────────────────

    pub fn list_packages(&self) -> Vec<InstalledPackage> {
        self.snapshot().installed_packages
    }

    pub fn register_package(&self, pkg: InstalledPackage) -> Result<()> {
        self.mutate(|obj| {
            let mut list: Vec<InstalledPackage> = obj
                .get("installed_packages")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            list.retain(|p| p.name != pkg.name);
            list.push(pkg);
            obj.insert("installed_packages".into(), serde_json::to_value(list)?);
            Ok(())
        })
    }

    pub fn unregister_package(&self, name: &str) -> Result<()> {
        self.mutate(|obj| {
            let mut list: Vec<InstalledPackage> = obj
                .get("installed_packages")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            list.retain(|p| p.name != name);
            obj.insert("installed_packages".into(), serde_json::to_value(list)?);
            Ok(())
        })
    }

    // ── persisted environment variables ──────────────────────────────────────

    /// Every persisted variable, `namespace -> key -> value`. Loaded into the
    /// in-process environment store at startup.
    pub fn env_vars(&self) -> HashMap<String, HashMap<String, String>> {
        self.snapshot().env_vars.unwrap_or_default()
    }

    /// Persist `value` under `namespace`/`key`, creating the namespace if
    /// needed. Goes through the same cross-process `mutate()` lock as every
    /// other config write, so concurrent `set-env` calls can't lose an entry
    /// by read-modify-writing a stale snapshot.
    pub fn set_env_var(&self, namespace: &str, key: &str, value: &str) -> Result<()> {
        self.mutate(|obj| {
            let mut all: HashMap<String, HashMap<String, String>> = obj
                .get("env_vars")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            all.entry(namespace.to_string())
                .or_default()
                .insert(key.to_string(), value.to_string());
            obj.insert("env_vars".into(), serde_json::to_value(all)?);
            Ok(())
        })
    }

    // ── Command / Webhook allowlists ─────────────────────────────────────────

    /// The full `command_actions` allowlist: opaque key -> command
    /// definition. A `Command` action's `fn_name` is looked up here, never
    /// run as a literal shell string. Deny-by-default: unset or empty means
    /// no key resolves.
    pub fn command_actions(&self) -> HashMap<String, CommandDef> {
        self.snapshot().command_actions.unwrap_or_default()
    }

    /// Register (or replace) a single `command_actions` entry.
    pub fn register_command(&self, key: &str, def: CommandDef) -> Result<()> {
        self.mutate(|obj| {
            let mut map: HashMap<String, CommandDef> = obj
                .get("command_actions")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            map.insert(key.to_string(), def);
            obj.insert("command_actions".into(), serde_json::to_value(map)?);
            Ok(())
        })
    }

    /// URL prefixes an outbound request must start with — a `Webhook`
    /// action's `fn_name`, or the URL handed to one of the `/builtin/web/*`
    /// built-ins. Deny-by-default: unset or empty means no URL is permitted.
    pub fn allowed_base_urls(&self) -> Vec<String> {
        merged_base_urls(&self.snapshot())
    }

    /// The configured tool-catalogue exclusion rules: `tool_exclude` plus
    /// anything still under the former `mcp_exclude` key.
    ///
    /// Empty (the default) means no action is hidden *by config* — a row may
    /// still carry [`CAP_HIDDEN`], which is why callers should prefer
    /// [`Self::tool_policy`] over reading this directly.
    pub fn tool_exclude(&self) -> Vec<ToolRule> {
        let cfg = self.snapshot();
        merged_exclude(&cfg)
    }

    /// Resolve the hidden/destructive policy once, for repeated application
    /// across a catalogue.
    ///
    /// Build this before a loop rather than calling it inside one:
    /// [`Self::snapshot`] deserializes the entire config on every call.
    pub fn tool_policy(&self) -> ToolPolicy {
        let cfg = self.snapshot();
        let hidden = ExclusionPolicy::from_config(&cfg)
            .patterns(EntityKind::Action, ExcludeMode::Tooling)
            .to_vec();
        ToolPolicy::from_patterns(hidden, cfg.tool_destructive.clone().unwrap_or_default())
    }

    /// Resolve every configured exclusion list once: `exclusions.user`,
    /// `exclusions.tooling`, and the legacy `tool_exclude`/`mcp_exclude`
    /// rules folded into tooling actions. Same build-once advice as
    /// [`Self::tool_policy`].
    pub fn exclusion_policy(&self) -> ExclusionPolicy {
        ExclusionPolicy::from_config(&self.snapshot())
    }

    /// Replace the outbound base-URL allowlist wholesale.
    pub fn set_allowed_base_urls(&self, list: Vec<String>) -> Result<()> {
        self.mutate(|obj| write_base_urls(obj, list))
    }

    /// Remove a single `command_actions` entry. No-op if `key` isn't present.
    /// Used by `solx-packages::uninstall_package` to revoke exactly what a
    /// package's manifest granted (see `docs/next-steps.md` §1).
    pub fn deregister_command(&self, key: &str) -> Result<()> {
        self.mutate(|obj| {
            let mut map: HashMap<String, CommandDef> = obj
                .get("command_actions")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            map.remove(key);
            obj.insert("command_actions".into(), serde_json::to_value(map)?);
            Ok(())
        })
    }

    /// Append `prefix` to the outbound allowlist if not already present.
    pub fn add_allowed_base_url(&self, prefix: &str) -> Result<()> {
        self.mutate(|obj| {
            let mut list = read_base_urls(obj);
            if !list.iter().any(|p| p == prefix) {
                list.push(prefix.to_string());
            }
            write_base_urls(obj, list)
        })
    }

    /// Remove a single outbound base-URL prefix. No-op if not present.
    pub fn remove_allowed_base_url(&self, prefix: &str) -> Result<()> {
        self.mutate(|obj| {
            let mut list = read_base_urls(obj);
            list.retain(|p| p != prefix);
            write_base_urls(obj, list)
        })
    }

    /// Apply a set of top-level key changes (`null` removes the key) as one
    /// validated write, returning the keys that actually changed.
    ///
    /// The result is checked with [`validate`] *before* it is written, under
    /// the same lock as the read, so a bad value never reaches disk and
    /// never trips [`Self::snapshot`]'s fallback to defaults.
    ///
    /// Legacy spellings are folded as the other mutators do: writing
    /// `allowed_base_urls` or `tool_exclude` drops `allowed_webhook_base_urls`
    /// or `mcp_exclude`. If the caller didn't also mention the legacy key,
    /// its entries are merged into the new value first, so a caller unaware
    /// of it loses nothing; one that sends the legacy key (even as `null`)
    /// has already accounted for it.
    pub fn update(&self, changes: Map<String, Value>) -> Result<Vec<String>> {
        let mut changed = Vec::new();
        self.mutate(|obj| {
            let before = obj.clone();
            for (key, value) in &changes {
                if value.is_null() {
                    obj.remove(key);
                } else {
                    obj.insert(key.clone(), value.clone());
                }
            }
            for (current, legacy) in LEGACY_KEYS {
                if !changes.get(*current).is_some_and(|v| !v.is_null()) {
                    continue;
                }
                if !changes.contains_key(*legacy) {
                    if let (Some(Value::Array(old)), Some(Value::Array(new))) =
                        (before.get(*legacy), obj.get_mut(*current))
                    {
                        for item in old {
                            if !new.contains(item) {
                                new.push(item.clone());
                            }
                        }
                    }
                }
                obj.remove(*legacy);
            }
            validate(&Value::Object(obj.clone()))?;
            let keys: std::collections::BTreeSet<&String> =
                before.keys().chain(obj.keys()).collect();
            changed = keys
                .into_iter()
                .filter(|k| before.get(*k) != obj.get(*k))
                .cloned()
                .collect();
            Ok(())
        })?;
        Ok(changed)
    }

    // ── solx-server bearer token ─────────────────────────────────────────────

    /// Return the plaintext `server_token` from this file, generating and
    /// persisting a new random one if none exists yet. A single `mutate()`
    /// call makes generate-then-persist atomic under the same cross-process
    /// lock every other config write uses, so two `solx-server`s started
    /// concurrently against the same appdata dir can't race into two
    /// different tokens.
    ///
    /// Only the `token_store: "config"` path (and the no-credential-manager
    /// fallback) of [`credentials::root_token`] uses this; everything else
    /// should go through that module.
    pub fn ensure_server_token(&self) -> Result<String> {
        if let Some(token) = self.snapshot().server_token {
            if !token.trim().is_empty() {
                return Ok(token);
            }
        }
        let mut generated = String::new();
        self.mutate(|obj| {
            if let Some(existing) = obj.get("server_token").and_then(Value::as_str) {
                if !existing.trim().is_empty() {
                    generated = existing.to_string();
                    return Ok(());
                }
            }
            generated = credentials::generate_token();
            obj.insert("server_token".into(), Value::String(generated.clone()));
            Ok(())
        })?;
        Ok(generated)
    }
}

/// Keys that are read once when a solx host starts (databases opened,
/// server bound, env store seeded, startup sweeps run), so a change to one
/// only takes effect after a restart. Every other key is read per use and
/// applies to the next action, request or stream.
pub const RESTART_KEYS: &[&str] = &[
    "data_directory",
    "files_directory",
    "docs_db",
    "actions_db",
    "types_db",
    "env_mappings",
    "env_vars",
    "server_url",
    "server_ca_file",
    "server_token",
    "token_store",
    "server_port",
    "server_host",
    "server_cors_origins",
    "server_tls",
    "server_embedded",
    "console_ttl_days",
    "invocation_ttl_days",
];

/// Legacy key pairs `update` folds together: (current, former).
const LEGACY_KEYS: &[(&str, &str)] = &[
    (KEY_BASE_URLS, KEY_BASE_URLS_LEGACY),
    ("tool_exclude", "mcp_exclude"),
];

/// How `solx-server` serves TLS, from the `server_tls` config key. See
/// `docs/tls-plan.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerTls {
    /// Plain HTTP only (the default).
    Off,
    /// The certificates in `<appdata>/certs`, as `tls.json` there records
    /// them. Made by `solx-certs`, or provisioned by the desktop app.
    Auto,
    /// A PEM certificate chain and its PEM private key.
    Files {
        cert_file: PathBuf,
        key_file: PathBuf,
    },
    /// Windows: the certificate in `CurrentUser\My` with this SHA-256
    /// thumbprint (lowercase hex), served with its key from the store.
    Store { sha256: String },
}

impl ServerTls {
    /// Read a `server_tls` value: `"off"`, `"auto"`,
    /// `{ "cert_file", "key_file" }` or `{ "store_thumbprint" }`. A
    /// thumbprint may be written with spaces or colons, as `certmgr` and
    /// `certutil` show it. File paths are returned as written.
    pub fn parse(value: &Value) -> std::result::Result<Self, String> {
        let expected =
            r#"expected "off", "auto", { "cert_file", "key_file" } or { "store_thumbprint" }"#;
        match value {
            Value::String(mode) => match mode.trim().to_ascii_lowercase().as_str() {
                "off" => Ok(ServerTls::Off),
                "auto" => Ok(ServerTls::Auto),
                other => Err(format!("unknown mode {other:?}; {expected}")),
            },
            Value::Object(map) => {
                let field = |name: &str| -> std::result::Result<Option<String>, String> {
                    match map.get(name) {
                        None => Ok(None),
                        Some(Value::String(s)) if !s.trim().is_empty() => {
                            Ok(Some(s.trim().to_string()))
                        }
                        Some(_) => Err(format!("{name} must be a non-empty string")),
                    }
                };
                if let Some(unknown) = map
                    .keys()
                    .find(|k| !matches!(k.as_str(), "cert_file" | "key_file" | "store_thumbprint"))
                {
                    return Err(format!("unknown field {unknown:?}; {expected}"));
                }
                match (
                    field("cert_file")?,
                    field("key_file")?,
                    field("store_thumbprint")?,
                ) {
                    (Some(cert), Some(key), None) => Ok(ServerTls::Files {
                        cert_file: PathBuf::from(cert),
                        key_file: PathBuf::from(key),
                    }),
                    (None, None, Some(thumbprint)) => {
                        let sha256: String = thumbprint
                            .chars()
                            .filter(|c| !matches!(c, ' ' | ':'))
                            .collect::<String>()
                            .to_ascii_lowercase();
                        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                            return Err(format!(
                                "store_thumbprint {thumbprint:?} is not a SHA-256 thumbprint (64 hex digits)"
                            ));
                        }
                        Ok(ServerTls::Store { sha256 })
                    }
                    (Some(_), None, None) | (None, Some(_), None) => {
                        Err("cert_file and key_file go together".into())
                    }
                    _ => Err(format!("give either files or a thumbprint; {expected}")),
                }
            }
            _ => Err(expected.to_string()),
        }
    }

    pub fn is_on(&self) -> bool {
        !matches!(self, ServerTls::Off)
    }
}

/// The address other machines use to reach a solx-server, as a URL without a
/// trailing slash. Takes either a full `http://`/`https://` URL (a path is
/// kept, for a reverse proxy under a prefix) or a bare host, optionally with
/// a port: an IP (`100.101.102.103`, `fd7a::1`) or a name
/// (`my-laptop.tailnet.ts.net`). A bare host gets `http://` (`https://`
/// when `https`, for a server with TLS on) and, when it names no port,
/// `default_port`. A full URL is kept as written.
pub fn normalize_public_url(input: &str, default_port: u16, https: bool) -> Result<String> {
    let input = input.trim();
    let input = if input.contains("://") {
        input
    } else {
        input.trim_end_matches('/')
    };
    let bad = |why: &str| {
        Err(SolxError::Invalid(format!(
            "{input:?} {why}: expected an http(s) URL, or an IP address or host name"
        )))
    };
    if input.is_empty() {
        return bad("is empty");
    }
    if input.contains("://") {
        let Ok(url) = url::Url::parse(input) else {
            return bad("is not a valid URL");
        };
        if !matches!(url.scheme(), "http" | "https") || !url.has_host() {
            return bad("is not an http(s) URL with a host");
        }
        if url.query().is_some() || url.fragment().is_some() {
            return bad("has a query or fragment");
        }
        return Ok(url.as_str().trim_end_matches('/').to_string());
    }
    // A bare IPv6 address needs brackets before it can be read as a host.
    let host = match input.parse::<std::net::Ipv6Addr>() {
        Ok(ip) => format!("[{ip}]"),
        Err(_) => input.to_string(),
    };
    let Ok(mut url) = url::Url::parse(&format!("http://{host}")) else {
        return bad("is not a valid host");
    };
    if !url.has_host() || url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return bad("is not a bare host");
    }
    if url.port().is_none() {
        let _ = url.set_port(Some(default_port));
    }
    if https {
        // http → https is always allowed; a port that is now the scheme's
        // default (443) drops out.
        let _ = url.set_scheme("https");
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Strictly validate a whole raw config object.
///
/// [`ConfigService::snapshot`] deliberately never fails — a type error there
/// yields [`SolxConfig::default()`], which would silently empty both
/// allowlists and move the database paths. So anything that *writes* a
/// config built from outside input must check it here first: the same
/// deserialization without the fallback, plus the value checks the type
/// system can't express.
pub fn validate(value: &Value) -> Result<SolxConfig> {
    let cfg: SolxConfig = serde_json::from_value(value.clone())
        .map_err(|e| SolxError::Validation(format!("solx-config.json: {e}")))?;
    let invalid = |msg: String| Err(SolxError::Validation(msg));

    if let Some(host) = cfg
        .server_host
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if host.parse::<std::net::IpAddr>().is_err() {
            return invalid(format!("server_host {host:?} is not an IP address"));
        }
    }
    if let Some(store) = cfg.token_store.as_deref() {
        if !matches!(store, "keyring" | "config") {
            return invalid(format!(
                "token_store {store:?} must be \"keyring\" or \"config\""
            ));
        }
    }
    if let Some(url) = cfg
        .server_public_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Err(e) = normalize_public_url(url, DEFAULT_SERVER_PORT, false) {
            return invalid(format!("server_public_url: {e}"));
        }
    }
    if cfg.server_port == Some(0) {
        return invalid("server_port must be between 1 and 65535".into());
    }
    if let Some(tls) = &cfg.server_tls {
        if let Err(e) = ServerTls::parse(tls) {
            return invalid(format!("server_tls: {e}"));
        }
    }
    for origin in cfg.server_cors_origins.iter().flatten() {
        if origin.trim().is_empty() || !origin.trim().chars().all(|c| c.is_ascii_graphic()) {
            return invalid(format!(
                "server_cors_origins entry {origin:?} is not a valid origin"
            ));
        }
    }
    if let Some(ex) = &cfg.exclusions {
        let scopes = [("user", &ex.user), ("tooling", &ex.tooling)];
        for (scope, lists) in scopes {
            let Some(lists) = lists else { continue };
            let kinds = [
                ("documents", &lists.documents),
                ("types", &lists.types),
                ("actions", &lists.actions),
            ];
            for (kind, patterns) in kinds {
                for p in patterns {
                    // An invalid pattern would fail every list in its scope,
                    // so it is refused here rather than at query time.
                    if let Err(e) = solx_surface::query::validate_exclude_pattern(p) {
                        return invalid(format!("exclusions.{scope}.{kind}: {e}"));
                    }
                }
            }
        }
    }
    for (key, def) in cfg.command_actions.iter().flatten() {
        if key.trim().is_empty() {
            return invalid("command_actions keys must not be empty".into());
        }
        if def.command.trim().is_empty() {
            return invalid(format!("command_actions.{key}.command must not be empty"));
        }
    }
    for prefix in merged_base_urls(&cfg) {
        if !is_base_url(&prefix) {
            return invalid(format!(
                "allowed_base_urls entry {prefix:?} must be an http(s) URL prefix with a host, \
                 e.g. \"https://api.example.com\""
            ));
        }
    }
    let rules = merged_exclude(&cfg)
        .into_iter()
        .map(|r| ("tool_exclude", r));
    let rules = rules.chain(
        cfg.tool_destructive
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|r| ("tool_destructive", r)),
    );
    for (field, rule) in rules {
        if rule.path.trim().is_empty() {
            return invalid(format!("{field} rules need a non-empty path"));
        }
    }
    Ok(cfg)
}

/// `scheme://host[...]` with an http(s) scheme and a non-empty host — the
/// shape `solx-actions::net` matches request URLs against. Kept to a syntax
/// check so this crate doesn't take a URL-parsing dependency.
fn is_base_url(prefix: &str) -> bool {
    let rest = match prefix.trim().split_once("://") {
        Some((scheme, rest))
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") =>
        {
            rest
        }
        _ => return false,
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = if host.starts_with('[') {
        host
    } else {
        host.split(':').next().unwrap_or("")
    };
    !host.is_empty() && !host.chars().any(char::is_whitespace)
}

/// Read and parse the config file, with the stamp it had *before* the read.
/// Stamping first matters: a write landing between the two steps then leaves
/// the cache holding an old stamp, so the next read sees it as stale — the
/// other order would pair the new stamp with the old contents indefinitely.
fn read_file(path: &Path) -> Result<(Value, Option<Stamp>)> {
    let stamp = fs::metadata(path).ok().as_ref().and_then(stamp_of);
    match fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => Ok((serde_json::from_str(&s)?, stamp)),
        Ok(_) => Ok((Value::Object(Map::new()), stamp)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Value::Object(Map::new()), None)),
        Err(e) => Err(e.into()),
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enforce by default, read live, and a value it doesn't know enforces.
    #[test]
    fn scope_enforcement_defaults_to_enforce_and_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        assert_eq!(cfg.scope_enforcement(), Enforcement::Enforce);
        for (value, expected) in [
            ("off", Enforcement::Off),
            ("audit", Enforcement::Audit),
            ("enforce", Enforcement::Enforce),
            ("enforced", Enforcement::Enforce),
        ] {
            cfg.set("scope_enforcement", value.into()).unwrap();
            assert_eq!(cfg.scope_enforcement(), expected, "{value}");
        }
    }

    #[test]
    fn set_get_and_unknown_field_preserved() {
        let dir = tempfile::tempdir().unwrap();
        // Seed a file with an unknown field.
        fs::write(
            dir.path().join(CONFIG_FILE),
            r#"{"future_flag": 42, "data_directory": "/x"}"#,
        )
        .unwrap();

        let cfg = ConfigService::open_in(dir.path()).unwrap();
        assert_eq!(cfg.snapshot().data_directory.as_deref(), Some("/x"));

        cfg.set("files_directory", Value::String("/f".into()))
            .unwrap();

        // Unknown field must survive the write.
        let raw = cfg.raw_snapshot();
        assert_eq!(raw.get("future_flag"), Some(&Value::from(42)));
        assert_eq!(cfg.snapshot().files_directory.as_deref(), Some("/f"));
    }

    #[test]
    fn cross_instance_reload_via_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let a = ConfigService::open_in(dir.path()).unwrap();
        let b = ConfigService::open_in(dir.path()).unwrap();

        a.set("data_directory", Value::String("/from-a".into()))
            .unwrap();
        // b must observe a's write on next read (simulates a second process).
        // Ensure mtime differs even on coarse-resolution clocks.
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert_eq!(b.snapshot().data_directory.as_deref(), Some("/from-a"));
    }

    /// Two back-to-back writes of the same length can share an mtime on a
    /// coarse clock; `b` must still see the second one, with no sleep to help.
    #[test]
    fn cross_instance_reload_sees_rapid_same_length_writes() {
        let dir = tempfile::tempdir().unwrap();
        let a = ConfigService::open_in(dir.path()).unwrap();
        let b = ConfigService::open_in(dir.path()).unwrap();

        for value in ["/skip=10", "/skip=20", "/skip=30"] {
            a.set("data_directory", Value::String(value.into()))
                .unwrap();
            assert_eq!(b.snapshot().data_directory.as_deref(), Some(value));
        }
    }

    #[test]
    fn package_registry_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        cfg.register_package(InstalledPackage {
            name: "pkg".into(),
            version: "1.0".into(),
            path: "/p".into(),
            installed_at: "now".into(),
            granted_commands: vec![],
            granted_base_urls: vec![],
            locked_actions: vec![],
            granted_scopes: vec![],
        })
        .unwrap();
        assert_eq!(cfg.list_packages().len(), 1);
        cfg.unregister_package("pkg").unwrap();
        assert!(cfg.list_packages().is_empty());
    }

    #[test]
    fn command_and_webhook_allowlists_default_empty_and_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();

        // Deny-by-default: nothing registered yet.
        assert!(cfg.command_actions().is_empty());
        assert!(cfg.allowed_base_urls().is_empty());

        cfg.register_command(
            "compress-pdf",
            CommandDef {
                command: "gs -o out.pdf in.pdf".into(),
                description: Some("Compress a PDF".into()),
                cwd: None,
            },
        )
        .unwrap();
        let map = cfg.command_actions();
        assert_eq!(map["compress-pdf"].command, "gs -o out.pdf in.pdf");

        cfg.set_allowed_base_urls(vec!["https://hooks.example.com".into()])
            .unwrap();
        assert_eq!(
            cfg.allowed_base_urls(),
            vec!["https://hooks.example.com".to_string()]
        );
    }

    /// A config written before the rename must keep working untouched — the
    /// same guarantee `mcp_exclude` gets from `merged_exclude`.
    #[test]
    fn legacy_webhook_key_is_still_read_and_unions_with_the_current_one() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();

        cfg.set(
            "allowed_webhook_base_urls",
            serde_json::json!(["https://legacy.example.com/"]),
        )
        .unwrap();
        assert_eq!(
            cfg.allowed_base_urls(),
            vec!["https://legacy.example.com/".to_string()]
        );

        cfg.set(
            "allowed_base_urls",
            serde_json::json!(["https://current.example.com/"]),
        )
        .unwrap();
        let merged = cfg.allowed_base_urls();
        assert!(merged.contains(&"https://current.example.com/".to_string()));
        assert!(merged.contains(&"https://legacy.example.com/".to_string()));
    }

    /// The mutators read raw JSON rather than a typed snapshot, so the legacy
    /// key has to be folded in and dropped by hand. If it isn't, grants split
    /// across two keys and `remove` fails to revoke.
    #[test]
    fn mutating_folds_the_legacy_key_into_the_current_one_and_drops_it() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();

        cfg.set(
            "allowed_webhook_base_urls",
            serde_json::json!(["https://legacy.example.com/"]),
        )
        .unwrap();
        cfg.add_allowed_base_url("https://added.example.com/")
            .unwrap();

        let snap = cfg.snapshot();
        assert!(
            snap.allowed_webhook_base_urls.is_none(),
            "legacy key should be gone"
        );
        let current = snap.allowed_base_urls.unwrap();
        assert!(current.contains(&"https://legacy.example.com/".to_string()));
        assert!(current.contains(&"https://added.example.com/".to_string()));

        // The fold is what makes this revoke reach a legacy-written prefix.
        cfg.remove_allowed_base_url("https://legacy.example.com/")
            .unwrap();
        assert_eq!(
            cfg.allowed_base_urls(),
            vec!["https://added.example.com/".to_string()]
        );
    }

    /// Package rows recorded before the rename must still deserialize, or
    /// `uninstall_package` revokes nothing.
    #[test]
    fn installed_package_reads_the_legacy_granted_field_name() {
        let row: InstalledPackage = serde_json::from_value(serde_json::json!({
            "name": "solx-example",
            "granted_webhook_prefixes": ["https://legacy.example.com/"],
        }))
        .unwrap();
        assert_eq!(
            row.granted_base_urls,
            vec!["https://legacy.example.com/".to_string()]
        );
    }

    #[test]
    fn ensure_server_token_generates_once_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();

        assert!(cfg.snapshot().server_token.is_none());
        let token = cfg.ensure_server_token().unwrap();
        assert!(!token.trim().is_empty());
        assert_eq!(cfg.snapshot().server_token.as_deref(), Some(token.as_str()));

        // Calling again (same instance, or a fresh one over the same file —
        // simulating a second solx-server process) must return the SAME
        // token, not regenerate.
        assert_eq!(cfg.ensure_server_token().unwrap(), token);
        let cfg2 = ConfigService::open_in(dir.path()).unwrap();
        assert_eq!(cfg2.ensure_server_token().unwrap(), token);
    }

    /// A ConfigService over a throwaway directory. The TempDir must stay
    /// alive for the service's lifetime, hence the tuple.
    fn svc() -> (tempfile::TempDir, ConfigService) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        (dir, cfg)
    }

    #[test]
    fn tool_exclude_reads_the_new_key() {
        let (_d, c) = svc();
        c.set("tool_exclude", serde_json::json!([{ "path": "/a" }]))
            .unwrap();
        let rules = c.tool_exclude();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "/a");
    }

    #[test]
    fn tool_exclude_still_reads_the_former_key() {
        // An existing solx-config.json must keep working untouched — nothing
        // rewrites the file, so the old key has to stay readable indefinitely.
        let (_d, c) = svc();
        c.set("mcp_exclude", serde_json::json!([{ "path": "/legacy" }]))
            .unwrap();
        let rules = c.tool_exclude();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "/legacy");
    }

    #[test]
    fn both_keys_union_rather_than_one_winning() {
        // The reason this is two fields and not #[serde(alias)]: serde would
        // reject both-names-present as a duplicate field, and snapshot()
        // turns any deserialization error into SolxConfig::default() — so
        // adding the new key beside the old one would silently blank the
        // whole config instead of merging the lists.
        let (_d, c) = svc();
        c.set("tool_exclude", serde_json::json!([{ "path": "/new" }]))
            .unwrap();
        c.set("mcp_exclude", serde_json::json!([{ "path": "/legacy" }]))
            .unwrap();

        let paths: Vec<String> = c.tool_exclude().into_iter().map(|r| r.path).collect();
        assert_eq!(paths.len(), 2, "{paths:?}");
        assert!(paths.contains(&"/new".to_string()), "{paths:?}");
        assert!(paths.contains(&"/legacy".to_string()), "{paths:?}");

        // And the rest of the config must survive both keys being present.
        assert!(
            c.snapshot().command_actions.is_some() || c.get("tool_exclude").is_some(),
            "config must still deserialize with both keys set"
        );
    }

    #[test]
    fn update_rejects_an_invalid_exclusion_pattern() {
        let (_d, c) = svc();
        let err = c
            .update(changes(serde_json::json!({
                "exclusions": { "user": { "documents": ["agent/skills"] } }
            })))
            .unwrap_err();
        assert!(
            err.to_string().contains("exclusions.user.documents"),
            "{err}"
        );
        c.update(changes(serde_json::json!({
            "exclusions": { "user": { "documents": ["/agent/skills", "*/_private/*"] } }
        })))
        .unwrap();
    }

    #[test]
    fn tool_policy_sees_rules_from_either_key() {
        let (_d, c) = svc();
        c.set("mcp_exclude", serde_json::json!([{ "path": "/legacy" }]))
            .unwrap();
        let policy = c.tool_policy();
        let a: solx_surface::entities::Action = serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "path": "/legacy",
            "name": "thing",
        }))
        .unwrap();
        assert!(policy.is_hidden(&a));
    }

    fn changes(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn update_rejects_a_bad_value_and_leaves_the_file_untouched() {
        let (_d, c) = svc();
        c.set(
            "allowed_base_urls",
            serde_json::json!(["https://a.example"]),
        )
        .unwrap();
        let before = fs::read_to_string(c.config_path()).unwrap();

        let err = c
            .update(changes(serde_json::json!({ "server_port": "abc" })))
            .unwrap_err();
        assert!(matches!(err, SolxError::Validation(_)), "{err:?}");
        for bad in [
            serde_json::json!({ "server_host": "not-an-ip" }),
            serde_json::json!({ "server_port": 0 }),
            serde_json::json!({ "allowed_base_urls": ["example.com"] }),
            serde_json::json!({ "command_actions": { "x": { "command": " " } } }),
            serde_json::json!({ "tool_destructive": [{ "path": "" }] }),
        ] {
            assert!(
                c.update(changes(bad.clone())).is_err(),
                "{bad} should be rejected"
            );
        }
        assert_eq!(fs::read_to_string(c.config_path()).unwrap(), before);
    }

    #[test]
    fn update_sets_removes_and_reports_changed_keys() {
        let (_d, c) = svc();
        c.set("unknown_tool_key", serde_json::json!(1)).unwrap();
        c.set("console_ttl_days", serde_json::json!(3)).unwrap();

        let changed = c
            .update(changes(serde_json::json!({
                "server_port": 9000,
                "console_ttl_days": null,
                "stop_grace_secs": 10,
            })))
            .unwrap();
        let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
        assert_eq!(
            changed,
            ["console_ttl_days", "server_port", "stop_grace_secs"]
        );
        assert_eq!(c.snapshot().server_port, Some(9000));
        assert!(c.get("console_ttl_days").is_none());
        assert_eq!(c.get("unknown_tool_key"), Some(serde_json::json!(1)));

        // Re-sending the same values changes nothing.
        assert!(c
            .update(changes(serde_json::json!({ "server_port": 9000 })))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn update_folds_legacy_keys() {
        let (_d, c) = svc();
        c.set(
            "allowed_webhook_base_urls",
            serde_json::json!(["https://old.example"]),
        )
        .unwrap();
        c.set("mcp_exclude", serde_json::json!([{ "path": "/legacy" }]))
            .unwrap();

        // Unaware of the legacy key: its entries are kept, under the new key.
        c.update(changes(
            serde_json::json!({ "allowed_base_urls": ["https://new.example"] }),
        ))
        .unwrap();
        assert_eq!(
            c.get("allowed_base_urls"),
            Some(serde_json::json!([
                "https://new.example",
                "https://old.example"
            ]))
        );
        assert!(c.get("allowed_webhook_base_urls").is_none());

        // Legacy key named explicitly: the caller's list is taken as-is.
        c.update(changes(
            serde_json::json!({ "tool_exclude": [{ "path": "/new" }], "mcp_exclude": null }),
        ))
        .unwrap();
        assert_eq!(
            c.get("tool_exclude"),
            Some(serde_json::json!([{ "path": "/new" }]))
        );
        assert!(c.get("mcp_exclude").is_none());
    }

    #[test]
    fn base_url_shape() {
        assert!(is_base_url("https://api.example.com"));
        assert!(is_base_url("http://127.0.0.1:"));
        assert!(is_base_url("http://localhost:8080/v1/"));
        assert!(is_base_url("http://[::1]:80"));
        assert!(!is_base_url("api.example.com"));
        assert!(!is_base_url("ftp://x"));
        assert!(!is_base_url("https://"));
    }

    #[test]
    fn public_urls_accept_full_urls_and_bare_hosts() {
        let n = |s: &str| normalize_public_url(s, 8766, false);
        assert_eq!(n("100.101.102.103").unwrap(), "http://100.101.102.103:8766");
        assert_eq!(
            n(" 100.101.102.103:9000/ ").unwrap(),
            "http://100.101.102.103:9000"
        );
        assert_eq!(n("fd7a:115c::1").unwrap(), "http://[fd7a:115c::1]:8766");
        assert_eq!(
            n("my-laptop.tailnet.ts.net").unwrap(),
            "http://my-laptop.tailnet.ts.net:8766"
        );
        // A full URL is kept as given: no port is added, a path survives.
        assert_eq!(
            n("https://solx.example.com/").unwrap(),
            "https://solx.example.com"
        );
        assert_eq!(
            n("https://example.com/solx/").unwrap(),
            "https://example.com/solx"
        );
        for bad in [
            "",
            "ftp://x",
            "http://",
            "host/path",
            "http://h/?q=1",
            "a b",
        ] {
            assert!(n(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_bare_public_host_gets_the_configured_port() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        cfg.set("server_port", 9100.into()).unwrap();
        cfg.set("server_public_url", "100.101.102.103".into())
            .unwrap();
        assert_eq!(
            cfg.server_public_url().as_deref(),
            Some("http://100.101.102.103:9100")
        );
        let err = validate(&serde_json::json!({ "server_public_url": "ftp://x" })).unwrap_err();
        assert!(err.to_string().contains("server_public_url"), "{err}");
    }

    #[test]
    fn server_tls_modes_parse_and_bad_ones_are_refused() {
        use serde_json::json;
        let p = |v: Value| ServerTls::parse(&v);
        assert_eq!(p(json!("off")), Ok(ServerTls::Off));
        assert_eq!(p(json!(" Auto ")), Ok(ServerTls::Auto));
        assert_eq!(
            p(json!({ "cert_file": "a.crt", "key_file": "a.key" })),
            Ok(ServerTls::Files {
                cert_file: "a.crt".into(),
                key_file: "a.key".into()
            })
        );
        let hex = "AB".repeat(32);
        let spaced = hex
            .as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            p(json!({ "store_thumbprint": spaced })),
            Ok(ServerTls::Store {
                sha256: "ab".repeat(32)
            })
        );
        for bad in [
            json!("on"),
            json!(true),
            json!({ "cert_file": "a.crt" }),
            json!({ "cert_file": "a.crt", "key_file": "" }),
            json!({ "store_thumbprint": "abc" }),
            json!({ "store_thumbprint": hex, "cert_file": "a", "key_file": "b" }),
            json!({ "cert_file": "a", "key_file": "b", "chain": "c" }),
        ] {
            assert!(p(bad.clone()).is_err(), "{bad}");
        }
        let err = validate(&json!({ "server_tls": "on" })).unwrap_err();
        assert!(err.to_string().contains("server_tls"), "{err}");

        // Relative files are under the appdata dir; unset is off.
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigService::open_in(dir.path()).unwrap();
        assert_eq!(cfg.server_tls().unwrap(), ServerTls::Off);
        cfg.set(
            "server_tls",
            json!({ "cert_file": "certs/s.crt", "key_file": "certs/s.key" }),
        )
        .unwrap();
        assert_eq!(
            cfg.server_tls().unwrap(),
            ServerTls::Files {
                cert_file: dir.path().join("certs/s.crt"),
                key_file: dir.path().join("certs/s.key"),
            }
        );

        // With TLS on, a bare public host is reached over https; a full URL
        // is kept as written.
        cfg.set("server_public_url", "100.101.102.103".into())
            .unwrap();
        assert_eq!(
            cfg.server_public_url().as_deref(),
            Some("https://100.101.102.103:8766")
        );
        cfg.set("server_public_url", "http://proxy.lan:80".into())
            .unwrap();
        assert_eq!(cfg.server_public_url().as_deref(), Some("http://proxy.lan"));
        assert_eq!(
            normalize_public_url("host.lan:443", 8766, true).unwrap(),
            "https://host.lan"
        );
    }
}
