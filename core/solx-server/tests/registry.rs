//! The token registry over HTTP: connect codes redeemed at `/auth/exchange`,
//! the static `/connect` page, root-token-from-loopback-only, and the
//! root-only policy of the `/builtin/auth/*` builtins at every entry point.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use solx_server::state::AppState;
use solx_surface::auth::TokenAuthority;
use solx_surface::entities::{ActionInput, ActionType};
use solx_surface::managers::Solx;
use solx_surface::Principal;

struct Server {
    _dir: tempfile::TempDir,
    app: Arc<solx_manager::App>,
    base: String,
    root: String,
}

async fn build_app() -> (tempfile::TempDir, Arc<solx_manager::App>, String) {
    let dir = tempfile::tempdir().unwrap();
    // Never touch the real OS credential manager from a test.
    solx_config::credentials::use_memory_store_for_tests();
    let app = solx_manager::App::build_local_in(dir.path()).await.unwrap();
    let root = solx_config::credentials::root_token(&app.config).unwrap();
    (dir, app, root)
}

/// A server whose peers are seen as they really are (loopback, here).
async fn spawn_server() -> Server {
    let (dir, app, root) = build_app().await;
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

/// A server that sees every request as coming from `10.0.0.5`.
async fn spawn_server_seen_from_the_network() -> Server {
    let (dir, app, root) = build_app().await;
    let router = solx_server::build_router(AppState::new(app.clone()).unwrap())
        .layer(MockConnectInfo(SocketAddr::from(([10, 0, 0, 5], 40000))));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server {
        _dir: dir,
        app,
        base: format!("http://{addr}"),
        root,
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

/// `POST /actions/builtin/auth/<name>` with `token`.
async fn auth_builtin(s: &Server, token: &str, name: &str, params: Value) -> reqwest::Response {
    http()
        .post(format!("{}/actions/builtin/auth/{name}", s.base))
        .bearer_auth(token)
        .json(&params)
        .send()
        .await
        .unwrap()
}

async fn ok_json(res: reqwest::Response) -> Value {
    let status = res.status();
    let body: Value = res.json().await.unwrap();
    assert!(status.is_success(), "{status}: {body}");
    body
}

async fn exchange(s: &Server, code: &str) -> reqwest::Response {
    http()
        .post(format!("{}/auth/exchange", s.base))
        .json(&json!({ "code": code }))
        .send()
        .await
        .unwrap()
}

async fn status_with(s: &Server, token: &str) -> u16 {
    http()
        .get(format!("{}/docs", s.base))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// Pull the 8 digits back out of a `1234-5678` display code.
fn digits(display: &Value) -> String {
    display.as_str().unwrap().replace('-', "")
}

#[tokio::test]
async fn a_connect_code_becomes_a_listed_revocable_client_token() {
    let s = spawn_server().await;

    let created = ok_json(
        auth_builtin(
            &s,
            &s.root,
            "create-connect-code",
            json!({ "label": "phone" }),
        )
        .await,
    )
    .await;
    let display = &created["result"]["display_code"];
    assert!(display.as_str().unwrap().contains('-'), "{created}");
    let link = created["result"]["url"].as_str().unwrap();
    assert!(
        link.ends_with(&format!("/connect#code={}", digits(display))),
        "{link}"
    );

    let exchanged = ok_json(exchange(&s, display.as_str().unwrap()).await).await;
    let token = exchanged["token"].as_str().unwrap().to_string();
    let client_id = exchanged["client_id"].as_str().unwrap().to_string();
    assert!(exchanged["expires_at"].is_string());
    // The address the client reached us on, since no public URL is set.
    assert_eq!(exchanged["server_url"], json!(s.base));

    assert_eq!(status_with(&s, &token).await, 200);

    let listed = ok_json(auth_builtin(&s, &s.root, "list-clients", json!({})).await).await;
    let clients = listed["result"]["clients"].as_array().unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0]["id"], json!(client_id));
    assert_eq!(clients[0]["label"], json!("phone"));
    assert!(!listed.to_string().contains(&token));

    ok_json(auth_builtin(&s, &s.root, "revoke-client", json!({ "id": client_id })).await).await;
    assert_eq!(status_with(&s, &token).await, 401);

    // Spent: the same code can't be redeemed again.
    assert_eq!(exchange(&s, &digits(display)).await.status(), 401);
}

#[tokio::test]
async fn issue_client_mints_a_working_token() {
    let s = spawn_server().await;
    let issued = ok_json(
        auth_builtin(
            &s,
            &s.root,
            "issue-client",
            json!({ "label": "ci", "token_ttl_secs": 600 }),
        )
        .await,
    )
    .await;
    let token = issued["result"]["token"].as_str().unwrap();
    assert_eq!(status_with(&s, token).await, 200);
}

#[tokio::test]
async fn the_connect_page_changes_nothing() {
    let s = spawn_server().await;
    let registry = s.app.registry().unwrap();
    let code = registry
        .create_connect_code(None, None, None, None)
        .await
        .unwrap();

    for path in ["/connect", &format!("/connect#code={}", code.code)] {
        let res = http()
            .get(format!("{}{path}", s.base))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert!(res.text().await.unwrap().contains("auth/exchange"));
    }
    assert!(registry.list_clients().await.unwrap().is_empty());
    assert_eq!(registry.failed_exchanges().await.unwrap(), 0);
    // Still pending: the page didn't spend it.
    registry.redeem(&code.code).await.unwrap();
}

#[tokio::test]
async fn a_failed_exchange_backs_off_that_peer() {
    let s = spawn_server().await;
    assert_eq!(exchange(&s, "0000-0000").await.status(), 401);
    let retry = exchange(&s, "0000-0001").await;
    assert_eq!(retry.status(), 429);
    // A backed-off attempt isn't a guess, so it isn't counted.
    let registry = s.app.registry().unwrap();
    assert_eq!(registry.failed_exchanges().await.unwrap(), 1);
}

#[tokio::test]
async fn the_root_token_is_refused_from_the_network() {
    let s = spawn_server_seen_from_the_network().await;
    let res = http()
        .get(format!("{}/docs", s.base))
        .bearer_auth(&s.root)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    let body: Value = res.json().await.unwrap();
    assert!(
        body["message"].as_str().unwrap().contains("connect code"),
        "{body}"
    );

    // A client token works from there.
    let issued = s
        .app
        .registry()
        .unwrap()
        .issue_client(Some("laptop"), None)
        .await
        .unwrap();
    assert_eq!(status_with(&s, &issued.token).await, 200);
}

// ── who may call the auth builtins ───────────────────────────────────────────

fn refused(body: &Value) -> bool {
    body.to_string().contains("needs the root token")
}

#[tokio::test]
async fn client_and_action_tokens_are_refused_over_rest() {
    let s = spawn_server().await;
    let registry = s.app.registry().unwrap();
    let client = registry.issue_client(None, None).await.unwrap();
    let action = registry
        .issue_action_token("/pkg/a", "inv", std::time::Duration::from_secs(60))
        .await
        .unwrap();

    for token in [client.token.as_str(), action.token.as_str()] {
        for name in ["create-connect-code", "list-clients", "issue-client"] {
            let res = auth_builtin(&s, token, name, json!({ "label": "x" })).await;
            let status = res.status();
            let body: Value = res.json().await.unwrap();
            assert!(
                !status.is_success() && refused(&body),
                "{name}: {status} {body}"
            );
        }
    }
}

#[tokio::test]
async fn local_calls_without_a_principal_are_refused() {
    let s = spawn_server().await;
    // Plain `exec`: stdio MCP, local CLI `exec`, plain Tauri `actions-exec`.
    let err = s
        .app
        .actions()
        .exec("/builtin/auth", "list-clients", json!({}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("needs the root token"), "{err}");

    // The trusted entry points run it as Root explicitly.
    let ok = s
        .app
        .actions()
        .exec_for(&Principal::Root, "/builtin/auth", "list-clients", json!({}))
        .await
        .unwrap();
    assert_eq!(ok.result, json!({ "clients": [] }));
}

/// The CLI with a host running: a remote `App` holding the loopback root
/// token. Its plain `exec` (and so every `.solx` script) runs with no
/// principal, as locally; only `exec_for(Root)` (`solx token …`) is root.
#[tokio::test]
async fn a_remote_exec_carries_no_principal() {
    let s = spawn_server().await;
    let remote =
        solx_manager::App::remote(s.app.config.clone(), s.base.clone(), s.root.clone()).unwrap();

    let err = remote
        .actions()
        .exec(
            "/builtin/auth",
            "issue-client",
            json!({ "label": "script" }),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("needs the root token"), "{err}");

    let ok = remote
        .actions()
        .exec_for(&Principal::Root, "/builtin/auth", "list-clients", json!({}))
        .await
        .unwrap();
    assert_eq!(ok.result, json!({ "clients": [] }));
}

#[tokio::test]
async fn a_nested_script_caller_is_refused_even_under_root() {
    let s = spawn_server().await;
    let bin = "mint.solx";
    s.app
        .files()
        .put(
            &solx_files::shared_action_file_path(bin),
            b"exec /builtin/auth/issue-client --json '{\"label\": \"sneaky\"}'".to_vec(),
        )
        .await
        .unwrap();
    s.app
        .actions()
        .save(
            "/t",
            "mint",
            ActionInput {
                action_type: Some(ActionType::Script),
                bin_name: Some(bin.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let res = http()
        .post(format!("{}/actions/t/mint", s.base))
        .bearer_auth(&s.root)
        .send()
        .await
        .unwrap();
    let body = res.text().await.unwrap();
    assert!(body.contains("needs the root token"), "{body}");
    assert!(s
        .app
        .registry()
        .unwrap()
        .list_clients()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn mcp_allows_root_and_refuses_a_client() -> anyhow::Result<()> {
    let s = spawn_server().await;
    let client = s.app.registry().unwrap().issue_client(None, None).await?;

    async fn list_clients(base: &str, token: &str) -> anyhow::Result<(bool, String)> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(format!("{base}/mcp"));
        config.auth_header = Some(token.to_string());
        let mcp = ().serve(StreamableHttpClientTransport::from_config(config)).await?;
        let outcome = mcp
            .call_tool(CallToolRequestParams::new(
                "act__builtin__auth__list-clients",
            ))
            .await;
        mcp.cancel().await?;
        Ok(match outcome {
            Ok(called) => (
                called.is_error != Some(true),
                serde_json::to_string(&called)?,
            ),
            Err(e) => (false, e.to_string()),
        })
    }

    let (ok, body) = list_clients(&s.base, &s.root).await?;
    assert!(ok, "{body}");
    assert!(body.contains("clients"), "{body}");

    let (ok, body) = list_clients(&s.base, &client.token).await?;
    assert!(!ok && body.contains("needs the root token"), "{body}");
    Ok(())
}

/// `PUT /actions/{ref}` of an action with scoped secrets is root-only: a
/// client token can neither create one (setting a key) nor modify one,
/// while it can still save an ordinary action.
#[tokio::test]
async fn only_the_root_token_can_save_a_keyed_action() {
    let s = spawn_server().await;
    let client = s
        .app
        .registry()
        .unwrap()
        .issue_client(Some("editor"), None)
        .await
        .unwrap()
        .token;
    let key = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
    let put = |token: String, name: &'static str, body: Value| {
        let url = format!("{}/actions/pkg/{name}", s.base);
        async move {
            http()
                .put(url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    // An action with secrets is locked, so its artifact must exist to pin.
    s.app
        .files()
        .put("files/actions/shared/good.wasm", b"\0asm".to_vec())
        .await
        .unwrap();
    let keyed = json!({
        "actionType": "wasm",
        "binName": "good.wasm",
        "actionConfig": { "secrets": { "S": key } }
    });

    assert_eq!(put(client.clone(), "keyed", keyed.clone()).await, 401);
    assert_eq!(put(s.root.clone(), "keyed", keyed).await, 200);
    assert_eq!(
        put(client.clone(), "keyed", json!({ "binName": "evil.wasm" })).await,
        401
    );
    assert_eq!(
        put(client, "plain", json!({ "actionType": "wasm", "binName": "x.wasm" })).await,
        200
    );

    let kept = s.app.actions().get("/pkg", "keyed").await.unwrap();
    assert_eq!(kept.bin_name.as_deref(), Some("good.wasm"));
}
