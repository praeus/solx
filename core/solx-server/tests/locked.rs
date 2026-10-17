//! Locked actions over solx-server: only the root token may change or delete
//! one over REST, and nothing reached through `/mcp` may, even with the root
//! token — a model's tool call is content, whatever token its client holds.

use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::model::CallToolRequestParams;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use solx_server::state::AppState;
use solx_surface::entities::{ActionInput, ActionType};
use solx_surface::managers::Solx;

struct Server {
    _dir: tempfile::TempDir,
    app: Arc<solx_manager::App>,
    base: String,
    root: String,
}

/// A server with `/p/locked` saved locked, through the operator path.
async fn spawn_server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    // Never touch the real OS credential manager from a test.
    solx_config::credentials::use_memory_store_for_tests();
    let app = solx_manager::App::build_local_in(dir.path()).await.unwrap();
    let root = solx_config::credentials::root_token(&app.config).unwrap();
    app.actions()
        .save(
            "/p",
            "locked",
            ActionInput {
                action_type: Some(ActionType::Internal),
                fn_name: Some("get-env".into()),
                caption: Some("original".into()),
                locked: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let router = solx_server::build_router(AppState::new(app.clone()).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Server {
        _dir: dir,
        app,
        base: format!("http://{addr}"),
        root,
    }
}

#[tokio::test]
async fn over_rest_only_the_root_token_replaces_a_pinned_artifact() {
    let s = spawn_server().await;
    let rel = solx_files::shared_action_file_path("locked.solx");
    s.app.files().put(&rel, b"json 42".to_vec()).await.unwrap();
    s.app
        .actions()
        .save(
            "/p",
            "script",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some("locked.solx".into()),
                locked: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let client = s.app.registry().unwrap().issue_client(None, None).await.unwrap();
    let http = reqwest::Client::new();
    let url = format!("{}/files/{rel}", s.base);

    let res = http
        .put(&url)
        .bearer_auth(&client.token)
        .body("json 666")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    let res = http.delete(&url).bearer_auth(&client.token).send().await.unwrap();
    assert_eq!(res.status(), 400);
    assert_eq!(s.app.files().get(&rel).await.unwrap(), b"json 42");

    let res = http
        .put(&url)
        .bearer_auth(&s.root)
        .body("json 666")
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
}

async fn caption(s: &Server) -> Option<String> {
    s.app.actions().get("/p", "locked").await.ok()?.caption
}

#[tokio::test]
async fn over_rest_only_the_root_token_changes_or_deletes_a_locked_action() {
    let s = spawn_server().await;
    let client = s.app.registry().unwrap().issue_client(None, None).await.unwrap();
    let http = reqwest::Client::new();
    let url = format!("{}/actions/p/locked", s.base);

    for body in [json!({"caption": "changed"}), json!({"locked": false})] {
        let res = http
            .put(&url)
            .bearer_auth(&client.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401, "{body}");
    }
    let res = http.delete(&url).bearer_auth(&client.token).send().await.unwrap();
    assert_eq!(res.status(), 401);
    assert_eq!(caption(&s).await.as_deref(), Some("original"));

    let res = http
        .put(&url)
        .bearer_auth(&s.root)
        .json(&json!({"caption": "changed"}))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    assert_eq!(caption(&s).await.as_deref(), Some("changed"));
    let res = http.delete(&url).bearer_auth(&s.root).send().await.unwrap();
    assert!(res.status().is_success(), "{}", res.status());
}

#[tokio::test]
async fn over_mcp_even_the_root_token_cant_change_a_locked_action() -> anyhow::Result<()> {
    let s = spawn_server().await;
    let mut config = StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", s.base));
    config.auth_header = Some(s.root.clone());
    let mcp = ().serve(StreamableHttpClientTransport::from_config(config)).await?;

    for (tool, args) in [
        (
            "act__builtin__action__entity-save-action",
            json!({"path": "/p", "name": "locked", "caption": "changed"}),
        ),
        (
            "act__builtin__action__entity-save-action",
            json!({"path": "/p", "name": "locked", "locked": false}),
        ),
        (
            "act__builtin__action__entity-delete-action",
            json!({"path": "/p", "name": "locked"}),
        ),
    ] {
        let Value::Object(args) = args else { unreachable!() };
        let called = mcp
            .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
            .await?;
        let body = serde_json::to_string(&called)?;
        assert_eq!(called.is_error, Some(true), "{tool}: {body}");
        assert!(body.contains("is locked"), "{tool}: {body}");
    }
    mcp.cancel().await?;

    let action = s.app.actions().get("/p", "locked").await?;
    assert!(action.locked);
    assert_eq!(action.caption.as_deref(), Some("original"));
    Ok(())
}
