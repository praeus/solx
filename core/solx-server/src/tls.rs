//! TLS for `solx-server`: turning the `server_tls` config key into a
//! `rustls::ServerConfig`, and the accept loop that serves it.
//!
//! One port carries both protocols. Each accepted connection's first byte
//! is peeked: `0x16` (a TLS handshake record) is served over TLS; anything
//! else is plain HTTP, which is accepted only from a loopback peer. So
//! Command children (`SOLX_SERVER_URL` stays `http://127.0.0.1:…`), the CLI
//! and the desktop app's `/health` probe keep working unchanged, while the
//! network only ever gets TLS. See `docs/tls-plan.md`.
//!
//! Certificates come from:
//! - **files**: a PEM chain and its PEM private key (PKCS#8, SEC1 or
//!   PKCS#1), for example from `solx-certs --backend file` or OpenSSL;
//! - **the Windows store**: a certificate in `CurrentUser\My`, found by
//!   SHA-256 thumbprint, whose non-exportable key signs each handshake
//!   through CNG (`rustls-cng`). No private key is ever read;
//! - **`"auto"`**: whichever of those `<appdata>/certs/tls.json` records
//!   (made by `solx-certs`, or by the desktop app).

use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use axum::extract::ConnectInfo;
use axum::Router;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::{GracefulShutdown, Watcher};
use rustls::crypto::CryptoProvider;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::ServerConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use solx_config::{ConfigService, ServerTls};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

/// How long a new connection gets to send its first byte and, for TLS,
/// finish the handshake. A peer that stalls only holds its own task, but
/// it shouldn't hold it forever.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// The first byte of a TLS record carrying a handshake (a ClientHello).
const TLS_HANDSHAKE: u8 = 0x16;

/// What a plain-HTTP peer that isn't on this machine is told.
const HTTPS_REQUIRED: &str =
    "this solx-server requires https:// from other machines; plain HTTP is only accepted from \
     the server's own machine.\n";

/// Where a server certificate and its key come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsSource {
    /// A PEM certificate chain (the server certificate first) and its PEM
    /// private key.
    Files {
        cert_file: PathBuf,
        key_file: PathBuf,
    },
    /// Windows: the certificate in `CurrentUser\My` with this SHA-256
    /// thumbprint (lowercase hex), its key used where it is. `extra_chain`
    /// is appended after the certificates Windows can chain to it, for a
    /// root that the store's chain building leaves out.
    Store {
        sha256: String,
        extra_chain: Vec<CertificateDer<'static>>,
    },
}

/// The [`TlsSource`] `config`'s `server_tls` names, or `None` when it is
/// off. `"auto"` reads `<appdata>/certs/tls.json` and fails if there is no
/// server certificate there yet.
pub fn source_from_config(config: &ConfigService) -> anyhow::Result<Option<TlsSource>> {
    Ok(match config.server_tls()? {
        ServerTls::Off => None,
        ServerTls::Files {
            cert_file,
            key_file,
        } => Some(TlsSource::Files {
            cert_file,
            key_file,
        }),
        ServerTls::Store { sha256 } => Some(TlsSource::Store {
            sha256,
            extra_chain: Vec::new(),
        }),
        ServerTls::Auto => Some(source_from_certs_dir(&solx_certs::certs_dir(config))?),
    })
}

/// The server certificate a `solx-certs` directory holds, as a
/// [`TlsSource`]. What `server_tls: "auto"` serves.
pub fn source_from_certs_dir(dir: &Path) -> anyhow::Result<TlsSource> {
    use solx_certs::state::{BackendKind, KeyRef, ROOT_CERT_FILE, SERVER_CERT_FILE};

    let state = solx_certs::TlsState::load(dir)?.ok_or_else(|| {
        anyhow!(
            "server_tls is \"auto\", but {} has no certificate authority yet. Create one with \
             `solx-certs init --dir \"{}\"`, or let the desktop app provision it.",
            dir.display(),
            dir.display()
        )
    })?;
    let server = state.server.as_ref().ok_or_else(|| {
        anyhow!(
            "server_tls is \"auto\", but {} has no server certificate. Issue one with \
             `solx-certs issue --dir \"{}\"`.",
            dir.display(),
            dir.display()
        )
    })?;
    let left = server.not_after - chrono::Utc::now();
    if left <= chrono::Duration::zero() {
        tracing::warn!(
            "the server certificate in {} expired on {}; clients will refuse it. Reissue it \
             with `solx-certs issue`.",
            dir.display(),
            server.not_after
        );
    } else if left < chrono::Duration::days(solx_certs::RENEW_WITHIN_DAYS) {
        tracing::warn!(
            "the server certificate in {} expires on {}; reissue it with `solx-certs issue`.",
            dir.display(),
            server.not_after
        );
    }
    match (state.backend, &server.key) {
        (BackendKind::File, KeyRef::File { path, wrapped }) => {
            if *wrapped {
                bail!(
                    "the server key in {} is wrapped, so it can't be served; reissue the server \
                     certificate with `solx-certs issue`",
                    dir.display()
                );
            }
            Ok(TlsSource::Files {
                cert_file: dir.join(SERVER_CERT_FILE),
                key_file: dir.join(path),
            })
        }
        (BackendKind::Cng, KeyRef::Cng { .. }) => {
            let root = CertificateDer::from_pem_file(dir.join(ROOT_CERT_FILE))
                .with_context(|| format!("read {}", dir.join(ROOT_CERT_FILE).display()))?;
            Ok(TlsSource::Store {
                sha256: server.sha256.clone(),
                extra_chain: vec![root],
            })
        }
        (backend, key) => bail!(
            "{} is inconsistent: a {} backend with a server key {key:?}",
            dir.join(solx_certs::state::STATE_FILE).display(),
            backend.as_str()
        ),
    }
}

/// A `rustls` server configuration for `source`: TLS 1.2 and 1.3 with the
/// `ring` provider, offering HTTP/2 and HTTP/1.1 over ALPN.
pub fn server_config(source: &TlsSource) -> anyhow::Result<Arc<ServerConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .context("TLS protocol versions")?
        .with_no_client_auth();
    let mut config = match source {
        TlsSource::Files {
            cert_file,
            key_file,
        } => {
            let (chain, key) = load_files(cert_file, key_file)?;
            builder.with_single_cert(chain, key).with_context(|| {
                format!(
                    "{} and {} don't make a usable certificate and key",
                    cert_file.display(),
                    key_file.display()
                )
            })?
        }
        TlsSource::Store {
            sha256,
            extra_chain,
        } => builder.with_cert_resolver(Arc::new(Fixed(Arc::new(store_key(
            sha256,
            extra_chain,
            &provider,
        )?)))),
    };
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

fn load_files(
    cert_file: &Path,
    key_file: &Path,
) -> anyhow::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let chain = CertificateDer::pem_file_iter(cert_file)
        .with_context(|| format!("read {}", cert_file.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("{} is not a PEM certificate file", cert_file.display()))?;
    if chain.is_empty() {
        bail!("{} holds no PEM certificate", cert_file.display());
    }
    let key = PrivateKeyDer::from_pem_file(key_file).with_context(|| {
        format!(
            "{} is not a readable PEM private key (PKCS#8, SEC1 or PKCS#1)",
            key_file.display()
        )
    })?;
    Ok((chain, key))
}

#[cfg(windows)]
fn store_key(
    sha256: &str,
    extra_chain: &[CertificateDer<'static>],
    _provider: &CryptoProvider,
) -> anyhow::Result<CertifiedKey> {
    use rustls_cng::signer::CngSigningKey;
    use rustls_cng::store::{CertStore, CertStoreType};

    // Checked first so a bad thumbprint from tls.json can't panic the slicing.
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("{sha256:?} is not a hex SHA-256 thumbprint");
    }
    let hash = (0..sha256.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&sha256[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| anyhow!("{sha256:?} is not a hex SHA-256 thumbprint"))?;
    let store = CertStore::open(CertStoreType::CurrentUser, "My")
        .map_err(|e| anyhow!("open the CurrentUser\\My certificate store: {e}"))?;
    let cert = store
        .find_by_sha256(&hash)
        .map_err(|e| anyhow!("search CurrentUser\\My: {e}"))?
        .into_iter()
        .next()
        .ok_or_else(|| {
            anyhow!("CurrentUser\\My has no certificate with SHA-256 thumbprint {sha256}")
        })?;
    let key = cert.acquire_key(true).map_err(|e| {
        anyhow!("the certificate {sha256} in CurrentUser\\My has no usable private key: {e}")
    })?;
    let key = CngSigningKey::new(key)
        .map_err(|e| anyhow!("the key of certificate {sha256} can't sign for TLS: {e}"))?;
    // Windows builds the chain from what its stores hold, leaving out a
    // self-signed root; `extra_chain` adds what it couldn't (or wouldn't).
    let mut chain: Vec<CertificateDer<'static>> = cert
        .as_chain_der()
        .unwrap_or_else(|_| vec![cert.as_der().to_vec()])
        .into_iter()
        .map(CertificateDer::from)
        .collect();
    for extra in extra_chain {
        if !chain.contains(extra) {
            chain.push(extra.clone());
        }
    }
    Ok(CertifiedKey::new(chain, Arc::new(key)))
}

#[cfg(not(windows))]
fn store_key(
    _sha256: &str,
    _extra_chain: &[CertificateDer<'static>],
    _provider: &CryptoProvider,
) -> anyhow::Result<CertifiedKey> {
    bail!(
        "server_tls store_thumbprint needs the Windows certificate store; use cert_file and \
         key_file on this platform"
    )
}

/// Always the same certificate, whatever the client asks for.
#[derive(Debug)]
struct Fixed(Arc<CertifiedKey>);

impl ResolvesServerCert for Fixed {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}

/// Whether a connection that didn't start a TLS handshake may be served as
/// plain HTTP: only a peer on this machine. (An IPv4 peer on a dual-stack
/// socket shows up as `::ffff:127.0.0.1`, hence `to_canonical`.)
pub fn plain_http_allowed(peer: SocketAddr) -> bool {
    peer.ip().to_canonical().is_loopback()
}

/// Serve `router` on `listener` until `shutdown` resolves, then stop
/// accepting and wait for in-flight connections to finish. Each
/// connection's handlers see its peer as `ConnectInfo<SocketAddr>`, as
/// with `into_make_service_with_connect_info`.
///
/// With `tls`, a connection that opens with a TLS handshake is served over
/// TLS (HTTP/2 or HTTP/1.1, by ALPN); one that doesn't is served as plain
/// HTTP only when [`plain_http_allowed`], and otherwise gets a 400 saying
/// https:// is required. Without `tls`, this is `axum::serve`.
pub async fn serve_listener(
    listener: TcpListener,
    router: Router,
    tls: Option<Arc<ServerConfig>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let Some(tls) = tls else {
        return axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown)
        .await;
    };
    let acceptor = TlsAcceptor::from(tls);
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(e) => {
                    if !is_connection_error(&e) {
                        // Out of file descriptors and the like: back off
                        // rather than spin, as `axum::serve` does.
                        tracing::warn!("solx-server: accept failed: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        tokio::spawn(serve_connection(
            stream,
            peer,
            router.clone(),
            acceptor.clone(),
            graceful.watcher(),
        ));
    }
    drop(listener);
    graceful.shutdown().await;
    Ok(())
}

async fn serve_connection(
    stream: TcpStream,
    peer: SocketAddr,
    router: Router,
    acceptor: TlsAcceptor,
    watcher: Watcher,
) {
    let mut first = [0u8; 1];
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut first)).await {
        Ok(Ok(1)) => {}
        // Closed, failed, or silent: nothing to serve.
        _ => return,
    }
    let service = hyper::service::service_fn(move |mut request: hyper::Request<Incoming>| {
        request.extensions_mut().insert(ConnectInfo(peer));
        router.clone().call(request)
    });
    let builder = Builder::new(TokioExecutor::new());
    let result = if first[0] == TLS_HANDSHAKE {
        let stream = match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => {
                tracing::debug!("solx-server: TLS handshake with {peer} failed: {e}");
                return;
            }
            Err(_) => return,
        };
        let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
        watcher.watch(connection.into_owned()).await
    } else if plain_http_allowed(peer) {
        let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
        watcher.watch(connection.into_owned()).await
    } else {
        refuse_plain(stream).await;
        return;
    };
    if let Err(e) = result {
        tracing::debug!("solx-server: connection from {peer} ended: {e}");
    }
}

/// Tell a plain-HTTP peer from another machine to use https://, then
/// close. The request is read first (as much as has arrived), so closing
/// with it unread doesn't reset the connection before the reply lands.
async fn refuse_plain(mut stream: TcpStream) {
    let mut buf = [0u8; 8192];
    let _ = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
    let response = format!(
        "HTTP/1.1 400 Bad Request\r\ncontent-type: text/plain; charset=utf-8\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{HTTPS_REQUIRED}",
        HTTPS_REQUIRED.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn is_connection_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

#[cfg(test)]
mod tests {
    use super::plain_http_allowed;

    #[test]
    fn plain_http_is_for_loopback_peers_only() {
        for ok in ["127.0.0.1:5000", "[::1]:5000", "[::ffff:127.0.0.1]:5000"] {
            assert!(plain_http_allowed(ok.parse().unwrap()), "{ok}");
        }
        for refused in [
            "192.168.1.20:5000",
            "[fd7a::1]:5000",
            "[::ffff:10.0.0.2]:5000",
        ] {
            assert!(!plain_http_allowed(refused.parse().unwrap()), "{refused}");
        }
    }
}
