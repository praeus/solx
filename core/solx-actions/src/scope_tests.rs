//! End-to-end scope checks through real dispatch, under each
//! `scope_enforcement` mode: who a call is checked against, what a
//! built-in needs on top of its `exec` scope, how lists narrow, and what
//! content may write.

use serde_json::{json, Value};
use solx_surface::entities::{ActionInput, ActionType};
use solx_surface::managers::ActionManager;
use solx_surface::{Actor, Grant, Principal, ScopeSet, SolxError};

use crate::test_support::setup_wired;
use crate::LocalActionManager;

fn client(scopes: &[&str]) -> Principal {
    Principal::Client {
        id: "c".into(),
        label: None,
        grant: Grant::scoped(ScopeSet::parse(scopes).unwrap()),
    }
}

async fn wired(mode: &str) -> (tempfile::TempDir, std::sync::Arc<LocalActionManager>) {
    let (dir, cfg, m) = setup_wired().await;
    cfg.set("scope_enforcement", mode.into()).unwrap();
    (dir, m)
}

/// A Script action running `source`, declaring `scopes` when given. Saved
/// as the operator, the only party that may declare scopes.
async fn save_script(m: &LocalActionManager, name: &str, source: &str, scopes: Option<&[&str]>) {
    let bin = format!("{name}.solx");
    m.files
        .put(
            &solx_files::shared_action_file_path(&bin),
            source.as_bytes().to_vec(),
        )
        .await
        .unwrap();
    m.save(
        "/t",
        name,
        ActionInput {
            action_type: Some(ActionType::Script),
            bin_name: Some(bin),
            action_config: scopes.map(|s| json!({ "scopes": s })),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

/// Save a document as a local caller (unlimited).
async fn save_doc(m: &LocalActionManager, path: &str, name: &str) {
    m.exec(
        "/builtin/document",
        "entity-save-document",
        json!({ "path": path, "name": name, "typeRef": "/types/core/Object", "contents": {} }),
    )
    .await
    .unwrap();
}

fn forbidden(result: solx_surface::Result<solx_surface::ActionExecResult>, needs: &str) {
    match result {
        Err(e) => assert!(
            e.to_string().contains(needs),
            "expected a refusal naming {needs}: {e}"
        ),
        Ok(r) => panic!("expected a refusal naming {needs}, got {r:?}"),
    }
}

/// A client's call is checked against its token; the action's nested call
/// against the action, whoever started it.
#[tokio::test]
async fn each_call_is_checked_against_whoever_makes_it() {
    let (_d, m) = wired("enforce").await;
    save_script(
        &m,
        "scoped",
        "exec /builtin/test/grant",
        Some(&["exec:/builtin/test/*"]),
    )
    .await;
    save_script(&m, "bare", "exec /builtin/test/grant", None).await;

    // The client needs exec on the action, not on what the action calls.
    let ok = m
        .exec_for(&client(&["exec:/t/*"]), "/t", "scoped", json!({}))
        .await
        .unwrap();
    assert_eq!(ok.result["result"], json!(["exec:/builtin/test/*"]));

    let refused = m
        .exec_for(&client(&["exec:/x/*"]), "/t", "scoped", json!({}))
        .await;
    assert!(
        matches!(refused, Err(SolxError::Forbidden(_))),
        "{refused:?}"
    );
    forbidden(refused, "exec:/t/scoped");

    // An action that declares no scopes can call nothing, even from the
    // CLI, which is otherwise unlimited.
    forbidden(
        m.exec("/t", "bare", json!({})).await,
        "exec:/builtin/test/grant",
    );
    forbidden(
        m.exec_for(&client(&["all"]), "/t", "bare", json!({})).await,
        "exec:/builtin/test/grant",
    );

    // A local call is unlimited.
    m.exec("/builtin/test", "grant", json!({})).await.unwrap();
}

/// Audit and off allow what enforce refuses.
#[tokio::test]
async fn audit_and_off_allow_what_enforce_refuses() {
    for mode in ["audit", "off"] {
        let (_d, m) = wired(mode).await;
        save_script(&m, "bare", "exec /builtin/test/grant", None).await;
        m.exec("/t", "bare", json!({})).await.unwrap();
        m.exec_for(&client(&[]), "/builtin/test", "grant", json!({}))
            .await
            .unwrap();
    }
}

/// `exec:/*` makes every built-in reachable, but data access stays with the
/// resource scopes.
#[tokio::test]
async fn a_builtin_needs_its_resource_scope_too() {
    let (_d, m) = wired("enforce").await;
    save_doc(&m, "/notes", "a").await;
    save_doc(&m, "/private", "b").await;
    let get = |path: &'static str, name: &'static str| json!({ "path": path, "name": name });

    let exec_only = client(&["exec:/*"]);
    let refused = m
        .exec_for(
            &exec_only,
            "/builtin/document",
            "entity-get-document",
            get("/notes", "a"),
        )
        .await;
    assert!(
        matches!(refused, Err(SolxError::Forbidden(_))),
        "{refused:?}"
    );

    let notes = client(&["exec:/*", "read:document:/notes/*"]);
    m.exec_for(
        &notes,
        "/builtin/document",
        "entity-get-document",
        get("/notes", "a"),
    )
    .await
    .unwrap();
    forbidden(
        m.exec_for(
            &notes,
            "/builtin/document",
            "entity-get-document",
            get("/private", "b"),
        )
        .await,
        "read:document:/private/b",
    );
    // Reading isn't writing.
    forbidden(
        m.exec_for(
            &notes,
            "/builtin/document",
            "entity-save-document",
            json!({ "path": "/notes", "name": "c", "typeRef": "/types/core/Object", "contents": {} }),
        )
        .await,
        "write:document:/notes/c",
    );
}

/// A list shows only what the caller may read, and a caller that may read
/// none of it is refused rather than shown everything.
#[tokio::test]
async fn lists_narrow_to_what_may_be_read() {
    let (_d, m) = wired("enforce").await;
    save_doc(&m, "/notes", "a").await;
    save_doc(&m, "/notes/deep", "b").await;
    save_doc(&m, "/private", "c").await;
    let names = |page: &Value| -> Vec<String> {
        let mut names: Vec<String> = page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                format!(
                    "{}/{}",
                    d["path"].as_str().unwrap(),
                    d["name"].as_str().unwrap()
                )
            })
            .collect();
        names.sort();
        names
    };

    let notes = client(&["exec:/*", "read:document:/notes/*"]);
    let listed = m
        .exec_for(
            &notes,
            "/builtin/document",
            "entity-list-documents",
            json!({}),
        )
        .await
        .unwrap();
    assert_eq!(names(&listed.result), ["/notes/a", "/notes/deep/b"]);

    // A caller's own path prefix can't escape the scope.
    let elsewhere = m
        .exec_for(
            &notes,
            "/builtin/document",
            "entity-list-documents",
            json!({ "pathPrefix": "/private" }),
        )
        .await
        .unwrap();
    assert_eq!(elsewhere.result["total"], 0);

    // Nor can it send its own `includePaths`.
    let widened = m
        .exec_for(
            &notes,
            "/builtin/document",
            "entity-list-documents",
            json!({ "includePaths": ["/*"] }),
        )
        .await
        .unwrap();
    assert_eq!(names(&widened.result), ["/notes/a", "/notes/deep/b"]);

    let no_reads = client(&["exec:/*"]);
    let refused = m
        .exec_for(
            &no_reads,
            "/builtin/document",
            "entity-list-documents",
            json!({}),
        )
        .await;
    assert!(
        matches!(refused, Err(SolxError::Forbidden(_))),
        "{refused:?}"
    );
}

/// Content writing an action needs `write:action` and the kind scope of
/// both the incoming and the stored type, and may never declare scopes.
#[tokio::test]
async fn action_writes_need_their_kind_and_never_set_scopes() {
    let (_d, m) = wired("enforce").await;
    let script = |scopes: Option<Value>| ActionInput {
        action_type: Some(ActionType::Script),
        bin_name: Some("x.solx".into()),
        action_config: scopes.map(|s| json!({ "scopes": s })),
        ..Default::default()
    };
    fn content(grant: &Grant) -> Actor<'_> {
        Actor::Content {
            acting: None,
            grant,
        }
    }
    let full = Grant::scoped(ScopeSet::parse(["write:action:/my/*", "write:script"]).unwrap());
    let no_kind = Grant::scoped(ScopeSet::parse(["write:action:/my/*"]).unwrap());

    m.authorize_action_change(content(&full), "/my", "tool", Some(&script(None)))
        .await
        .unwrap();
    let err = m
        .authorize_action_change(content(&no_kind), "/my", "tool", Some(&script(None)))
        .await
        .unwrap_err();
    assert!(matches!(err, SolxError::Forbidden(_)), "{err:?}");
    assert!(err.to_string().contains("write:script"), "{err}");
    let err = m
        .authorize_action_change(content(&full), "/elsewhere", "tool", Some(&script(None)))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("write:action:/elsewhere/tool"),
        "{err}"
    );

    // Scopes are root-only, whatever the writer holds.
    let all = Grant::scoped(ScopeSet::all());
    let err = m
        .authorize_action_change(
            content(&all),
            "/my",
            "tool",
            Some(&script(Some(json!(["exec:/builtin/web/*"])))),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("declared scopes"), "{err}");

    // An action that declares scopes is locked, so content can't change
    // it either, even to keep its scopes.
    save_script(
        &m,
        "granted",
        "exec /builtin/test/grant",
        Some(&["exec:/builtin/test/*"]),
    )
    .await;
    let locked = m.get("/t", "granted").await.unwrap();
    assert!(locked.locked);
    let err = m
        .authorize_action_change(
            content(&all),
            "/t",
            "granted",
            Some(&ActionInput::default()),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("locked"), "{err}");
}

/// Files: writes need `write:file` on the path the store will use, and
/// `file-list` shows only what may be read.
#[tokio::test]
async fn file_scopes_follow_the_stored_path() {
    let (_d, m) = wired("enforce").await;
    for rel in ["files/notes/a.txt", "files/private/b.txt"] {
        m.files.put(rel, b"x".to_vec()).await.unwrap();
    }
    let grant = Grant::scoped(
        ScopeSet::parse(["write:file:/files/notes/*", "read:file:/files/notes/*"]).unwrap(),
    );
    let actor = Actor::Content {
        acting: None,
        grant: &grant,
    };
    m.authorize_file_change(actor, ["files/notes/c.txt"])
        .await
        .unwrap();
    // An alias of a path is judged as the path the store writes: here a
    // backslash-separated one outside the scope, and a doubled slash
    // inside it.
    let err = m
        .authorize_file_change(actor, ["files\\private\\c.txt"])
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("write:file:/files/private/c.txt"),
        "{err}"
    );
    m.authorize_file_change(actor, ["files//notes//d.txt"])
        .await
        .unwrap();

    let reader = client(&["exec:/*", "read:file:/files/notes/*"]);
    let listed = m
        .exec_for(
            &reader,
            "/builtin/file",
            "file-list",
            json!({ "prefix": "files" }),
        )
        .await
        .unwrap();
    assert_eq!(listed.result["files"], json!(["files/notes/a.txt"]));
    forbidden(
        m.exec_for(
            &reader,
            "/builtin/file",
            "file-get",
            json!({ "rel_path": "files/private/b.txt" }),
        )
        .await,
        "read:file:/files/private/b.txt",
    );
}
