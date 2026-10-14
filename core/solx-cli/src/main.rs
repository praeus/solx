//! `solx` — the command-line binary. The commands, script runner and
//! package handling live in the library (`lib.rs`) so the desktop app can
//! run the same `install.solx` scripts; this file only builds the `App`.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use solx_cli::{run_command, Commands};
use solx_config::credentials;
use solx_manager::App;

/// The `solx` binary's command line: every [`Commands`] verb, plus
/// commands that only make sense typed by a person at this machine.
/// Those are deliberately not in [`Commands`], which `.solx` scripts
/// (including a package's `install.solx`) also parse, so a script can
/// never read the server token.
#[derive(Parser)]
#[command(
    name = "solx",
    about = "Structured documents and extensible actions via CLI",
    version
)]
struct BinCli {
    #[command(subcommand)]
    command: BinCommand,
}

#[derive(Subcommand)]
enum BinCommand {
    /// Show or rotate this machine's solx-server bearer token, kept in the
    /// OS credential manager.
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    #[command(flatten)]
    Core(Commands),
}

#[derive(Subcommand)]
enum TokenAction {
    /// Print the token HTTP clients use to reach this machine's solx-server
    /// (generated if there is none yet).
    Show,
    /// Replace the token with a new random one. Running solx hosts keep
    /// accepting the old one until they restart.
    Rotate,
}

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

    let result = match BinCli::parse().command {
        BinCommand::Token { action } => run_token(action)?,
        BinCommand::Core(command) => {
            let app = build_app().await?;
            run_command(&app, command, None).await?
        }
    };
    match result {
        Value::String(s) => println!("{s}"),
        other => println!("{}", serde_json::to_string_pretty(&other)?),
    }
    Ok(())
}

fn run_token(action: TokenAction) -> Result<Value> {
    let config = solx_config::ConfigService::open().context("open config")?;
    let token = match action {
        TokenAction::Show => credentials::root_token(&config),
        TokenAction::Rotate => credentials::rotate_root_token(&config),
    }
    .context("resolve server token")?;
    let mut out = json!({
        "token": token,
        "stored_in": credentials::root_token_location(&config),
    });
    if matches!(action, TokenAction::Rotate) {
        out["note"] =
            "restart running solx hosts (solx-server, the desktop app) to use the new token".into();
    }
    Ok(out)
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
        // A client of that host: `App::build` resolves the token the way
        // any client does (never minting one, which the running host
        // wouldn't accept anyway).
        std::env::set_var("SOLX_SERVER_URL", format!("http://127.0.0.1:{port}"));
        return App::build().await;
    }

    let app = App::build_local().await?;
    match solx_server::state::AppState::new(app.clone()) {
        Ok(state) => {
            if let Err(e) = solx_server::spawn_embedded(state, port).await {
                tracing::warn!(
                    "could not start embedded solx-server on 127.0.0.1:{port} ({e}); \
                     actions that call back into solx-server over HTTP may fail"
                );
            }
        }
        Err(e) => tracing::warn!("could not prepare embedded solx-server: {e}"),
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
