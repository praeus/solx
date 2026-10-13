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

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use solx_config::{CommandDef, ConfigService, InstalledPackage};
use solx_scripts::{execute_script, CommandRunner};
use solx_surface::error::{Result, SolxError};

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
/// `install.solx`, then register it.
pub async fn install_package(
    runner: &dyn CommandRunner,
    config: &ConfigService,
    dir: &Path,
) -> Result<InstallOutcome> {
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
    let version = meta
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();
    let grants = parse_grants(&meta, meta_file)?;
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

    run_in_dir(runner, dir, &script).await?;

    let pkg = InstalledPackage {
        name,
        version,
        path: pkg_dir.to_string_lossy().into_owned(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        granted_commands: grants.commands.into_keys().collect(),
        granted_base_urls: grants.base_urls,
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
        run_in_dir(runner, dir, &script).await?;
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
    config: &ConfigService,
    zip: &Path,
) -> Result<InstallOutcome> {
    let bytes =
        std::fs::read(zip).map_err(|e| SolxError::Io(format!("read {}: {e}", zip.display())))?;
    install_package_zip_bytes(runner, config, bytes).await
}

/// Install a package from the bytes of a zip. See the module doc.
pub async fn install_package_zip_bytes(
    runner: &dyn CommandRunner,
    config: &ConfigService,
    bytes: Vec<u8>,
) -> Result<InstallOutcome> {
    let packages = config.packages_dir();
    std::fs::create_dir_all(&packages)
        .map_err(|e| SolxError::Io(format!("create {}: {e}", packages.display())))?;

    // Unpack beside the destination so the final move is a rename.
    let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let staging = packages.join(format!(".staging-{stamp}"));
    let result = install_from_staging(runner, config, &packages, &staging, bytes, stamp).await;
    let _ = std::fs::remove_dir_all(&staging);
    result
}

async fn install_from_staging(
    runner: &dyn CommandRunner,
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
    match install_package(runner, config, &target).await {
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
async fn run_in_dir(runner: &dyn CommandRunner, dir: &Path, script: &str) -> Result<()> {
    let original = std::env::current_dir().map_err(|e| SolxError::Io(format!("read cwd: {e}")))?;
    std::env::set_current_dir(dir)
        .map_err(|e| SolxError::Io(format!("set cwd to {}: {e}", dir.display())))?;
    let result = execute_script(runner, script).await;
    let restore = std::env::set_current_dir(&original)
        .map_err(|e| SolxError::Io(format!("restore cwd: {e}")));
    result?;
    restore?;
    Ok(())
}
