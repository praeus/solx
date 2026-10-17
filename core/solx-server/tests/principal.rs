//! The authenticated `Principal` reaches action execution from both of
//! solx-server's entry points — REST exec and the `/mcp` transport — rather
//! than stopping at `require_bearer`. Uses `solx_actions::test_probe`, a
//! handler that echoes what its `InternalCallCtx` carries.

use std::sync::Arc;

use rmcp::model::CallToolRequestParams;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use solx_server::state::AppState;
use solx_surface::entities::{ActionInput, ActionType};
use solx_surface::managers::Solx;

/// Server root URL and root token, with `/t/probe` saved.
async fn spawn_server() -> (tempfile::TempDir, Arc<solx_manager::App>, String, String) {
    let dir = tempfile::tempdir().unwrap();
    // Never touch the real OS credential manager from a test.
    solx_config::credentials::use_memory_store_for_tests();
    let app = solx_manager::App::build_local_in(dir.path()).await.unwrap();
    let token = solx_config::credentials::root_token(&app.config).unwrap();

    app.actions()
        .save(
            "/t",
            "probe",
            ActionInput {
                action_type: Some(ActionType::Internal),
                fn_name: Some(solx_actions::test_probe::FN_NAME.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let state = AppState::new(app.clone()).unwrap();
    let router = solx_server::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    (dir, app, format!("http://{addr}"), token)
}

fn root_probe(exclude_mode: &str) -> Value {
    json!({"principal": "root", "caller": null, "external": "root", "excludeMode": exclude_mode})
}

#[tokio::test]
async fn rest_exec_runs_as_the_authenticated_principal() {
    let (_dir, _app, base, token) = spawn_server().await;

    let res = reqwest::Client::new()
        .post(format!("{base}/actions/t/probe"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["result"], root_probe("none"));
}

/// `X-Solx-Scope: tooling` is how an MCP server proxying to this host keeps
/// its calls in the tooling scope; it restricts, so it needs no privilege.
#[tokio::test]
async fn rest_exec_honors_the_tooling_scope_header() {
    let (_dir, _app, base, token) = spawn_server().await;

    let res = reqwest::Client::new()
        .post(format!("{base}/actions/t/probe"))
        .bearer_auth(&token)
        .header(solx_surface::auth::EXEC_SCOPE_HEADER, "tooling")
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["result"], root_probe("tooling"));
}

/// Confirms the rmcp streamable HTTP transport hands the request's
/// `http::request::Parts` — and so `require_bearer`'s `Principal` — to
/// `call_tool`, even though tool calls run on rmcp's own session tasks.
#[tokio::test]
async fn mcp_tool_call_runs_as_the_authenticated_principal() -> anyhow::Result<()> {
    let (_dir, _app, base, token) = spawn_server().await;

    let mut config = StreamableHttpClientTransportConfig::with_uri(format!("{base}/mcp"));
    config.auth_header = Some(token);
    let client = ()
        .serve(StreamableHttpClientTransport::from_config(config))
        .await?;

    let called = client
        .call_tool(CallToolRequestParams::new("act__t__probe"))
        .await?;
    assert_eq!(called.structured_content, Some(root_probe("tooling")));

    client.cancel().await?;
    Ok(())
}
