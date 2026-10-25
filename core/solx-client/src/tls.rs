//! The one HTTP client a solx client process uses, and the roots it trusts
//! for `https://` servers.
//!
//! Always rustls (reqwest's default features would otherwise pick the
//! platform's TLS stack). An unpinned client trusts:
//! - the built-in webpki roots, for servers behind a public certificate;
//! - the OS trust store, so a solx root someone imported there (`certutil
//!   -user -addstore Root solx-root.crt`, Keychain Access,
//!   `update-ca-certificates`) is honoured;
//! - extra roots from a PEM file: `server_ca_file` in solx-config.json, or
//!   `SOLX_CA_FILE`, for a server whose root isn't in the OS store.
//!
//! A **pinned** client (a `#pin=` on its server URL, see
//! `solx_config::credentials`) trusts none of those: only a chain the
//! server presents that contains the certificate whose public key the pin
//! names, checked as a root for that one connection. solx servers always
//! send their root, so the pin needs nothing else. See `docs/tls-plan.md`.

use std::error::Error as StdError;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, TrustAnchor, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, OtherError, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use solx_surface::error::SolxError;

/// The environment variable naming a PEM file of extra roots to trust, in
/// addition to `server_ca_file`.
pub const CA_FILE_ENV: &str = "SOLX_CA_FILE";

/// Which roots a solx client trusts for `https://`: beyond the built-in and
/// OS ones, or, with a pin, instead of all of them.
#[derive(Clone, Default)]
pub struct ClientTls {
    extra_roots: Vec<reqwest::Certificate>,
    pin: Option<String>,
}

impl std::fmt::Debug for ClientTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientTls")
            .field("extra_roots", &self.extra_roots.len())
            .field("pin", &self.pin)
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

    /// Trust only the root `pin` names (a `solx_config::credentials` pin),
    /// in place of every other root, when `pin` is given.
    pub fn with_pin(mut self, pin: Option<&str>) -> Self {
        self.pin = pin.map(str::to_string);
        self
    }
}

/// A client trusting `tls`'s roots plus the built-in and OS ones, or only
/// its pinned root. Build one per process (or per connection setup) and
/// share it: it pools connections, and loading the OS roots isn't free.
pub fn http_client(tls: &ClientTls) -> Result<reqwest::Client, SolxError> {
    let setup = |e: &dyn std::fmt::Display| {
        SolxError::Config(format!("could not set up the HTTP client: {e}"))
    };
    let mut builder = reqwest::Client::builder();
    if let Some(pin) = &tls.pin {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| setup(&e))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedRoot {
                pin: pin.clone(),
                provider,
            }))
            .with_no_client_auth();
        // A pin protects nothing over plain HTTP, so refuse it.
        builder = builder.use_preconfigured_tls(config).https_only(true);
    } else {
        builder = builder.use_rustls_tls();
        for root in &tls.extra_roots {
            builder = builder.add_root_certificate(root.clone());
        }
    }
    builder.build().map_err(|e| setup(&e))
}

/// [`http_client`] trusting the roots in `ca_file` (normally
/// `server_ca_file`) and `SOLX_CA_FILE` too (see [`ClientTls::load`]), or
/// only the root `pin` names.
pub fn http_client_for(
    ca_file: Option<&Path>,
    pin: Option<&str>,
) -> Result<reqwest::Client, SolxError> {
    http_client(&ClientTls::load(ca_file)?.with_pin(pin))
}

/// For the constructors that take no client (the Node bindings use them):
/// `base_url` without its fragment, and a client pinned to the `#pin=` it
/// carried, else [`default_client`]. A malformed pin is still a pin: it
/// matches no root, so every request fails rather than going unpinned.
pub(crate) fn client_for_url(base_url: String) -> (String, reqwest::Client) {
    let Some((base, fragment)) = base_url.split_once('#') else {
        return (base_url, default_client());
    };
    let base = base.trim_end_matches('/').to_string();
    match fragment
        .split('&')
        .find_map(|param| param.strip_prefix("pin="))
    {
        Some(pin) => {
            let tls = ClientTls::default().with_pin(Some(pin.trim()));
            // Building a pinned client only fails if ring offers no TLS
            // version, which it always does; never fall back to unpinned.
            let client = http_client(&tls).expect("a pinned HTTP client always builds");
            (base, client)
        }
        None => (base, default_client()),
    }
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

/// The pin of `cert`'s public key: `base64url(SHA-256(SPKI))`, as
/// solx-certs computes it for a root. `None` if `cert` can't be parsed.
pub fn pin_of_cert(cert: &CertificateDer<'_>) -> Option<String> {
    let mut store = RootCertStore::empty();
    store.add(cert.clone().into_owned()).ok()?;
    Some(pin_of_anchor(&store.roots[0]))
}

fn pin_of_anchor(anchor: &TrustAnchor<'_>) -> String {
    // The anchor holds the SPKI's contents; the pin hashes the whole DER
    // SEQUENCE.
    let spki = der_sequence(anchor.subject_public_key_info.as_ref());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(spki))
}

fn der_sequence(contents: &[u8]) -> Vec<u8> {
    let mut out = vec![0x30];
    let len = contents.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(contents);
    out
}

/// Whether `pin` decodes as a pin (32 bytes of base64url).
pub(crate) fn looks_like_pin(pin: &str) -> bool {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(pin)
        .is_ok_and(|bytes| bytes.len() == 32)
}

/// A pin as people compare it: its first 64 bits as grouped uppercase hex
/// (`3F7A-91C2-0B4E-D518`), as solx-certs and Settings show it.
pub fn fingerprint_of_pin(pin: &str) -> String {
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(pin) else {
        return pin.to_string();
    };
    bytes
        .chunks(2)
        .take(4)
        .map(|pair| pair.iter().map(|b| format!("{b:02X}")).collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Accepts a chain only when one of the certificates after the server's
/// own has the pinned public key, and the chain verifies (signatures, host
/// name, validity, usage) with that certificate as its only root.
#[derive(Debug)]
struct PinnedRoot {
    pin: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedRoot {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut presented = None;
        // The root comes last in what a solx server sends.
        for cert in intermediates.iter().rev() {
            let mut roots = RootCertStore::empty();
            if roots.add(cert.clone().into_owned()).is_err() {
                continue;
            }
            let pin = pin_of_anchor(&roots.roots[0]);
            if pin != self.pin {
                presented.get_or_insert(pin);
                continue;
            }
            let verifier =
                WebPkiServerVerifier::builder_with_provider(Arc::new(roots), self.provider.clone())
                    .build()
                    .map_err(|e| rustls::Error::General(e.to_string()))?;
            return verifier.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            );
        }
        Err(rustls::Error::InvalidCertificate(CertificateError::Other(
            OtherError(Arc::new(PinMismatch {
                pinned: self.pin.clone(),
                presented,
            })),
        )))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A pinned server presented a chain without its pinned root.
#[derive(Debug)]
struct PinMismatch {
    pinned: String,
    /// The pin of the last certificate it sent, if it sent more than its
    /// own.
    presented: Option<String>,
}

impl std::fmt::Display for PinMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the server's certificate authority changed: this machine pinned {}, ",
            fingerprint_of_pin(&self.pinned)
        )?;
        match &self.presented {
            Some(pin) => write!(f, "but the server presented {}", fingerprint_of_pin(pin))?,
            None => write!(f, "but the server presented no certificate authority")?,
        }
        write!(
            f,
            ". If the server's certificates were reset, reconnect with a new connect link from \
             it; otherwise this may not be your server."
        )
    }
}

impl StdError for PinMismatch {}

/// Why a request didn't reach the server, as told to a person: a pin
/// mismatch on its own, an untrusted certificate with what to do about it,
/// anything else with its innermost cause.
pub(crate) fn describe_transport_error(e: &reqwest::Error) -> SolxError {
    let causes = causes(e);
    for cause in &causes {
        if let Some(mismatch) = as_pin_mismatch(*cause) {
            return SolxError::Config(mismatch.to_string());
        }
    }
    let untrusted = causes.iter().any(|c| {
        matches!(
            c.downcast_ref::<rustls::Error>(),
            Some(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer
            ))
        )
    });
    let mut message = format!("solx-server request failed: {e}");
    if let Some(last) = causes.last().filter(|c| causes.len() > 1 && c.to_string() != e.to_string())
    {
        message.push_str(&format!(": {last}"));
    }
    if untrusted {
        message.push_str(
            ". This machine doesn't trust the server's certificate authority: connect with a \
             connect link from the server (it carries the server's pin), or set server_ca_file \
             to a copy of its solx-root.crt",
        );
    }
    SolxError::Io(message)
}

fn as_pin_mismatch<'a>(e: &'a (dyn StdError + 'static)) -> Option<&'a PinMismatch> {
    if let Some(mismatch) = e.downcast_ref::<PinMismatch>() {
        return Some(mismatch);
    }
    match e.downcast_ref::<rustls::Error>() {
        Some(rustls::Error::InvalidCertificate(CertificateError::Other(other))) => {
            other.0.downcast_ref::<PinMismatch>()
        }
        _ => None,
    }
}

/// `e` and everything under it. An `io::Error`'s `source` skips the error
/// it wraps, so that is followed through `get_ref` instead.
fn causes<'a>(e: &'a (dyn StdError + 'static)) -> Vec<&'a (dyn StdError + 'static)> {
    let mut out = Vec::new();
    let mut next = Some(e);
    while let Some(err) = next {
        out.push(err);
        next = match err.downcast_ref::<std::io::Error>().and_then(|io| io.get_ref()) {
            Some(inner) => Some(inner as &(dyn StdError + 'static)),
            None => err.source(),
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{client_for_url, der_sequence, fingerprint_of_pin, ClientTls};

    #[test]
    fn a_ca_file_must_hold_a_certificate() {
        let mut tls = ClientTls::default();
        assert!(tls.add_pem(b"").is_err());
        assert!(tls.add_pem(b"not a certificate").is_err());

        let missing = std::path::Path::new("definitely/not/here.crt");
        let err = ClientTls::load(Some(missing)).unwrap_err().to_string();
        assert!(err.contains("here.crt"), "{err}");
    }

    #[tokio::test]
    async fn a_pin_on_the_url_is_taken_off_and_enforced() {
        let pin = "q83vASNFZ4mrze8BI0VniavN7wEjRWeJq83vASNFZ4k";
        let (base, _) = client_for_url(format!("https://h:8766/#pin={pin}"));
        assert_eq!(base, "https://h:8766");
        let (base, _) = client_for_url("https://h:8766".into());
        assert_eq!(base, "https://h:8766");

        // A pinned client never speaks plain HTTP: it fails before any
        // connection is made (port 9 on loopback; nothing is listening).
        let (base, pinned) = client_for_url(format!("http://127.0.0.1:9#pin={pin}"));
        let err = pinned.get(format!("{base}/health")).send().await.unwrap_err();
        assert!(err.is_builder(), "{err:?}");
    }

    #[test]
    fn der_lengths_short_and_long() {
        assert_eq!(der_sequence(&[1, 2]), vec![0x30, 2, 1, 2]);
        let long = der_sequence(&[0; 300]);
        assert_eq!(&long[..4], &[0x30, 0x82, 0x01, 0x2c]);
        assert_eq!(long.len(), 304);
        assert_eq!(
            fingerprint_of_pin("q83vASNFZ4mrze8BI0VniavN7wEjRWeJq83vASNFZ4k"),
            "ABCD-EF01-2345-6789"
        );
    }
}
