//! `solx-registry` — scoped, expiring bearer tokens, replacing the single
//! shared root token as the [`TokenAuthority`] when solx is built with the
//! (default-on) `registry` feature.
//!
//! - **Root token.** Still accepted, checked in memory before any database
//!   access, and still what local hosts and the CLI use.
//! - **Action tokens.** One per external Command invocation, handed over as
//!   `SOLX_SERVER_TOKEN` and deleted when the invocation ends (or, after a
//!   crash, when it expires).
//! - **Client tokens.** One per client (a laptop, a browser, an MCP
//!   client), issued directly or by redeeming a short **connect code**, and
//!   listed and revoked individually.
//!
//! Everything lives in `solx-registry.db` rather than process memory, so a
//! revocation in one process applies in every other one on its next
//! request, `solx token connect-code` (a CLI process) can mint a code a
//! running host redeems, and a sibling `solx-server` accepts action tokens
//! minted by a process that serves no HTTP itself (stdio `solx-mcp`).
//!
//! Only SHA-256 hashes of tokens and codes are stored, and lookup is by
//! hash, so no constant-time comparison is needed past the root token.
//! A code's hash hides little (there are only 10^8 codes); see SECURITY.md.

pub mod actions;
mod db;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use libsql::{Connection, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use solx_config::credentials::{self, ServerUrlSlot};
use solx_config::ConfigService;
use solx_surface::auth::{constant_time_eq, ActionCredential, Principal, TokenAuthority};
use solx_surface::error::{Result, SolxError};
use uuid::Uuid;

use db::{map_db, Db};

/// Extra life an action token gets past its invocation's timeout, so a
/// command finishing right at its deadline can still make its last call.
const ACTION_GRACE: Duration = Duration::from_secs(60);
/// A connect code's lifetime when the caller names none.
pub const DEFAULT_CODE_TTL: Duration = Duration::from_secs(10 * 60);
/// The longest a connect code may live.
const MAX_CODE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Failed exchanges, across every process and peer, after which every
/// pending connect code is deleted.
pub const FAILURE_BUDGET: i64 = 10;
/// How often `last_used_at` is written, at most, per token.
const LAST_USED_INTERVAL: Duration = Duration::from_secs(60);
/// How often a long-lived host's sweeper runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);

const FAILED_EXCHANGES: &str = "failed_exchanges";

/// The token registry. See the crate docs.
pub struct Registry {
    db: Db,
    config: Arc<ConfigService>,
    root_token: String,
    server_url: ServerUrlSlot,
    /// Where other machines reach this host when `server_public_url` is
    /// unset; see [`Self::public_url`].
    advertised: ServerUrlSlot,
    /// When each token's `last_used_at` was last written by this process.
    last_used: Mutex<HashMap<String, Instant>>,
    /// Dropped action credentials whose rows are still to delete.
    pending: Arc<Mutex<PendingReleases>>,
}

/// A freshly created connect code. The code itself is secret until it is
/// redeemed, so this type has no `Debug`.
#[derive(Serialize)]
pub struct ConnectCode {
    /// The 8 digits, as `/auth/exchange` accepts them.
    pub code: String,
    /// The code as shown to a person: `1234-5678`.
    pub display_code: String,
    /// `<public url>/connect#code=12345678`, when this host knows its URL,
    /// with `&pin=<tls_pin>` when it has one.
    pub url: Option<String>,
    /// The address to type next to the code on the other device.
    pub server_url: Option<String>,
    /// The root pin clients should hold for this server (see
    /// [`Registry::tls_pin`]).
    pub tls_pin: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// A newly issued client token. Shown once; only its hash is kept.
#[derive(Serialize)]
pub struct IssuedClient {
    pub token: String,
    pub client_id: String,
    pub label: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// A client token as `list_clients` reports it: never the token or hash.
#[derive(Debug, Clone, Serialize)]
pub struct ClientInfo {
    pub id: String,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl Registry {
    /// Open (creating if needed) `solx-registry.db` for `config`'s appdata
    /// dir, resolve the root token, and sweep expired rows.
    pub async fn open(config: Arc<ConfigService>) -> Result<Arc<Self>> {
        let db = Db::open(&config.registry_db_path()).await?;
        db::migrate(&db.connect().await?).await?;
        let registry = Arc::new(Registry {
            db,
            root_token: credentials::root_token(&config)?,
            server_url: ServerUrlSlot::new(Some(credentials::sibling_server_url(&config))),
            advertised: ServerUrlSlot::new(Some(configured_public_url(&config))),
            config,
            last_used: Mutex::new(HashMap::new()),
            pending: Arc::default(),
        });
        if let Err(e) = registry.sweep().await {
            tracing::warn!("token registry sweep failed: {e}");
        }
        Ok(registry)
    }

    /// Sweep every 10 minutes for as long as this registry is alive. For
    /// long-lived hosts; a one-shot process relies on the sweep in `open`.
    pub fn spawn_sweeper(self: &Arc<Self>) {
        let weak: Weak<Self> = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SWEEP_INTERVAL);
            tick.tick().await; // the first tick is immediate; `open` swept
            loop {
                tick.tick().await;
                let Some(registry) = weak.upgrade() else {
                    return;
                };
                if let Err(e) = registry.sweep().await {
                    tracing::warn!("token registry sweep failed: {e}");
                }
            }
        });
    }

    /// Delete expired action tokens, client tokens that can never be used
    /// again (expired or revoked), and expired or consumed connect codes.
    pub async fn sweep(&self) -> Result<()> {
        self.release_pending().await;
        let now = now_secs();
        let conn = self.db.connect().await?;
        conn.execute(
            "DELETE FROM tokens WHERE (expires_at IS NOT NULL AND expires_at <= ?1) \
             OR revoked_at IS NOT NULL",
            [now],
        )
        .await
        .map_err(map_db)?;
        conn.execute(
            "DELETE FROM connect_codes WHERE expires_at <= ?1 OR consumed_at IS NOT NULL",
            [now],
        )
        .await
        .map_err(map_db)?;
        Ok(())
    }

    fn take_pending(&self) -> Vec<String> {
        std::mem::take(&mut self.pending.lock().unwrap().queue)
    }

    /// Delete the rows of action credentials dropped without `release`
    /// (see [`ReleaseGuard`]). On failure they're queued again.
    async fn release_pending(&self) {
        let ids = self.take_pending();
        if ids.is_empty() {
            return;
        }
        if let Err(e) = delete_action_tokens(&self.db, &ids).await {
            tracing::warn!("token registry: could not release dropped action tokens: {e}");
            self.pending.lock().unwrap().queue.extend(ids);
        }
    }

    /// Where other machines reach this host, for connect links and the
    /// exchange's `server_url`: `server_public_url`, else the address the
    /// server is bound on (see [`advertised_url`]), else as configured.
    pub fn public_url(&self) -> Option<String> {
        self.config
            .server_public_url()
            .or_else(|| self.advertised.get())
            .or_else(|| self.server_url.get())
    }

    /// The pin of the root this host's server presents, for clients to
    /// hold: `server_tls: "auto"`'s, once its certificates exist (see
    /// [`ConfigService::server_tls_pin`]). Handed out with connect codes
    /// and issued tokens; never secret.
    pub fn tls_pin(&self) -> Option<String> {
        self.config.server_tls_pin()
    }

    /// Record the address a host's server is reachable on from other
    /// machines, once bound. See [`advertised_url`].
    pub fn set_advertised_url(&self, url: String) {
        self.advertised.set(url);
    }

    // ── connect codes ────────────────────────────────────────────────────────

    /// The port this host's server is reached on: that of [`Self::public_url`],
    /// else `server_port`. What a bare host given as a link address gets.
    fn public_port(&self) -> u16 {
        self.public_url()
            .and_then(|u| url::Url::parse(&u).ok())
            .and_then(|u| u.port_or_known_default())
            .unwrap_or_else(|| {
                self.config
                    .snapshot()
                    .server_port
                    .unwrap_or(solx_config::DEFAULT_SERVER_PORT)
            })
    }

    /// A new single-use code, valid for `code_ttl` (default 10 minutes),
    /// that redeems for a client token valid for `token_ttl` (default
    /// `client_token_ttl_days`).
    ///
    /// `public_url` overrides [`Self::public_url`] for this code's link and
    /// server address, e.g. a Tailscale IP when the other device isn't on
    /// the LAN. It takes a URL or a bare host (see
    /// [`solx_config::normalize_public_url`]); a bare host gets this
    /// server's port, and `https://` when it serves TLS. It only shapes
    /// what is shown: the code redeems at any address the server answers
    /// on.
    pub async fn create_connect_code(
        &self,
        label: Option<&str>,
        code_ttl: Option<Duration>,
        token_ttl: Option<Duration>,
        public_url: Option<&str>,
    ) -> Result<ConnectCode> {
        let base = match public_url.map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => {
                let https = self.config.server_tls().is_ok_and(|tls| tls.is_on());
                Some(solx_config::normalize_public_url(
                    u,
                    self.public_port(),
                    https,
                )?)
            }
            None => self.public_url(),
        };
        let code_ttl = code_ttl.unwrap_or(DEFAULT_CODE_TTL);
        if code_ttl.is_zero() || code_ttl > MAX_CODE_TTL {
            return Err(SolxError::Invalid(format!(
                "a connect code's lifetime must be between 1 second and {} seconds",
                MAX_CODE_TTL.as_secs()
            )));
        }
        if token_ttl.is_some_and(|t| t.is_zero()) {
            return Err(SolxError::Invalid(
                "a client token's lifetime must be at least 1 second".into(),
            ));
        }
        let label = clean_label(label);
        let now = now_secs();
        let expires_at = now + secs(code_ttl);
        let conn = self.db.connect().await?;
        for _ in 0..16 {
            let code = random_code();
            let hash = hash_secret(&code);
            // A spent or expired row holding the same hash would block the
            // insert; a pending one means pick another code.
            conn.execute(
                "DELETE FROM connect_codes WHERE code_hash = ?1 \
                 AND (consumed_at IS NOT NULL OR expires_at <= ?2)",
                libsql::params![hash.clone(), now],
            )
            .await
            .map_err(map_db)?;
            let inserted = conn
                .execute(
                    "INSERT OR IGNORE INTO connect_codes \
                     (code_hash, label, token_ttl_secs, created_at, expires_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    libsql::params![hash, label.clone(), token_ttl.map(secs), now, expires_at],
                )
                .await
                .map_err(map_db)?;
            if inserted == 1 {
                tracing::info!(label = ?label, "token registry: created a connect code");
                // Only on an https:// address: a client refuses a pin on
                // plain HTTP.
                let tls_pin = self
                    .tls_pin()
                    .filter(|_| base.as_deref().is_some_and(|b| b.starts_with("https://")));
                let url = base.as_ref().map(|base| match &tls_pin {
                    Some(pin) => format!("{base}/connect#code={code}&pin={pin}"),
                    None => format!("{base}/connect#code={code}"),
                });
                return Ok(ConnectCode {
                    display_code: format!("{}-{}", &code[..4], &code[4..]),
                    code,
                    url,
                    server_url: base,
                    tls_pin,
                    expires_at: to_datetime(expires_at),
                });
            }
        }
        Err(SolxError::Conflict(
            "could not find an unused connect code; try again".into(),
        ))
    }

    /// Redeem a connect code for a client token. Input ignores everything
    /// but digits, so `12345678`, `1234 5678` and `1234-5678` all work.
    ///
    /// A failed redemption counts against the global failure budget: at
    /// [`FAILURE_BUDGET`] failures every pending code is deleted and the
    /// count starts over. Neither a success nor a new code resets it, so an
    /// attacker can't refill it.
    pub async fn redeem(&self, code: &str) -> Result<IssuedClient> {
        let invalid = || {
            SolxError::Unauthorized(
                "the connect code is not valid: it may have expired or been used already. \
                 Generate a new one on the solx host."
                    .into(),
            )
        };
        // Only a well-formed code is a guess worth counting.
        let Some(digits) = credentials::connect_code_digits(code) else {
            return Err(invalid());
        };
        let hash = hash_secret(&digits);
        let now = now_secs();
        let conn = self.db.connect().await?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .await
            .map_err(map_db)?;
        let consumed = tx
            .execute(
                "UPDATE connect_codes SET consumed_at = ?1 \
                 WHERE code_hash = ?2 AND consumed_at IS NULL AND expires_at > ?1",
                libsql::params![now, hash.clone()],
            )
            .await
            .map_err(map_db)?;
        if consumed != 1 {
            let failures = record_failure(&tx).await?;
            tx.commit().await.map_err(map_db)?;
            tracing::warn!(failures, "token registry: a connect code exchange failed");
            return Err(invalid());
        }
        let mut rows = tx
            .query(
                "SELECT label, token_ttl_secs FROM connect_codes WHERE code_hash = ?1",
                [hash],
            )
            .await
            .map_err(map_db)?;
        let row = rows.next().await.map_err(map_db)?.ok_or_else(invalid)?;
        let label: Option<String> = row.get(0).map_err(map_db)?;
        let token_ttl: Option<i64> = row.get(1).map_err(map_db)?;
        drop(rows);
        let ttl = token_ttl.map(|s| Duration::from_secs(s.max(1) as u64));
        let issued = self.insert_client(&tx, label, ttl).await?;
        tx.commit().await.map_err(map_db)?;
        tracing::info!(
            client_id = %issued.client_id,
            label = ?issued.label,
            "token registry: redeemed a connect code"
        );
        Ok(issued)
    }

    /// The global failed-exchange count, for tests and diagnostics.
    pub async fn failed_exchanges(&self) -> Result<i64> {
        let conn = self.db.connect().await?;
        read_failures(&conn).await
    }

    // ── client tokens ────────────────────────────────────────────────────────

    /// Mint a client token directly, valid for `ttl` (default
    /// `client_token_ttl_days`).
    pub async fn issue_client(
        &self,
        label: Option<&str>,
        ttl: Option<Duration>,
    ) -> Result<IssuedClient> {
        if ttl.is_some_and(|t| t.is_zero()) {
            return Err(SolxError::Invalid(
                "a client token's lifetime must be at least 1 second".into(),
            ));
        }
        let conn = self.db.connect().await?;
        let issued = self.insert_client(&conn, clean_label(label), ttl).await?;
        tracing::info!(
            client_id = %issued.client_id,
            label = ?issued.label,
            "token registry: issued a client token"
        );
        Ok(issued)
    }

    async fn insert_client(
        &self,
        conn: &Connection,
        label: Option<String>,
        ttl: Option<Duration>,
    ) -> Result<IssuedClient> {
        let ttl = ttl.unwrap_or_else(|| {
            Duration::from_secs(
                (self.config.client_token_ttl_days() as u64).saturating_mul(24 * 60 * 60),
            )
        });
        let now = now_secs();
        let expires_at = now + secs(ttl);
        let token = credentials::generate_token();
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tokens (id, kind, token_hash, label, created_at, expires_at) \
             VALUES (?1, 'client', ?2, ?3, ?4, ?5)",
            libsql::params![
                id.clone(),
                hash_secret(&token),
                label.clone(),
                now,
                expires_at
            ],
        )
        .await
        .map_err(map_db)?;
        Ok(IssuedClient {
            token,
            client_id: id,
            label,
            expires_at: Some(to_datetime(expires_at)),
        })
    }

    /// Every client token that still works, oldest first.
    pub async fn list_clients(&self) -> Result<Vec<ClientInfo>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, label, created_at, expires_at, last_used_at FROM tokens \
                 WHERE kind = 'client' AND revoked_at IS NULL \
                 AND (expires_at IS NULL OR expires_at > ?1) \
                 ORDER BY created_at, id",
                [now_secs()],
            )
            .await
            .map_err(map_db)?;
        let mut clients = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_db)? {
            clients.push(ClientInfo {
                id: row.get(0).map_err(map_db)?,
                label: row.get(1).map_err(map_db)?,
                created_at: to_datetime(row.get(2).map_err(map_db)?),
                expires_at: row.get::<Option<i64>>(3).map_err(map_db)?.map(to_datetime),
                last_used_at: row.get::<Option<i64>>(4).map_err(map_db)?.map(to_datetime),
            });
        }
        Ok(clients)
    }

    /// Revoke a client token. Takes effect in every process on its next
    /// request. `NotFound` if no working client token has this id.
    pub async fn revoke_client(&self, id: &str) -> Result<()> {
        let conn = self.db.connect().await?;
        let changed = conn
            .execute(
                "UPDATE tokens SET revoked_at = ?1 \
                 WHERE id = ?2 AND kind = 'client' AND revoked_at IS NULL",
                libsql::params![now_secs(), id.to_string()],
            )
            .await
            .map_err(map_db)?;
        if changed == 0 {
            return Err(SolxError::NotFound(format!("no client token with id {id}")));
        }
        tracing::info!(client_id = %id, "token registry: revoked a client token");
        Ok(())
    }

    // ── authentication ───────────────────────────────────────────────────────

    async fn lookup(&self, bearer: &str) -> Result<Option<(String, Principal)>> {
        let conn = self.db.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, kind, label, action_ref, invocation_id FROM tokens \
                 WHERE token_hash = ?1 AND revoked_at IS NULL \
                 AND (expires_at IS NULL OR expires_at > ?2)",
                libsql::params![hash_secret(bearer), now_secs()],
            )
            .await
            .map_err(map_db)?;
        let Some(row) = rows.next().await.map_err(map_db)? else {
            return Ok(None);
        };
        let id: String = row.get(0).map_err(map_db)?;
        let kind: String = row.get(1).map_err(map_db)?;
        let principal = match kind.as_str() {
            "client" => Principal::Client {
                id: id.clone(),
                label: row.get(2).map_err(map_db)?,
            },
            _ => Principal::Action {
                action_ref: row
                    .get::<Option<String>>(3)
                    .map_err(map_db)?
                    .unwrap_or_default(),
                invocation_id: row
                    .get::<Option<String>>(4)
                    .map_err(map_db)?
                    .unwrap_or_default(),
            },
        };
        Ok(Some((id, principal)))
    }

    /// Write `last_used_at`, at most once a minute per token per process.
    async fn touch(&self, id: &str) {
        {
            let mut seen = self.last_used.lock().unwrap();
            let now = Instant::now();
            if seen
                .get(id)
                .is_some_and(|t| now.duration_since(*t) < LAST_USED_INTERVAL)
            {
                return;
            }
            seen.insert(id.to_string(), now);
        }
        let result = async {
            let conn = self.db.connect().await?;
            conn.execute(
                "UPDATE tokens SET last_used_at = ?1 WHERE id = ?2",
                libsql::params![now_secs(), id.to_string()],
            )
            .await
            .map_err(map_db)
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("token registry: could not record last use: {e}");
        }
    }
}

#[async_trait]
impl TokenAuthority for Registry {
    async fn authenticate(&self, bearer: &str) -> Option<Principal> {
        if constant_time_eq(bearer.as_bytes(), self.root_token.as_bytes()) {
            return Some(Principal::Root);
        }
        if bearer.is_empty() {
            return None;
        }
        self.release_pending().await;
        // No cache: a revocation by another process applies on the next
        // request, at the cost of one indexed read.
        match self.lookup(bearer).await {
            Ok(Some((id, principal))) => {
                self.touch(&id).await;
                Some(principal)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("token registry lookup failed: {e}");
                None
            }
        }
    }

    async fn issue_action_token(
        &self,
        action_ref: &str,
        invocation_id: &str,
        ttl: Duration,
    ) -> Option<ActionCredential> {
        self.release_pending().await;
        let token = credentials::generate_token();
        let id = Uuid::new_v4().to_string();
        let now = now_secs();
        let expires_at = now + secs(ttl) + secs(ACTION_GRACE);
        let inserted = async {
            let conn = self.db.connect().await?;
            conn.execute(
                "INSERT INTO tokens \
                 (id, kind, token_hash, action_ref, invocation_id, created_at, expires_at) \
                 VALUES (?1, 'action', ?2, ?3, ?4, ?5, ?6)",
                libsql::params![
                    id.clone(),
                    hash_secret(&token),
                    action_ref.to_string(),
                    invocation_id.to_string(),
                    now,
                    expires_at
                ],
            )
            .await
            .map_err(map_db)
        }
        .await;
        if let Err(e) = inserted {
            // The action runs without server access rather than not at all.
            tracing::warn!(
                action_ref,
                "token registry: could not issue an action token: {e}"
            );
            return None;
        }
        let guard = ReleaseGuard {
            pending: self.pending.clone(),
            id: id.clone(),
        };
        Some(
            ActionCredential::new(self.server_url.get(), token)
                .with_id(id)
                .with_guard(guard),
        )
    }

    async fn release(&self, credential: &ActionCredential) {
        let Some(id) = &credential.id else {
            return;
        };
        // Its guard then has nothing to queue when the credential drops.
        self.pending.lock().unwrap().released.insert(id.clone());
        let mut ids = self.take_pending();
        ids.push(id.clone());
        if let Err(e) = delete_action_tokens(&self.db, &ids).await {
            tracing::warn!("token registry: could not release an action token: {e}");
        }
    }

    fn set_server_url(&self, url: String) {
        self.server_url.set(url);
    }
}

/// Action credentials dropped without [`TokenAuthority::release`], waiting
/// for the registry's next database call to delete their rows.
#[derive(Default)]
struct PendingReleases {
    /// Ids whose guard dropped first: still to delete.
    queue: Vec<String>,
    /// Ids already deleted by `release`, so their guard queues nothing.
    released: HashSet<String>,
}

/// Fallback release for an action credential dropped without
/// [`TokenAuthority::release`] (an early return or a panic).
///
/// `Drop` does no I/O. It only queues the id; the registry deletes queued
/// rows at the start of its next database call (`authenticate`,
/// `issue_action_token`, `release`, `sweep`), so in this process a dropped
/// token is rejected from the very next authentication. Opening a
/// connection here instead (in a spawned task) could run while a runtime or
/// the process was shutting down. A row whose process never touches the
/// registry again dies at `expires_at`.
struct ReleaseGuard {
    pending: Arc<Mutex<PendingReleases>>,
    id: String,
}

impl Drop for ReleaseGuard {
    fn drop(&mut self) {
        let id = std::mem::take(&mut self.id);
        // A poisoned lock means a panic elsewhere; leave the row to expire.
        if let Ok(mut pending) = self.pending.lock() {
            if !pending.released.remove(&id) {
                pending.queue.push(id);
            }
        }
    }
}

async fn delete_action_tokens(db: &Db, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let conn = db.connect().await?;
    for id in ids {
        conn.execute(
            "DELETE FROM tokens WHERE id = ?1 AND kind = 'action'",
            [id.clone()],
        )
        .await
        .map_err(map_db)?;
    }
    Ok(())
}

async fn read_failures(conn: &Connection) -> Result<i64> {
    let mut rows = conn
        .query(
            "SELECT value FROM registry_state WHERE key = ?1",
            [FAILED_EXCHANGES],
        )
        .await
        .map_err(map_db)?;
    Ok(match rows.next().await.map_err(map_db)? {
        Some(row) => row.get::<String>(0).map_err(map_db)?.parse().unwrap_or(0),
        None => 0,
    })
}

/// Count one failed exchange; at the budget, delete every pending code and
/// start over. Returns the count after this failure (0 after a reset).
async fn record_failure(conn: &Connection) -> Result<i64> {
    let mut failures = read_failures(conn).await? + 1;
    if failures >= FAILURE_BUDGET {
        conn.execute("DELETE FROM connect_codes WHERE consumed_at IS NULL", ())
            .await
            .map_err(map_db)?;
        tracing::warn!(
            "token registry: {FAILURE_BUDGET} failed connect-code exchanges; \
             every pending connect code was invalidated"
        );
        failures = 0;
    }
    conn.execute(
        "INSERT INTO registry_state (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        libsql::params![FAILED_EXCHANGES, failures.to_string()],
    )
    .await
    .map_err(map_db)?;
    Ok(failures)
}

/// The URL other machines use to reach a server bound on `bound`, over
/// `https://` when it serves TLS. An unspecified bind (`0.0.0.0`/`::`) is
/// advertised on this machine's primary LAN address, falling back to
/// loopback.
pub fn advertised_url(bound: SocketAddr, https: bool) -> String {
    let mut addr = bound;
    if addr.ip().is_unspecified() {
        addr.set_ip(primary_ip().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }
    let scheme = if https { "https" } else { "http" };
    format!("{scheme}://{addr}")
}

/// [`advertised_url`] for where this appdata dir's server is configured
/// to listen, before (or without) a bound server of this process's own.
fn configured_public_url(config: &ConfigService) -> String {
    let snap = config.snapshot();
    let port = snap.server_port.unwrap_or(solx_config::DEFAULT_SERVER_PORT);
    let host = snap
        .server_host
        .as_deref()
        .and_then(|h| h.trim().parse::<IpAddr>().ok())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    // A `server_tls` that doesn't parse stops the server from starting, so
    // whatever it would have said doesn't matter here.
    let https = config.server_tls().is_ok_and(|tls| tls.is_on());
    advertised_url(SocketAddr::new(host, port), https)
}

/// The local address of this machine's default route. Connecting a UDP
/// socket sends nothing; it only picks the interface.
fn primary_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn hash_secret(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

/// 8 random decimal digits from the OS RNG.
fn random_code() -> String {
    use rand::Rng;
    format!("{:08}", rand::rngs::OsRng.gen_range(0..100_000_000u32))
}

fn clean_label(label: Option<&str>) -> Option<String> {
    label
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.chars().take(200).collect())
}

fn now_secs() -> i64 {
    Utc::now().timestamp()
}

/// A lifetime in whole seconds, capped at 1000 years so `now + secs(..)`
/// can't overflow and stays a date `to_datetime` can represent.
fn secs(d: Duration) -> i64 {
    const MAX: i64 = 1000 * 365 * 24 * 60 * 60;
    i64::try_from(d.as_secs()).unwrap_or(MAX).min(MAX)
}

fn to_datetime(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0).single().unwrap_or_default()
}

#[cfg(test)]
mod tests;
