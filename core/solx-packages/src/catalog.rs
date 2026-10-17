//! The official package catalog: the packages built and released from
//! `praeus/solx-packages`, and installing them by name from a release.
//!
//! [`CATALOG`] is the curated list a UI shows (and the only names
//! [`install_from_catalog`] accepts — a caller never hands over a URL). The
//! zips themselves come from a [`PackageSource`]: by default the latest
//! published GitHub release, or a local folder of zips for testing before a
//! release exists (`SOLX_PACKAGES_SOURCE=<dir>`).
//!
//! Release zips are named by `scripts/package.mjs` in that repo:
//! `<package>-<version>.zip` for wasm/node/script packages and
//! `<package>-<version>-<platform>.zip` for packages with a native binary,
//! one per [`PLATFORMS`] entry. A release also carries `SHA256SUMS.txt`,
//! which [`fetch`] checks downloads against.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};
use solx_config::ConfigService;
use solx_scripts::CommandRunner;
use solx_surface::error::{Result, SolxError};

use crate::{install_package_zip_bytes, InstallOutcome, PackageActions, MANIFEST};

/// One package the catalog offers.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Shown first: many other packages build on these.
    pub recommended: bool,
    /// Packages this one needs installed first (its `install.solx` or setup
    /// runs their actions). Installed automatically by [`install_from_catalog`].
    pub requires: &'static [&'static str],
    /// Ships a native binary, so there is one zip per platform.
    pub native: bool,
}

const fn entry(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    recommended: bool,
    requires: &'static [&'static str],
    native: bool,
) -> CatalogEntry {
    CatalogEntry {
        name,
        title,
        description,
        recommended,
        requires,
        native,
    }
}

/// The official packages, recommended ones first.
pub const CATALOG: &[CatalogEntry] = &[
    entry(
        "solx-ollama",
        "Ollama",
        "Chat, generate and embed with a local Ollama server.",
        true,
        &[],
        false,
    ),
    entry(
        "solx-firefox",
        "Firefox",
        "A dedicated, automatable Firefox instance for browsing actions.",
        true,
        &["solx-mcp-actions"],
        true,
    ),
    entry(
        "solx-quickjs",
        "QuickJS",
        "Builds JavaScript actions into wasm components.",
        true,
        &[],
        true,
    ),
    entry(
        "solx-mcp-actions",
        "MCP actions",
        "Import tools from MCP servers as solx actions.",
        true,
        &[],
        true,
    ),
    entry(
        "solx-names",
        "Names",
        "Random name generator actions.",
        true,
        &[],
        false,
    ),
    entry(
        "solx-agent",
        "Agent",
        "A supervised agent thread over the solx action catalogue.",
        false,
        &[],
        false,
    ),
    entry(
        "solx-inquiry",
        "Inquiry",
        "Ask questions answered from your solx documents.",
        false,
        &[],
        false,
    ),
    entry(
        "solx-git",
        "Git",
        "Clone, branch, patch and commit git repositories.",
        false,
        &[],
        true,
    ),
    entry(
        "solx-media",
        "Media",
        "Image, audio and video extraction via Ollama, ffmpeg and whisper.",
        false,
        &[],
        true,
    ),
    entry(
        "solx-omniparse",
        "Omniparse",
        "Document parsing and OCR for extraction.",
        false,
        &[],
        true,
    ),
    entry(
        "solx-google",
        "Google",
        "Docs, Drive, Gmail, Tasks and Calendar actions.",
        false,
        &[],
        false,
    ),
    entry(
        "solx-facebook",
        "Facebook",
        "Facebook Pages post actions over the Graph API.",
        false,
        &[],
        false,
    ),
    entry(
        "solx-reddit",
        "Reddit",
        "Reddit extraction and browsing.",
        false,
        &["solx-quickjs"],
        false,
    ),
    entry(
        "solx-livejournal",
        "LiveJournal",
        "LiveJournal extraction.",
        false,
        &["solx-quickjs"],
        false,
    ),
];

/// The catalog entry named `name`.
pub fn catalog_entry(name: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.name == name)
}

/// Platform names used in release zip names (see `scripts/package.mjs`).
pub const PLATFORMS: &[&str] = &["linux-x64", "macos-arm64", "windows-x64"];

/// The release platform this build runs on; `None` where no native zips are
/// built (native packages are then unavailable).
pub const PLATFORM: Option<&str> = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some("windows-x64")
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("linux-x64")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("macos-arm64")
} else {
    None
};

pub const DEFAULT_REPO: &str = "praeus/solx-packages";

/// Where release zips come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageSource {
    /// A GitHub release of `repo` (`owner/name`); `tag` `None` is the latest
    /// published (non-draft, non-prerelease) release.
    Github { repo: String, tag: Option<String> },
    /// A folder of package zips, e.g. downloaded from a CI run.
    Dir(PathBuf),
}

impl Default for PackageSource {
    fn default() -> Self {
        PackageSource::Github {
            repo: DEFAULT_REPO.into(),
            tag: None,
        }
    }
}

impl PackageSource {
    /// `SOLX_PACKAGES_SOURCE` if set (see [`PackageSource::parse`]), else the
    /// latest release of [`DEFAULT_REPO`].
    pub fn from_env() -> Self {
        match std::env::var("SOLX_PACKAGES_SOURCE") {
            Ok(s) if !s.trim().is_empty() => Self::parse(&s),
            _ => Self::default(),
        }
    }

    /// `github:owner/repo[@tag]`, or anything else as a folder path.
    pub fn parse(spec: &str) -> Self {
        let spec = spec.trim();
        match spec.strip_prefix("github:") {
            Some(rest) => {
                let (repo, tag) = match rest.split_once('@') {
                    Some((r, t)) => (r, Some(t.to_string()).filter(|t| !t.is_empty())),
                    None => (rest, None),
                };
                PackageSource::Github {
                    repo: repo.to_string(),
                    tag,
                }
            }
            None => PackageSource::Dir(PathBuf::from(spec)),
        }
    }

    /// A short description for display.
    pub fn label(&self) -> String {
        match self {
            PackageSource::Github { repo, tag: None } => format!("{repo} (latest release)"),
            PackageSource::Github { repo, tag: Some(t) } => format!("{repo} @ {t}"),
            PackageSource::Dir(dir) => format!("local folder {}", dir.display()),
        }
    }
}

/// A downloadable package zip, already filtered to this platform.
#[derive(Debug, Clone, Serialize)]
pub struct Asset {
    pub name: String,
    pub version: String,
    pub platform: Option<String>,
    pub file_name: String,
    pub size: u64,
    #[serde(skip)]
    pub location: Location,
    #[serde(skip)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Url(String),
    File(PathBuf),
}

/// What a source currently offers.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Release {
    /// The release tag (GitHub sources).
    pub tag: Option<String>,
    /// Why nothing is offered, e.g. no release has been published yet.
    pub message: Option<String>,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// Split a release zip's file name into `(package, version, platform)`.
pub fn parse_asset_name(file: &str) -> Option<(String, String, Option<String>)> {
    let stem = file.strip_suffix(".zip")?;
    let (stem, platform) = match PLATFORMS.iter().find(|p| stem.ends_with(&format!("-{p}"))) {
        Some(p) => (&stem[..stem.len() - p.len() - 1], Some(p.to_string())),
        None => (stem, None),
    };
    let (name, version) = stem.rsplit_once('-')?;
    if name.is_empty() || !version.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some((name.to_string(), version.to_string(), platform))
}

/// Parse `sha256sum` output (`<hex>  <file>` or `<hex> *<file>`).
pub fn parse_sha256sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (hash, file) = line.trim().split_once(char::is_whitespace)?;
            let file = file.trim_start().trim_start_matches('*');
            (hash.len() == 64 && !file.is_empty())
                .then(|| (file.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

/// Whether an asset built for `platform` installs here.
fn fits(platform: Option<&str>) -> bool {
    platform.is_none() || platform == PLATFORM
}

/// Keep the highest version of each package.
fn keep_latest(assets: Vec<Asset>) -> Vec<Asset> {
    let mut out: Vec<Asset> = Vec::new();
    for a in assets {
        match out.iter_mut().find(|b| b.name == a.name) {
            Some(b) if version_newer(&a.version, &b.version) => *b = a,
            Some(_) => {}
            None => out.push(a),
        }
    }
    out
}

/// Whether version `a` is newer than `b`, comparing dot-separated numeric
/// parts (anything after a `-` or `+` is ignored).
pub fn version_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

const USER_AGENT: &str = concat!("solx/", env!("CARGO_PKG_VERSION"));
/// Larger than any package zip so far (~20 MB) by a wide margin.
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;
/// GitHub allows 60 unauthenticated API requests an hour; don't spend one
/// every time the settings panel opens.
const CACHE_TTL: Duration = Duration::from_secs(300);

fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| SolxError::Other(format!("http client: {e}")))
}

static CACHE: Mutex<Option<(Instant, PackageSource, Release)>> = Mutex::new(None);

/// [`resolve`], reusing a result less than a few minutes old for the same
/// source unless `refresh`.
pub async fn resolve_cached(source: &PackageSource, refresh: bool) -> Result<Release> {
    if !refresh {
        if let Some((at, src, rel)) = CACHE.lock().unwrap().as_ref() {
            if src == source && at.elapsed() < CACHE_TTL {
                return Ok(rel.clone());
            }
        }
    }
    let release = resolve(source).await?;
    *CACHE.lock().unwrap() = Some((Instant::now(), source.clone(), release.clone()));
    Ok(release)
}

/// What `source` offers for this platform.
pub async fn resolve(source: &PackageSource) -> Result<Release> {
    match source {
        PackageSource::Dir(dir) => resolve_dir(dir),
        PackageSource::Github { repo, tag } => resolve_github(repo, tag.as_deref()).await,
    }
}

fn resolve_dir(dir: &Path) -> Result<Release> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| SolxError::Io(format!("read {}: {e}", dir.display())))?;
    let mut assets = Vec::new();
    for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
        let Some(file) = path
            .file_name()
            .and_then(|f| f.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if !file.ends_with(".zip") || !path.is_file() {
            continue;
        }
        // CI artifacts aren't always named like release assets, so trust the
        // manifest inside for name and version, and the file name only for
        // which platform it was built for.
        let platform = PLATFORMS
            .iter()
            .find(|p| file.contains(*p))
            .map(|p| p.to_string());
        if !fits(platform.as_deref()) {
            continue;
        }
        let Some((name, version)) = zip_manifest(&path) else {
            tracing::warn!("skipping {}: no {MANIFEST} with a name", path.display());
            continue;
        };
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        assets.push(Asset {
            name,
            version,
            platform,
            file_name: file,
            size,
            location: Location::File(path),
            sha256: None,
        });
    }
    let message = assets
        .is_empty()
        .then(|| format!("no package zips for this platform in {}", dir.display()));
    Ok(Release {
        tag: None,
        message,
        assets: keep_latest(assets),
    })
}

/// `(name, version)` from the manifest at the root of a package zip or in
/// one of its top-level folders.
fn zip_manifest(path: &Path) -> Option<(String, String)> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let index = (0..archive.len()).find(|&i| {
        archive.name_for_index(i).is_some_and(|n| {
            let parts: Vec<&str> = n.split('/').collect();
            parts.last() == Some(&MANIFEST) && parts.len() <= 2
        })
    })?;
    let entry = archive.by_index(index).ok()?;
    let meta: serde_json::Value = serde_json::from_reader(entry).ok()?;
    let name = meta
        .get("name")?
        .as_str()
        .filter(|s| !s.is_empty())?
        .to_string();
    let version = meta
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();
    Some((name, version))
}

#[derive(serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(serde::Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

async fn resolve_github(repo: &str, tag: Option<&str>) -> Result<Release> {
    let url = match tag {
        Some(t) => format!("https://api.github.com/repos/{repo}/releases/tags/{t}"),
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    let client = http()?;
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| SolxError::Other(format!("can't reach GitHub: {e}")))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Release {
            tag: tag.map(str::to_string),
            message: Some(format!("{repo} has no published release yet")),
            assets: Vec::new(),
        });
    }
    if !resp.status().is_success() {
        return Err(SolxError::Other(format!(
            "GitHub returned {} for {url}",
            resp.status()
        )));
    }
    let rel: GhRelease = resp
        .json()
        .await
        .map_err(|e| SolxError::Other(format!("unexpected GitHub response: {e}")))?;

    let sums = match rel.assets.iter().find(|a| a.name == "SHA256SUMS.txt") {
        Some(a) => match fetch_url(&client, &a.browser_download_url).await {
            Ok(bytes) => parse_sha256sums(&String::from_utf8_lossy(&bytes)),
            Err(e) => {
                tracing::warn!("release {}: can't read SHA256SUMS.txt: {e}", rel.tag_name);
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let assets = rel
        .assets
        .into_iter()
        .filter_map(|a| {
            let (name, version, platform) = parse_asset_name(&a.name)?;
            fits(platform.as_deref()).then(|| Asset {
                sha256: sums
                    .iter()
                    .find(|(f, _)| *f == a.name)
                    .map(|(_, h)| h.clone()),
                name,
                version,
                platform,
                size: a.size,
                location: Location::Url(a.browser_download_url),
                file_name: a.name,
            })
        })
        .collect();
    Ok(Release {
        tag: Some(rel.tag_name),
        message: None,
        assets: keep_latest(assets),
    })
}

async fn fetch_url(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| SolxError::Other(format!("download {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(SolxError::Other(format!(
            "download {url}: HTTP {}",
            resp.status()
        )));
    }
    if resp.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
        return Err(SolxError::Invalid(format!("download {url}: too large")));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| SolxError::Other(format!("download {url}: {e}")))?
    {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > MAX_DOWNLOAD {
            return Err(SolxError::Invalid(format!("download {url}: too large")));
        }
    }
    Ok(bytes)
}

/// The zip bytes of `asset`, checked against its published sha256 if any.
pub async fn fetch(asset: &Asset) -> Result<Vec<u8>> {
    let bytes = match &asset.location {
        Location::File(path) => std::fs::read(path)
            .map_err(|e| SolxError::Io(format!("read {}: {e}", path.display())))?,
        Location::Url(url) => fetch_url(&http()?, url).await?,
    };
    if let Some(expected) = &asset.sha256 {
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if &actual != expected {
            return Err(SolxError::Invalid(format!(
                "{} failed its checksum (expected {expected}, got {actual})",
                asset.file_name
            )));
        }
    }
    Ok(bytes)
}

/// Install catalog package `name` from `release`, installing any of its
/// [`CatalogEntry::requires`] that aren't installed yet first. The returned
/// warnings include those of the requirements' installs.
pub async fn install_from_catalog(
    runner: &dyn CommandRunner,
    actions: &dyn PackageActions,
    config: &ConfigService,
    release: &Release,
    name: &str,
) -> Result<InstallOutcome> {
    let mut warnings = Vec::new();
    let outcome =
        install_with_requires(runner, actions, config, release, name, &mut warnings, 0).await?;
    Ok(InstallOutcome {
        package: outcome.package,
        warnings,
    })
}

async fn install_with_requires(
    runner: &dyn CommandRunner,
    actions: &dyn PackageActions,
    config: &ConfigService,
    release: &Release,
    name: &str,
    warnings: &mut Vec<String>,
    depth: usize,
) -> Result<InstallOutcome> {
    let entry = catalog_entry(name)
        .ok_or_else(|| SolxError::Invalid(format!("'{name}' is not in the package catalog")))?;
    if depth > CATALOG.len() {
        return Err(SolxError::Invalid(format!("'{name}' requires itself")));
    }
    for req in entry.requires {
        if !config.list_packages().iter().any(|p| p.name == *req) {
            let out = Box::pin(install_with_requires(
                runner,
                actions,
                config,
                release,
                req,
                warnings,
                depth + 1,
            ))
            .await
            .map_err(|e| SolxError::Other(format!("installing {req} (required by {name}): {e}")))?;
            warnings.push(format!(
                "installed {} {} (required by {name})",
                out.package.name, out.package.version
            ));
        }
    }
    let asset = release.asset(name).ok_or_else(|| {
        SolxError::NotFound(match release.message.as_deref() {
            Some(m) => format!("{name}: {m}"),
            None => format!(
                "{name} is not in this release for {}",
                PLATFORM.unwrap_or("this platform")
            ),
        })
    })?;
    let bytes = fetch(asset).await?;
    let outcome = install_package_zip_bytes(runner, actions, config, bytes).await?;
    if outcome.package.name != name {
        warnings.push(format!(
            "{} installed as '{}', not '{name}'",
            asset.file_name, outcome.package.name
        ));
    }
    warnings.extend(outcome.warnings.iter().cloned());
    Ok(outcome)
}

/// Installed catalog packages that require `name`.
pub fn dependents(config: &ConfigService, name: &str) -> Vec<&'static str> {
    let installed = config.list_packages();
    CATALOG
        .iter()
        .filter(|e| e.requires.contains(&name) && installed.iter().any(|p| p.name == e.name))
        .map(|e| e.name)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    Installed,
    UpdateAvailable,
    Available,
    NotReleased,
    UnsupportedPlatform,
}

/// A catalog entry with what's installed and what the source offers.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntryView {
    #[serde(flatten)]
    pub entry: CatalogEntry,
    pub state: EntryState,
    pub installed_version: Option<String>,
    pub available_version: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogView {
    pub source: String,
    pub platform: Option<&'static str>,
    pub tag: Option<String>,
    pub message: Option<String>,
    pub entries: Vec<CatalogEntryView>,
}

/// [`CATALOG`] joined with the installed packages and `release`.
pub fn catalog_view(
    config: &ConfigService,
    source: &PackageSource,
    release: &Release,
) -> CatalogView {
    let installed = config.list_packages();
    let entries = CATALOG
        .iter()
        .map(|e| {
            let inst = installed.iter().find(|p| p.name == e.name);
            let asset = release.asset(e.name);
            let state = match (inst, asset) {
                (Some(p), Some(a)) if version_newer(&a.version, &p.version) => {
                    EntryState::UpdateAvailable
                }
                (Some(_), _) => EntryState::Installed,
                (None, Some(_)) => EntryState::Available,
                (None, None) if e.native && PLATFORM.is_none() => EntryState::UnsupportedPlatform,
                (None, None) => EntryState::NotReleased,
            };
            CatalogEntryView {
                entry: e.clone(),
                state,
                installed_version: inst.map(|p| p.version.clone()),
                available_version: asset.map(|a| a.version.clone()),
                size: asset.map(|a| a.size),
            }
        })
        .collect();
    CatalogView {
        source: source.label(),
        platform: PLATFORM,
        tag: release.tag.clone(),
        message: release.message.clone(),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_asset_names() {
        assert_eq!(
            parse_asset_name("solx-ollama-0.1.0.zip"),
            Some(("solx-ollama".into(), "0.1.0".into(), None))
        );
        assert_eq!(
            parse_asset_name("solx-mcp-actions-0.1.0-windows-x64.zip"),
            Some((
                "solx-mcp-actions".into(),
                "0.1.0".into(),
                Some("windows-x64".into())
            ))
        );
        assert_eq!(parse_asset_name("SHA256SUMS.txt"), None);
        assert_eq!(parse_asset_name("solx-ollama.zip"), None);
    }

    #[test]
    fn parses_sha256sums() {
        let h = "a".repeat(64);
        let text = format!("{h}  solx-names-0.1.0.zip\n{h} *solx-git-0.1.0-linux-x64.zip\njunk\n");
        let sums = parse_sha256sums(&text);
        assert_eq!(sums.len(), 2);
        assert_eq!(sums[0], ("solx-names-0.1.0.zip".into(), h.clone()));
        assert_eq!(sums[1].0, "solx-git-0.1.0-linux-x64.zip");
    }

    #[test]
    fn compares_versions() {
        assert!(version_newer("0.2.0", "0.1.9"));
        assert!(version_newer("1.0.0", "0.10.0"));
        assert!(!version_newer("0.1.0", "0.1.0"));
        assert!(!version_newer("0.1.0-beta", "0.1.0"));
    }

    #[test]
    fn parses_sources() {
        assert_eq!(
            PackageSource::parse("github:me/pkgs@v1"),
            PackageSource::Github {
                repo: "me/pkgs".into(),
                tag: Some("v1".into())
            }
        );
        assert_eq!(
            PackageSource::parse("C:\\zips"),
            PackageSource::Dir("C:\\zips".into())
        );
    }

    #[test]
    fn catalog_is_consistent() {
        for e in CATALOG {
            for r in e.requires {
                assert!(
                    catalog_entry(r).is_some(),
                    "{} requires unknown {r}",
                    e.name
                );
            }
        }
        let recommended: Vec<_> = CATALOG
            .iter()
            .filter(|e| e.recommended)
            .map(|e| e.name)
            .collect();
        assert_eq!(
            recommended,
            [
                "solx-ollama",
                "solx-firefox",
                "solx-quickjs",
                "solx-mcp-actions",
                "solx-names"
            ]
        );
    }
}
