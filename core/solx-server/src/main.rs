use std::net::IpAddr;

use anyhow::Context;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use solx_server::state::AppState;

#[derive(Parser)]
#[command(
    name = "solx-server",
    about = "HTTP server hosting the local solx managers",
    version
)]
struct Cli {
    /// Address to bind on. Defaults to config's `server_host`, else
    /// `127.0.0.1`. Use `0.0.0.0` (or a specific interface IP) to serve
    /// other machines.
    #[arg(long)]
    host: Option<IpAddr>,
    /// Port to bind on. Defaults to config's `server_port`, else
    /// `solx_config::DEFAULT_SERVER_PORT`.
    #[arg(long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    // A tokio::spawn'd task (what `action-start` uses) isn't cancelled when
    // an axum handler future drops, so detachment survives client
    // disconnect here — unlike solx-cli, which exits the instant `exec`
    // returns and would kill a just-started invocation before it could be
    // polled. See `docs/async-actions-plan.md` §6.
    solx_actions::set_long_lived_host(true);

    // Always local — must never proxy to itself, even if this appdata
    // dir's config happens to contain a stray `server_url`.
    let app = solx_manager::App::build_local()
        .await
        .context("build local App")?;

    let snap = app.config.snapshot();
    let addr = solx_server::bind_addr(&snap, cli.host, cli.port)?;
    let host = addr.ip();

    let cors = solx_server::cors_layer(
        host,
        snap.server_cors_origins.as_deref().unwrap_or_default(),
    );
    let tls = solx_server::tls_config(&app.config).context("server_tls")?;
    let state = AppState::new(app.clone())?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    solx_server::announce_bound(&state, listener.local_addr()?, tls.is_some());
    let router = solx_server::build_router_with_cors(state, cors);
    let scheme = if tls.is_some() { "https" } else { "http" };
    tracing::info!("solx-server listening on {scheme}://{addr}");
    println!("solx-server listening on {scheme}://{addr}");
    if tls.is_some() {
        println!("Plain http:// is accepted from this machine only.");
    } else if !host.is_loopback() {
        // Off loopback, anyone who can reach the port can attempt auth, and
        // without TLS the bearer token travels in cleartext.
        let warning = "solx-server is reachable from the network over unencrypted HTTP, so \
                       bearer tokens travel in cleartext. Turn on TLS: create certificates \
                       with `solx-certs init` and set server_tls to \"auto\", or keep the \
                       server on a private network such as Tailscale/WireGuard. Connect \
                       other devices with `solx token connect-code`.";
        tracing::warn!("{warning}");
        eprintln!("WARNING: {warning}");
    }

    let (signalled, on_signal) = tokio::sync::oneshot::channel();
    let serve = solx_server::serve_listener(listener, router, tls, async move {
        shutdown_signal().await;
        let _ = signalled.send(());
    });
    // A long-lived stream (a console tail, an SSE client) would hold the
    // drain open indefinitely; cap it.
    let served = tokio::select! {
        served = serve => served,
        () = async {
            let _ = on_signal.await;
            tokio::time::sleep(DRAIN_TIMEOUT).await;
        } => {
            tracing::warn!("open connections didn't close within {DRAIN_TIMEOUT:?}; stopping anyway");
            Ok(())
        }
    };
    // After in-flight requests drain: stop the background commands this
    // host runs, rather than leave them to the job handle.
    app.shutdown().await;
    served.context("serve")?;
    Ok(())
}

/// How long a shutdown waits for open connections to close.
const DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Ctrl-C, or SIGTERM on POSIX (what service managers send).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                term.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
    tracing::info!("solx-server shutting down");
}
