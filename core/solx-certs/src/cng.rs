//! The Windows backend: persisted CNG keys that can sign but never leave
//! their key storage provider, and certificates installed in
//! `CurrentUser\My` bound to them.
//!
//! - **Keys** are created with `windows-sys` (`NCryptCreatePersistedKey`),
//!   because neither `rustls-cng` nor `schannel` can create one. The export
//!   policy is set to none before the key is finalized. On the TPM (the
//!   Microsoft Platform Crypto Provider) the key can't be exported whatever
//!   the policy says.
//! - **Signing** hashes with SHA-256 and calls `NCryptSignHash`, then
//!   DER-encodes CNG's raw `r ‖ s` for X.509.
//! - **Certificates** go through `schannel`: added to `CurrentUser\My` and
//!   bound to their key's container with `CERT_KEY_PROV_INFO`, which is
//!   how `solx-server` (through `rustls-cng`) and Windows' own tools find
//!   the key from the certificate. No admin rights are needed for any of
//!   it.

use std::ptr;

use rcgen::{PublicKeyData, SignatureAlgorithm, SigningKey, PKCS_ECDSA_P256_SHA256};
use schannel::cert_context::{CertContext, HashAlgorithm};
use schannel::cert_store::{CertAdd, CertStore};
use sha2::{Digest, Sha256};
use windows_sys::core::HRESULT;
use windows_sys::Win32::Foundation::NTE_BAD_KEYSET;
use windows_sys::Win32::Security::Cryptography::{
    NCryptCreatePersistedKey, NCryptDeleteKey, NCryptExportKey, NCryptFinalizeKey,
    NCryptFreeObject, NCryptGetProperty, NCryptOpenKey, NCryptOpenStorageProvider,
    NCryptSetProperty, NCryptSignHash, BCRYPT_ECCKEY_BLOB, BCRYPT_ECCPUBLIC_BLOB,
    BCRYPT_ECDSA_P256_ALGORITHM, NCRYPT_ALLOW_EXPORT_FLAG, NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG,
    NCRYPT_ALLOW_SIGNING_FLAG, NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_KEY_USAGE_PROPERTY,
    NCRYPT_PERSIST_FLAG, NCRYPT_SILENT_FLAG,
};

use crate::backend::{CaKey, KeyBackend, KeyReport, KeyRole};
use crate::state::{BackendKind, KeyRef};
use crate::{hex, Error, Provider, Result};

pub(crate) const PLATFORM_PROVIDER: &str = "Microsoft Platform Crypto Provider";
pub(crate) const SOFTWARE_PROVIDER: &str = "Microsoft Software Key Storage Provider";

/// `ERROR_CANCELLED` as an HRESULT: someone said no to Windows' trust
/// confirmation.
const HRESULT_CANCELLED: i32 = 0x800704C7_u32 as i32;

pub(crate) struct CngBackend {
    dir_key: String,
    root_provider: Provider,
    server_provider: Provider,
}

impl CngBackend {
    pub(crate) fn new(dir: &std::path::Path, root: Provider, server: Provider) -> Self {
        Self {
            dir_key: crate::dir_key(dir),
            root_provider: root,
            server_provider: server,
        }
    }
}

impl KeyBackend for CngBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Cng
    }

    fn create_key(&self, role: KeyRole) -> Result<Box<dyn CaKey>> {
        let (provider, role_name) = match role {
            KeyRole::Root => (self.root_provider, "ca"),
            KeyRole::Server => (self.server_provider, "server"),
        };
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let container = format!("solx-{role_name}-{}-{unique:x}", self.dir_key);
        let candidates: &[&str] = match provider {
            Provider::Auto => &[PLATFORM_PROVIDER, SOFTWARE_PROVIDER],
            Provider::Tpm => &[PLATFORM_PROVIDER],
            Provider::Software => &[SOFTWARE_PROVIDER],
        };
        let mut failures = Vec::new();
        for name in candidates {
            match CngKey::create(name, &container) {
                Ok(key) => return Ok(Box::new(key)),
                Err(e) => failures.push(format!("{name}: {e}")),
            }
        }
        Err(Error::Platform(format!(
            "could not create a non-exportable key ({})",
            failures.join("; ")
        )))
    }

    fn open_key(&self, key: &KeyRef) -> Result<Box<dyn CaKey>> {
        let (provider, container) = cng_ref(key)?;
        Ok(Box::new(CngKey::open(provider, container)?))
    }

    fn key_exists(&self, key: &KeyRef) -> bool {
        cng_ref(key).is_ok_and(|(p, c)| CngKey::open(p, c).is_ok())
    }

    fn delete_key(&self, key: &KeyRef) -> Result<()> {
        let (provider, container) = cng_ref(key)?;
        let key = match CngKey::open(provider, container) {
            Ok(key) => key,
            Err(KeyError::NotFound) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        // NCryptDeleteKey frees the handle when it succeeds.
        let handle = key.handle.take();
        let hr = unsafe { NCryptDeleteKey(handle, 0) };
        if hr != 0 {
            unsafe { NCryptFreeObject(handle) };
        }
        check(hr, &format!("delete key {container}"))
    }

    fn install_cert(&self, der: &[u8], key: &KeyRef, friendly_name: &str) -> Result<()> {
        let (provider, container) = cng_ref(key)?;
        let context =
            CertContext::new(der).map_err(|e| platform("decode the new certificate", e))?;
        let mut store =
            CertStore::open_current_user("My").map_err(|e| platform(r"open CurrentUser\My", e))?;
        let added = store
            .add_cert(&context, CertAdd::ReplaceExisting)
            .map_err(|e| platform(r"add the certificate to CurrentUser\My", e))?;
        let bound = added
            .set_key_prov_info()
            .container(container)
            .provider(provider)
            .set()
            .map_err(|e| platform("bind the certificate to its key", e))
            .and_then(|()| {
                added
                    .set_friendly_name(friendly_name)
                    .map_err(|e| platform("name the certificate", e))
            });
        if let Err(e) = bound {
            let _ = added.delete();
            return Err(e);
        }
        Ok(())
    }

    fn remove_cert(&self, sha256: &str) -> Result<()> {
        remove_from_store("My", sha256)
    }

    fn cert_installed(&self, sha256: &str) -> bool {
        // An unreadable store counts as present: reissuing wouldn't help.
        find_in_store("My", sha256).map_or(true, |c| c.is_some())
    }

    fn describe_key(&self, key: &KeyRef) -> KeyReport {
        let Ok((provider, container)) = cng_ref(key) else {
            return KeyReport {
                location: "unknown".into(),
                hardware: false,
                exportable: None,
                present: false,
            };
        };
        let hardware = provider == PLATFORM_PROVIDER;
        let opened = CngKey::open(provider, container);
        let exportable = match &opened {
            Ok(key) => match key.export_policy() {
                Some(policy) => Some(
                    policy & (NCRYPT_ALLOW_EXPORT_FLAG | NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG) != 0,
                ),
                // TPM keys can't be exported whether or not the provider
                // reports a policy.
                None if hardware => Some(false),
                None => None,
            },
            Err(_) => None,
        };
        KeyReport {
            location: format!("{provider} ({container})"),
            hardware,
            exportable,
            present: opened.is_ok(),
        }
    }
}

fn cng_ref(key: &KeyRef) -> Result<(&str, &str)> {
    match key {
        KeyRef::Cng {
            provider,
            container,
        } => Ok((provider, container)),
        other => Err(Error::State(format!(
            "the CNG backend can't use a {other:?} key"
        ))),
    }
}

// ── keys ─────────────────────────────────────────────────────────────────────

/// A CNG handle, freed on drop.
struct Handle(usize);

impl Handle {
    /// Give up ownership, for a call that frees the handle itself.
    fn take(mut self) -> usize {
        std::mem::replace(&mut self.0, 0)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { NCryptFreeObject(self.0) };
        }
    }
}

enum KeyError {
    NotFound,
    Other(Error),
}

impl From<KeyError> for Error {
    fn from(e: KeyError) -> Self {
        match e {
            KeyError::NotFound => Error::State("the key no longer exists".into()),
            KeyError::Other(e) => e,
        }
    }
}

impl From<Error> for KeyError {
    fn from(e: Error) -> Self {
        KeyError::Other(e)
    }
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::NotFound => f.write_str("the key no longer exists"),
            KeyError::Other(e) => write!(f, "{e}"),
        }
    }
}

pub(crate) struct CngKey {
    handle: Handle,
    /// Uncompressed EC point `04 ‖ X ‖ Y`, what X.509 puts in the
    /// SubjectPublicKeyInfo's BIT STRING.
    public: Vec<u8>,
    key_ref: KeyRef,
}

impl CngKey {
    fn create(provider_name: &str, container: &str) -> Result<Self> {
        let provider = open_provider(provider_name)?;
        let name = wide(container);
        let mut raw = 0usize;
        check(
            unsafe {
                NCryptCreatePersistedKey(
                    provider.0,
                    &mut raw,
                    BCRYPT_ECDSA_P256_ALGORITHM,
                    name.as_ptr(),
                    0,
                    0,
                )
            },
            "create the key",
        )?;
        let handle = Handle(raw);
        let hardware = provider_name == PLATFORM_PROVIDER;
        let configured = (|| {
            // Non-exportable is already the default for a new key; set it
            // anyway so it never depends on provider defaults. The TPM
            // provider may refuse the property, and its keys can't leave the
            // TPM regardless.
            match set_u32(&handle, NCRYPT_EXPORT_POLICY_PROPERTY, 0) {
                Err(_) if hardware => {}
                other => other?,
            }
            match set_u32(
                &handle,
                NCRYPT_KEY_USAGE_PROPERTY,
                NCRYPT_ALLOW_SIGNING_FLAG,
            ) {
                Err(_) if hardware => {}
                other => other?,
            }
            check(
                unsafe { NCryptFinalizeKey(handle.0, NCRYPT_SILENT_FLAG) },
                "finalize the key",
            )?;
            export_public(&handle)
        })();
        match configured {
            Ok(public) => Ok(Self {
                handle,
                public,
                key_ref: KeyRef::Cng {
                    provider: provider_name.to_string(),
                    container: container.to_string(),
                },
            }),
            Err(e) => {
                // Remove the half-made key, finalized or not.
                let raw = handle.take();
                if unsafe { NCryptDeleteKey(raw, 0) } != 0 {
                    unsafe { NCryptFreeObject(raw) };
                }
                Err(e)
            }
        }
    }

    fn open(provider_name: &str, container: &str) -> std::result::Result<Self, KeyError> {
        let provider = open_provider(provider_name)?;
        let name = wide(container);
        let mut raw = 0usize;
        let hr =
            unsafe { NCryptOpenKey(provider.0, &mut raw, name.as_ptr(), 0, NCRYPT_SILENT_FLAG) };
        if hr == NTE_BAD_KEYSET {
            return Err(KeyError::NotFound);
        }
        check(hr, &format!("open key {container}"))?;
        let handle = Handle(raw);
        let public = export_public(&handle)?;
        Ok(Self {
            handle,
            public,
            key_ref: KeyRef::Cng {
                provider: provider_name.to_string(),
                container: container.to_string(),
            },
        })
    }

    fn sign_digest(&self, digest: &[u8]) -> Result<Vec<u8>> {
        let mut len = 0u32;
        check(
            unsafe {
                NCryptSignHash(
                    self.handle.0,
                    ptr::null(),
                    digest.as_ptr(),
                    digest.len() as u32,
                    ptr::null_mut(),
                    0,
                    &mut len,
                    NCRYPT_SILENT_FLAG,
                )
            },
            "size the signature",
        )?;
        let mut signature = vec![0u8; len as usize];
        check(
            unsafe {
                NCryptSignHash(
                    self.handle.0,
                    ptr::null(),
                    digest.as_ptr(),
                    digest.len() as u32,
                    signature.as_mut_ptr(),
                    len,
                    &mut len,
                    NCRYPT_SILENT_FLAG,
                )
            },
            "sign",
        )?;
        signature.truncate(len as usize);
        crate::ecdsa_raw_to_der(&signature)
    }

    /// `NCRYPT_EXPORT_POLICY_PROPERTY`, if the provider reports it.
    fn export_policy(&self) -> Option<u32> {
        get_u32(&self.handle, NCRYPT_EXPORT_POLICY_PROPERTY)
    }

    /// Try to export the private key in every format CNG offers. For tests:
    /// it must always fail.
    #[cfg(test)]
    pub(crate) fn private_export_succeeds(&self) -> bool {
        use windows_sys::Win32::Security::Cryptography::{
            BCRYPT_ECCPRIVATE_BLOB, NCRYPT_PKCS8_PRIVATE_KEY_BLOB,
        };
        [BCRYPT_ECCPRIVATE_BLOB, NCRYPT_PKCS8_PRIVATE_KEY_BLOB]
            .into_iter()
            .any(|blob| {
                let mut len = 0u32;
                let hr = unsafe {
                    NCryptExportKey(
                        self.handle.0,
                        0,
                        blob,
                        ptr::null(),
                        ptr::null_mut(),
                        0,
                        &mut len,
                        NCRYPT_SILENT_FLAG,
                    )
                };
                if hr != 0 || len == 0 {
                    return false;
                }
                let mut out = vec![0u8; len as usize];
                let hr = unsafe {
                    NCryptExportKey(
                        self.handle.0,
                        0,
                        blob,
                        ptr::null(),
                        out.as_mut_ptr(),
                        len,
                        &mut len,
                        NCRYPT_SILENT_FLAG,
                    )
                };
                hr == 0
            })
    }

    #[cfg(test)]
    pub(crate) fn open_for_tests(key: &KeyRef) -> Result<Self> {
        let (provider, container) = cng_ref(key)?;
        Ok(Self::open(provider, container)?)
    }

    #[cfg(test)]
    pub(crate) fn sign_for_tests(&self, digest: &[u8]) -> Result<Vec<u8>> {
        self.sign_digest(digest)
    }
}

impl PublicKeyData for CngKey {
    fn der_bytes(&self) -> &[u8] {
        &self.public
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

impl SigningKey for CngKey {
    fn sign(&self, msg: &[u8]) -> std::result::Result<Vec<u8>, rcgen::Error> {
        self.sign_digest(&Sha256::digest(msg)).map_err(|e| {
            eprintln!("solx-certs: CNG signing failed: {e}");
            rcgen::Error::RemoteKeyError
        })
    }
}

impl CaKey for CngKey {
    fn key_ref(&self) -> &KeyRef {
        &self.key_ref
    }
}

fn open_provider(name: &str) -> Result<Handle> {
    let wide_name = wide(name);
    let mut raw = 0usize;
    check(
        unsafe { NCryptOpenStorageProvider(&mut raw, wide_name.as_ptr(), 0) },
        &format!("open {name}"),
    )?;
    Ok(Handle(raw))
}

fn export_public(key: &Handle) -> Result<Vec<u8>> {
    let mut len = 0u32;
    check(
        unsafe {
            NCryptExportKey(
                key.0,
                0,
                BCRYPT_ECCPUBLIC_BLOB,
                ptr::null(),
                ptr::null_mut(),
                0,
                &mut len,
                0,
            )
        },
        "size the public key",
    )?;
    let mut blob = vec![0u8; len as usize];
    check(
        unsafe {
            NCryptExportKey(
                key.0,
                0,
                BCRYPT_ECCPUBLIC_BLOB,
                ptr::null(),
                blob.as_mut_ptr(),
                len,
                &mut len,
                0,
            )
        },
        "export the public key",
    )?;
    // BCRYPT_ECCKEY_BLOB { dwMagic, cbKey } then X and Y, cbKey bytes each.
    let header = std::mem::size_of::<BCRYPT_ECCKEY_BLOB>();
    let coord = blob
        .get(4..8)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .unwrap_or_default();
    if coord != 32 || blob.len() < header + 2 * coord {
        return Err(Error::Platform("the key is not an ECDSA P-256 key".into()));
    }
    let mut point = Vec::with_capacity(1 + 2 * coord);
    point.push(0x04);
    point.extend_from_slice(&blob[header..header + 2 * coord]);
    Ok(point)
}

fn set_u32(key: &Handle, property: windows_sys::core::PCWSTR, value: u32) -> Result<()> {
    let bytes = value.to_le_bytes();
    let set = |flags| unsafe {
        NCryptSetProperty(key.0, property, bytes.as_ptr(), bytes.len() as u32, flags)
    };
    // Persisted with the key where the provider supports that.
    if set(NCRYPT_PERSIST_FLAG | NCRYPT_SILENT_FLAG) == 0 {
        return Ok(());
    }
    check(set(NCRYPT_SILENT_FLAG), "set a key property")
}

fn get_u32(key: &Handle, property: windows_sys::core::PCWSTR) -> Option<u32> {
    let mut value = [0u8; 4];
    let mut len = 0u32;
    let hr = unsafe {
        NCryptGetProperty(
            key.0,
            property,
            value.as_mut_ptr(),
            value.len() as u32,
            &mut len,
            NCRYPT_SILENT_FLAG,
        )
    };
    (hr == 0 && len == 4).then(|| u32::from_le_bytes(value))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn check(hr: HRESULT, what: &str) -> Result<()> {
    if hr == 0 {
        return Ok(());
    }
    let message = std::io::Error::from_raw_os_error(hr).to_string();
    Err(Error::Platform(format!(
        "{what}: {message} (0x{:08X})",
        hr as u32
    )))
}

fn platform(what: &str, e: std::io::Error) -> Error {
    Error::Platform(format!("{what}: {e}"))
}

// ── stores ───────────────────────────────────────────────────────────────────

fn find_in_store(store_name: &str, sha256: &str) -> Result<Option<CertContext>> {
    let store = CertStore::open_current_user(store_name)
        .map_err(|e| platform(&format!(r"open CurrentUser\{store_name}"), e))?;
    for cert in store.certs() {
        if let Ok(print) = cert.fingerprint(HashAlgorithm::sha256()) {
            if hex(&print).eq_ignore_ascii_case(sha256) {
                return Ok(Some(cert));
            }
        }
    }
    Ok(None)
}

fn remove_from_store(store_name: &str, sha256: &str) -> Result<()> {
    match find_in_store(store_name, sha256)? {
        Some(cert) => cert.delete().map_err(|e| {
            cancelled_or(
                e,
                &format!(r"remove the certificate from CurrentUser\{store_name}"),
            )
        }),
        None => Ok(()),
    }
}

fn cancelled_or(e: std::io::Error, what: &str) -> Error {
    if e.raw_os_error() == Some(HRESULT_CANCELLED) || e.raw_os_error() == Some(1223) {
        Error::Invalid("cancelled in the Windows confirmation dialog".into())
    } else {
        platform(what, e)
    }
}

/// Add the root to `CurrentUser\Root`. Windows asks the person to confirm,
/// and this blocks until they answer.
pub(crate) fn trust(der: &[u8], _root_path: &std::path::Path) -> Result<()> {
    let context = CertContext::new(der).map_err(|e| platform("decode the root", e))?;
    let mut store =
        CertStore::open_current_user("Root").map_err(|e| platform(r"open CurrentUser\Root", e))?;
    store
        .add_cert(&context, CertAdd::ReplaceExisting)
        .map(|_| ())
        .map_err(|e| cancelled_or(e, r"add the root to CurrentUser\Root"))
}

pub(crate) fn untrust(sha256: &str, _root_path: &std::path::Path) -> Result<()> {
    remove_from_store("Root", sha256)
}

pub(crate) fn is_trusted(sha256: &str) -> Option<bool> {
    find_in_store("Root", sha256).ok().map(|c| c.is_some())
}
