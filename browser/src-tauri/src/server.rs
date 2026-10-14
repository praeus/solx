//! Server build: solx-core in-process, shared between the UI (over IPC) and
//! an optional embedded solx-server (over HTTP) — one `Arc<App>`, so both
//! see the same managers, caches and running actions.
//!
//! Startup mirrors the standalone `solx-server` binary (`main.rs` there),
//! plus the "is a solx host already live?" check `solx-cli`'s `build_app`
//! does: two processes must never open the same appdata databases.

use std::sync::{Arc, Mutex};

use anyhow::Context;
use solx_server::state::AppState;
use solx_tauri_plugin::{EmbeddedServer, SolxState};
use tauri::{App, AppHandle, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

/// Stops the embedded server (gracefully) when the app exits.
struct Shutdown(Mutex<Option<oneshot::Sender<()>>>);

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
        let token = solx_config::credentials::root_token(&config).context("ensure server token")?;
        let app = solx_manager::App::remote(config, url, token)?;
        handle.plugin(solx_tauri_plugin::local(app))?;
        return Ok(());
    }

    // Always local, like solx-server: never proxy to a stray `server_url`.
    let app = solx_manager::App::build_local().await.context("build local solx App")?;
    handle.plugin(solx_tauri_plugin::local(app.clone()))?;

    if !app.config.server_embedded() {
        return Ok(());
    }
    let token = solx_config::credentials::root_token(&app.config).context("ensure server token")?;
    let addr = solx_server::bind_addr(&snap, None, None)?;
    let cors = solx_server::cors_layer(addr.ip(), snap.server_cors_origins.as_deref().unwrap_or_default());
    let (tx, rx) = oneshot::channel::<()>();
    let state = AppState::new(app)?;
    match solx_server::serve(state, addr, cors, async move {
        let _ = rx.await;
    })
    .await
    {
        Ok(bound) => {
            eprintln!("solx: embedded solx-server listening on http://{bound}");
            handle
                .state::<SolxState>()
                .set_embedded(Some(EmbeddedServer { url: format!("http://{bound}"), token }));
            handle.manage(Shutdown(Mutex::new(Some(tx))));
        }
        // Not fatal: the UI works over IPC regardless. Only HTTP clients
        // (and Command actions that call back into solx-server) lose out.
        Err(e) => eprintln!("solx: could not start embedded solx-server on {addr}: {e}"),
    }
    Ok(())
}

pub fn shutdown(handle: &AppHandle) {
    if let Some(shutdown) = handle.try_state::<Shutdown>() {
        if let Some(tx) = shutdown.0.lock().unwrap().take() {
            let _ = tx.send(());
        }
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
