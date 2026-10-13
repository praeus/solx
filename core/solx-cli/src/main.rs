//! `solx` — the command-line binary. The commands, script runner and
//! package handling live in the library (`lib.rs`) so the desktop app can
//! run the same `install.solx` scripts; this file only builds the `App`.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::Value;

use solx_cli::{run_command, Cli};
use solx_manager::App;

#[tokio::main]
async fn main() -> Result<()> {
    // Writer is stderr, not the default stdout — stdout is the single JSON
    // result `solx exec ... | jq` depends on, same reasoning as solx-mcp
    // (whose stdout is its JSON-RPC channel). Without this, every
    // `tracing::*` call in solx-core was silently dropped under the CLI —
    // the primary development surface had no logging at all.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let cli = Cli::parse();
    let app = build_app().await?;
    let result = run_command(&app, cli.command, None).await?;
    match result {
        Value::String(s) => println!("{s}"),
        other => println!("{}", serde_json::to_string_pretty(&other)?),
    }
    Ok(())
}

/// Build the `App`, auto-detecting a locally-reachable `solx-server` before
/// falling back to opening local storage directly.
///
/// Without this, every CLI invocation opens the local libsql storage
/// itself (`App::build()`'s default when no `server_url` is configured),
/// racing a `solx-server` already running against the same appdata dir for
/// the same database files. And with no server running at all, any spawned Command action
/// that talks to `solx-server` over HTTP for the file store (e.g.
/// `solx-quickjs`'s `build-javascript-action`, via
/// `solx-package-lib::ServerConfig`) has nothing to reach.
///
/// So: an explicit `server_url` (env or config) is honored unchanged. With
/// none set, probe the default local port — if something's already
/// listening there, wire remote against it instead of touching local
/// storage. If nothing's listening, build local as today and additionally
/// serve this same `App` over HTTP for the rest of this process's
/// lifetime, so a spawned child still has a server to reach.
async fn build_app() -> Result<Arc<App>> {
    let config = solx_config::ConfigService::open().context("open config")?;
    let snap = config.snapshot();

    let explicit = std::env::var("SOLX_SERVER_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| snap.server_url.clone().filter(|s| !s.trim().is_empty()));
    if explicit.is_some() {
        return App::build().await;
    }

    let port = snap.server_port.unwrap_or(solx_config::DEFAULT_SERVER_PORT);

    if health_check(port).await {
        let token = config
            .ensure_server_token()
            .context("ensure server token")?;
        std::env::set_var("SOLX_SERVER_URL", format!("http://127.0.0.1:{port}"));
        std::env::set_var("SOLX_SERVER_TOKEN", &token);
        return App::build().await;
    }

    let app = App::build_local().await?;
    match app.config.ensure_server_token() {
        Ok(token) => {
            let state = solx_server::state::AppState {
                app: app.clone(),
                token: Arc::from(token.as_str()),
            };
            if let Err(e) = solx_server::spawn_embedded(state, port).await {
                tracing::warn!(
                    "could not start embedded solx-server on 127.0.0.1:{port} ({e}); \
                     actions that call back into solx-server over HTTP may fail"
                );
            }
        }
        Err(e) => tracing::warn!("could not prepare embedded solx-server token: {e}"),
    }
    Ok(app)
}

async fn health_check(port: u16) -> bool {
    reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/health"))
        .timeout(std::time::Duration::from_millis(300))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}
