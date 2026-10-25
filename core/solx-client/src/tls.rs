//! The one HTTP client a solx client process uses, and the roots it trusts
//! for `https://` servers.
//!
//! Always rustls (reqwest's default features would otherwise pick the
//! platform's TLS stack), trusting:
//! - the built-in webpki roots, for servers behind a public certificate;
//! - the OS trust store, so a solx root someone imported there (`certutil
//!   -user -addstore Root solx-root.crt`, Keychain Access,
//!   `update-ca-certificates`) is honoured;
//! - extra roots from a PEM file: `server_ca_file` in solx-config.json, or
//!   `SOLX_CA_FILE`, for a server whose root isn't in the OS store.
//!
//! See `docs/tls-plan.md`. A pinned root (phase 4) will replace all of these
//! for the server it was pinned for.

use std::path::Path;

use solx_surface::error::SolxError;

/// The environment variable naming a PEM file of extra roots to trust, in
/// addition to `server_ca_file`.
pub const CA_FILE_ENV: &str = "SOLX_CA_FILE";

/// Which roots a solx client trusts for `https://`, beyond the built-in and
/// OS ones.
#[derive(Clone, Default)]
pub struct ClientTls {
    extra_roots: Vec<reqwest::Certificate>,
}

impl std::fmt::Debug for ClientTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientTls")
            .field("extra_roots", &self.extra_roots.len())
            .finish()
    }
}

impl ClientTls {
    /// The roots in `ca_file` (normally `server_ca_file`) and in the file
    /// `SOLX_CA_FILE` names, when set. A file that can't be read, or holds
    /// no PEM certificate, is an error rather than silently trusting less.
    pub fn load(ca_file: Option<&Path>) -> Result<Self, SolxError> {
        let mut tls = ClientTls::default();
        if let Some(path) = ca_file {
            tls.add_ca_file(path)?;
        }
        if let Some(path) = std::env::var_os(CA_FILE_ENV).filter(|p| !p.is_empty()) {
            tls.add_ca_file(Path::new(&path))
                .map_err(|e| SolxError::Config(format!("{CA_FILE_ENV}: {e}")))?;
        }
        Ok(tls)
    }

    /// Trust the PEM certificates in `path` as roots too.
    pub fn add_ca_file(&mut self, path: &Path) -> Result<(), SolxError> {
        let pem = std::fs::read(path)
            .map_err(|e| SolxError::Config(format!("read CA file {}: {e}", path.display())))?;
        self.add_pem(&pem)
            .map_err(|e| SolxError::Config(format!("CA file {}: {e}", path.display())))
    }

    /// Trust the PEM certificates in `pem` as roots too.
    pub fn add_pem(&mut self, pem: &[u8]) -> Result<(), SolxError> {
        let certs = reqwest::Certificate::from_pem_bundle(pem)
            .map_err(|e| SolxError::Invalid(format!("not a PEM certificate file: {e}")))?;
        if certs.is_empty() {
            return Err(SolxError::Invalid("holds no PEM certificate".into()));
        }
        self.extra_roots.extend(certs);
        Ok(())
    }
}

/// A client trusting `tls`'s roots plus the built-in and OS ones. Build one
/// per process (or per connection setup) and share it: it pools
/// connections, and loading the OS roots isn't free.
pub fn http_client(tls: &ClientTls) -> Result<reqwest::Client, SolxError> {
    let mut builder = reqwest::Client::builder().use_rustls_tls();
    for root in &tls.extra_roots {
        builder = builder.add_root_certificate(root.clone());
    }
    builder
        .build()
        .map_err(|e| SolxError::Config(format!("could not set up the HTTP client: {e}")))
}

/// [`http_client`] trusting the roots in `ca_file` (normally
/// `server_ca_file`) and `SOLX_CA_FILE` too: see [`ClientTls::load`].
pub fn http_client_for(ca_file: Option<&Path>) -> Result<reqwest::Client, SolxError> {
    http_client(&ClientTls::load(ca_file)?)
}

/// [`http_client`] for roots from `SOLX_CA_FILE` only, for the
/// constructors that take no client. A bad `SOLX_CA_FILE` is logged and
/// left out, since those constructors can't fail.
pub(crate) fn default_client() -> reqwest::Client {
    let tls = ClientTls::load(None).unwrap_or_else(|e| {
        tracing::warn!("{e}; trusting only the built-in and OS roots");
        ClientTls::default()
    });
    http_client(&tls).unwrap_or_else(|e| {
        tracing::warn!("{e}; using reqwest's defaults");
        reqwest::Client::new()
    })
}

#[cfg(test)]
mod tests {
    use super::ClientTls;

    #[test]
    fn a_ca_file_must_hold_a_certificate() {
        let mut tls = ClientTls::default();
        assert!(tls.add_pem(b"").is_err());
        assert!(tls.add_pem(b"not a certificate").is_err());

        let missing = std::path::Path::new("definitely/not/here.crt");
        let err = ClientTls::load(Some(missing)).unwrap_err().to_string();
        assert!(err.contains("here.crt"), "{err}");
    }
}
