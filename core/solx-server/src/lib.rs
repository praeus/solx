//! HTTP server hosting the local solx managers (`solx_manager::App::build_local()`)
//! so multiple `solx-cli`/`solx-mcp` processes can share one appdata dir
//! concurrently, proxying through `solx-client` instead of each opening
//! their own local libsql/SQLite storage directly.
//!
//! The surface is REST: an entity's `path`/`name` are the URL
//! (`GET /docs/research/ai/note`), `list`/`search` options are the query
//! string, and the method carries the verb. `docs/http-api.md` documents it
//! for anyone writing a client; `routes/refs.rs` explains how a URL is read
//! back into a `(path, name)` pair, and each route module's `router()`
//! records why its non-CRUD operations sit where they do.
//!
//! Also mounts `/mcp` (`routes::mcp`), the MCP Streamable HTTP transport —
//! sharing this same in-process `Arc<App>` and bearer-auth gate, so an MCP
//! client can reach solx over HTTP instead of spawning `solx-mcp` as a
//! stdio subprocess (which otherwise keeps the exe locked for the session's
//! lifetime). `solx-mcp` still exists and works over stdio for clients that
//! need a subprocess transport; this is an additional way in, not a
//! replacement.

pub mod auth;
pub mod error;
pub mod routes;
pub mod state;
#[cfg(feature = "tls")]
pub mod tls;

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::http::HeaderValue;
use axum::middleware;
use axum::routing::get;
use axum::Router;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use state::AppState;

/// What this binary was built from. Stamped by `build.rs` and reported by
/// `/health`, because a stale server is otherwise indistinguishable from a
/// current one until something behaves impossibly.
pub const BUILD_COMMIT: &str = env!("SOLX_BUILD_COMMIT");
pub const BUILD_COMMIT_DATE: &str = env!("SOLX_BUILD_COMMIT_DATE");

/// Build the full router: an unauthenticated `/health` and `/ui/*` (and,
/// with the token registry, `/auth/exchange` and `/connect`), plus every
/// data route behind the bearer-auth middleware.
///
/// Serve it with `into_make_service_with_connect_info::<SocketAddr>()`
/// (as [`serve`] does): with the registry on, the root token is accepted
/// only from loopback peers, and a server without peer addresses treats
/// every peer as remote.
///
/// CORS is wide open (`CorsLayer::permissive()`) so browser-based clients
/// (e.g. `solx-web`'s frontend, via `@solx/http`) can call this server
/// directly. This is a deliberate widening of the trust boundary — until
/// now only Rust/Node clients could reach `solx-server` — but it's
/// consistent with the existing model on a loopback bind: the bearer token
/// is the only real gate, same posture `solx-web`'s Bun backend already
/// used (CORS `*`) for its own callers. A non-loopback bind uses
/// `build_router_with_cors` with an allow-list instead (see `cors_layer`).
pub fn build_router(state: AppState) -> Router {
    build_router_with_cors(state, CorsLayer::permissive())
}

/// `build_router` with an explicit CORS policy. The layer is outermost
/// (after `.merge`) so a preflight `OPTIONS` is answered before it ever
/// reaches `require_bearer`. That matters for ordinary calls too, since
/// browsers always preflight `PUT`/`DELETE`.
pub fn build_router_with_cors(state: AppState, cors: CorsLayer) -> Router {
    let protected = routes::router(&state).route_layer(middleware::from_fn_with_state(
        state.clone(),
        auth::require_bearer,
    ));

    #[allow(unused_mut)]
    let mut public = Router::new();
    #[cfg(feature = "registry")]
    if state.registry.is_some() {
        public = public.merge(routes::connect::router());
    }

    Router::new()
        // Stays unauthenticated, and deliberately reports only what the
        // binary was built from -- enough to spot a stale server, nothing
        // about the data it holds.
        .route(
            "/health",
            get(|| async {
                axum::Json(serde_json::json!({
                    "status": "ok",
                    "version": env!("CARGO_PKG_VERSION"),
                    "commit": BUILD_COMMIT,
                    "commit_date": BUILD_COMMIT_DATE,
                }))
            }),
        )
        // The web UI, when one has been published to the file store; see
        // `routes::ui` for why it is outside the bearer gate.
        .merge(routes::ui::router())
        .merge(public)
        .merge(protected)
        .layer(cors)
        .with_state(state)
}

/// CORS policy for a server bound on `ip`. On loopback it's permissive, as
/// described on `build_router`. Off loopback, only the listed `origins` are
/// allowed (none if the list is empty), so an arbitrary web page can't
/// script requests at a network-exposed server. Non-browser clients ignore
/// CORS entirely, so they're unaffected either way. Origins that don't parse
/// as header values are skipped with a warning.
pub fn cors_layer(ip: IpAddr, origins: &[String]) -> CorsLayer {
    if ip.is_loopback() {
        return CorsLayer::permissive();
    }
    let allowed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| match HeaderValue::from_str(o.trim()) {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!("ignoring invalid server_cors_origins entry {o:?}");
                None
            }
        })
        .collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(allowed))
        .allow_methods(Any)
        .allow_headers(Any)
}

/// Bind `127.0.0.1:port` and start serving in the background. Returns once
/// the listener is bound; the spawned task keeps running until the process
/// exits. For a process (like `solx-cli`) that wants a real HTTP surface
/// available to child processes it spawns, without itself being
/// `solx-server` — see `solx-cli`'s `build_app` for the caller.
pub async fn spawn_embedded(state: AppState, port: u16) -> std::io::Result<SocketAddr> {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    // Loopback only, so plain HTTP whatever `server_tls` says.
    serve_with_tls(
        state,
        addr,
        CorsLayer::permissive(),
        None,
        std::future::pending(),
    )
    .await
}

/// Bind `addr` and serve in the background with the given CORS policy until
/// `shutdown` resolves (then drains in-flight requests). Returns once the
/// listener is bound. For hosts that embed the server alongside their own
/// use of the same `Arc<App>` — e.g. the solx desktop app's server build —
/// and need a configurable bind and a clean stop, which `spawn_embedded`
/// doesn't offer.
///
/// Serves TLS as the app's `server_tls` config asks (see [`tls_config`]);
/// a `server_tls` that can't be served fails here, before binding.
pub async fn serve(
    state: AppState,
    addr: SocketAddr,
    cors: CorsLayer,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<SocketAddr> {
    let tls = tls_config(&state.app.config).map_err(|e| std::io::Error::other(format!("{e:#}")))?;
    serve_with_tls(state, addr, cors, tls, shutdown).await
}

/// [`serve`] with an explicit TLS configuration instead of the one
/// `server_tls` names. `None` serves plain HTTP.
pub async fn serve_with_tls(
    state: AppState,
    addr: SocketAddr,
    cors: CorsLayer,
    tls: Option<TlsConfig>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    announce_bound(&state, bound, tls.is_some());
    let router = build_router_with_cors(state, cors);
    tokio::spawn(async move {
        if let Err(e) = serve_listener(listener, router, tls, shutdown).await {
            tracing::warn!("embedded solx-server on {bound} exited: {e}");
        }
    });
    Ok(bound)
}

/// A TLS configuration ready to serve.
#[cfg(feature = "tls")]
pub type TlsConfig = std::sync::Arc<rustls::ServerConfig>;

/// Without the `tls` feature there is no TLS configuration to give, so an
/// `Option<TlsConfig>` is always `None`.
#[cfg(not(feature = "tls"))]
#[derive(Debug, Clone)]
pub enum TlsConfig {}

/// The TLS configuration `config`'s `server_tls` asks for, or `None` when
/// it is off. Fails when it names certificates that can't be loaded, or
/// when this build has no `tls` feature.
pub fn tls_config(config: &solx_config::ConfigService) -> anyhow::Result<Option<TlsConfig>> {
    #[cfg(feature = "tls")]
    {
        tls::source_from_config(config)?
            .map(|source| tls::server_config(&source))
            .transpose()
    }
    #[cfg(not(feature = "tls"))]
    {
        if config.server_tls()?.is_on() {
            anyhow::bail!("server_tls is set, but this solx-server was built without TLS support");
        }
        Ok(None)
    }
}

/// Serve `router` on `listener` until `shutdown` resolves, then drain
/// in-flight requests. Handlers see each peer as `ConnectInfo<SocketAddr>`.
/// With `tls`, plain HTTP is still served to loopback peers on the same
/// port (see [`tls::serve_listener`]); without it, this is `axum::serve`.
pub async fn serve_listener(
    listener: tokio::net::TcpListener,
    router: Router,
    tls: Option<TlsConfig>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    #[cfg(feature = "tls")]
    {
        tls::serve_listener(listener, router, tls, shutdown).await
    }
    #[cfg(not(feature = "tls"))]
    match tls {
        None => {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown)
            .await
        }
        Some(never) => match never {},
    }
}

/// Tell the authority where this process now serves HTTP, and start the
/// token registry's periodic sweep. For every host that binds a server,
/// [`serve`] included. `https` says whether other machines reach it over
/// TLS; this machine's own callers always get plain `http://` on loopback.
pub fn announce_bound(state: &AppState, bound: SocketAddr, https: bool) {
    state.auth.set_server_url(server_url(bound));
    #[cfg(feature = "registry")]
    if let Some(registry) = &state.registry {
        // A loopback bind isn't reachable from elsewhere, so it says nothing
        // about where other devices should connect; keep the configured
        // address for links (the CLI's embedded server binds loopback even
        // when the host it stands in for is on the network).
        if !bound.ip().is_loopback() {
            registry.set_advertised_url(solx_registry::advertised_url(bound, https));
        }
        registry.spawn_sweeper();
    }
    #[cfg(not(feature = "registry"))]
    let _ = https;
}

/// The URL a spawned Command action uses to reach a server bound on
/// `bound`. An unspecified bind (`0.0.0.0`/`::`) is reached over loopback,
/// since the action runs on this same machine. Always `http://`, even with
/// TLS on: plain HTTP stays open to loopback peers, so packages need no CA.
pub fn server_url(bound: SocketAddr) -> String {
    let mut addr = bound;
    if addr.ip().is_unspecified() {
        addr.set_ip(match addr {
            SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            SocketAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        });
    }
    format!("http://{addr}")
}

/// Resolve the address to bind on: explicit `host`/`port` overrides, else
/// config's `server_host`/`server_port`, else `127.0.0.1` and
/// `solx_config::DEFAULT_SERVER_PORT`.
pub fn bind_addr(
    snap: &solx_config::SolxConfig,
    host: Option<IpAddr>,
    port: Option<u16>,
) -> anyhow::Result<SocketAddr> {
    let host = match host {
        Some(host) => host,
        None => match snap
            .server_host
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(s) => s
                .parse::<IpAddr>()
                .map_err(|_| anyhow::anyhow!("config server_host {s:?} is not an IP address"))?,
            None => IpAddr::V4(Ipv4Addr::LOCALHOST),
        },
    };
    let port = port
        .or(snap.server_port)
        .unwrap_or(solx_config::DEFAULT_SERVER_PORT);
    Ok(SocketAddr::new(host, port))
}

#[cfg(test)]
mod tests {
    use super::server_url;

    #[test]
    fn server_url_reaches_unspecified_binds_over_loopback() {
        assert_eq!(
            server_url("0.0.0.0:8766".parse().unwrap()),
            "http://127.0.0.1:8766"
        );
        assert_eq!(
            server_url("[::]:8766".parse().unwrap()),
            "http://[::1]:8766"
        );
        assert_eq!(
            server_url("10.0.0.5:8766".parse().unwrap()),
            "http://10.0.0.5:8766"
        );
    }
}
