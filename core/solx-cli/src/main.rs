//! `solx` — the command-line binary. The commands, script runner and
//! package handling live in the library (`lib.rs`) so the desktop app can
//! run the same `install.solx` scripts; this file only builds the `App`.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use solx_actions::guard::scope as scope_gate;
use solx_cli::{run_command, Commands};
use solx_config::credentials::{self, ConnectCredential};
use solx_manager::App;
use solx_surface::managers::Solx;
use solx_surface::scope::{declared_scopes, PathFilter, Scope, ScopeSet};
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
    /// Scope reports. See `solx action scopes` to grant an action scopes.
    Scopes {
        #[command(subcommand)]
        op: ScopesOp,
    },
    #[command(flatten)]
    Core(Commands),
}

/// Not in `Commands`, so no `.solx` script can lock or unlock anything, or
/// grant an action scopes beyond what its package's manifest lists.
#[derive(Subcommand)]
enum ActionOp {
    /// Lock an action (`/path/name`), pinning its artifact.
    Lock { reference: String },
    /// Unlock an action, dropping its pin.
    Unlock { reference: String },
    /// Show or change the scopes an action declares (`action_config.scopes`):
    /// what it may do when it calls anything. Declaring scopes locks the
    /// action. With no option, shows them.
    Scopes {
        reference: String,
        /// Replace the declared scopes with these. Repeatable.
        #[arg(long, value_name = "SCOPE", conflicts_with_all = ["add", "remove", "clear"])]
        set: Vec<String>,
        /// Declare this scope too. Repeatable.
        #[arg(long, value_name = "SCOPE")]
        add: Vec<String>,
        /// Stop declaring this scope. Repeatable.
        #[arg(long, value_name = "SCOPE")]
        remove: Vec<String>,
        /// Drop the declaration: the action may then call nothing (but its
        /// own package's actions). It stays locked.
        #[arg(long, conflicts_with_all = ["add", "remove"])]
        clear: bool,
    },
}

#[derive(Subcommand)]
enum ScopesOp {
    /// What `scope_enforcement: "audit"` recorded on this machine: calls
    /// that `enforce` would refuse, grouped by the action or client making
    /// them, with the `solx action scopes` command that would allow an
    /// action's. Needs that an action's current scopes grant are left out.
    Audit {
        /// Only this action's findings.
        #[arg(long, value_name = "REF")]
        action: Option<String>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
        /// Delete the recorded findings instead.
        #[arg(long)]
        clear: bool,
    },
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
        #[command(flatten)]
        scopes: ScopeArgs,
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
        #[command(flatten)]
        scopes: ScopeArgs,
    },
}

/// What a new client token may do. With neither option it holds `all`, as
/// tokens did before scopes existed.
#[derive(clap::Args)]
struct ScopeArgs {
    /// A scope the token holds (`read:document`, `exec:/builtin/web/*`, ...).
    /// Repeatable.
    #[arg(long = "scope", value_name = "SCOPE")]
    scopes: Vec<String>,
    /// A named bundle added to the scopes: read-only, tools, editor or full.
    #[arg(long)]
    preset: Option<String>,
}

impl ScopeArgs {
    /// Merge into a built-in's params, leaving both out when neither was
    /// given.
    fn add_to(self, params: &mut Value) {
        if !self.scopes.is_empty() {
            params["scopes"] = json!(self.scopes);
        }
        if let Some(preset) = self.preset {
            params["preset"] = json!(preset);
        }
    }
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
        BinCommand::Scopes { op } => run_scopes(op).await?,
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
        ActionOp::Scopes {
            reference,
            set,
            add,
            remove,
            clear,
        } => return action_scopes(reference, set, add, remove, clear).await,
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

/// `solx action scopes`: show, or change and show, what `reference`
/// declares.
async fn action_scopes(
    reference: String,
    set: Vec<String>,
    add: Vec<String>,
    remove: Vec<String>,
    clear: bool,
) -> Result<Value> {
    let (path, name) = solx_surface::path::split_ref(&reference).map_err(solx_cli::to_anyhow)?;
    let parse = |scopes: &[String]| ScopeSet::parse(scopes).map_err(solx_cli::to_anyhow);
    let app = build_app().await?;
    let mut action = app
        .actions()
        .get(&path, &name)
        .await
        .map_err(solx_cli::to_anyhow)?;
    let declared = declared_scopes(action.action_config.as_ref()).map_err(solx_cli::to_anyhow)?;

    let next = if clear {
        Some(None)
    } else if !set.is_empty() {
        Some(Some(parse(&set)?))
    } else if !add.is_empty() || !remove.is_empty() {
        let current = declared.unwrap_or_default();
        let removing = parse(&remove)?;
        if let Some(missing) = removing.iter().find(|s| !current.iter().any(|c| c == *s)) {
            bail!("{reference} doesn't declare {missing}");
        }
        let kept = current.iter().filter(|s| !removing.iter().any(|r| r == *s));
        Some(Some(ScopeSet::new(
            kept.chain(parse(&add)?.iter()).cloned(),
        )))
    } else {
        None
    };
    if let Some(next) = next {
        let mut config = action.action_config.take().unwrap_or_else(|| json!({}));
        let fields = config
            .as_object_mut()
            .context("action_config is not a JSON object")?;
        match next {
            Some(scopes) => {
                fields.insert("scopes".into(), serde_json::to_value(scopes)?);
            }
            None => {
                fields.remove("scopes");
            }
        }
        action = app
            .actions()
            .save(
                &path,
                &name,
                solx_surface::entities::ActionInput {
                    action_config: Some(config),
                    ..Default::default()
                },
            )
            .await
            .map_err(solx_cli::to_anyhow)?;
    }

    let config = action.action_config.as_ref();
    Ok(json!({
        "action": format!("{}/{}", action.path, action.name),
        "declared": declared_scopes(config).map_err(solx_cli::to_anyhow)?,
        "runsWith": solx_surface::action_scopes(&action.path, config),
        "locked": action.locked,
    }))
}

async fn run_scopes(op: ScopesOp) -> Result<Value> {
    let ScopesOp::Audit {
        action,
        json: as_json,
        clear,
    } = op;
    let app = build_app().await?;
    if clear {
        scope_gate::clear_audit_log(&app.config).map_err(solx_cli::to_anyhow)?;
        return Ok(Value::String("cleared the scope audit log".into()));
    }
    let entries = scope_gate::read_audit_log(&app.config).map_err(solx_cli::to_anyhow)?;
    let report = audit_report(&app, entries, action.as_deref()).await;
    if as_json {
        return Ok(serde_json::to_value(report)?);
    }
    Ok(Value::String(render_audit(&report)))
}

/// One party's findings in `solx scopes audit`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditParty {
    party: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
    needs: Vec<AuditNeed>,
    /// For an action: the scopes `solx action scopes --add` would declare.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    grant: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditNeed {
    need: String,
    /// For a list: what it would have been narrowed to, or empty if it
    /// would have been refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    list: Option<Vec<String>>,
    count: usize,
    last: String,
}

/// Group `entries` by party and need, dropping what an action's current
/// scopes already grant (it was granted since) and, with `only`, other
/// actions' findings.
async fn audit_report(
    app: &Arc<App>,
    entries: Vec<scope_gate::AuditEntry>,
    only: Option<&str>,
) -> Vec<AuditParty> {
    let mut parties: Vec<AuditParty> = Vec::new();
    for entry in entries {
        if only.is_some_and(|o| entry.action.as_deref() != Some(o)) {
            continue;
        }
        let party = match parties.iter_mut().position(|p| p.party == entry.party) {
            Some(i) => &mut parties[i],
            None => {
                parties.push(AuditParty {
                    party: entry.party.clone(),
                    action: entry.action.clone(),
                    needs: Vec::new(),
                    grant: Vec::new(),
                });
                parties.last_mut().unwrap()
            }
        };
        match party
            .needs
            .iter_mut()
            .find(|n| n.need == entry.need && n.list == entry.list)
        {
            Some(need) => {
                need.count += 1;
                need.last = entry.at;
            }
            None => party.needs.push(AuditNeed {
                need: entry.need,
                list: entry.list,
                count: 1,
                last: entry.at,
            }),
        }
    }

    for party in &mut parties {
        let Some(reference) = party.action.clone() else {
            continue;
        };
        let current = match solx_surface::path::split_ref(&reference) {
            Ok((path, name)) => match app.actions().get(&path, &name).await {
                Ok(action) => solx_surface::action_scopes(&path, action.action_config.as_ref()),
                Err(_) => ScopeSet::default(),
            },
            Err(_) => ScopeSet::default(),
        };
        party.needs.retain(|n| match n.need.parse::<Scope>() {
            // A list it would have refused is allowed once any of that
            // read is granted.
            Ok(Scope::Resource {
                access, resource, ..
            }) if n.list.as_ref().is_some_and(Vec::is_empty) => {
                current.path_filter(access, resource) == PathFilter::Nothing
            }
            Ok(need) => !current.iter().any(|s| s.covers(&need)),
            Err(_) => true,
        });
        party.grant = party
            .needs
            .iter()
            .filter(|n| n.list.is_none())
            .map(|n| n.need.clone())
            .collect();
    }
    parties.retain(|p| !p.needs.is_empty());
    parties
}

fn render_audit(report: &[AuditParty]) -> String {
    if report.is_empty() {
        return "no scope audit findings (scope_enforcement \"audit\" records them here as \
                calls are made)"
            .into();
    }
    let mut out = String::new();
    for party in report {
        out.push_str(&party.party);
        out.push('\n');
        for need in &party.needs {
            let what = match &need.list {
                None => need.need.clone(),
                Some(paths) if paths.is_empty() => {
                    format!("{} (a list; enforce would refuse it)", need.need)
                }
                Some(paths) => format!(
                    "{} (a list; enforce would narrow it to {})",
                    need.need,
                    paths.join(", ")
                ),
            };
            out.push_str(&format!("  {what}  x{}, last {}\n", need.count, need.last));
        }
        if let Some(action) = &party.action {
            if !party.grant.is_empty() {
                let adds: Vec<String> = party.grant.iter().map(|s| format!("--add {s}")).collect();
                out.push_str(&format!(
                    "  grant: solx action scopes {action} {}\n",
                    adds.join(" ")
                ));
            }
            if party.needs.iter().any(|n| n.list.is_some()) {
                out.push_str("  (and, for each list, a read scope on the paths it should see)\n");
            }
            if let Some(package) = solx_surface::scope::package_of(action) {
                out.push_str(&format!(
                    "  (and list them in {package}'s solx-package.json \"scopes\")\n"
                ));
            }
        }
    }
    out.trim_end().to_string()
}

async fn run_token(action: TokenAction) -> Result<Value> {
    let (name, params) = match action {
        TokenAction::Show | TokenAction::Rotate => return root_token(action),
        TokenAction::ConnectCode {
            label,
            ttl,
            token_ttl,
            url,
            scopes,
        } => {
            let mut params = json!({
                "label": label,
                "code_ttl_secs": ttl,
                "token_ttl_secs": token_ttl,
                "public_url": url,
            });
            scopes.add_to(&mut params);
            ("create-connect-code", params)
        }
        TokenAction::Clients => ("list-clients", json!({})),
        TokenAction::Revoke { id } => ("revoke-client", json!({ "id": id })),
        TokenAction::Issue { label, ttl, scopes } => {
            let mut params = json!({ "label": label, "token_ttl_secs": ttl });
            scopes.add_to(&mut params);
            ("issue-client", params)
        }
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
    let config = Arc::new(solx_config::ConfigService::open().context("open config")?);
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
    let mut pin = parsed.pin;

    let (token, client_id, expires_at) = match parsed.credential {
        ConnectCredential::Code(code) => {
            // Pinned before the code is sent: a server without the pinned
            // root never sees it.
            let http =
                solx_client::http_client_for(config.server_ca_file().as_deref(), pin.as_deref())?;
            let exchanged = solx_client::exchange_connect_code(&http, &server_url, &code)
                .await
                .context("redeem the connect code")?;
            if let Some(url) = exchanged.server_url.as_ref().filter(|u| !u.trim().is_empty()) {
                server_url = url.clone();
            }
            pin = pin.or_else(|| exchanged.adopted_pin(&server_url));
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

    let pinned_url = credentials::with_server_pin(&server_url, pin.as_deref());
    let app = App::remote(config.clone(), pinned_url.clone(), token.clone())?;
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
    config.set("server_url", Value::String(pinned_url))?;
    Ok(json!({
        "connected": server_url,
        "client_id": client_id,
        "expires_at": expires_at,
        "pinned": pin.as_deref().map(solx_client::fingerprint_of_pin),
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
