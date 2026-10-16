use std::sync::Arc;
use std::time::Duration;

use solx_config::ConfigService;
use solx_surface::auth::{Principal, TokenAuthority};

use super::*;

async fn open_in(dir: &std::path::Path) -> Arc<Registry> {
    // Never touch the real OS credential manager from a test.
    credentials::use_memory_store_for_tests();
    let config = Arc::new(ConfigService::open_in(dir).unwrap());
    Registry::open(config).await.unwrap()
}

async fn setup() -> (tempfile::TempDir, Arc<Registry>) {
    let dir = tempfile::tempdir().unwrap();
    let registry = open_in(dir.path()).await;
    (dir, registry)
}

async fn exec(r: &Registry, sql: &str) {
    r.db.connect()
        .await
        .unwrap()
        .execute(sql, ())
        .await
        .unwrap();
}

async fn count(r: &Registry, sql: &str) -> i64 {
    let conn = r.db.connect().await.unwrap();
    let mut rows = conn.query(sql, ()).await.unwrap();
    rows.next().await.unwrap().unwrap().get(0).unwrap()
}

const MINUTE: Duration = Duration::from_secs(60);

#[tokio::test]
async fn root_token_authenticates_as_root() {
    let (_d, r) = setup().await;
    let root = credentials::root_token(&r.config).unwrap();
    assert_eq!(r.authenticate(&root).await, Some(Principal::Root));
    assert_eq!(r.authenticate("nope").await, None);
    assert_eq!(r.authenticate("").await, None);
}

#[tokio::test]
async fn action_token_lives_until_released() {
    let (_d, r) = setup().await;
    let cred = r
        .issue_action_token("/pkg/a", "inv-1", MINUTE)
        .await
        .unwrap();
    assert!(cred.id.is_some());
    assert_eq!(cred.server_url.as_deref(), Some("http://127.0.0.1:8766"));
    assert_eq!(
        r.authenticate(&cred.token).await,
        Some(Principal::Action {
            action_ref: "/pkg/a".into(),
            invocation_id: "inv-1".into(),
        })
    );
    r.release(&cred).await;
    assert_eq!(r.authenticate(&cred.token).await, None);
}

#[tokio::test]
async fn dropping_an_action_credential_releases_it() {
    let (_d, r) = setup().await;
    let cred = r
        .issue_action_token("/pkg/a", "inv-1", MINUTE)
        .await
        .unwrap();
    let token = cred.token.clone();
    drop(cred);
    // Queued on drop, deleted before the very next lookup.
    assert_eq!(r.authenticate(&token).await, None);
    assert_eq!(count(&r, "SELECT COUNT(*) FROM tokens").await, 0);
}

/// The guard does no I/O, so dropping a credential where there is no
/// runtime at all (a plain thread, or after a runtime has shut down) is
/// fine; the row goes on the registry's next call.
#[tokio::test]
async fn a_credential_dropped_off_the_runtime_is_still_released() {
    let (_d, r) = setup().await;
    let cred = r.issue_action_token("/pkg/a", "inv", MINUTE).await.unwrap();
    let token = cred.token.clone();
    std::thread::spawn(move || drop(cred)).join().unwrap();
    assert_eq!(count(&r, "SELECT COUNT(*) FROM tokens").await, 1);
    r.sweep().await.unwrap();
    assert_eq!(count(&r, "SELECT COUNT(*) FROM tokens").await, 0);
    assert_eq!(r.authenticate(&token).await, None);
}

#[tokio::test]
async fn a_released_credential_queues_nothing_when_dropped() {
    let (_d, r) = setup().await;
    let cred = r.issue_action_token("/pkg/a", "inv", MINUTE).await.unwrap();
    r.release(&cred).await;
    drop(cred);
    let pending = r.pending.lock().unwrap();
    assert!(pending.queue.is_empty() && pending.released.is_empty());
}

#[tokio::test]
async fn set_server_url_overrides_the_sibling_default() {
    let (_d, r) = setup().await;
    r.set_server_url("http://127.0.0.1:1".into());
    let cred = r.issue_action_token("/a", "i", MINUTE).await.unwrap();
    assert_eq!(cred.server_url.as_deref(), Some("http://127.0.0.1:1"));
}

#[tokio::test]
async fn expired_tokens_are_rejected_and_swept() {
    let (_d, r) = setup().await;
    let client = r.issue_client(Some("laptop"), None).await.unwrap();
    let action = r.issue_action_token("/a", "i", MINUTE).await.unwrap();
    exec(&r, "UPDATE tokens SET expires_at = 1").await;
    assert_eq!(r.authenticate(&client.token).await, None);
    assert_eq!(r.authenticate(&action.token).await, None);
    assert!(r.list_clients().await.unwrap().is_empty());

    r.sweep().await.unwrap();
    assert_eq!(count(&r, "SELECT COUNT(*) FROM tokens").await, 0);
}

#[tokio::test]
async fn client_tokens_default_to_the_configured_lifetime() {
    let (_d, r) = setup().await;
    let issued = r.issue_client(None, None).await.unwrap();
    let days = (issued.expires_at.unwrap() - Utc::now()).num_days();
    assert!((89..=90).contains(&days), "{days}");

    r.config.set("client_token_ttl_days", 7.into()).unwrap();
    let issued = r.issue_client(None, None).await.unwrap();
    let days = (issued.expires_at.unwrap() - Utc::now()).num_days();
    assert!((6..=7).contains(&days), "{days}");
}

#[tokio::test]
async fn clients_are_listed_without_secrets_and_revoked() {
    let (_d, r) = setup().await;
    let issued = r.issue_client(Some(" laptop "), None).await.unwrap();
    assert_eq!(
        r.authenticate(&issued.token).await,
        Some(Principal::Client {
            id: issued.client_id.clone(),
            label: Some("laptop".into()),
        })
    );

    let clients = r.list_clients().await.unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].id, issued.client_id);
    let listed = serde_json::to_string(&clients).unwrap();
    assert!(!listed.contains(&issued.token));
    assert!(!listed.contains("hash"));

    r.revoke_client(&issued.client_id).await.unwrap();
    assert_eq!(r.authenticate(&issued.token).await, None);
    assert!(r.list_clients().await.unwrap().is_empty());
    assert!(matches!(
        r.revoke_client(&issued.client_id).await,
        Err(SolxError::NotFound(_))
    ));
}

#[tokio::test]
async fn last_used_is_written_at_most_once_a_minute() {
    let (_d, r) = setup().await;
    let issued = r.issue_client(None, None).await.unwrap();
    let last_used = "SELECT COALESCE(last_used_at, 0) FROM tokens";

    r.authenticate(&issued.token).await.unwrap();
    assert!(count(&r, last_used).await > 0);

    exec(&r, "UPDATE tokens SET last_used_at = 0").await;
    r.authenticate(&issued.token).await.unwrap();
    assert_eq!(count(&r, last_used).await, 0, "throttled");

    // Once the minute is up (simulated), it is written again.
    r.last_used.lock().unwrap().clear();
    r.authenticate(&issued.token).await.unwrap();
    assert!(count(&r, last_used).await > 0);
}

// ── connect codes ────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_code_redeems_once_for_a_working_client_token() {
    let (_d, r) = setup().await;
    let code = r
        .create_connect_code(Some("phone"), None, Some(Duration::from_secs(3600)), None)
        .await
        .unwrap();
    assert_eq!(code.code.len(), 8);
    assert_eq!(
        code.display_code,
        format!("{}-{}", &code.code[..4], &code.code[4..])
    );
    assert_eq!(
        code.url.as_deref(),
        Some(format!("http://127.0.0.1:8766/connect#code={}", code.code).as_str())
    );

    let issued = r.redeem(&code.display_code).await.unwrap();
    assert_eq!(issued.label.as_deref(), Some("phone"));
    let secs = (issued.expires_at.unwrap() - Utc::now()).num_seconds();
    assert!((3590..=3600).contains(&secs), "{secs}");
    assert!(matches!(
        r.authenticate(&issued.token).await,
        Some(Principal::Client { .. })
    ));

    assert!(matches!(
        r.redeem(&code.code).await,
        Err(SolxError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn codes_redeem_in_any_spacing() {
    let (_d, r) = setup().await;
    for spell in [
        |c: &str| c.to_string(),
        |c: &str| format!("{} {}", &c[..4], &c[4..]),
        |c: &str| format!("{}-{}", &c[..4], &c[4..]),
    ] {
        let code = r.create_connect_code(None, None, None, None).await.unwrap();
        r.redeem(&spell(&code.code)).await.unwrap();
    }
}

#[tokio::test]
async fn an_expired_code_is_rejected() {
    let (_d, r) = setup().await;
    let code = r.create_connect_code(None, None, None, None).await.unwrap();
    exec(&r, "UPDATE connect_codes SET expires_at = 1").await;
    assert!(r.redeem(&code.code).await.is_err());
}

#[tokio::test]
async fn the_failure_budget_invalidates_pending_codes() {
    let (_d, r) = setup().await;
    let code = r.create_connect_code(None, None, None, None).await.unwrap();
    let wrong = |i: u32| {
        let n = (code.code.parse::<u32>().unwrap() + 1 + i) % 100_000_000;
        format!("{n:08}")
    };
    for i in 0..(FAILURE_BUDGET as u32 - 1) {
        assert!(r.redeem(&wrong(i)).await.is_err());
    }
    assert_eq!(r.failed_exchanges().await.unwrap(), FAILURE_BUDGET - 1);
    // Malformed input isn't a guess and isn't counted.
    assert!(r.redeem("123").await.is_err());
    assert_eq!(r.failed_exchanges().await.unwrap(), FAILURE_BUDGET - 1);

    assert!(r.redeem(&wrong(99)).await.is_err());
    assert_eq!(
        r.failed_exchanges().await.unwrap(),
        0,
        "reset after the purge"
    );
    assert!(
        r.redeem(&code.code).await.is_err(),
        "the right code was invalidated with the others"
    );
}

#[tokio::test]
async fn neither_success_nor_new_codes_refill_the_budget() {
    let (_d, r) = setup().await;
    assert!(r.redeem("00000000").await.is_err());
    assert!(r.redeem("00000001").await.is_err());
    let code = r.create_connect_code(None, None, None, None).await.unwrap();
    // Tiny chance the random code is one of the two guesses above.
    if code.code != "00000000" && code.code != "00000001" {
        r.redeem(&code.code).await.unwrap();
        assert_eq!(r.failed_exchanges().await.unwrap(), 2);
    }
}

#[tokio::test]
async fn sweep_removes_spent_and_expired_codes() {
    let (_d, r) = setup().await;
    let used = r.create_connect_code(None, None, None, None).await.unwrap();
    r.redeem(&used.code).await.unwrap();
    r.create_connect_code(None, None, None, None).await.unwrap();
    let pending = r.create_connect_code(None, None, None, None).await.unwrap();
    exec(
        &r,
        &format!(
            "UPDATE connect_codes SET expires_at = 1 WHERE consumed_at IS NULL AND code_hash != x'{}'",
            hash_secret(&pending.code)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
    )
    .await;
    r.sweep().await.unwrap();
    assert_eq!(count(&r, "SELECT COUNT(*) FROM connect_codes").await, 1);
    r.redeem(&pending.code).await.unwrap();
}

#[tokio::test]
async fn code_lifetimes_are_bounded() {
    let (_d, r) = setup().await;
    assert!(r
        .create_connect_code(None, Some(Duration::ZERO), None, None)
        .await
        .is_err());
    assert!(r
        .create_connect_code(None, Some(Duration::from_secs(2 * 86400)), None, None)
        .await
        .is_err());
    assert!(r
        .create_connect_code(None, None, Some(Duration::ZERO), None)
        .await
        .is_err());
}

#[tokio::test]
async fn server_public_url_shapes_the_link() {
    let (_d, r) = setup().await;
    r.config
        .set("server_public_url", "https://solx.example.com/".into())
        .unwrap();
    let code = r.create_connect_code(None, None, None, None).await.unwrap();
    assert_eq!(
        code.url.unwrap(),
        format!("https://solx.example.com/connect#code={}", code.code)
    );
    assert_eq!(code.server_url.as_deref(), Some("https://solx.example.com"));
}

#[tokio::test]
async fn a_code_can_name_its_own_link_address() {
    let (_d, r) = setup().await;
    r.config
        .set("server_public_url", "https://solx.example.com".into())
        .unwrap();
    r.set_advertised_url("http://192.168.1.5:9000".into());

    // A bare IP takes the port the server is reached on (here the public
    // URL's, https → 443).
    let code = r
        .create_connect_code(None, None, None, Some("100.101.102.103"))
        .await
        .unwrap();
    assert_eq!(
        code.server_url.as_deref(),
        Some("http://100.101.102.103:443")
    );

    r.config
        .set("server_public_url", serde_json::Value::Null)
        .unwrap();
    let code = r
        .create_connect_code(None, None, None, Some("100.101.102.103"))
        .await
        .unwrap();
    assert_eq!(
        code.url.unwrap(),
        format!("http://100.101.102.103:9000/connect#code={}", code.code)
    );

    let code = r
        .create_connect_code(None, None, None, Some("https://laptop.tailnet.ts.net"))
        .await
        .unwrap();
    assert_eq!(
        code.server_url.as_deref(),
        Some("https://laptop.tailnet.ts.net")
    );

    assert!(r
        .create_connect_code(None, None, None, Some("not a host"))
        .await
        .is_err());
}

// ── across processes ─────────────────────────────────────────────────────────

#[tokio::test]
async fn two_registries_on_one_appdata_dir_agree() {
    let dir = tempfile::tempdir().unwrap();
    let a = open_in(dir.path()).await;
    let b = open_in(dir.path()).await;

    // A revoke in one is rejected by the other at once.
    let client = a.issue_client(None, None).await.unwrap();
    assert!(b.authenticate(&client.token).await.is_some());
    a.revoke_client(&client.client_id).await.unwrap();
    assert_eq!(b.authenticate(&client.token).await, None);

    // An action token minted by one is accepted by the other: the
    // sibling-server case.
    let cred = a.issue_action_token("/a", "i", MINUTE).await.unwrap();
    assert!(matches!(
        b.authenticate(&cred.token).await,
        Some(Principal::Action { .. })
    ));

    // A code created by one (the CLI) is redeemed by the other (the host).
    let code = a.create_connect_code(None, None, None, None).await.unwrap();
    b.redeem(&code.code).await.unwrap();
}
