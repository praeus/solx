//! Connecting another device: `POST /auth/exchange` redeems a connect code
//! for a client token, and `GET /connect` is the page a connect link
//! (`<public url>/connect#code=12345678`) opens.
//!
//! Both are mounted **outside** the bearer gate, and only with the token
//! registry on. The exchange is the one unauthenticated, state-changing
//! endpoint, so every failure counts against the registry's global failure
//! budget, and a peer that just failed waits a moment before its next try.
//!
//! `GET /connect` changes nothing: it is a static page. The code travels in
//! the URL fragment, which browsers never send, so it stays out of access
//! logs, proxies and `Referer`. Link previewers and mail scanners fetch
//! with GET and don't submit the page's form, so they can't use a code up.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use solx_surface::error::SolxError;

use crate::error::ApiError;
use crate::state::AppState;

/// How long a peer waits after a failed exchange before it may try again.
const BACKOFF: Duration = Duration::from_secs(1);

/// Per-IP backoff after a failed exchange: an extra layer on top of the
/// registry's global failure budget, which is what actually bounds guessing
/// (an attacker can spread guesses across many addresses).
#[derive(Default)]
pub struct Backoff(Mutex<HashMap<IpAddr, Instant>>);

impl Backoff {
    fn waiting(&self, ip: IpAddr) -> bool {
        let map = self.0.lock().unwrap();
        map.get(&ip).is_some_and(|until| *until > Instant::now())
    }

    fn failed(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut map = self.0.lock().unwrap();
        map.retain(|_, until| *until > now);
        map.insert(ip, now + BACKOFF);
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/exchange", post(exchange))
        .route("/connect", get(connect_page))
}

#[derive(Deserialize)]
struct ExchangeRequest {
    code: String,
}

/// `POST /auth/exchange` `{ "code": "1234-5678" }` →
/// `{ "token", "client_id", "expires_at", "server_url" }`.
async fn exchange(
    State(state): State<AppState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(body): Json<ExchangeRequest>,
) -> Result<Response, ApiError> {
    let Some(registry) = state.registry.clone() else {
        return Err(SolxError::NotFound("this server has no token registry".into()).into());
    };
    let ip = peer.map(|ConnectInfo(addr)| addr.ip().to_canonical());
    if ip.is_some_and(|ip| state.exchange_backoff.waiting(ip)) {
        let body =
            SolxError::Unauthorized("too many attempts: wait a second, then try again".into());
        return Ok((StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response());
    }
    match registry.redeem(&body.code).await {
        Ok(issued) => {
            // What the client should store: the configured public address,
            // else the address it just reached us on.
            let server_url = state
                .app
                .config
                .server_public_url()
                .or_else(|| request_origin(&headers))
                .or_else(|| registry.public_url());
            Ok(Json(json!({
                "token": issued.token,
                "client_id": issued.client_id,
                "expires_at": issued.expires_at,
                "server_url": server_url,
            }))
            .into_response())
        }
        Err(e) => {
            if let (Some(ip), SolxError::Unauthorized(_)) = (ip, &e) {
                state.exchange_backoff.failed(ip);
            }
            Err(e.into())
        }
    }
}

/// `http://<Host>` for the request, when it carries a usable `Host`.
fn request_origin(headers: &HeaderMap) -> Option<String> {
    let host = headers.get(header::HOST)?.to_str().ok()?.trim();
    let valid = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-.:[]".contains(c));
    valid.then(|| format!("http://{host}"))
}

/// `GET /connect`: the static page a connect link opens.
async fn connect_page() -> impl IntoResponse {
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Html(CONNECT_PAGE),
    )
}

const CONNECT_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Connect to solx</title>
<style>
  :root { color-scheme: light dark; --fg: #1d1d1f; --muted: #6e6e73; --bg: #fff; --line: #d2d2d7; --accent: #0a66c2; --err: #b3261e; }
  @media (prefers-color-scheme: dark) { :root { --fg: #f5f5f7; --muted: #a1a1a6; --bg: #1c1c1e; --line: #3a3a3c; --accent: #4c9bf0; --err: #f2b8b5; } }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.5 system-ui, sans-serif; }
  main { max-width: 32rem; margin: 0 auto; padding: 3rem 1rem; }
  h1 { font-size: 1.5rem; margin: 0 0 .5rem; }
  p { color: var(--muted); }
  form { display: flex; gap: .5rem; margin: 1.5rem 0; }
  input, textarea { font: inherit; color: inherit; background: transparent; border: 1px solid var(--line); border-radius: 8px; padding: .6rem .75rem; }
  input { flex: 1; min-width: 0; font-size: 1.4rem; letter-spacing: .1em; font-variant-numeric: tabular-nums; }
  textarea { width: 100%; font-family: ui-monospace, monospace; font-size: .9rem; resize: none; }
  button { font: inherit; border: 0; border-radius: 8px; padding: .6rem 1rem; background: var(--accent); color: #fff; cursor: pointer; }
  button:disabled { opacity: .6; cursor: default; }
  #error { color: var(--err); min-height: 1.5em; }
  code { font-family: ui-monospace, monospace; font-size: .9em; }
</style>
</head>
<body>
<main>
  <h1>Connect to solx</h1>
  <p>Enter the 8-digit connect code shown on the solx host.</p>
  <form id="form">
    <input id="code" inputmode="numeric" autocomplete="one-time-code" placeholder="1234-5678" maxlength="9" required aria-label="Connect code">
    <button id="submit">Connect</button>
  </form>
  <p id="error" role="alert"></p>
  <section id="done" hidden>
    <p>Connected. This browser's token for <strong id="server"></strong> is below, valid until <span id="expires"></span>. It is shown only once.</p>
    <textarea id="token" rows="2" readonly aria-label="Token"></textarea>
    <p><button id="copy" type="button">Copy token</button></p>
    <p>Paste it into solx-web's Settings or an MCP client's configuration, or on another machine run <code>solx connect &lt;server&gt; &lt;token&gt;</code>.</p>
  </section>
</main>
<script>
  const $ = (id) => document.getElementById(id);
  // Read the code from the fragment, then drop it from the address bar and
  // history. The form still needs a click, so a scanner that runs scripts
  // can't spend the code just by opening the link.
  const match = location.hash.match(/code=([0-9 -]+)/);
  if (match) {
    $("code").value = match[1];
    history.replaceState(null, "", location.pathname + location.search);
  }
  $("form").addEventListener("submit", async (event) => {
    event.preventDefault();
    $("error").textContent = "";
    const digits = $("code").value.replace(/[\s-]/g, "");
    if (!/^\d{8}$/.test(digits)) {
      $("error").textContent = "A connect code is 8 digits.";
      return;
    }
    $("submit").disabled = true;
    try {
      const res = await fetch(new URL("auth/exchange", location.href), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ code: digits }),
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(body.message || ("HTTP " + res.status));
      const server = body.server_url || location.origin;
      try {
        localStorage.setItem("solx.serverUrl", server);
        localStorage.setItem("solx.serverToken", body.token);
      } catch (_) {}
      $("form").hidden = true;
      $("server").textContent = server;
      $("expires").textContent = body.expires_at ? new Date(body.expires_at).toLocaleString() : "revoked";
      $("token").value = body.token;
      $("done").hidden = false;
    } catch (err) {
      $("error").textContent = err.message;
    } finally {
      $("submit").disabled = false;
    }
  });
  $("copy").addEventListener("click", async () => {
    $("token").select();
    try { await navigator.clipboard.writeText($("token").value); $("copy").textContent = "Copied"; }
    catch (_) { document.execCommand("copy"); }
  });
</script>
</body>
</html>
"#;
