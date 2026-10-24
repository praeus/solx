//! A local certificate authority for `solx-server` TLS.
//!
//! One certificates directory (by default `<appdata>/certs`) holds one root
//! CA and the server certificate it signed. Clients trust (and later pin)
//! the root, never the server certificate, so the server certificate can be
//! reissued whenever its names change or it nears expiry without touching
//! any client. See `docs/tls-plan.md`.
//!
//! Keys live in a backend:
//! - **CNG** (Windows): persisted, non-exportable CNG keys, on the TPM
//!   (Microsoft Platform Crypto Provider) when there is one, else the
//!   Microsoft Software Key Storage Provider. Both certificates are also
//!   installed in `CurrentUser\My`, bound to their keys, so `solx-server`
//!   can serve from the store with no private key on disk.
//! - **File** (everywhere): PKCS#8 PEM files in the directory. The root key
//!   is encrypted under a key kept in the OS credential manager when one is
//!   available. Any process running as the user can still read these keys.
//!
//! Whatever the backend, the directory always gets the public files clients
//! need: `solx-root.crt`, and `solx-server.crt` (the server certificate
//! followed by the root). `tls.json` records the rest.

mod backend;
#[cfg(all(windows, feature = "cng"))]
mod cng;
mod file;
mod provision;
pub mod state;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use rcgen::string::Ia5String;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyUsagePurpose, SanType, SerialNumber,
};
use rustls_pki_types::CertificateDer;
use sha2::{Digest, Sha256};

pub use backend::KeyReport;
use backend::{CaKey, KeyBackend, KeyRole};
pub use file::FileProtection;
pub use provision::{certs_dir, provision, wanted_sans};
pub use state::{BackendKind, CertInfo, KeyRef, TlsState};

/// Root certificates are valid for ten years.
pub const ROOT_VALIDITY_DAYS: i64 = 3650;
/// Server certificates are valid for 397 days, the longest that browsers
/// accept for a publicly trusted certificate. A local root isn't held to
/// that, but there's no reason to differ.
pub const SERVER_VALIDITY_DAYS: i64 = 397;
/// [`Authority::ensure_server`] reissues a server certificate with fewer
/// days than this left.
pub const RENEW_WITHIN_DAYS: i64 = 30;

/// Credential-manager service the file backend's wrapping key is kept
/// under, alongside solx's tokens.
pub const KEYRING_SERVICE: &str = "solx";

// ── errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("building a certificate failed: {0}")]
    Cert(#[from] rcgen::Error),
    /// A failure in the OS key store or certificate store.
    #[error("{0}")]
    Platform(String),
    /// Bad input: a malformed name, an unsupported option.
    #[error("{0}")]
    Invalid(String),
    /// The directory's state doesn't allow the operation (no CA yet, a CA
    /// already, a missing key).
    #[error("{0}")]
    State(String),
}

impl Error {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ── names ────────────────────────────────────────────────────────────────────

/// A subject alternative name on a server certificate.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum San {
    Dns(String),
    Ip(IpAddr),
}

impl FromStr for San {
    type Err = Error;

    /// An IP address (`192.168.1.10`, `::1`, `[::1]`), else a DNS name,
    /// lowercased. A leading `*.` wildcard is allowed.
    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        let unbracketed = s
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .unwrap_or(s);
        if let Ok(ip) = unbracketed.parse::<IpAddr>() {
            return Ok(San::Ip(ip));
        }
        let name = s.trim_end_matches('.').to_ascii_lowercase();
        let labels = name.strip_prefix("*.").unwrap_or(&name);
        let valid = !labels.is_empty()
            && name.len() <= 253
            && labels.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            });
        if !valid {
            return Err(Error::Invalid(format!(
                "{s:?} is neither an IP address nor a DNS name"
            )));
        }
        Ok(San::Dns(name))
    }
}

impl fmt::Display for San {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            San::Dns(name) => f.write_str(name),
            San::Ip(ip) => write!(f, "{ip}"),
        }
    }
}

impl San {
    fn to_rcgen(&self) -> Result<SanType> {
        Ok(match self {
            San::Dns(name) => SanType::DnsName(
                Ia5String::try_from(name.as_str()).map_err(|e| Error::Invalid(e.to_string()))?,
            ),
            San::Ip(ip) => SanType::IpAddress(*ip),
        })
    }
}

/// The names every server certificate carries, wherever it's bound:
/// `localhost`, `127.0.0.1`, `::1`, and this computer's name (bare and
/// `.local`). Callers add the bound or interface addresses and the host of
/// `server_public_url`.
pub fn default_sans() -> Vec<San> {
    let mut sans = vec![
        San::Dns("localhost".into()),
        San::Ip(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        San::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST)),
    ];
    if let Some(host) = computer_name() {
        if let Ok(San::Dns(name)) = host.parse::<San>() {
            if !name.contains('.') {
                sans.push(San::Dns(format!("{name}.local")));
            }
            sans.push(San::Dns(name));
        }
    }
    normalize_sans(&sans)
}

/// Sorted and deduplicated, so two lists naming the same set compare equal.
pub fn normalize_sans(sans: &[San]) -> Vec<San> {
    sans.iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn computer_name() -> Option<String> {
    let from_env = |var: &str| {
        std::env::var(var)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if cfg!(windows) {
        return from_env("COMPUTERNAME");
    }
    from_env("HOSTNAME").or_else(|| {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

// ── options ──────────────────────────────────────────────────────────────────

/// Which CNG key storage provider holds a key. Windows only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// The TPM if one is usable, else software.
    Auto,
    /// The Microsoft Platform Crypto Provider (TPM 2.0) or fail.
    Tpm,
    /// The Microsoft Software Key Storage Provider.
    Software,
}

impl FromStr for Provider {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Provider::Auto),
            "tpm" => Ok(Provider::Tpm),
            "software" => Ok(Provider::Software),
            other => Err(Error::Invalid(format!(
                "unknown key provider {other:?}; use auto, tpm or software"
            ))),
        }
    }
}

/// How [`Authority::create`] sets up a directory.
#[derive(Debug, Clone)]
pub struct CreateOptions {
    pub backend: BackendKind,
    /// CNG only: where the root key goes. Defaults to [`Provider::Auto`].
    pub root_provider: Provider,
    /// CNG only: where server keys go. Defaults to software. The server key
    /// signs every TLS handshake, and TPM signing is far slower (see
    /// `docs/tls-plan.md`); it is still non-exportable.
    pub server_provider: Provider,
    /// File only: whether the root key is wrapped under a credential-manager
    /// key.
    pub file_protection: FileProtection,
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            root_provider: Provider::Auto,
            server_provider: Provider::Software,
            file_protection: FileProtection::Keyring,
        }
    }
}

/// CNG where it's built in (Windows), else files.
pub fn default_backend() -> BackendKind {
    if cfg!(all(windows, feature = "cng")) {
        BackendKind::Cng
    } else {
        BackendKind::File
    }
}

// ── the authority ────────────────────────────────────────────────────────────

/// One certificates directory: its root CA and current server certificate.
pub struct Authority {
    dir: PathBuf,
    state: TlsState,
    backend: Box<dyn KeyBackend>,
}

impl fmt::Debug for Authority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Authority")
            .field("dir", &self.dir)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// `<appdata>/certs`, the directory solx-browser provisions.
pub fn default_dir() -> PathBuf {
    solx_config::appdata_dir().join("certs")
}

impl Authority {
    /// The CA in `dir`, or `None` if it has none yet.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Option<Self>> {
        Self::open_with(dir, &CreateOptions::default())
    }

    /// [`open`](Self::open), with the providers and file protection new
    /// server keys should use. The backend itself always comes from
    /// `tls.json`.
    pub fn open_with(dir: impl Into<PathBuf>, options: &CreateOptions) -> Result<Option<Self>> {
        let dir = dir.into();
        let Some(state) = TlsState::load(&dir)? else {
            return Ok(None);
        };
        let backend = open_backend(&dir, state.backend, options)?;
        Ok(Some(Self {
            dir,
            state,
            backend,
        }))
    }

    /// Create a root CA in `dir`. Fails if `dir` already has one: replacing
    /// a root means every client has to trust it again, so it's never done
    /// implicitly ([`delete`](Self::delete) first).
    pub fn create(dir: impl Into<PathBuf>, options: &CreateOptions) -> Result<Self> {
        let dir = dir.into();
        if TlsState::load(&dir)?.is_some() {
            return Err(Error::State(format!(
                "{} already has a certificate authority",
                dir.display()
            )));
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::io(format!("create {}", dir.display()), e))?;
        let backend = open_backend(&dir, options.backend, options)?;

        let key = backend.create_key(KeyRole::Root)?;
        let result = (|| {
            let now = Utc::now();
            let not_after = now + Duration::days(ROOT_VALIDITY_DAYS);
            let subject = format!("solx local CA ({})", root_label(&dir));
            let mut params = CertificateParams::new(Vec::<String>::new())?;
            params.distinguished_name = distinguished_name(&subject);
            params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
            params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
            params.serial_number = Some(random_serial());
            params.not_before = to_time(now - Duration::days(1));
            params.not_after = to_time(not_after);
            let cert = params.self_signed(&&*key)?;

            let key_ref = backend.commit_key(key.key_ref())?;
            let der = cert.der().to_vec();
            backend.install_cert(&der, &key_ref, "solx local CA")?;
            write_atomic(
                &dir.join(state::ROOT_CERT_FILE),
                der_to_pem(&der).as_bytes(),
                false,
            )?;

            let info = CertInfo {
                sha256: hex(&Sha256::digest(&der)),
                subject: format!("CN={subject}, O=solx"),
                not_before: now - Duration::days(1),
                not_after,
                sans: Vec::new(),
                key: key_ref,
            };
            let state = TlsState::new(backend.kind(), pin_of(&*key), info);
            state.save(&dir)?;
            Ok(state)
        })();
        match result {
            Ok(state) => Ok(Self {
                dir,
                state,
                backend,
            }),
            Err(e) => {
                // Don't leave an orphaned key behind a CA that never existed.
                let _ = backend.delete_key(key.key_ref());
                Err(e)
            }
        }
    }

    /// [`open`](Self::open), else [`create`](Self::create).
    pub fn open_or_create(dir: impl Into<PathBuf>, options: &CreateOptions) -> Result<Self> {
        let dir = dir.into();
        match Self::open_with(&dir, options)? {
            Some(ca) => Ok(ca),
            None => Self::create(dir, options),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn state(&self) -> &TlsState {
        &self.state
    }

    /// What clients pin: `base64url(SHA-256(root SPKI))`.
    pub fn pin(&self) -> &str {
        &self.state.pin
    }

    /// The pin as people compare it: its first 64 bits as grouped hex,
    /// `3F7A-91C2-0B4E-D518`.
    pub fn fingerprint(&self) -> String {
        fingerprint_of_pin(&self.state.pin)
    }

    pub fn root_cert_path(&self) -> PathBuf {
        self.dir.join(state::ROOT_CERT_FILE)
    }

    pub fn server_cert_path(&self) -> PathBuf {
        self.dir.join(state::SERVER_CERT_FILE)
    }

    /// The root certificate, DER.
    pub fn root_der(&self) -> Result<Vec<u8>> {
        let pem = std::fs::read_to_string(self.root_cert_path())
            .map_err(|e| Error::io(format!("read {}", self.root_cert_path().display()), e))?;
        pem_to_der(&pem)
    }

    /// Where the root and server keys live.
    pub fn key_reports(&self) -> (KeyReport, Option<KeyReport>) {
        (
            self.backend.describe_key(&self.state.root.key),
            self.state
                .server
                .as_ref()
                .map(|s| self.backend.describe_key(&s.key)),
        )
    }

    /// Issue a new server certificate for `sans`, valid for `days`, and
    /// retire the previous one (its certificate and key).
    pub fn issue_server(&mut self, sans: &[San], days: i64) -> Result<&CertInfo> {
        let sans = normalize_sans(sans);
        if sans.is_empty() {
            return Err(Error::Invalid(
                "a server certificate needs at least one name".into(),
            ));
        }
        if !(1..=ROOT_VALIDITY_DAYS).contains(&days) {
            return Err(Error::Invalid(format!(
                "server certificate validity must be 1 to {ROOT_VALIDITY_DAYS} days"
            )));
        }
        let root_key = self.backend.open_key(&self.state.root.key).map_err(|e| {
            Error::State(format!(
                "the root key is not available ({e}). Run `solx-certs delete` and \
                 `solx-certs init` to start over; every client will have to trust the new root"
            ))
        })?;
        let root_der = self.root_der()?;
        let issuer = Issuer::from_ca_cert_der(&CertificateDer::from(root_der.clone()), &*root_key)?;

        let key = self.backend.create_key(KeyRole::Server)?;
        let result = (|| {
            let now = Utc::now();
            let not_after = (now + Duration::days(days)).min(self.state.root.not_after);
            let subject = sans
                .iter()
                .find_map(|s| match s {
                    San::Dns(name) if name != "localhost" => Some(name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "solx server".into());
            let mut params = CertificateParams::new(Vec::<String>::new())?;
            params.distinguished_name = distinguished_name(&subject);
            params.subject_alt_names = sans.iter().map(San::to_rcgen).collect::<Result<_>>()?;
            params.is_ca = IsCa::ExplicitNoCa;
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
            params.use_authority_key_identifier_extension = true;
            params.serial_number = Some(random_serial());
            params.not_before = to_time(now - Duration::days(1));
            params.not_after = to_time(not_after);
            let cert = params.signed_by(&&*key, &issuer)?;

            let key_ref = self.backend.commit_key(key.key_ref())?;
            let der = cert.der().to_vec();
            self.backend.install_cert(&der, &key_ref, "solx server")?;
            let chain = format!("{}{}", der_to_pem(&der), der_to_pem(&root_der));
            write_atomic(&self.server_cert_path(), chain.as_bytes(), false)?;
            Ok(CertInfo {
                sha256: hex(&Sha256::digest(&der)),
                subject: format!("CN={subject}, O=solx"),
                not_before: now - Duration::days(1),
                not_after,
                sans: sans.iter().map(San::to_string).collect(),
                key: key_ref,
            })
        })();
        let info = match result {
            Ok(info) => info,
            Err(e) => {
                let _ = self.backend.delete_key(key.key_ref());
                return Err(e);
            }
        };

        let previous = self.state.server.replace(info);
        self.state.save(&self.dir)?;
        if let Some(old) = previous {
            if old.sha256 != self.state.server.as_ref().map_or("", |s| &s.sha256) {
                let _ = self.backend.remove_cert(&old.sha256);
            }
            // The file backend reuses one key path; don't delete the new key.
            if Some(&old.key) != self.state.server.as_ref().map(|s| &s.key) {
                let _ = self.backend.delete_key(&old.key);
            }
        }
        Ok(self.state.server.as_ref().expect("just set"))
    }

    /// Reissue the server certificate if there is none, its names differ
    /// from `sans`, its key is gone, or it has under [`RENEW_WITHIN_DAYS`]
    /// left. Returns whether it reissued. What solx-browser calls at
    /// startup.
    pub fn ensure_server(&mut self, sans: &[San]) -> Result<bool> {
        let wanted: Vec<String> = normalize_sans(sans).iter().map(San::to_string).collect();
        let fresh = self.state.server.as_ref().is_some_and(|s| {
            s.sans == wanted
                && s.not_after - Utc::now() > Duration::days(RENEW_WITHIN_DAYS)
                && self.backend.key_exists(&s.key)
        });
        if fresh {
            return Ok(false);
        }
        self.issue_server(sans, SERVER_VALIDITY_DAYS)?;
        Ok(true)
    }

    /// Remove the CA: both certificates (including from the OS store), both
    /// keys, and the directory's files. Every client that trusts or pins
    /// this root has to be set up again.
    pub fn delete(self) -> Result<()> {
        let mut errors = Vec::new();
        let mut note = |r: Result<()>| {
            if let Err(e) = r {
                errors.push(e.to_string());
            }
        };
        if let Some(server) = &self.state.server {
            note(self.backend.remove_cert(&server.sha256));
            note(self.backend.delete_key(&server.key));
        }
        note(self.backend.remove_cert(&self.state.root.sha256));
        note(self.backend.delete_key(&self.state.root.key));
        note(self.backend.forget());
        for name in [
            state::SERVER_CERT_FILE,
            state::ROOT_CERT_FILE,
            state::SERVER_KEY_FILE,
            state::ROOT_KEY_FILE,
            state::STATE_FILE,
        ] {
            match std::fs::remove_file(self.dir.join(name)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => errors.push(format!("remove {name}: {e}")),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Platform(format!(
                "the CA was only partly removed: {}",
                errors.join("; ")
            )))
        }
    }

    /// Trust this root on this machine: add it to `CurrentUser\Root`.
    /// Windows asks the person to confirm. Elsewhere, returns an
    /// [`Error::Invalid`] with the command to run instead, since a system
    /// trust store needs root.
    pub fn trust_on_this_machine(&self) -> Result<()> {
        platform::trust(&self.root_der()?, &self.root_cert_path())
    }

    /// Undo [`trust_on_this_machine`](Self::trust_on_this_machine).
    pub fn untrust_on_this_machine(&self) -> Result<()> {
        platform::untrust(&self.state.root.sha256, &self.root_cert_path())
    }

    /// Whether this root is in this machine's user trust store. `None` where
    /// that can't be checked.
    pub fn trusted_on_this_machine(&self) -> Option<bool> {
        platform::is_trusted(&self.state.root.sha256)
    }
}

fn open_backend(
    dir: &Path,
    kind: BackendKind,
    options: &CreateOptions,
) -> Result<Box<dyn KeyBackend>> {
    match kind {
        BackendKind::File => Ok(Box::new(file::FileBackend::new(
            dir,
            options.file_protection.clone(),
        ))),
        #[cfg(all(windows, feature = "cng"))]
        BackendKind::Cng => Ok(Box::new(cng::CngBackend::new(
            dir,
            options.root_provider,
            options.server_provider,
        ))),
        #[cfg(not(all(windows, feature = "cng")))]
        BackendKind::Cng => Err(Error::Invalid(
            "the CNG backend is only available on Windows builds with the `cng` feature".into(),
        )),
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// A short, stable label for the directory, so CAs from separate appdata
/// dirs (or machines) are told apart in a certificate store: the computer
/// name plus 8 hex digits of [`dir_key`].
fn root_label(dir: &Path) -> String {
    let key = dir_key(dir);
    match computer_name() {
        Some(host) => format!("{host} {}", &key[..8]),
        None => key[..8].to_string(),
    }
}

/// FNV-1a over the directory's canonical path, lowercased on Windows. The
/// same scheme as `solx_config::credentials`' appdata key, so names are
/// stable across Rust versions and don't spell out the path.
pub(crate) fn dir_key(dir: &Path) -> String {
    let path = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut text = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn distinguished_name(common_name: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    dn.push(DnType::OrganizationName, "solx");
    dn
}

/// 16 random bytes, positive as a DER INTEGER.
fn random_serial() -> SerialNumber {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes[0] &= 0x7f;
    bytes[0] |= 0x01;
    SerialNumber::from_slice(&bytes)
}

fn to_time(at: DateTime<Utc>) -> time::OffsetDateTime {
    time::OffsetDateTime::from_unix_timestamp(at.timestamp())
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
}

pub(crate) fn pin_of(key: &dyn CaKey) -> String {
    pin_of_spki(&key.subject_public_key_info())
}

/// `base64url(SHA-256(spki))`, unpadded.
pub fn pin_of_spki(spki_der: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(spki_der))
}

/// The first 64 bits of a pin as grouped uppercase hex. Empty if `pin`
/// doesn't decode.
pub fn fingerprint_of_pin(pin: &str) -> String {
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(pin) else {
        return String::new();
    };
    bytes
        .chunks(2)
        .take(4)
        .map(|pair| pair.iter().map(|b| format!("{b:02X}")).collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn der_to_pem(der: &[u8]) -> String {
    pem_block("CERTIFICATE", der)
}

pub(crate) fn pem_block(label: &str, der: &[u8]) -> String {
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// The first PEM block's DER.
pub(crate) fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN "))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END "))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|e| Error::State(format!("not a PEM file: {e}")))
}

/// Write through a temporary file in the same directory and rename it into
/// place. `private` restricts the file to its owner where the platform has
/// Unix permissions; on Windows the appdata directory is already per-user.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    let tmp = path.with_extension("tmp");
    write_file(&tmp, bytes, private)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::io(format!("replace {}", path.display()), e)
    })
}

pub(crate) fn write_file(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options
        .open(path)
        .map_err(|e| Error::io(format!("write {}", path.display()), e))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| Error::io(format!("write {}", path.display()), e))
}

/// DER-encode an ECDSA signature given as CNG returns it, `r ‖ s` with
/// both halves the same length, as the X.509 `Ecdsa-Sig-Value`
/// `SEQUENCE { r INTEGER, s INTEGER }`. PKCS#11 returns the same shape, so
/// the `pkcs11` backend will use it too.
#[cfg_attr(not(all(windows, feature = "cng")), allow(dead_code))]
pub(crate) fn ecdsa_raw_to_der(raw: &[u8]) -> Result<Vec<u8>> {
    if raw.is_empty() || !raw.len().is_multiple_of(2) || raw.len() > 132 {
        return Err(Error::Platform(format!(
            "unexpected ECDSA signature length {}",
            raw.len()
        )));
    }
    fn integer(bytes: &[u8]) -> Vec<u8> {
        let trimmed = match bytes.iter().position(|&b| b != 0) {
            Some(i) => &bytes[i..],
            None => &[0u8][..],
        };
        let mut out = vec![0x02];
        let pad = trimmed[0] & 0x80 != 0;
        out.push((trimmed.len() + usize::from(pad)) as u8);
        if pad {
            out.push(0);
        }
        out.extend_from_slice(trimmed);
        out
    }
    let (r, s) = raw.split_at(raw.len() / 2);
    let mut body = integer(r);
    body.extend(integer(s));
    let mut out = vec![0x30];
    if body.len() < 0x80 {
        out.push(body.len() as u8);
    } else {
        out.extend([0x81, body.len() as u8]);
    }
    out.extend(body);
    Ok(out)
}

// ── trust stores ─────────────────────────────────────────────────────────────

#[cfg(all(windows, feature = "cng"))]
mod platform {
    pub(crate) use crate::cng::{is_trusted, trust, untrust};
}

#[cfg(not(all(windows, feature = "cng")))]
mod platform {
    use std::path::Path;

    use crate::{Error, Result};

    fn instructions(root_path: &Path) -> String {
        let path = root_path.display();
        if cfg!(target_os = "macos") {
            format!(
                "add it with: security add-trusted-cert -r trustRoot \
                 -k ~/Library/Keychains/login.keychain-db {path}"
            )
        } else if cfg!(windows) {
            format!("add it with: certutil -user -addstore Root {path}")
        } else {
            format!(
                "this needs root. Debian/Ubuntu: sudo cp {path} \
                 /usr/local/share/ca-certificates/solx-root.crt && sudo update-ca-certificates. \
                 Fedora/Arch: sudo trust anchor --store {path}"
            )
        }
    }

    pub(crate) fn trust(_der: &[u8], root_path: &Path) -> Result<()> {
        Err(Error::Invalid(format!(
            "solx-certs can't change this system's trust store itself; {}",
            instructions(root_path)
        )))
    }

    pub(crate) fn untrust(_sha256: &str, root_path: &Path) -> Result<()> {
        Err(Error::Invalid(format!(
            "solx-certs can't change this system's trust store itself; remove the \
             certificate you added from {} with the same tool you used to add it",
            root_path.display()
        )))
    }

    pub(crate) fn is_trusted(_sha256: &str) -> Option<bool> {
        None
    }
}
