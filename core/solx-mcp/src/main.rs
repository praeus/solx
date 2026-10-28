//! `solx-mcp` — an MCP server exposing solx documents, types, files, and
//! actions to LLM clients over stdio.
//!
//! stdout is the JSON-RPC channel; nothing but protocol frames may be
//! written to it. All logging goes to stderr, and the subscriber is
//! installed before anything else runs (including `App::build()`), so even
//! startup wiring errors are safely off stdout.
//!
//! `SOLX_MCP_PATH_PREFIX`, if set, restricts the exposed tool catalogue to
//! that path (and everything under it) — e.g. `/builtin` or
//! `/packages/solx-google` — so a client can run several scoped instances
//! instead of always seeing the full action catalogue. Unset (the default)
//! exposes every registered action, unchanged from before this existed.
//!
//! Like `solx-cli`, with no `server_url` configured this attaches to a
//! `solx-server` already listening on the local `server_port` rather than
//! opening the same database files beside it (`App::build_or_attach`), and
//! only builds local when nothing answers. Unlike `solx-cli` it doesn't
//! embed a server of its own when building local: `solx-server` depends on
//! this crate for `/mcp`, so it can't depend back.

use rmcp::transport::stdio;
use rmcp::ServiceExt;
use solx_mcp::SolxMcpServer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    // See the identical call in solx-server/src/main.rs — same reasoning:
    // solx-mcp is a long-lived stdio server, unlike solx-cli.
    solx_actions::set_long_lived_host(true);

    let path_prefix = std::env::var("SOLX_MCP_PATH_PREFIX")
        .ok()
        .filter(|s| !s.is_empty());

    let app = match solx_manager::App::build_or_attach().await? {
        Some(app) => app,
        None => solx_manager::App::build_local().await?,
    };
    match app.remote_url() {
        Some(url) => tracing::info!("attached to solx host at {url}"),
        None => tracing::info!("no solx host running; using local storage"),
    }
    let service = SolxMcpServer::new(app.clone(), path_prefix)
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serve error: {e:?}"))?;
    let served = service.waiting().await;
    // Built local, this process hosted any backgrounds its calls started.
    app.shutdown().await;
    served?;
    Ok(())
}
