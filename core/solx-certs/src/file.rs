//! The portable backend: keys as PKCS#8 PEM files in the certificates
//! directory, readable by OpenSSL and anything else that reads PEM.
//!
//! The root key is wrapped (AES-256-GCM) under a random key kept in the OS
//! credential manager, so a copy of the directory alone doesn't carry a
//! usable CA. The server key stays plain PEM, because a TLS server has to
//! load it. Either way, any process running as this user can get at both:
//! this backend is for platforms without a non-exportable key store, and
//! for people who want plain files.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine as _;
use rcgen::{KeyPair, PublicKeyData, SignatureAlgorithm, SigningKey, PKCS_ECDSA_P256_SHA256};

use crate::backend::{CaKey, KeyBackend, KeyReport, KeyRole};
use crate::state::{self, BackendKind, KeyRef};
use crate::{Error, Result, KEYRING_SERVICE};

const WRAPPED_LABEL: &str = "SOLX WRAPPED PRIVATE KEY";
const NONCE_LEN: usize = 12;

/// How the file backend protects the root key.
#[derive(Clone, PartialEq, Eq)]
pub enum FileProtection {
    /// Wrap it under a key kept in the OS credential manager. If there is no
    /// credential manager, fall back to [`Plain`](Self::Plain) with a
    /// warning.
    Keyring,
    /// Plain PKCS#8 PEM, owner-only where the platform has Unix permissions.
    Plain,
    /// Wrap it under this key. Tests only, so they never touch the real
    /// credential manager.
    #[doc(hidden)]
    Static([u8; 32]),
}

impl std::fmt::Debug for FileProtection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FileProtection::Keyring => "Keyring",
            FileProtection::Plain => "Plain",
            FileProtection::Static(_) => "Static(..)",
        })
    }
}

pub(crate) struct FileBackend {
    dir: PathBuf,
    protection: FileProtection,
    /// Credential-manager account of the wrapping key.
    account: String,
}

impl FileBackend {
    pub(crate) fn new(dir: &Path, protection: FileProtection) -> Self {
        Self {
            dir: dir.to_path_buf(),
            protection,
            account: format!("certs-wrap:{}", crate::dir_key(dir)),
        }
    }

    fn path(&self, key: &KeyRef) -> Result<PathBuf> {
        match key {
            KeyRef::File { path, .. } => Ok(self.dir.join(path)),
            other => Err(Error::State(format!(
                "the file backend can't use a {other:?} key"
            ))),
        }
    }

    /// The wrapping key, creating it on first use. `None` means "write the
    /// key unwrapped".
    fn wrap_key_for_create(&self) -> Option<[u8; 32]> {
        match &self.protection {
            FileProtection::Plain => None,
            FileProtection::Static(key) => Some(*key),
            FileProtection::Keyring => match keyring_wrap_key(&self.account, true) {
                Ok(key) => key,
                Err(e) => {
                    eprintln!(
                        "solx-certs: OS credential manager unavailable ({e}); writing the CA \
                         key unencrypted"
                    );
                    None
                }
            },
        }
    }

    fn wrap_key_for_open(&self) -> Result<[u8; 32]> {
        match &self.protection {
            FileProtection::Static(key) => Ok(*key),
            _ => keyring_wrap_key(&self.account, false)
                .map_err(Error::Platform)?
                .ok_or_else(|| {
                    Error::State(format!(
                        "the CA key is encrypted, but its key is missing from the OS credential \
                         manager (service {KEYRING_SERVICE:?}, account {:?})",
                        self.account
                    ))
                }),
        }
    }
}

impl KeyBackend for FileBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::File
    }

    fn create_key(&self, role: KeyRole) -> Result<Box<dyn CaKey>> {
        let pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let final_name = match role {
            KeyRole::Root => state::ROOT_KEY_FILE,
            KeyRole::Server => state::SERVER_KEY_FILE,
        };
        // Written beside the final name until `commit_key`, so a failed
        // issue never replaces a working key.
        let name = format!("{final_name}.new");
        let wrap = match role {
            KeyRole::Root => self.wrap_key_for_create(),
            KeyRole::Server => None,
        };
        let contents = match wrap {
            Some(key) => wrap_pkcs8(&key, &pair.serialize_der())?,
            None => pair.serialize_pem(),
        };
        crate::write_file(&self.dir.join(&name), contents.as_bytes(), true)?;
        Ok(Box::new(FileKey {
            pair,
            key_ref: KeyRef::File {
                path: name,
                wrapped: wrap.is_some(),
            },
        }))
    }

    fn open_key(&self, key: &KeyRef) -> Result<Box<dyn CaKey>> {
        let path = self.path(key)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Error::io(format!("read {}", path.display()), e))?;
        let pair = if text.contains(&format!("-----BEGIN {WRAPPED_LABEL}-----")) {
            let der = unwrap_pkcs8(&self.wrap_key_for_open()?, &text)?;
            KeyPair::try_from(der.as_slice())?
        } else {
            KeyPair::from_pem(&text)?
        };
        if pair.algorithm() != &PKCS_ECDSA_P256_SHA256 {
            return Err(Error::State(format!(
                "{} is not an ECDSA P-256 key",
                path.display()
            )));
        }
        Ok(Box::new(FileKey {
            pair,
            key_ref: key.clone(),
        }))
    }

    fn key_exists(&self, key: &KeyRef) -> bool {
        self.path(key).is_ok_and(|p| p.is_file())
    }

    fn commit_key(&self, key: &KeyRef) -> Result<KeyRef> {
        let KeyRef::File { path, wrapped } = key else {
            return Err(Error::State("not a file key".into()));
        };
        let Some(final_name) = path.strip_suffix(".new") else {
            return Ok(key.clone());
        };
        let from = self.dir.join(path);
        let to = self.dir.join(final_name);
        std::fs::rename(&from, &to)
            .map_err(|e| Error::io(format!("replace {}", to.display()), e))?;
        Ok(KeyRef::File {
            path: final_name.to_string(),
            wrapped: *wrapped,
        })
    }

    fn delete_key(&self, key: &KeyRef) -> Result<()> {
        let path = self.path(key)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(format!("remove {}", path.display()), e)),
        }
    }

    fn install_cert(&self, _der: &[u8], _key: &KeyRef, _friendly_name: &str) -> Result<()> {
        Ok(())
    }

    fn remove_cert(&self, _sha256: &str) -> Result<()> {
        Ok(())
    }

    fn describe_key(&self, key: &KeyRef) -> KeyReport {
        let path = self.path(key).ok();
        KeyReport {
            location: match (key, &path) {
                (KeyRef::File { wrapped: true, .. }, Some(p)) => {
                    format!("{} (encrypted)", p.display())
                }
                (_, Some(p)) => p.display().to_string(),
                (_, None) => "unknown".into(),
            },
            hardware: false,
            exportable: Some(true),
            present: path.is_some_and(|p| p.is_file()),
        }
    }

    fn forget(&self) -> Result<()> {
        if self.protection != FileProtection::Keyring {
            return Ok(());
        }
        let entry = keyring::Entry::new(KEYRING_SERVICE, &self.account)
            .map_err(|e| Error::Platform(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Platform(format!(
                "remove the CA key's wrapping key from the credential manager: {e}"
            ))),
        }
    }
}

struct FileKey {
    pair: KeyPair,
    key_ref: KeyRef,
}

impl PublicKeyData for FileKey {
    fn der_bytes(&self) -> &[u8] {
        self.pair.der_bytes()
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        self.pair.algorithm()
    }
}

impl SigningKey for FileKey {
    fn sign(&self, msg: &[u8]) -> std::result::Result<Vec<u8>, rcgen::Error> {
        self.pair.sign(msg)
    }
}

impl CaKey for FileKey {
    fn key_ref(&self) -> &KeyRef {
        &self.key_ref
    }
}

/// The wrapping key in the credential manager, generated and stored when
/// `create` is set and there is none. Reads back after writing, so a
/// backend that drops writes is caught before a key is wrapped under it.
fn keyring_wrap_key(account: &str, create: bool) -> std::result::Result<Option<[u8; 32]>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| e.to_string())?;
    let decode = |text: &str| -> std::result::Result<[u8; 32], String> {
        base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .ok_or_else(|| format!("credential {account:?} is not a 32-byte key"))
    };
    match entry.get_password() {
        Ok(text) => return decode(&text).map(Some),
        Err(keyring::Error::NoEntry) if create => {}
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    let key: [u8; 32] = Aes256Gcm::generate_key(&mut OsRng).into();
    let text = base64::engine::general_purpose::STANDARD.encode(key);
    entry.set_password(&text).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(read) if read == text => Ok(Some(key)),
        Ok(_) | Err(_) => Err("the credential manager did not keep the key just stored".into()),
    }
}

fn wrap_pkcs8(key: &[u8; 32], pkcs8_der: &[u8]) -> Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, pkcs8_der)
        .map_err(|_| Error::Platform("encrypting the CA key failed".into()))?;
    let mut blob = nonce.to_vec();
    blob.extend(ciphertext);
    Ok(crate::pem_block(WRAPPED_LABEL, &blob))
}

fn unwrap_pkcs8(key: &[u8; 32], pem: &str) -> Result<Vec<u8>> {
    let blob = crate::pem_to_der(pem)?;
    if blob.len() <= NONCE_LEN {
        return Err(Error::State("the encrypted CA key is truncated".into()));
    }
    let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
    Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key))
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| {
            Error::State(
                "the CA key could not be decrypted: its wrapping key in the credential manager \
                 doesn't match"
                    .into(),
            )
        })
}
