//! The seam between certificate building (shared, in `lib.rs`) and where
//! keys and certificates are kept (`file`, `cng`, and later `pkcs11` and
//! `keychain`). A backend never builds a certificate: it creates and opens
//! keys that can sign, and installs finished certificates where its
//! platform expects them.

use crate::state::{BackendKind, KeyRef};
use crate::Result;

/// Which certificate a key is for. Backends may name, place or protect the
/// two differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyRole {
    Root,
    Server,
}

/// A private key that can sign: ECDSA P-256 with SHA-256, the only kind
/// solx-certs issues.
pub(crate) trait CaKey: rcgen::SigningKey {
    fn key_ref(&self) -> &KeyRef;
}

/// What can be said about where a key lives, for `solx-certs show` and the
/// desktop app's Settings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KeyReport {
    /// For example `Microsoft Platform Crypto Provider` or `solx-root.key`.
    pub location: String,
    /// Whether the key is held in hardware (a TPM) rather than software.
    pub hardware: bool,
    /// `Some(false)` when the platform enforces non-exportability, `None`
    /// when it can't be determined (or the key is missing).
    pub exportable: Option<bool>,
    /// Whether the key is still there.
    pub present: bool,
}

pub(crate) trait KeyBackend {
    fn kind(&self) -> BackendKind;

    /// Create a new key for `role`. It may be written somewhere temporary
    /// until [`commit_key`](Self::commit_key).
    fn create_key(&self, role: KeyRole) -> Result<Box<dyn CaKey>>;

    fn open_key(&self, key: &KeyRef) -> Result<Box<dyn CaKey>>;

    fn key_exists(&self, key: &KeyRef) -> bool;

    /// Make a key from [`create_key`](Self::create_key) permanent, once the
    /// certificate for it has been issued. Returns where it now lives.
    fn commit_key(&self, key: &KeyRef) -> Result<KeyRef> {
        Ok(key.clone())
    }

    fn delete_key(&self, key: &KeyRef) -> Result<()>;

    /// Install `der` where this platform's TLS stack finds it, bound to
    /// `key`. A no-op for backends whose certificates are only files.
    fn install_cert(&self, der: &[u8], key: &KeyRef, friendly_name: &str) -> Result<()>;

    /// Remove an installed certificate by SHA-256 thumbprint (lowercase
    /// hex). Not finding it is not an error.
    fn remove_cert(&self, sha256: &str) -> Result<()>;

    /// Whether an installed certificate is still where
    /// [`install_cert`](Self::install_cert) put it. Always true for
    /// backends whose certificates are only files.
    fn cert_installed(&self, _sha256: &str) -> bool {
        true
    }

    fn describe_key(&self, key: &KeyRef) -> KeyReport;

    /// Remove anything this backend keeps outside the directory, apart from
    /// keys and certificates (for example the file backend's wrapping key).
    fn forget(&self) -> Result<()> {
        Ok(())
    }
}
