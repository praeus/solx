//! `solx` — the command-line binary. The commands, script runner and
//! package handling live in the library (`lib.rs`) so the desktop app can
//! run the same `install.solx` scripts; this file only builds the `App`.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use solx_cli::{run_command, Commands};
use solx_config::credentials::{self, ConnectCredential};
use solx_manager::App;
use solx_surface::managers::Solx;
use solx_surface::Principal;

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
    /// Manage this machine's solx-server tokens: the root token (kept in
    /// the OS credential manager), connect codes and client tokens.
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    /// Connect this machine to a solx-server on another machine, with a
    /// connect link, or a server address plus a connect code or token.
    Connect {
        /// A connect link (`http://host:8766/connect#code=12345678`), or the
        /// server's address (`http://host:8766`).
        target: String,
        /// The connect code (`1234-5678`) or token, when `target` is an
        /// address. Prompted for when omitted.
        credential: Option<String>,
        /// Only print the token (e.g. for an MCP client's configuration);
        /// don't save it or point this machine's solx at the server.
        #[arg(long)]
        print: bool,
    },
    /// Lock or unlock an action. A locked action, and the artifact it was
    /// locked against, change only through the solx CLI, a package
    /// install, the root token, or the desktop app's Settings window.
    /// Locking a Script/WASM action pins its current artifact; locking again
    /// re-pins it.
    Action {
        #[command(subcommand)]
        op: ActionOp,
    },
    #[command(flatten)]
    Core(Commands),
}

/// Not in `Commands`, so no `.solx` script can lock or unlock anything.
#[derive(Subcommand)]
enum ActionOp {
    /// Lock an action (`/path/name`), pinning its artifact.
    Lock { reference: String },
    /// Unlock an action, dropping its pin.
    Unlock { reference: String },
}

#[derive(Subcommand)]
enum TokenAction {
    /// Print the root token HTTP clients on this machine use to reach its
    /// solx-server (generated if there is none yet).
    Show,
    /// Replace the root token with a new random one. Running solx hosts
    /// keep accepting the old one until they restart.
    Rotate,
    /// Create a single-use connect code another device redeems for its own
    /// client token, with `solx connect` or the link in a browser.
    ConnectCode {
        /// Name for the client this code creates, shown by `solx token clients`.
        #[arg(long)]
        label: Option<String>,
        /// How long the code stays valid, in seconds (default 600).
        #[arg(long, value_name = "SECS")]
        ttl: Option<u64>,
        /// How long the client token it redeems for stays valid, in seconds
        /// (default `client_token_ttl_days`, 90 days).
        #[arg(long, value_name = "SECS")]
        token_ttl: Option<u64>,
        /// Address to put in the link instead of `server_public_url` or the
        /// bound address, e.g. this machine's Tailscale IP. A URL, or an IP
        /// or host name (which gets `http://` and the server's port).
        #[arg(long, value_name = "URL|HOST")]
        url: Option<String>,
    },
    /// List client tokens that still work (never the tokens themselves).
    Clients,
    /// Revoke a client token by id.
    Revoke { id: String },
    /// Mint a client token directly and print it.
    Issue {
        /// Name for the client, shown by `solx token clients`.
        #[arg(long)]
        label: String,
        /// How long the token stays valid, in seconds (default
        /// `client_token_ttl_days`, 90 days).
        #[arg(long, value_name = "SECS")]
        ttl: Option<u64>,
    },
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
        BinCommand::Token { action } => run_token(action).await?,
        BinCommand::Connect {
            target,
            credential,
            print,
        } => run_connect(target, credential, print).await?,
        BinCommand::Action { op } => run_action(op).await?,
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

async fn run_action(op: ActionOp) -> Result<Value> {
    let (reference, locked) = match op {
        ActionOp::Lock { reference } => (reference, true),
        ActionOp::Unlock { reference } => (reference, false),
    };
    let (path, name) = solx_surface::path::split_ref(&reference).map_err(solx_cli::to_anyhow)?;
    let app = build_app().await?;
    let action = app
        .actions()
        .save(
            &path,
            &name,
            solx_surface::entities::ActionInput {
                locked: Some(locked),
                ..Default::default()
            },
        )
        .await
        .map_err(solx_cli::to_anyhow)?;
    Ok(json!({
        "action": format!("{}/{}", action.path, action.name),
        "locked": action.locked,
        "artifactPin": action.artifact_pin,
    }))
}

async fn run_token(action: TokenAction) -> Result<Value> {
    let (name, params) = match action {
        TokenAction::Show | TokenAction::Rotate => return root_token(action),
        TokenAction::ConnectCode {
            label,
            ttl,
            token_ttl,
            url,
        } => (
            "create-connect-code",
            json!({
                "label": label,
                "code_ttl_secs": ttl,
                "token_ttl_secs": token_ttl,
                "public_url": url,
            }),
        ),
        TokenAction::Clients => ("list-clients", json!({})),
        TokenAction::Revoke { id } => ("revoke-client", json!({ "id": id })),
        TokenAction::Issue { label, ttl } => (
            "issue-client",
            json!({ "label": label, "token_ttl_secs": ttl }),
        ),
    };
    run_auth_builtin(name, params).await
}

/// Run a `/builtin/auth/*` builtin as `Principal::Root`. This is a trusted
/// local entry point: a binary-only command a person types at this machine,
/// which `.solx` scripts can't reach. In-process when no host is running;
/// otherwise against the host, which derives Root from the loopback root
/// token, so the policy is the same either way.
async fn run_auth_builtin(name: &str, params: Value) -> Result<Value> {
    let app = build_app().await?;
    let out = app
        .actions()
        .exec_for(&Principal::Root, "/builtin/auth", name, params)
        .await
        .with_context(|| format!("solx token: {name} failed"))?;
    let mut result = out.result;
    if name == "create-connect-code" && app.remote_url().is_none() {
        result["note"] = "no solx host is running on this machine: start one (solx-server or \
                          the desktop app) before the code expires so it can be redeemed"
            .into();
    }
    Ok(result)
}

/// `solx connect`: redeem a code (or take a token), then save the token
/// and point this machine's solx at the server, or with `--print` only
/// print the token.
async fn run_connect(target: String, credential: Option<String>, print: bool) -> Result<Value> {
    let (server, input) = if target.contains("#code=") {
        if credential.is_some() {
            bail!("a connect link already carries its code; pass the link alone");
        }
        (None, target)
    } else {
        let input = match credential {
            Some(c) => c,
            None => prompt("Connect code or token: ")?,
        };
        (Some(target), input)
    };
    let parsed = credentials::parse_connect_input(server.as_deref(), &input)?;
    let mut server_url = parsed
        .server_url
        .context("no server address: pass the server's URL, or a connect link")?;

    let (token, client_id, expires_at) = match parsed.credential {
        ConnectCredential::Code(code) => {
            let exchanged = solx_client::exchange_connect_code(&server_url, &code)
                .await
                .context("redeem the connect code")?;
            if let Some(url) = exchanged.server_url.filter(|u| !u.trim().is_empty()) {
                server_url = url;
            }
            (
                exchanged.token,
                Some(exchanged.client_id),
                exchanged.expires_at,
            )
        }
        ConnectCredential::Token(token) => (token, None, None),
    };
    if print {
        return Ok(Value::String(token));
    }

    let config = Arc::new(solx_config::ConfigService::open().context("open config")?);
    let app = App::remote(config.clone(), server_url.clone(), token.clone())?;
    if let Err(e) = app
        .docs()
        .list(solx_surface::query::ListOptions {
            limit: Some(1),
            ..Default::default()
        })
        .await
    {
        let looks_like_a_code = input
            .chars()
            .filter(|c| !matches!(c, ' ' | '-'))
            .all(|c| c.is_ascii_digit());
        let hint = if looks_like_a_code {
            " (a connect code is 8 digits)"
        } else {
            ""
        };
        bail!("{server_url} did not accept the credential{hint}: {e}");
    }
    credentials::set_client_token(&config, &server_url, &token)?;
    config.set("server_url", Value::String(server_url.clone()))?;
    Ok(json!({
        "connected": server_url,
        "client_id": client_id,
        "expires_at": expires_at,
        "note": "solx on this machine now uses that server; remove server_url from \
                 solx-config.json to go back to local storage",
    }))
}

fn prompt(label: &str) -> Result<String> {
    use std::io::Write;
    eprint!("{label}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn root_token(action: TokenAction) -> Result<Value> {
    let config = solx_config::ConfigService::open().context("open config")?;
    let token = match action {
        TokenAction::Rotate => credentials::rotate_root_token(&config),
        _ => credentials::root_token(&config),
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
