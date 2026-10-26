//! `solx-packages` — install/uninstall solx packages.
//!
//! A package is a directory containing `solx-package.json` (at least `name`,
//! and a `version`) and an `install.solx` script (and optionally
//! `uninstall.solx`).
//!
//! The manifest used to be `package.json`, which is npm's file. Most package
//! directories here are Rust or wasm and were never node projects, so a
//! `package.json` in them misled every tool that saw one; the directories that
//! *are* node projects now keep the two manifests apart instead of overloading
//! shared keys like `name` and `version`, whose rules differ between the two.
//! `package.json` is still read as a fallback (with a warning) so third-party
//! packages keep installing.
//! Installation runs the script through [`solx_scripts`] against the CLI's
//! command runner, then records the package in `solx-config.json`.
//!
//! ## Command/Webhook allowlist grants
//!
//! solx's outbound allowlists are deny-by-default (see `docs/next-steps.md`
//! §1): a Command action's `fn_name` must be a key registered in
//! `command_actions`, and any outbound URL — a Webhook action's, or one
//! handed to a `/builtin/web/*` built-in — must match a prefix in
//! `allowed_base_urls`. A package that registers such actions in its
//! `install.solx`, or that reaches the network from a wasm guest, needs its
//! own entries granted, or it is unusable the moment it's installed.
//!
//! `solx-package.json` may declare `command_actions` and `allowed_base_urls`
//! in the exact same shape as `solx-config.json` itself (see
//! [`solx_config::CommandDef`]).
//! [`install_package`] grants every declared entry into the global
//! allowlist before running `install.solx` (so the script — or a later
//! `verify.solx` — can exec the package's own actions), and records exactly
//! what it granted on the [`InstalledPackage`] row. [`uninstall_package`]
//! revokes exactly that, unless another currently-installed package also
//! declares the same key/prefix (checked via every other package's own
//! recorded grants), in which case it's left alone.
//!
//! A key or prefix already granted by a **different** installed package is
//! not an install error — the newer package's value wins (last-write-wins,
//! the same semantics as reinstalling a package over itself), but a warning
//! naming both packages is logged and returned in [`InstallOutcome::warnings`]
//! so the collision isn't silent.
//!
//! Existing packages predating this feature (or a manifest with neither
//! field) are simply not affected — reinstalling is how they pick up
//! allowlist grants, exactly as before this feature existed for anything
//! else in the manifest.
//!
//! A relative `cwd` in a declared [`solx_config::CommandDef`] is resolved
//! against the package directory at install time, so a manifest can say
//! `"cwd": "bin"` instead of hard-coding where the package is checked out.
//! An absolute `cwd` is kept as is.
//!
//! ## Installed actions are locked
//!
//! Every action an install leaves under `/packages/<name>/` is locked (see
//! `solx_actions::guard_locked_action`): content — an MCP client, a WASM
//! guest, a widget — can run it but not change or delete it, and a locked
//! Script/WASM action runs only the artifact it was locked against. The
//! manifest may leave some unlocked with `"unlocked_actions": ["<name>",
//! ...]`, each relative to `/packages/<name>/`. The locked refs are recorded
//! on [`InstalledPackage::locked_actions`].
//!
//! A reinstall first unlocks the package's own actions: its `install.solx`
//! may rebuild an artifact through an action that writes it back over
//! HTTP with its own token (as `solx-quickjs`'s builds do), which a pin
//! would refuse. Everything is locked again once the script has run, or,
//! if it fails, what the previous install had locked.
//!
//! A keyed action (scoped secrets, or an `auth` keyring reference) is
//! always locked: it is not unlocked for a reinstall, and is locked even
//! when `unlocked_actions` lists it. Its
//! artifact must therefore be written by the script itself (`save file`,
//! the operator path), not rebuilt over HTTP with an action token.
//!
//! ## A package's scripts reach only its own actions
//!
//! `install.solx` and `uninstall.solx` run as the operator, which may save
//! an action that declares secrets. The key is chosen by path (everything
//! under `/packages/<name>/` shares `package:<name>`), so a script saving
//! `/packages/<other>/steal` would read the other package's secrets. While
//! a script runs, [`guard_package_script_action`] refuses any action save
//! or delete outside the package's own `/packages/<name>/`, and a script
//! may not install or uninstall a package (a nested install would run
//! under another name). A package name must be a single path segment for
//! the same reason: `a/b` would install under `/packages/a/`.
//!
//! ## Installing from a zip
//!
//! [`install_package_zip`] takes a zip of a package (the manifest at its root,
//! or inside the one top-level directory that has one, ignoring build-time
//! siblings), unpacks it to `<packages_directory>/<name>/` — see
//! [`ConfigService::packages_dir`] — and installs from there, so the package
//! outlives the zip. Reinstalling replaces the old directory, restoring it if
//! the new install fails. [`uninstall_package`] deletes a package's directory
//! when it lives under the packages directory; a package installed from some
//! other directory is never deleted.
//!
//! ## The official catalog
//!
//! [`catalog`] lists the packages released from `praeus/solx-packages` and
//! installs them by name from a release (or a local folder of zips).

pub mod catalog;

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use solx_config::{CommandDef, ConfigService, InstalledPackage};
use solx_scripts::{execute_script, CommandRunner};
use solx_surface::entities::ActionInput;
use solx_surface::error::{Result, SolxError};
use solx_surface::managers::ActionManager;
use solx_surface::query::ListOptions;

/// What an install needs from the action store: the refs of the actions
/// under a package's path, and setting their lock. Implemented for a host's
/// `Arc<dyn ActionManager>` (an operator path, which is what may lock and
/// unlock).
#[async_trait]
pub trait PackageActions: Send + Sync {
    /// `(path, name)` of every action whose path starts with `base` — a
    /// string prefix, like `ListOptions::path_prefix`; the install keeps
    /// only those at `base` or below it.
    async fn actions_under(&self, base: &str) -> Result<Vec<(String, String)>>;
    async fn set_locked(&self, path: &str, name: &str, locked: bool) -> Result<()>;
    /// Whether the action is keyed: it has scoped secrets or an `auth`
    /// keyring reference (see [`solx_surface::is_keyed`]). Such an action is
    /// locked even when the manifest lists it under `unlocked_actions`:
    /// unlocked, its artifact could be replaced by content and run with its
    /// credential.
    async fn is_keyed(&self, path: &str, name: &str) -> Result<bool>;
}

#[async_trait]
impl PackageActions for Arc<dyn ActionManager> {
    async fn actions_under(&self, base: &str) -> Result<Vec<(String, String)>> {
        const PAGE: usize = 500;
        let mut out = Vec::new();
        loop {
            let page = self
                .list(ListOptions {
                    path_prefix: Some(base.to_string()),
                    limit: Some(PAGE),
                    offset: Some(out.len()),
                    ..Default::default()
                })
                .await?;
            let n = page.items.len();
            out.extend(page.items.into_iter().map(|a| (a.path, a.name)));
            if n < PAGE {
                break;
            }
        }
        Ok(out)
    }

    async fn set_locked(&self, path: &str, name: &str, locked: bool) -> Result<()> {
        self.save(
            path,
            name,
            ActionInput {
                locked: Some(locked),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
    }

    async fn is_keyed(&self, path: &str, name: &str) -> Result<bool> {
        let action = self.get(path, name).await?;
        Ok(solx_surface::is_keyed(action.action_config.as_ref()))
    }
}

/// `install.solx` failed after the install unlocked the package's actions.
/// Lock again whatever the previous install had locked, so a failed
/// reinstall doesn't leave them open to content. Best effort: a lock that
/// fails (its artifact is gone, say) is added to the returned error.
async fn relock_after_failure(
    actions: &dyn PackageActions,
    config: &ConfigService,
    name: &str,
    err: SolxError,
) -> SolxError {
    let previous = config
        .list_packages()
        .into_iter()
        .find(|p| p.name == name)
        .map(|p| p.locked_actions)
        .unwrap_or_default();
    // Only what still exists: `set_locked` saves, and a save creates.
    let present = match own_actions(actions, &package_actions_base(name)).await {
        Ok(present) => present,
        Err(e) => return SolxError::Other(format!("{err}; also failed to re-lock: {e}")),
    };
    let mut failed = Vec::new();
    for full in previous {
        let Ok((path, action)) = solx_surface::path::split_ref(&full) else {
            continue;
        };
        if !present.iter().any(|(p, n)| *p == path && *n == action) {
            continue;
        }
        if let Err(e) = actions.set_locked(&path, &action, true).await {
            failed.push(format!("{full}: {e}"));
        }
    }
    if failed.is_empty() {
        err
    } else {
        SolxError::Other(format!(
            "{err}; also failed to re-lock: {}",
            failed.join("; ")
        ))
    }
}

/// Where a package's actions live.
fn package_actions_base(name: &str) -> String {
    format!("/packages/{name}")
}

tokio::task_local! {
    /// The package whose `install.solx` or `uninstall.solx` is running.
    static RUNNING_PACKAGE: String;
}

/// Refuse, while a package's script runs, a save or delete of an action at
/// `path` outside that package's `/packages/<name>/`. A no-op outside a
/// package script. Every operator path a script can reach calls this (the
/// `save action` and `delete action` commands); the `/builtin/entity/*`
/// actions are content paths and already refuse scoped secrets. See the
/// module doc.
pub fn guard_package_script_action(path: &str) -> Result<()> {
    let Ok(package) = RUNNING_PACKAGE.try_with(String::clone) else {
        return Ok(());
    };
    let base = package_actions_base(&package);
    let path = solx_surface::path::normalize_path(path)?;
    if path == base || path.starts_with(&format!("{base}/")) {
        return Ok(());
    }
    Err(SolxError::Unauthorized(format!(
        "package '{package}' may only save or delete actions under {base}/, not in {path}"
    )))
}

/// Refuse to install or uninstall a package from inside a package script.
fn refuse_in_package_script(verb: &str) -> Result<()> {
    match RUNNING_PACKAGE.try_with(String::clone) {
        Ok(package) => Err(SolxError::Unauthorized(format!(
            "package '{package}' can't {verb} a package from its scripts"
        ))),
        Err(_) => Ok(()),
    }
}

/// The actions at `base` or below it. `actions_under` matches a string
/// prefix, which would also take `/packages/ab` for `/packages/a`.
async fn own_actions(actions: &dyn PackageActions, base: &str) -> Result<Vec<(String, String)>> {
    let mut out = actions.actions_under(base).await?;
    out.retain(|(path, _)| path == base || path.starts_with(&format!("{base}/")));
    Ok(out)
}

/// [`install_package`]'s result: the recorded package plus any
/// allowlist-collision warnings. Warnings never block the install —
/// see the module doc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOutcome {
    #[serde(flatten)]
    pub package: InstalledPackage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// The manifest filename, and the npm-owned name still accepted as a
/// fallback so packages written before the split keep installing.
const MANIFEST: &str = "solx-package.json";
const MANIFEST_LEGACY: &str = "package.json";

/// A manifest's allowlist declarations, parsed once and reused for both
/// granting (install) and the `InstalledPackage.granted_*` record.
struct Grants {
    commands: HashMap<String, CommandDef>,
    base_urls: Vec<String>,
}

fn parse_grants(meta: &serde_json::Value, file: &str) -> Result<Grants> {
    let commands: HashMap<String, CommandDef> = match meta.get("command_actions") {
        Some(v) if !v.is_null() => serde_json::from_value(v.clone())
            .map_err(|e| SolxError::Invalid(format!("{file} 'command_actions': {e}")))?,
        _ => HashMap::new(),
    };
    // `allowed_webhook_base_urls` is the former spelling, still read for the
    // same reason the config key is — see `solx_config`'s `merged_base_urls`.
    let (key, raw) = match meta.get("allowed_base_urls") {
        Some(v) if !v.is_null() => ("allowed_base_urls", Some(v)),
        _ => match meta.get("allowed_webhook_base_urls") {
            Some(v) if !v.is_null() => ("allowed_webhook_base_urls", Some(v)),
            _ => ("allowed_base_urls", None),
        },
    };
    let base_urls: Vec<String> = match raw {
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| SolxError::Invalid(format!("{file} '{key}': {e}")))?,
        None => Vec::new(),
    };
    Ok(Grants {
        commands,
        base_urls,
    })
}

/// Locate the package manifest, preferring [`MANIFEST`]. Returns the path,
/// its filename (for error messages), and whether the legacy name was used.
fn manifest_path(dir: &Path) -> (std::path::PathBuf, &'static str, bool) {
    let current = dir.join(MANIFEST);
    if current.exists() {
        (current, MANIFEST, false)
    } else {
        (dir.join(MANIFEST_LEGACY), MANIFEST_LEGACY, true)
    }
}

/// Install the package at `dir`: grant its declared allowlist entries, run
/// `install.solx`, lock its actions, then register it.
pub async fn install_package(
    runner: &dyn CommandRunner,
    actions: &dyn PackageActions,
    config: &ConfigService,
    dir: &Path,
) -> Result<InstallOutcome> {
    refuse_in_package_script("install")?;
    let (meta_path, meta_file, legacy_manifest) = manifest_path(dir);
    let install_path = dir.join("install.solx");

    let meta_str = std::fs::read_to_string(&meta_path)
        .map_err(|e| SolxError::Io(format!("read {}: {e}", meta_path.display())))?;
    let meta: serde_json::Value = serde_json::from_str(&meta_str)?;
    let name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SolxError::Invalid(format!("{meta_file} missing 'name'")))?
        .to_string();
    // The name is one segment of its actions' path, and so of its secret
    // scope — see the module doc.
    if !is_dir_safe(&name) {
        return Err(SolxError::Invalid(format!(
            "package name '{name}' must be letters, digits, '-', '_' and '.' only, \
             and not start with '.'"
        )));
    }
    let version = meta
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();
    let grants = parse_grants(&meta, meta_file)?;
    let unlocked: Vec<String> = match meta.get("unlocked_actions") {
        Some(v) if !v.is_null() => serde_json::from_value(v.clone())
            .map_err(|e| SolxError::Invalid(format!("{meta_file} 'unlocked_actions': {e}")))?,
        _ => Vec::new(),
    };
    let pkg_dir = package_dir(dir);

    let mut warnings = Vec::new();
    if legacy_manifest {
        let msg = format!(
            concat!(
                "package '{name}' has no {manifest}; read its manifest from ",
                "{legacy} instead. Rename it: {legacy} is npm's file, and ",
                "solx will stop falling back to it."
            ),
            name = name,
            manifest = MANIFEST,
            legacy = MANIFEST_LEGACY
        );
        tracing::warn!("{msg}");
        warnings.push(msg);
    }

    // Grant before running the script, so install.solx (or a later
    // verify.solx) can exec the package's own Command/Webhook actions.
    // Collisions with a *different* installed package are warned about,
    // not fatal — see the module doc.
    let others: Vec<InstalledPackage> = config
        .list_packages()
        .into_iter()
        .filter(|p| p.name != name)
        .collect();

    for (key, def) in &grants.commands {
        if let Some(owner) = others
            .iter()
            .find(|p| p.granted_commands.iter().any(|k| k == key))
        {
            let msg = format!(
                "command key '{key}' was granted by package '{}'; now overwritten by '{name}'",
                owner.name
            );
            tracing::warn!("{msg}");
            warnings.push(msg);
        }
        config.register_command(key, resolve_cwd(def, &pkg_dir))?;
    }
    for prefix in &grants.base_urls {
        if let Some(owner) = others
            .iter()
            .find(|p| p.granted_base_urls.iter().any(|p| p == prefix))
        {
            let msg = format!(
                "base URL prefix '{prefix}' was granted by package '{}'; now also granted to '{name}'",
                owner.name
            );
            tracing::warn!("{msg}");
            warnings.push(msg);
        }
        config.add_allowed_base_url(prefix)?;
    }

    let script = std::fs::read_to_string(&install_path)
        .map_err(|e| SolxError::Io(format!("read {}: {e}", install_path.display())))?;

    // Unlock what a previous install locked, so this one can replace it —
    // see the module doc. A keyed action stays locked: the
    // script replaces its artifact as the operator (`save file`) and its
    // `save action` re-pins it, so content never gets a window to swap in
    // code that would run with the package's key.
    let base = package_actions_base(&name);
    for (path, action) in own_actions(actions, &base).await? {
        if !actions.is_keyed(&path, &action).await? {
            actions.set_locked(&path, &action, false).await?;
        }
    }

    if let Err(e) = run_in_dir(runner, dir, &name, &script).await {
        return Err(relock_after_failure(actions, config, &name, e).await);
    }

    // Lock everything, even past a failure, so one missing artifact doesn't
    // leave the rest open to content.
    let mut locked_actions = Vec::new();
    let mut failed = Vec::new();
    for (path, action) in own_actions(actions, &base).await? {
        let full = format!("{path}/{action}");
        let relative = full.strip_prefix(&format!("{base}/")).unwrap_or(&full);
        if unlocked.iter().any(|u| u.trim_matches('/') == relative) {
            if !actions.is_keyed(&path, &action).await? {
                continue;
            }
            let msg = format!(
                "{full} is listed in unlocked_actions but has scoped secrets, declared \
                 scopes or a keyring reference; locked anyway"
            );
            tracing::warn!("{msg}");
            warnings.push(msg);
        }
        match actions.set_locked(&path, &action, true).await {
            Ok(()) => locked_actions.push(full),
            Err(e) => failed.push(format!("locking {full}: {e}")),
        }
    }
    if !failed.is_empty() {
        return Err(SolxError::Other(failed.join("; ")));
    }

    let pkg = InstalledPackage {
        name,
        version,
        path: pkg_dir.to_string_lossy().into_owned(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        granted_commands: grants.commands.into_keys().collect(),
        granted_base_urls: grants.base_urls,
        locked_actions,
    };
    config.register_package(pkg.clone())?;
    Ok(InstallOutcome {
        package: pkg,
        warnings,
    })
}

/// Uninstall `name`: run `uninstall.solx` (if present), revoke whatever this
/// package's install granted (unless another installed package still
/// declares the same key/prefix), then unregister.
pub async fn uninstall_package(
    runner: &dyn CommandRunner,
    config: &ConfigService,
    name: &str,
) -> Result<()> {
    refuse_in_package_script("uninstall")?;
    let pkg = config
        .list_packages()
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| SolxError::NotFound(format!("package '{name}'")))?;

    let dir = Path::new(&pkg.path);
    let uninstall_path = dir.join("uninstall.solx");
    if uninstall_path.exists() {
        let script = std::fs::read_to_string(&uninstall_path)
            .map_err(|e| SolxError::Io(format!("read {}: {e}", uninstall_path.display())))?;
        run_in_dir(runner, dir, &pkg.name, &script).await?;
    }

    // Revoke after the script ran (symmetric with granting before install.solx
    // runs), and only what no other installed package still claims.
    let others: Vec<InstalledPackage> = config
        .list_packages()
        .into_iter()
        .filter(|p| p.name != name)
        .collect();
    for key in &pkg.granted_commands {
        if !others
            .iter()
            .any(|p| p.granted_commands.iter().any(|k| k == key))
        {
            config.deregister_command(key)?;
        }
    }
    for prefix in &pkg.granted_base_urls {
        if !others
            .iter()
            .any(|p| p.granted_base_urls.iter().any(|p| p == prefix))
        {
            config.remove_allowed_base_url(prefix)?;
        }
    }

    config.unregister_package(name)?;

    // A zip install put the files there for solx to own; a directory
    // installed in place belongs to whoever pointed us at it.
    if is_within(dir, &config.packages_dir()) {
        if let Err(e) = std::fs::remove_dir_all(dir) {
            tracing::warn!(
                "uninstalled '{name}' but could not delete {}: {e}",
                dir.display()
            );
        }
    }
    Ok(())
}

/// Install a package from the zip file at `zip`. See the module doc.
pub async fn install_package_zip(
    runner: &dyn CommandRunner,
    actions: &dyn PackageActions,
    config: &ConfigService,
    zip: &Path,
) -> Result<InstallOutcome> {
    let bytes =
        std::fs::read(zip).map_err(|e| SolxError::Io(format!("read {}: {e}", zip.display())))?;
    install_package_zip_bytes(runner, actions, config, bytes).await
}

/// Install a package from the bytes of a zip. See the module doc.
pub async fn install_package_zip_bytes(
    runner: &dyn CommandRunner,
    actions: &dyn PackageActions,
    config: &ConfigService,
    bytes: Vec<u8>,
) -> Result<InstallOutcome> {
    // Before unpacking, which replaces the target package's directory.
    refuse_in_package_script("install")?;
    let packages = config.packages_dir();
    std::fs::create_dir_all(&packages)
        .map_err(|e| SolxError::Io(format!("create {}: {e}", packages.display())))?;

    // Unpack beside the destination so the final move is a rename.
    let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let staging = packages.join(format!(".staging-{stamp}"));
    let result = install_from_staging(runner, actions, config, &packages, &staging, bytes, stamp).await;
    let _ = std::fs::remove_dir_all(&staging);
    result
}

async fn install_from_staging(
    runner: &dyn CommandRunner,
    actions: &dyn PackageActions,
    config: &ConfigService,
    packages: &Path,
    staging: &Path,
    bytes: Vec<u8>,
    stamp: i64,
) -> Result<InstallOutcome> {
    extract_zip(&bytes, staging)?;
    let root = find_package_root(staging)?;
    let (meta_path, meta_file, _) = manifest_path(&root);
    let meta: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&meta_path)
            .map_err(|e| SolxError::Io(format!("read {}: {e}", meta_path.display())))?,
    )?;
    let name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SolxError::Invalid(format!("{meta_file} missing 'name'")))?;
    if !is_dir_safe(name) {
        return Err(SolxError::Invalid(format!(
            "package name '{name}' can't be used as a directory name \
             (letters, digits, '-', '_' and '.' only)"
        )));
    }

    let target = packages.join(name);
    let backup = packages.join(format!(".backup-{stamp}"));
    let had_old = target.exists();
    if had_old {
        std::fs::rename(&target, &backup).map_err(|e| {
            SolxError::Io(format!(
                "replace {}: {e} (is something in it still running?)",
                target.display()
            ))
        })?;
    }
    let restore = || {
        let _ = std::fs::remove_dir_all(&target);
        if had_old {
            let _ = std::fs::rename(&backup, &target);
        }
    };
    if let Err(e) = std::fs::rename(&root, &target) {
        restore();
        return Err(SolxError::Io(format!(
            "move into {}: {e}",
            target.display()
        )));
    }
    match install_package(runner, actions, config, &target).await {
        Ok(outcome) => {
            if had_old {
                let _ = std::fs::remove_dir_all(&backup);
            }
            Ok(outcome)
        }
        Err(e) => {
            restore();
            Err(e)
        }
    }
}

/// Unpack `bytes` into `dest`, refusing entries that would land outside it.
fn extract_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| SolxError::Invalid(format!("not a valid zip file: {e}")))?;
    std::fs::create_dir_all(dest)
        .map_err(|e| SolxError::Io(format!("create {}: {e}", dest.display())))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| SolxError::Invalid(format!("read zip entry {i}: {e}")))?;
        let rel = entry.enclosed_name().ok_or_else(|| {
            SolxError::Invalid(format!(
                "zip entry '{}' escapes the package directory",
                entry.name()
            ))
        })?;
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| SolxError::Io(format!("create {}: {e}", out.display())))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SolxError::Io(format!("create {}: {e}", parent.display())))?;
        }
        let mut file = std::fs::File::create(&out)
            .map_err(|e| SolxError::Io(format!("create {}: {e}", out.display())))?;
        std::io::copy(&mut entry, &mut file)
            .map_err(|e| SolxError::Io(format!("write {}: {e}", out.display())))?;
    }
    Ok(())
}

fn has_manifest(dir: &Path) -> bool {
    dir.join(MANIFEST).exists() || dir.join(MANIFEST_LEGACY).exists()
}

/// Whether `dir` looks like a package among siblings. A bare `package.json`
/// is npm's, so it only counts next to an `install.solx`.
fn is_package_dir(dir: &Path) -> bool {
    dir.is_dir()
        && (dir.join(MANIFEST).exists() || (has_manifest(dir) && dir.join("install.solx").exists()))
}

/// The directory under `dir` that holds the package manifest: `dir` itself,
/// or the one child directory that has a manifest.
fn find_package_root(dir: &Path) -> Result<PathBuf> {
    if has_manifest(dir) {
        return Ok(dir.to_path_buf());
    }
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| SolxError::Io(format!("read {}: {e}", dir.display())))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    // Build bundles carry siblings next to the package (a path dependency
    // like `../solx-widgets`, needed to build it, not to run it); only the
    // directory with the manifest is the package.
    let packages: Vec<&PathBuf> = entries.iter().filter(|p| is_package_dir(p)).collect();
    match packages.as_slice() {
        [one] => return Ok((*one).clone()),
        [] => {}
        several => {
            let names: Vec<_> = several
                .iter()
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect();
            return Err(SolxError::Invalid(format!(
                "the zip holds several packages ({}); install them one at a time",
                names.join(", ")
            )));
        }
    }
    Err(SolxError::Invalid(format!(
        "the zip has no {MANIFEST} at its root or in a top-level folder"
    )))
}

fn is_dir_safe(name: &str) -> bool {
    !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Whether `path` is strictly inside `parent` (both must exist).
fn is_within(path: &Path, parent: &Path) -> bool {
    match (path.canonicalize(), parent.canonicalize()) {
        (Ok(p), Ok(base)) => p != base && p.starts_with(base),
        _ => false,
    }
}

/// List installed packages.
pub fn list_packages(config: &ConfigService) -> Vec<InstalledPackage> {
    config.list_packages()
}

/// `dir` canonicalized, without Windows' `\\?\` verbatim prefix — it is
/// recorded as the package's path and joined into command `cwd`s, where the
/// verbatim form is noise at best and rejected by some tools at worst.
fn package_dir(dir: &Path) -> PathBuf {
    let path = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let s = path.to_string_lossy().into_owned();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

/// `def` with a relative `cwd` joined onto the package directory, so a
/// manifest can say `"cwd": "bin"` rather than hard-coding where the package
/// happens to be checked out. `.` components are dropped, so `"cwd": "."`
/// records the package directory itself.
fn resolve_cwd(def: &CommandDef, pkg_dir: &Path) -> CommandDef {
    let mut def = def.clone();
    if let Some(cwd) = def.cwd.as_deref().filter(|c| Path::new(c).is_relative()) {
        let joined: PathBuf = pkg_dir
            .join(cwd)
            .components()
            .filter(|c| !matches!(c, Component::CurDir))
            .collect();
        def.cwd = Some(joined.to_string_lossy().into_owned());
    }
    def
}

/// Run a package script with the process CWD temporarily set to the package
/// directory (so relative paths in the script resolve), restoring it after.
/// Run `package`'s `script` in `dir`, confined to that package's actions
/// (see [`guard_package_script_action`]).
async fn run_in_dir(
    runner: &dyn CommandRunner,
    dir: &Path,
    package: &str,
    script: &str,
) -> Result<()> {
    let original = std::env::current_dir().map_err(|e| SolxError::Io(format!("read cwd: {e}")))?;
    std::env::set_current_dir(dir)
        .map_err(|e| SolxError::Io(format!("set cwd to {}: {e}", dir.display())))?;
    let result = RUNNING_PACKAGE
        .scope(package.to_string(), execute_script(runner, script))
        .await;
    let restore = std::env::set_current_dir(&original)
        .map_err(|e| SolxError::Io(format!("restore cwd: {e}")));
    result?;
    restore?;
    Ok(())
}
