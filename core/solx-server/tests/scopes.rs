//! Scoped client tokens over REST and MCP, with `scope_enforcement` set to
//! `enforce`: each route allowed with its scope and refused (403,
//! `{"kind":"forbidden"}`) without it, lists narrowed rather than refused,
//! and discovery limited to what the token may call.

use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::model::CallToolRequestParams;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Value};
use solx_server::state::AppState;
use solx_surface::ScopeSet;

struct Server {
    _dir: tempfile::TempDir,
    app: Arc<solx_manager::App>,
    base: String,
    root: String,
}

async fn spawn_server(mode: &str) -> Server {
    let dir = tempfile::tempdir().unwrap();
    // Never touch the real OS credential manager from a test.
    solx_config::credentials::use_memory_store_for_tests();
    let app = solx_manager::App::build_local_in(dir.path()).await.unwrap();
    app.config.set("scope_enforcement", mode.into()).unwrap();
    let root = solx_config::credentials::root_token(&app.config).unwrap();
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
    let s = Server {
        _dir: dir,
        app,
        base: format!("http://{addr}"),
        root,
    };
    for doc in ["notes/a", "notes/deep/b", "private/c"] {
        let res = s
            .send(
                &s.root,
                reqwest::Method::PUT,
                &format!("/docs/{doc}"),
                Some(doc_body()),
            )
            .await;
        assert!(res.status().is_success(), "{doc}: {}", res.status());
    }
    for file in ["files/notes/a.txt", "files/private/b.txt"] {
        let res = http()
            .put(format!("{}/files/{file}", s.base))
            .bearer_auth(&s.root)
            .body("x")
            .send()
            .await
            .unwrap();
        assert!(res.status().is_success(), "{file}: {}", res.status());
    }
    s
}

fn doc_body() -> Value {
    json!({ "typeRef": "/types/core/Object", "contents": {} })
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

impl Server {
    /// A client token holding exactly `scopes`.
    async fn token(&self, scopes: &[&str]) -> String {
        self.app
            .registry()
            .unwrap()
            .issue_scoped_client(None, None, &ScopeSet::parse(scopes).unwrap())
            .await
            .unwrap()
            .token
    }

    async fn send(
        &self,
        token: &str,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
    ) -> reqwest::Response {
        let mut req = http()
            .request(method, format!("{}{route}", self.base))
            .bearer_auth(token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        req.send().await.unwrap()
    }

    async fn status(
        &self,
        token: &str,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
    ) -> u16 {
        self.send(token, method, route, body)
            .await
            .status()
            .as_u16()
    }

    async fn get_json(&self, token: &str, route: &str) -> Value {
        let res = self.send(token, reqwest::Method::GET, route, None).await;
        assert!(res.status().is_success(), "{route}: {}", res.status());
        res.json().await.unwrap()
    }
}

fn refs(page: &Value) -> Vec<String> {
    let mut refs: Vec<String> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let path = e["path"].as_str().unwrap();
            let name = e["name"].as_str().unwrap();
            format!("{}/{name}", path.trim_end_matches('/'))
        })
        .collect();
    refs.sort();
    refs
}

use reqwest::Method;

#[tokio::test]
async fn single_document_routes_follow_read_and_write_scopes() {
    let s = spawn_server("enforce").await;
    let reader = s.token(&["read:document:/notes/*"]).await;

    assert_eq!(
        s.status(&reader, Method::GET, "/docs/notes/a", None).await,
        200
    );
    let refused = s.send(&reader, Method::GET, "/docs/private/c", None).await;
    assert_eq!(refused.status(), 403);
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["kind"], "forbidden", "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("read:document:/private/c"),
        "{body}"
    );

    // Reading doesn't grant writing, and a write scope doesn't reach past
    // its path.
    assert_eq!(
        s.status(&reader, Method::PUT, "/docs/notes/new", Some(doc_body()))
            .await,
        403
    );
    let writer = s.token(&["write:document:/notes/*"]).await;
    assert_eq!(
        s.status(&writer, Method::PUT, "/docs/notes/new", Some(doc_body()))
            .await,
        200
    );
    assert_eq!(
        s.status(&writer, Method::PUT, "/docs/private/new", Some(doc_body()))
            .await,
        403
    );
    assert_eq!(
        s.status(&writer, Method::DELETE, "/docs/notes/new", None)
            .await,
        204
    );

    // The root token holds every scope.
    assert_eq!(
        s.status(&s.root, Method::GET, "/docs/private/c", None)
            .await,
        200
    );
}

#[tokio::test]
async fn lists_narrow_and_a_list_with_no_grant_is_refused() {
    let s = spawn_server("enforce").await;
    let reader = s.token(&["read:document:/notes/*"]).await;

    assert_eq!(
        refs(&s.get_json(&reader, "/docs").await),
        ["/notes/a", "/notes/deep/b"]
    );
    assert_eq!(
        refs(&s.get_json(&reader, "/search").await),
        ["/notes/a", "/notes/deep/b"]
    );
    let elsewhere = s.get_json(&reader, "/docs?pathPrefix=/private").await;
    assert_eq!(elsewhere["total"], 0);
    // A caller's own includePaths is ignored, never a way out.
    let widened = s.get_json(&reader, "/docs?includePaths=/*").await;
    assert_eq!(refs(&widened), ["/notes/a", "/notes/deep/b"]);
    let facets = s.get_json(&reader, "/docs-paths").await;
    assert!(!facets.to_string().contains("/private"), "{facets}");

    // No `read:document` at all is refused, not an unrestricted list.
    let types_only = s.token(&["read:type"]).await;
    for route in ["/docs", "/search", "/docs-paths"] {
        assert_eq!(
            s.status(&types_only, Method::GET, route, None).await,
            403,
            "{route}"
        );
    }
}

#[tokio::test]
async fn action_routes_need_read_write_kind_and_exec() {
    let s = spawn_server("enforce").await;
    let script = || json!({ "actionType": "script", "binName": "x.solx" });

    let reader = s.token(&["read:action:/builtin/test/*"]).await;
    assert_eq!(
        s.status(&reader, Method::GET, "/actions/builtin/test/probe", None)
            .await,
        200
    );
    assert_eq!(
        s.status(
            &reader,
            Method::GET,
            "/actions/builtin/web/http-request",
            None
        )
        .await,
        403
    );
    let listed = s.get_json(&reader, "/actions").await;
    assert!(
        refs(&listed)
            .iter()
            .all(|r| r.starts_with("/builtin/test/")),
        "{listed}"
    );

    // Saving needs `write:action` and the kind scope.
    let no_kind = s.token(&["write:action:/my/*"]).await;
    assert_eq!(
        s.status(&no_kind, Method::PUT, "/actions/my/tool", Some(script()))
            .await,
        403
    );
    let writer = s.token(&["write:action:/my/*", "write:script"]).await;
    assert_eq!(
        s.status(&writer, Method::PUT, "/actions/my/tool", Some(script()))
            .await,
        200
    );
    // A Command row needs `write:command` (known gap 7, closed).
    let command = json!({ "actionType": "command", "fnName": "k" });
    assert_eq!(
        s.status(&writer, Method::PUT, "/actions/my/cmd", Some(command))
            .await,
        403
    );
    // Declaring scopes is root-only, whatever the token holds.
    let all = s.token(&["all"]).await;
    let declaring =
        json!({ "actionType": "script", "binName": "x.solx", "actionConfig": { "scopes": [] } });
    assert_eq!(
        s.status(&all, Method::PUT, "/actions/my/granted", Some(declaring))
            .await,
        401
    );

    // Exec needs `exec` on the action.
    let caller = s.token(&["exec:/builtin/test/*"]).await;
    assert_eq!(
        s.status(
            &caller,
            Method::POST,
            "/actions/builtin/test/probe",
            Some(json!({}))
        )
        .await,
        200
    );
    assert_eq!(
        s.status(
            &caller,
            Method::POST,
            "/actions/builtin/env/get-env",
            Some(json!({ "name": "X" }))
        )
        .await,
        403
    );
}

/// `X-Solx-Principal: none` from a client token keeps its scopes.
#[tokio::test]
async fn dropping_the_principal_does_not_drop_the_scopes() {
    let s = spawn_server("enforce").await;
    let caller = s.token(&["exec:/builtin/test/*"]).await;
    for scope in [None, Some("tooling")] {
        let mut req = http()
            .post(format!("{}/actions/builtin/env/get-env", s.base))
            .bearer_auth(&caller)
            .header(solx_surface::auth::NO_PRINCIPAL_HEADER, "none")
            .json(&json!({ "name": "X" }));
        if let Some(scope) = scope {
            req = req.header(solx_surface::auth::EXEC_SCOPE_HEADER, scope);
        }
        assert_eq!(req.send().await.unwrap().status(), 403, "{scope:?}");
    }
}

#[tokio::test]
async fn file_routes_follow_file_scopes() {
    let s = spawn_server("enforce").await;
    let reader = s.token(&["read:file:/files/notes/*"]).await;
    assert_eq!(
        s.status(&reader, Method::GET, "/files/files/notes/a.txt", None)
            .await,
        200
    );
    assert_eq!(
        s.status(&reader, Method::GET, "/files/files/private/b.txt", None)
            .await,
        403
    );
    let listed = s.get_json(&reader, "/files").await;
    assert_eq!(listed["paths"], json!(["files/notes/a.txt"]));

    let put = |token: String, file: &'static str| {
        let base = s.base.clone();
        async move {
            http()
                .put(format!("{base}/files/{file}"))
                .bearer_auth(token)
                .body("y")
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };
    assert_eq!(put(reader.clone(), "files/notes/new.txt").await, 403);
    let writer = s.token(&["write:file:/files/notes/*"]).await;
    assert_eq!(put(writer.clone(), "files/notes/new.txt").await, 200);
    assert_eq!(put(writer, "files/private/new.txt").await, 403);
}

/// Under audit (the default) the same calls are allowed.
#[tokio::test]
async fn audit_allows_what_enforce_refuses() {
    let s = spawn_server("audit").await;
    let nothing = s.token(&[]).await;
    assert_eq!(
        s.status(&nothing, Method::GET, "/docs/private/c", None)
            .await,
        200
    );
    assert_eq!(refs(&s.get_json(&nothing, "/docs").await).len(), 3);
    assert_eq!(
        s.status(
            &nothing,
            Method::POST,
            "/actions/builtin/test/probe",
            Some(json!({}))
        )
        .await,
        200
    );
}

/// A model is only shown the tools its token may call, and is refused one
/// it calls by name anyway.
#[tokio::test]
async fn mcp_discovery_follows_exec_scopes() -> anyhow::Result<()> {
    let s = spawn_server("enforce").await;
    let token = s.token(&["exec:/builtin/test/*"]).await;
    let mut config = StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", s.base));
    config.auth_header = Some(token);
    let client = ().serve(StreamableHttpClientTransport::from_config(config)).await?;

    let listed = client
        .call_tool(
            CallToolRequestParams::new("explore_tools").with_arguments(
                json!({ "discovery_mode": "list_all" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await?;
    let text = serde_json::to_string(&listed.content)?;
    assert!(text.contains("act__builtin__test__probe"), "{text}");
    assert!(!text.contains("entity-get-document"), "{text}");

    let refused = client
        .call_tool(CallToolRequestParams::new("act__builtin__env__get-env"))
        .await?;
    assert_eq!(refused.is_error, Some(true), "{refused:?}");
    let text = serde_json::to_string(&refused.content)?;
    assert!(text.contains("forbidden"), "{text}");

    client.cancel().await?;
    Ok(())
}
