//! Server build: solx-core in-process, shared between the UI (over IPC) and
//! an optional embedded solx-server (over HTTP) — one `Arc<App>`, so both
//! see the same managers, caches and running actions.
//!
//! Startup mirrors the standalone `solx-server` binary (`main.rs` there),
//! plus the "is a solx host already live?" check `solx-cli`'s `build_app`
//! does: two processes must never open the same appdata databases.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use solx_config::{ConfigService, ServerTls};
use solx_server::state::AppState;
use solx_tauri_plugin::{EmbeddedServer, SolxState};
use tauri::{App, AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

/// Stops the embedded server (gracefully) when the app exits.
struct Shutdown(Mutex<Option<oneshot::Sender<()>>>);

/// The local `App`, when this process built one: its background commands
/// stop when the app exits.
struct LocalApp(Arc<solx_manager::App>);

pub fn setup(app: &mut App) -> anyhow::Result<()> {
    // Detached (`action-start`) invocations need a host that outlives the
    // call that started them — true of this app, as of `solx-server`.
    solx_actions::set_long_lived_host(true);
    let handle = app.handle().clone();
    tauri::async_runtime::block_on(start(&handle))
}

async fn start(handle: &AppHandle) -> anyhow::Result<()> {
    let config = Arc::new(solx_config::ConfigService::open().context("open solx config")?);
    let snap = config.snapshot();
    let port = snap.server_port.unwrap_or(solx_config::DEFAULT_SERVER_PORT);

    // After `restart_app`, the previous process may still be serving this
    // port for a moment; don't mistake it for another host and go remote.
    if std::env::var_os(crate::RESTART_ENV).is_some() {
        std::env::remove_var(crate::RESTART_ENV);
        for _ in 0..50 {
            if !solx_host_running(port).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    // Another solx host (solx-server, a solx-cli session, another copy of
    // this app) already owns this appdata dir: use it rather than open the
    // same libsql databases a second time. Same token, same appdata.
    if solx_host_running(port).await {
        let url = format!("http://127.0.0.1:{port}");
        eprintln!("solx: a solx host is already serving {url}; connecting to it instead of opening local storage");
        // A client of that host, so never mint a token it wouldn't accept.
        let token = solx_config::credentials::client_token(&config, &url)
            .with_context(|| format!("no token saved for {url} (run `solx token show`)"))?;
        let app = solx_manager::App::remote(config, url, token)?;
        handle.plugin(solx_tauri_plugin::local(app))?;
        return Ok(());
    }

    // Always local, like solx-server: never proxy to a stray `server_url`.
    let app = solx_manager::App::build_local().await.context("build local solx App")?;
    handle.plugin(solx_tauri_plugin::local(app.clone()))?;
    handle.manage(LocalApp(app.clone()));

    if !app.config.server_embedded() {
        return Ok(());
    }
    let addr = solx_server::bind_addr(&snap, None, None)?;
    let cors = solx_server::cors_layer(addr.ip(), snap.server_cors_origins.as_deref().unwrap_or_default());
    let tls = match server_tls(&app.config, addr) {
        Ok(tls) => tls,
        // Not served at all rather than served without the TLS it was
        // configured for. Settings shows why.
        Err(e) => {
            let error = format!("{e:#}");
            eprintln!("solx: not starting the embedded solx-server: {error}");
            handle.state::<SolxState>().set_server_error(Some(error));
            return Ok(());
        }
    };
    let https = tls.is_some();
    let (tx, rx) = oneshot::channel::<()>();
    let state = AppState::new(app)?;
    match solx_server::serve_with_tls(state, addr, cors, tls, async move {
        let _ = rx.await;
    })
    .await
    {
        Ok(bound) => {
            let scheme = if https { "https" } else { "http" };
            eprintln!("solx: embedded solx-server listening on {scheme}://{bound}");
            handle
                .state::<SolxState>()
                .set_embedded(Some(EmbeddedServer { url: format!("http://{bound}"), https }));
            handle.manage(Shutdown(Mutex::new(Some(tx))));
        }
        // Not fatal: the UI works over IPC regardless. Only HTTP clients
        // (and Command actions that call back into solx-server) lose out.
        Err(e) => {
            eprintln!("solx: could not start embedded solx-server on {addr}: {e}");
            handle.state::<SolxState>().set_server_error(Some(format!("could not listen on {addr}: {e}")));
        }
    }
    Ok(())
}

/// The TLS the embedded server should serve, provisioning certificates for
/// `server_tls: "auto"`: the CA is created on first use and the server
/// certificate reissued when it's missing, near expiry, or its names no
/// longer match the bind address and `server_public_url`.
///
/// On Windows, where the keys stay non-exportable in the OS key store, a
/// network bind with `server_tls` unset turns it on: `"auto"` is written to
/// solx-config.json, so Settings, connect links and other solx processes see
/// it too, and setting `"off"` opts out. A loopback bind stays plain.
fn server_tls(config: &ConfigService, addr: SocketAddr) -> anyhow::Result<Option<solx_server::TlsConfig>> {
    let unset = config.raw_snapshot().get("server_tls").is_none_or(|v| v.is_null());
    if unset && cfg!(windows) && !addr.ip().is_loopback() {
        config.set("server_tls", "auto".into()).context("turn on server_tls")?;
        eprintln!("solx: serving on {addr}, so TLS is on (server_tls: \"auto\"); set it to \"off\" to opt out");
    }
    if config.server_tls()? != ServerTls::Auto {
        return solx_server::tls_config(config);
    }
    let (ca, issued) = solx_certs::provision(config).context("provision TLS certificates")?;
    if issued {
        eprintln!("solx: issued a server certificate in {}", ca.dir().display());
    }
    let source = solx_server::tls::source_from_certs_dir(ca.dir())?;
    Ok(Some(solx_server::tls::server_config(&source)?))
}

/// On the way out: stop the embedded server, and every background command
/// this process runs. Blocks for up to `stop_grace_secs` while they exit,
/// so call it from the main thread, never from inside the async runtime.
pub fn shutdown(handle: &AppHandle) {
    if let Some(shutdown) = handle.try_state::<Shutdown>() {
        if let Some(tx) = shutdown.0.lock().unwrap().take() {
            let _ = tx.send(());
        }
    }
    if let Some(app) = handle.try_state::<LocalApp>() {
        tauri::async_runtime::block_on(app.0.shutdown());
    }
}

/// Whether a solx host answers `GET /health` on loopback `port`. A raw
/// HTTP/1.0 request rather than an HTTP client dependency, since this is
/// the only HTTP this crate makes itself.
async fn solx_host_running(port: u16) -> bool {
    let probe = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        stream
            .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .ok()?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.ok()?;
        let text = String::from_utf8_lossy(&buf);
        Some(text.starts_with("HTTP/1.1 200") && text.contains("\"status\":\"ok\""))
    };
    tokio::time::timeout(std::time::Duration::from_millis(500), probe)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}
