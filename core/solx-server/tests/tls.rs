//! TLS serving: `server_tls` certificates from a `solx-certs` directory (the
//! file backend, so nothing touches the credential manager or the Windows
//! store), HTTPS over HTTP/2 and HTTP/1.1, plain HTTP from loopback on the
//! same port, the peer address reaching `require_bearer` through TLS, and
//! graceful shutdown. The refusal of plain HTTP from other machines is a
//! unit test of its predicate (`tls::plain_http_allowed`), since every peer
//! here is on loopback.
#![cfg(feature = "tls")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use axum::Router;
use solx_certs::{Authority, BackendKind, CreateOptions, FileProtection, San};
use solx_server::state::AppState;
use tokio::sync::oneshot;

struct Host {
    dir: tempfile::TempDir,
    app: Arc<solx_manager::App>,
    root_token: String,
    root_pem: Vec<u8>,
    /// What clients pin for this CA.
    pin: String,
}

/// An app whose appdata holds a file-backend CA and a server certificate
/// for `127.0.0.1` and `localhost`. `server_tls` is left unset.
async fn host() -> Host {
    let dir = tempfile::tempdir().unwrap();
    // Never touch the real OS credential manager from a test.
    solx_config::credentials::use_memory_store_for_tests();
    let options = CreateOptions {
        backend: BackendKind::File,
        file_protection: FileProtection::Static([7u8; 32]),
        ..CreateOptions::default()
    };
    let mut ca = Authority::create(dir.path().join("certs"), &options).unwrap();
    let sans: Vec<San> = ["127.0.0.1", "localhost"]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    ca.issue_server(&sans, 30).unwrap();
    let root_pem = std::fs::read(ca.root_cert_path()).unwrap();
    let pin = ca.pin().to_string();

    let app = solx_manager::App::build_local_in(dir.path()).await.unwrap();
    let root_token = solx_config::credentials::root_token(&app.config).unwrap();
    Host {
        dir,
        app,
        root_token,
        root_pem,
        pin,
    }
}

/// `solx_server::serve` on an ephemeral loopback port, as `server_tls`
/// says. The sender stops it.
async fn serve(host: &Host) -> std::io::Result<(SocketAddr, oneshot::Sender<()>)> {
    let (tx, rx) = oneshot::channel::<()>();
    let state = AppState::new(host.app.clone()).unwrap();
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound = solx_server::serve(
        state,
        addr,
        solx_server::cors_layer(addr.ip(), &[]),
        async move {
            let _ = rx.await;
        },
    )
    .await?;
    Ok((bound, tx))
}

/// A client that trusts only `root_pem` (when given).
fn client(root_pem: Option<&[u8]>, http1_only: bool) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false);
    if let Some(pem) = root_pem {
        builder = builder.add_root_certificate(reqwest::Certificate::from_pem(pem).unwrap());
    }
    if http1_only {
        builder = builder.http1_only();
    }
    builder.build().unwrap()
}

#[tokio::test]
async fn auto_serves_https_over_h2_and_http1_with_the_peer_address() {
    let host = host().await;
    host.app.config.set("server_tls", "auto".into()).unwrap();
    let (bound, _stop) = serve(&host).await.unwrap();

    for (http1_only, version) in [
        (false, reqwest::Version::HTTP_2),
        (true, reqwest::Version::HTTP_11),
    ] {
        let http = client(Some(&host.root_pem), http1_only);
        let health = http
            .get(format!("https://127.0.0.1:{}/health", bound.port()))
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), 200);
        assert_eq!(health.version(), version);

        // The root token is accepted only from a loopback peer, so this
        // passing shows the peer address survived the TLS accept loop.
        let docs = http
            .get(format!("https://localhost:{}/docs", bound.port()))
            .bearer_auth(&host.root_token)
            .send()
            .await
            .unwrap();
        assert_eq!(docs.status(), 200, "{}", docs.text().await.unwrap());
    }
}

#[tokio::test]
async fn plain_http_from_loopback_shares_the_port() {
    let host = host().await;
    host.app.config.set("server_tls", "auto".into()).unwrap();
    let (bound, _stop) = serve(&host).await.unwrap();

    let http = reqwest::Client::new();
    let docs = http
        .get(format!("http://{bound}/docs"))
        .bearer_auth(&host.root_token)
        .send()
        .await
        .unwrap();
    assert_eq!(docs.status(), 200);
}

#[tokio::test]
async fn a_client_that_doesnt_trust_the_root_is_refused() {
    let host = host().await;
    host.app.config.set("server_tls", "auto".into()).unwrap();
    let (bound, _stop) = serve(&host).await.unwrap();

    let err = client(None, false)
        .get(format!("https://127.0.0.1:{}/health", bound.port()))
        .send()
        .await
        .unwrap_err();
    assert!(err.is_connect(), "{err:?}");
}

#[tokio::test]
async fn pem_files_serve_too() {
    let host = host().await;
    // Relative paths are under the appdata dir.
    host.app
        .config
        .set(
            "server_tls",
            serde_json::json!({
                "cert_file": "certs/solx-server.crt",
                "key_file": "certs/solx-server.key",
            }),
        )
        .unwrap();
    let (bound, _stop) = serve(&host).await.unwrap();

    let health = client(Some(&host.root_pem), false)
        .get(format!("https://127.0.0.1:{}/health", bound.port()))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
}

#[tokio::test]
async fn auto_without_certificates_fails_before_binding() {
    let host = host().await;
    std::fs::remove_dir_all(host.dir.path().join("certs")).unwrap();
    host.app.config.set("server_tls", "auto".into()).unwrap();
    let err = serve(&host).await.unwrap_err();
    assert!(err.to_string().contains("solx-certs init"), "{err}");
}

#[tokio::test]
async fn shutdown_drains_requests_in_flight() {
    let host = host().await;
    host.app.config.set("server_tls", "auto".into()).unwrap();
    let tls = solx_server::tls_config(&host.app.config).unwrap();
    assert!(tls.is_some());
    let router = Router::new().route(
        "/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            "done"
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = oneshot::channel::<()>();
    let server = tokio::spawn(solx_server::serve_listener(
        listener,
        router,
        tls,
        async move {
            let _ = rx.await;
        },
    ));

    let http = client(Some(&host.root_pem), false);
    let request = tokio::spawn({
        let http = http.clone();
        async move {
            http.get(format!("https://127.0.0.1:{port}/slow"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    tx.send(()).unwrap();

    assert_eq!(request.await.unwrap(), "done");
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the server stops once drained")
        .unwrap()
        .unwrap();
    // No longer accepting.
    assert!(client(Some(&host.root_pem), false)
        .get(format!("https://127.0.0.1:{port}/slow"))
        .send()
        .await
        .is_err());
}

/// The Windows store path: a CNG CA (software provider, so it's quick and
/// needs no TPM) whose server key never leaves CNG, served by thumbprint
/// through `"auto"` and explicitly. The keys and store entries are deleted
/// afterwards.
#[cfg(windows)]
#[tokio::test]
async fn a_cng_certificate_is_served_from_the_store() {
    use solx_certs::Provider;

    let host = host().await;
    let certs = host.dir.path().join("cng-certs");
    let options = CreateOptions {
        backend: BackendKind::Cng,
        root_provider: Provider::Software,
        server_provider: Provider::Software,
        ..CreateOptions::default()
    };
    let mut ca = Authority::create(&certs, &options).unwrap();
    let sans: Vec<San> = vec!["127.0.0.1".parse().unwrap()];
    let thumbprint = ca.issue_server(&sans, 30).unwrap().sha256.clone();
    let root_pem = std::fs::read(ca.root_cert_path()).unwrap();

    let result = async {
        // `"auto"` serves the leaf followed by the root from solx-root.crt.
        let source = solx_server::tls::source_from_certs_dir(&certs).unwrap();
        assert!(matches!(source, solx_server::tls::TlsSource::Store { .. }));
        host.app
            .config
            .set(
                "server_tls",
                serde_json::json!({ "store_thumbprint": thumbprint }),
            )
            .unwrap();
        let (bound, _stop) = serve(&host).await.unwrap();
        client(Some(&root_pem), false)
            .get(format!("https://127.0.0.1:{}/health", bound.port()))
            .send()
            .await
            .map(|r| r.status())
    }
    .await;
    ca.delete().unwrap();
    assert_eq!(result.unwrap(), 200);
}

/// A remote `App` (what solx-cli and the desktop client build) reaches an
/// https:// server whose root `server_ca_file` names, and refuses it
/// without.
#[tokio::test]
async fn a_remote_app_trusts_its_server_ca_file() {
    use solx_surface::managers::Solx;
    use solx_surface::query::ListOptions;

    let host = host().await;
    host.app.config.set("server_tls", "auto".into()).unwrap();
    let (bound, _stop) = serve(&host).await.unwrap();
    let url = format!("https://127.0.0.1:{}", bound.port());

    let client_dir = tempfile::tempdir().unwrap();
    let config = Arc::new(solx_config::ConfigService::open_in(client_dir.path()).unwrap());
    let list = |app: Arc<solx_manager::App>| async move {
        app.docs()
            .list(ListOptions {
                limit: Some(1),
                ..Default::default()
            })
            .await
    };

    let untrusting =
        solx_manager::App::remote(config.clone(), url.clone(), host.root_token.clone()).unwrap();
    assert!(list(untrusting).await.is_err());

    let root = host.dir.path().join("certs").join("solx-root.crt");
    config
        .set("server_ca_file", root.display().to_string().into())
        .unwrap();
    let trusting = solx_manager::App::remote(config, url, host.root_token.clone()).unwrap();
    list(trusting).await.unwrap();
}

// ── pinning ──────────────────────────────────────────────────────────────────

/// List one document through a remote `App` for `url` (which may carry a
/// `#pin=`), from a fresh client appdata dir with no `server_ca_file`.
async fn remote_list(url: &str, token: &str) -> Result<(), solx_surface::error::SolxError> {
    use solx_surface::managers::Solx;
    use solx_surface::query::ListOptions;

    let client_dir = tempfile::tempdir().unwrap();
    let config = Arc::new(solx_config::ConfigService::open_in(client_dir.path()).unwrap());
    let app = solx_manager::App::remote(config, url.to_string(), token.to_string()).unwrap();
    app.docs()
        .list(ListOptions {
            limit: Some(1),
            ..Default::default()
        })
        .await
        .map(|_| ())
}

#[tokio::test]
async fn the_client_computes_the_pin_solx_certs_records() {
    use rustls_pki_types::pem::PemObject;

    let host = host().await;
    let root = rustls_pki_types::CertificateDer::from_pem_slice(&host.root_pem).unwrap();
    assert_eq!(
        solx_client::pin_of_cert(&root).as_deref(),
        Some(host.pin.as_str())
    );
}

/// The right pin is all a client needs; any other root, even one the
/// client would otherwise trust, gives the mismatch error.
#[tokio::test]
async fn a_pinned_client_trusts_only_its_pin() {
    let host = host().await;
    host.app.config.set("server_tls", "auto".into()).unwrap();
    let (bound, _stop) = serve(&host).await.unwrap();
    let url = format!("https://127.0.0.1:{}", bound.port());

    remote_list(&format!("{url}#pin={}", host.pin), &host.root_token)
        .await
        .unwrap();

    let wrong = format!("{url}#pin={}", "A".repeat(43));
    let err = remote_list(&wrong, &host.root_token).await.unwrap_err();
    assert!(
        err.to_string().contains("certificate authority changed"),
        "{err}"
    );

    // Unpinned and untrusted: refused, saying what to do.
    let err = remote_list(&url, &host.root_token).await.unwrap_err();
    assert!(err.to_string().contains("connect link"), "{err}");
}

/// Connect codes carry the pin; with the wrong pin the code never reaches
/// the server, so it still works with the right one, and the exchange
/// reports the pin.
#[tokio::test]
async fn a_wrong_pin_never_sends_the_code() {
    let host = host().await;
    host.app.config.set("server_tls", "auto".into()).unwrap();
    let (bound, _stop) = serve(&host).await.unwrap();
    let url = format!("https://127.0.0.1:{}", bound.port());

    let registry = host.app.registry().unwrap();
    let code = registry
        .create_connect_code(None, None, None, Some(&url))
        .await
        .unwrap();
    assert_eq!(code.tls_pin.as_deref(), Some(host.pin.as_str()));
    let link = code.url.unwrap();
    assert!(link.ends_with(&format!("&pin={}", host.pin)), "{link}");

    let parsed = solx_config::credentials::parse_connect_input(None, &link).unwrap();
    assert_eq!(parsed.pin.as_deref(), Some(host.pin.as_str()));

    let wrong = solx_client::http_client_for(None, Some(&"A".repeat(43))).unwrap();
    assert!(solx_client::exchange_connect_code(&wrong, &url, &code.code)
        .await
        .is_err());

    let right = solx_client::http_client_for(None, parsed.pin.as_deref()).unwrap();
    let exchanged = solx_client::exchange_connect_code(&right, &url, &code.code)
        .await
        .unwrap();
    assert_eq!(exchanged.tls_pin.as_deref(), Some(host.pin.as_str()));
    assert_eq!(
        exchanged.adopted_pin(&url).as_deref(),
        Some(host.pin.as_str())
    );
    remote_list(&format!("{url}#pin={}", host.pin), &exchanged.token)
        .await
        .unwrap();
}
