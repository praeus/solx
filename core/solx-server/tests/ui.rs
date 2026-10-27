//! The web UI over solx-server: `/ui/*` serves the store's `web/` directory
//! without a token, and only the root token may write that directory.

use std::net::SocketAddr;
use std::sync::Arc;

use serde_json::{json, Value};
use solx_server::state::AppState;
use solx_surface::managers::Solx;

struct Server {
    _dir: tempfile::TempDir,
    app: Arc<solx_manager::App>,
    base: String,
    root: String,
}

async fn spawn_server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    // Never touch the real OS credential manager from a test.
    solx_config::credentials::use_memory_store_for_tests();
    let app = solx_manager::App::build_local_in(dir.path()).await.unwrap();
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
    Server {
        _dir: dir,
        app,
        base: format!("http://{addr}"),
        root,
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

/// A client token minted with the root token.
async fn client_token(s: &Server) -> String {
    let res = http()
        .post(format!("{}/actions/builtin/auth/issue-client", s.base))
        .bearer_auth(&s.root)
        .json(&json!({ "label": "ui-test", "token_ttl_secs": 600 }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success());
    let body: Value = res.json().await.unwrap();
    body["result"]["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn nothing_is_served_until_the_ui_is_published() {
    let s = spawn_server().await;
    let res = http().get(format!("{}/ui/", s.base)).send().await.unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn the_published_ui_is_served_without_a_token() {
    let s = spawn_server().await;
    s.app
        .files()
        .put("web/index.html", b"<!doctype html>hi".to_vec())
        .await
        .unwrap();
    s.app
        .files()
        .put("web/assets/app.js", b"export {}".to_vec())
        .await
        .unwrap();
    // Outside `web/`, so never reachable from `/ui`.
    s.app
        .files()
        .put("secret.txt", b"no".to_vec())
        .await
        .unwrap();

    let bare = http().get(format!("{}/ui", s.base)).send().await.unwrap();
    assert_eq!(bare.status(), 308);
    assert_eq!(bare.headers()["location"], "/ui/");

    let index = http().get(format!("{}/ui/", s.base)).send().await.unwrap();
    assert_eq!(index.status(), 200);
    assert!(index.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert_eq!(index.headers()["x-content-type-options"], "nosniff");
    let etag = index.headers()["etag"].clone();
    assert_eq!(index.text().await.unwrap(), "<!doctype html>hi");

    let again = http()
        .get(format!("{}/ui/", s.base))
        .header("if-none-match", etag)
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 304);

    let js = http()
        .get(format!("{}/ui/assets/app.js", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(js.status(), 200);
    assert!(js.headers()["content-type"]
        .to_str()
        .unwrap()
        .contains("javascript"));

    for escape in ["../secret.txt", "%2e%2e/secret.txt", "..%5csecret.txt"] {
        let res = http()
            .get(format!("{}/ui/{escape}", s.base))
            .send()
            .await
            .unwrap();
        assert_ne!(res.status(), 200, "{escape}");
    }
}

#[tokio::test]
async fn only_the_root_token_writes_the_ui() {
    let s = spawn_server().await;
    let client = client_token(&s).await;

    let put = |token: String, path: &'static str| {
        let base = s.base.clone();
        async move {
            http()
                .put(format!("{base}/files/{path}"))
                .bearer_auth(token)
                .body("alert(1)")
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(put(client.clone(), "web/assets/app.js").await, 400);
    assert_eq!(put(client.clone(), "Web/./assets/app.js").await, 400);
    assert!(put(client.clone(), "widgets/app.js").await.is_success());
    assert!(put(s.root.clone(), "web/assets/app.js").await.is_success());

    let delete = http()
        .delete(format!("{}/files/web/assets/app.js", s.base))
        .bearer_auth(&client)
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), 400);

    // The file built-ins are content too, whatever token runs them.
    let builtin = http()
        .post(format!("{}/actions/builtin/file/file-put", s.base))
        .bearer_auth(&s.root)
        .json(&json!({ "rel_path": "web/index.html", "content": "<script>" }))
        .send()
        .await
        .unwrap();
    let body: Value = builtin.json().await.unwrap();
    assert!(
        body.to_string().contains("web UI"),
        "file-put into web/ should be refused: {body}"
    );
    assert!(s.app.files().get("web/index.html").await.is_err());
}
