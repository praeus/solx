//! `tls.json`: what a certificates directory holds, and where each key
//! lives. Public, because solx-server (phase 2) reads it for
//! `server_tls: "auto"` and solx-browser's Settings show it.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const STATE_FILE: &str = "tls.json";
pub const ROOT_CERT_FILE: &str = "solx-root.crt";
pub const SERVER_CERT_FILE: &str = "solx-server.crt";
pub const ROOT_KEY_FILE: &str = "solx-root.key";
pub const SERVER_KEY_FILE: &str = "solx-server.key";

const STATE_VERSION: u32 = 1;

/// Where a directory's keys live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// Windows CNG: non-exportable keys, certificates in `CurrentUser\My`.
    Cng,
    /// PKCS#8 PEM files in the directory.
    File,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Cng => "cng",
            BackendKind::File => "file",
        }
    }
}

/// A private key, by reference. Never the key itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeyRef {
    /// A persisted CNG key: its key storage provider and container name.
    Cng { provider: String, container: String },
    /// A PEM file, relative to the certificates directory. `wrapped` means
    /// it is encrypted under a key kept in the OS credential manager.
    File { path: String, wrapped: bool },
}

/// One issued certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertInfo {
    /// SHA-256 of the certificate's DER, lowercase hex. Also its Windows
    /// SHA-256 thumbprint, for `server_tls: { "store_thumbprint" }`.
    pub sha256: String,
    pub subject: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    /// Subject alternative names, as given (server certificates only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sans: Vec<String>,
    pub key: KeyRef,
}

/// The contents of `tls.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsState {
    pub version: u32,
    pub backend: BackendKind,
    /// `base64url(SHA-256(root SubjectPublicKeyInfo))`, unpadded: what
    /// clients pin.
    pub pin: String,
    pub root: CertInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<CertInfo>,
}

impl TlsState {
    pub(crate) fn new(backend: BackendKind, pin: String, root: CertInfo) -> Self {
        Self {
            version: STATE_VERSION,
            backend,
            pin,
            root,
            server: None,
        }
    }

    /// Read `<dir>/tls.json`, or `None` if the directory has no CA.
    pub fn load(dir: &Path) -> Result<Option<Self>> {
        let path = dir.join(STATE_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::io(format!("read {}", path.display()), e)),
        };
        let state: Self = serde_json::from_str(&text)
            .map_err(|e| Error::State(format!("{} is not valid: {e}", path.display())))?;
        if state.version != STATE_VERSION {
            return Err(Error::State(format!(
                "{} has version {}; this build understands version {STATE_VERSION}",
                path.display(),
                state.version
            )));
        }
        Ok(Some(state))
    }

    /// Write `<dir>/tls.json` through a temporary file, so a crash never
    /// leaves it half-written.
    pub(crate) fn save(&self, dir: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::State(format!("serialize tls.json: {e}")))?;
        crate::write_atomic(&dir.join(STATE_FILE), text.as_bytes(), false)
    }
}
