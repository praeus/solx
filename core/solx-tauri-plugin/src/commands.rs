//! One command per manager-trait method. Each mirrors a `solx-server` route
//! (noted on the command) and behaves identically, since both just call the
//! same `Arc<dyn _>` manager: local in the server build, a `solx-client`
//! `Remote*` proxy in the client build. Errors are the tagged `SolxError`
//! (`{"kind":"...","message":"..."}`) — the same body an HTTP error carries,
//! so a JS client maps both the same way.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use solx_config::credentials::ConnectCredential;
use solx_config::{ConfigService, EntityKind};
use solx_surface::auth::Principal;
use solx_surface::entities::{
    Action, ActionExecResult, ActionInput, Document, DocumentInput, TypeEntity, TypeInput,
};
use solx_surface::error::{Result, SolxError};
use solx_surface::managers::Solx;
use solx_surface::path::split_ref;
use solx_surface::query::{
    ActionSearchQuery, ExcludeMode, ListOptions, Page, PathFacet, SearchQuery,
};
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{command, State};

use crate::state::{Backend, EmbeddedServer, SolxState};

/// Header carrying `files_put`'s relative path (percent-encoded, since
/// header values must be ASCII), as the body is the raw file bytes.
const FILE_PATH_HEADER: &str = "x-solx-path";

/// Normalize `(path, name)` exactly as `solx-server` does for a URL ref
/// (`routes/refs.rs`): join, then `split_ref`. Guarding the name first
/// matches `@solx/http`'s `entityPath`, where an empty name or one with a
/// `/` would otherwise silently address a different entity.
fn entity(path: &str, name: &str) -> Result<(String, String)> {
    if name.is_empty() || name.contains('/') {
        return Err(SolxError::Invalid(format!("invalid entity name: '{name}'")));
    }
    split_ref(&format!("{}/{}", path.trim_end_matches('/'), name))
}

/// The exclusion scope a command applies when the caller names none - the
/// same `user` default as `solx-server`'s entity routes, since both serve
/// the web UI. Exec is never filtered (the agent widget reaches the builtins
/// through it and asks for the tooling scope itself).
const DEFAULT_MODE: ExcludeMode = ExcludeMode::User;

/// Expand `opts`' mode (or [`DEFAULT_MODE`]) into its `exclude_paths`.
fn scoped(app: &solx_manager::App, kind: EntityKind, opts: Option<ListOptions>) -> ListOptions {
    let mut opts = opts.unwrap_or_default();
    app.config
        .exclusion_policy()
        .apply(kind, &mut opts, DEFAULT_MODE);
    opts
}

// ── types ── (`/types`, `/types/{*ref}`, `/types-paths`, `/validate`)

/// `PUT /types/{*ref}`
#[command]
pub(crate) async fn types_save(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    input: TypeInput,
    exclude_mode: Option<ExcludeMode>,
) -> Result<TypeEntity> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Type,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        true,
    )
    .await?;
    app.types().save(&path, &name, input).await
}

/// `GET /types/{*ref}`
#[command]
pub(crate) async fn types_get(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    exclude_mode: Option<ExcludeMode>,
) -> Result<TypeEntity> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Type,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        false,
    )
    .await?;
    app.types().get(&path, &name).await
}

/// `DELETE /types/{*ref}`
#[command]
pub(crate) async fn types_delete(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    exclude_mode: Option<ExcludeMode>,
) -> Result<()> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Type,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        false,
    )
    .await?;
    app.types().delete(&path, &name).await
}

/// `GET /types`
#[command]
pub(crate) async fn types_list(
    state: State<'_, SolxState>,
    opts: Option<ListOptions>,
) -> Result<Page<TypeEntity>> {
    let app = state.app()?;
    app.types().list(scoped(&app, EntityKind::Type, opts)).await
}

/// `GET /types-paths`
#[command]
pub(crate) async fn types_paths(
    state: State<'_, SolxState>,
    opts: Option<ListOptions>,
) -> Result<Page<PathFacet>> {
    let app = state.app()?;
    app.types()
        .paths(scoped(&app, EntityKind::Type, opts))
        .await
}

/// No route of its own — `@solx/http` resolves via `GET /types/{*ref}`.
#[command]
pub(crate) async fn types_resolve(
    state: State<'_, SolxState>,
    type_ref: String,
) -> Result<TypeEntity> {
    state.app()?.types().resolve(&type_ref).await
}

/// `POST /validate`
#[command]
pub(crate) async fn types_validate(
    state: State<'_, SolxState>,
    value: Value,
    type_ref: String,
) -> Result<()> {
    state.app()?.types().validate(&value, &type_ref).await
}

// ── docs ── (`/docs`, `/docs/{*ref}`, `/docs-paths`, `/search`)

/// `PUT /docs/{*ref}`
#[command]
pub(crate) async fn docs_save(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    input: DocumentInput,
    exclude_mode: Option<ExcludeMode>,
) -> Result<Document> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Document,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        true,
    )
    .await?;
    app.docs().save(&path, &name, input).await
}

/// `GET /docs/{*ref}`
#[command]
pub(crate) async fn docs_get(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    exclude_mode: Option<ExcludeMode>,
) -> Result<Document> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Document,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        false,
    )
    .await?;
    app.docs().get(&path, &name).await
}

/// `DELETE /docs/{*ref}`
#[command]
pub(crate) async fn docs_delete(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    exclude_mode: Option<ExcludeMode>,
) -> Result<()> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Document,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        false,
    )
    .await?;
    app.docs().delete(&path, &name).await
}

/// `GET /docs`
#[command]
pub(crate) async fn docs_list(
    state: State<'_, SolxState>,
    opts: Option<ListOptions>,
) -> Result<Page<Document>> {
    let app = state.app()?;
    app.docs()
        .list(scoped(&app, EntityKind::Document, opts))
        .await
}

/// `GET /docs-paths`
#[command]
pub(crate) async fn docs_paths(
    state: State<'_, SolxState>,
    opts: Option<ListOptions>,
) -> Result<Page<PathFacet>> {
    let app = state.app()?;
    app.docs()
        .paths(scoped(&app, EntityKind::Document, opts))
        .await
}

/// `GET /search`
#[command]
pub(crate) async fn docs_search(
    state: State<'_, SolxState>,
    query: Option<SearchQuery>,
) -> Result<Page<Document>> {
    let app = state.app()?;
    let mut query = query.unwrap_or_default();
    app.config
        .exclusion_policy()
        .apply_search(&mut query, DEFAULT_MODE);
    app.docs().search(query).await
}

// ── actions ── (`/actions`, `/actions/{*ref}`, `/actions-paths`, `/actions-search`)

/// `PUT /actions/{*ref}`
#[command]
pub(crate) async fn actions_save(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    input: ActionInput,
    exclude_mode: Option<ExcludeMode>,
) -> Result<Action> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Action,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        true,
    )
    .await?;
    app.actions().save(&path, &name, input).await
}

/// `GET /actions/{*ref}`
#[command]
pub(crate) async fn actions_get(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    exclude_mode: Option<ExcludeMode>,
) -> Result<Action> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Action,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        false,
    )
    .await?;
    app.actions().get(&path, &name).await
}

/// `DELETE /actions/{*ref}`
#[command]
pub(crate) async fn actions_delete(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    exclude_mode: Option<ExcludeMode>,
) -> Result<()> {
    let (path, name) = entity(&path, &name)?;
    let app = state.app()?;
    app.guard_excluded(
        EntityKind::Action,
        exclude_mode.unwrap_or(DEFAULT_MODE),
        &path,
        &name,
        false,
    )
    .await?;
    app.actions().delete(&path, &name).await
}

/// `GET /actions`
#[command]
pub(crate) async fn actions_list(
    state: State<'_, SolxState>,
    opts: Option<ListOptions>,
) -> Result<Page<Action>> {
    let app = state.app()?;
    app.actions()
        .list(scoped(&app, EntityKind::Action, opts))
        .await
}

/// `GET /actions-paths`
#[command]
pub(crate) async fn actions_paths(
    state: State<'_, SolxState>,
    opts: Option<ListOptions>,
) -> Result<Page<PathFacet>> {
    let app = state.app()?;
    app.actions()
        .paths(scoped(&app, EntityKind::Action, opts))
        .await
}

/// `GET /actions-search`
#[command]
pub(crate) async fn actions_search(
    state: State<'_, SolxState>,
    query: Option<ActionSearchQuery>,
) -> Result<Page<Action>> {
    let app = state.app()?;
    let mut query = query.unwrap_or_default();
    query.list.exclude_mode = query.effective_mode();
    query.exclude_hidden = false;
    app.config
        .exclusion_policy()
        .apply(EntityKind::Action, &mut query.list, DEFAULT_MODE);
    app.actions().search(query).await
}

/// `POST /actions/{*ref}` — `params` defaults to `{}`, like an empty POST body.
#[command]
pub(crate) async fn actions_exec(
    state: State<'_, SolxState>,
    path: String,
    name: String,
    params: Option<Value>,
) -> Result<ActionExecResult> {
    let (path, name) = entity(&path, &name)?;
    let params = params.unwrap_or_else(|| json!({}));
    state.app()?.actions().exec(&path, &name, params).await
}

// ── files ── (`/files`, `/files/{*rel_path}`) — content is raw bytes both ways

/// `PUT /files/{*rel_path}`. The body is the raw bytes (a `Uint8Array`/
/// `ArrayBuffer` passed as `invoke`'s args); the path rides in the
/// `x-solx-path` header. Returns the stored (normalized) relative path.
#[command]
pub(crate) async fn files_put(state: State<'_, SolxState>, request: Request<'_>) -> Result<String> {
    let rel_path = request
        .headers()
        .get(FILE_PATH_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            SolxError::Invalid(format!("files_put: missing {FILE_PATH_HEADER} header"))
        })?;
    let rel_path = percent_encoding::percent_decode_str(rel_path)
        .decode_utf8()
        .map_err(|e| {
            SolxError::Invalid(format!("files_put: {FILE_PATH_HEADER} is not UTF-8: {e}"))
        })?
        .into_owned();
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(_) => {
            return Err(SolxError::Invalid(
                "files_put: expected a raw byte body".into(),
            ));
        }
    };
    state.app()?.files().put(&rel_path, bytes).await
}

/// `GET /files/{*rel_path}` — answered as a raw `ArrayBuffer`, not JSON.
#[command]
pub(crate) async fn files_get(state: State<'_, SolxState>, rel_path: String) -> Result<Response> {
    let bytes = state.app()?.files().get(&rel_path).await?;
    Ok(Response::new(bytes))
}

/// `DELETE /files/{*rel_path}`
#[command]
pub(crate) async fn files_delete(state: State<'_, SolxState>, rel_path: String) -> Result<()> {
    state.app()?.files().delete(&rel_path).await
}

/// `GET /files?prefix=` — the bare path list (HTTP wraps it as `{paths}`).
#[command]
pub(crate) async fn files_list(
    state: State<'_, SolxState>,
    prefix: Option<String>,
) -> Result<Vec<String>> {
    state
        .app()?
        .files()
        .list(prefix.as_deref().unwrap_or(""))
        .await
}

// ── host ── (no HTTP equivalent, beyond `/health`)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostInfo {
    /// `local` (managers in-process), `remote` (proxying to `serverUrl`) or
    /// `disconnected` (a client with no server configured yet).
    mode: &'static str,
    /// `true` for a client build, which can `connect`/`disconnect`.
    client: bool,
    version: &'static str,
    server_url: Option<String>,
    embedded_server: Option<EmbeddedServer>,
}

/// Stands in for `GET /health`: which mode the host is in, and where.
#[command]
pub(crate) fn info(state: State<'_, SolxState>) -> HostInfo {
    let app = state.current();
    let mode = match &app {
        None => "disconnected",
        Some(app) if app.remote_url().is_some() => "remote",
        Some(_) => "local",
    };
    HostInfo {
        mode,
        client: matches!(state.backend, Backend::Client { .. }),
        version: env!("CARGO_PKG_VERSION"),
        server_url: app
            .as_ref()
            .and_then(|a| a.remote_url().map(str::to_string)),
        embedded_server: state.embedded(),
    }
}

/// Client builds only: point at a `solx-server`. `credential` is the one
/// "connect code or token" field: a connect code (or a whole connect link,
/// which also supplies the server URL) is redeemed at the server's
/// `/auth/exchange` first; a token is used as given. The connection is then
/// checked (one cheap authenticated call) before it's persisted, so a wrong
/// URL or token is reported here rather than on the next unrelated request.
#[command]
pub(crate) async fn connect(
    state: State<'_, SolxState>,
    server_url: String,
    credential: String,
) -> Result<()> {
    let Backend::Client { config } = &state.backend else {
        return Err(SolxError::Invalid(
            "this build runs solx locally; there is nothing to connect to".into(),
        ));
    };
    let parsed = solx_config::credentials::parse_connect_input(Some(&server_url), &credential)?;
    let Some(mut server_url) = parsed.server_url else {
        return Err(SolxError::Invalid(
            "enter the server's address, or paste a connect link".into(),
        ));
    };
    let token = match parsed.credential {
        ConnectCredential::Code(code) => {
            let exchanged = solx_client::exchange_connect_code(&server_url, &code).await?;
            if let Some(url) = exchanged.server_url.filter(|u| !u.trim().is_empty()) {
                server_url = url.trim().trim_end_matches('/').to_string();
            }
            exchanged.token
        }
        ConnectCredential::Token(token) => token,
    };
    let app = solx_manager::App::remote(config.clone(), server_url.clone(), token.clone())
        .map_err(|e| SolxError::Config(format!("{e:#}")))?;
    app.docs()
        .list(ListOptions {
            limit: Some(1),
            ..Default::default()
        })
        .await?;

    // The token goes to the OS credential manager, not the config file.
    solx_config::credentials::set_client_token(config, &server_url, &token)?;
    config.patch(json!({ "server_url": server_url }))?;
    state.replace(Some(app));
    Ok(())
}

/// Client builds only: forget the server URL/token.
#[command]
pub(crate) fn disconnect(state: State<'_, SolxState>) -> Result<()> {
    let Backend::Client { config } = &state.backend else {
        return Err(SolxError::Invalid(
            "this build runs solx locally; there is nothing to disconnect".into(),
        ));
    };
    if let Some(url) = config.snapshot().server_url {
        solx_config::credentials::forget_client_token(config, &url)?;
    }
    config.mutate(|obj| {
        obj.remove("server_url");
        obj.remove("server_token");
        Ok(())
    })?;
    state.replace(None);
    Ok(())
}

// ── auth ── (`solx:auth` permission only)
//
// The desktop host's trusted way into the `/builtin/auth/*` builtins, which
// refuse a call with no principal — and ordinary Tauri IPC has none, so a
// package widget can't mint access for itself. This command runs them as
// Root instead, and is granted to the Settings window alone.

/// The builtins `auth_exec` may run, by name under `/builtin/auth`.
const AUTH_ACTIONS: &[&str] = &[
    "create-connect-code",
    "list-clients",
    "revoke-client",
    "issue-client",
];

fn check_auth_action(name: &str) -> Result<()> {
    if AUTH_ACTIONS.contains(&name) {
        Ok(())
    } else {
        Err(SolxError::Invalid(format!(
            "auth_exec: '{name}' is not one of {}",
            AUTH_ACTIONS.join(", ")
        )))
    }
}

/// Run one `/builtin/auth/*` builtin as `Principal::Root` and return its
/// result. Host builds only: a client's tokens are managed on its server.
#[command]
pub(crate) async fn auth_exec(
    state: State<'_, SolxState>,
    name: String,
    params: Option<Value>,
) -> Result<Value> {
    if matches!(state.backend, Backend::Client { .. }) {
        return Err(SolxError::Invalid(
            "connect codes and client tokens are managed on the solx host, not from a client"
                .into(),
        ));
    }
    check_auth_action(&name)?;
    let out = state
        .app()?
        .actions()
        .exec_for(
            &Principal::Root,
            "/builtin/auth",
            &name,
            params.unwrap_or_else(|| json!({})),
        )
        .await?;
    Ok(out.result)
}

// ── config ── (no HTTP equivalent, by design: `solx:config` permission only)
//
// Granted to the desktop app's Settings window alone, never the main
// window: package widgets run as JS in the main webview, and config holds
// the `command_actions`/`allowed_base_urls` allowlists that solx keeps out
// of reach of actions, MCP and scripts.

/// The shared appdata config of a host that owns it. A client build's only
/// config is its own connection, which `connect`/`disconnect` manage.
fn host_config(state: &SolxState) -> Result<Arc<ConfigService>> {
    if matches!(state.backend, Backend::Client { .. }) {
        return Err(SolxError::Invalid(
            "solx config is edited on the solx-server host, not from a client".into(),
        ));
    }
    Ok(state.app()?.config.clone())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigView {
    /// `solx-config.json` as stored (snake_case keys, unknown keys included).
    raw: Value,
    /// What the accessors resolve unset or invalid keys to, by key.
    effective: Value,
    path: String,
    appdata: String,
    /// Keys only read at startup; see `solx_config::RESTART_KEYS`.
    restart_keys: &'static [&'static str],
}

fn config_view(config: &ConfigService) -> ConfigView {
    let snap = config.snapshot();
    let file_name = |p: std::path::PathBuf| p.file_name().map(|f| f.to_string_lossy().into_owned());
    // Never hand the bearer token to a webview, even when it is still in
    // the file (`token_store: "config"`, or no credential manager).
    let mut raw = config.raw_snapshot();
    if let Some(obj) = raw.as_object_mut() {
        obj.remove("server_token");
    }
    ConfigView {
        raw,
        effective: json!({
            "data_directory": config.db_dir(),
            "files_directory": config.files_dir(),
            "packages_directory": config.packages_dir(),
            "docs_db": file_name(config.docs_db_path()),
            "actions_db": file_name(config.actions_db_path()),
            "types_db": file_name(config.types_db_path()),
            "server_host": snap.server_host.filter(|h| !h.trim().is_empty()).unwrap_or_else(|| "127.0.0.1".into()),
            "server_port": snap.server_port.unwrap_or(solx_config::DEFAULT_SERVER_PORT),
            "server_embedded": config.server_embedded(),
            "client_token_ttl_days": config.client_token_ttl_days(),
            "console_max_entries": config.console_max_entries(),
            "console_ttl_days": config.console_ttl_days(),
            "background_timeout_secs": config.background_timeout_secs(),
            "stop_grace_secs": config.stop_grace_secs(),
            "invocation_ttl_days": config.invocation_ttl_days(),
            "http_stream_idle_ttl_secs": config.http_stream_idle_ttl_secs(),
            "http_stream_max_buffer_bytes": config.http_stream_max_buffer_bytes(),
        }),
        path: config.config_path().display().to_string(),
        appdata: config.appdata().display().to_string(),
        restart_keys: solx_config::RESTART_KEYS,
    }
}

#[command]
pub(crate) fn config_get(state: State<'_, SolxState>) -> Result<ConfigView> {
    let config = host_config(&state)?;
    Ok(config_view(&config))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigUpdateResult {
    config: ConfigView,
    changed_keys: Vec<String>,
    /// The changed keys that only take effect after a restart.
    restart_required: Vec<String>,
}

/// Apply top-level key changes (`null` removes a key) as one validated
/// write — see `ConfigService::update`. A rejected change leaves the file
/// untouched and fails with a `validation` error naming the problem.
#[command]
pub(crate) fn config_update(
    state: State<'_, SolxState>,
    changes: serde_json::Map<String, Value>,
) -> Result<ConfigUpdateResult> {
    let config = host_config(&state)?;
    let changed_keys = config.update(changes)?;
    let restart_required = changed_keys
        .iter()
        .filter(|k| solx_config::RESTART_KEYS.contains(&k.as_str()))
        .cloned()
        .collect();
    Ok(ConfigUpdateResult {
        config: config_view(&config),
        changed_keys,
        restart_required,
    })
}

/// What installing or uninstalling a package leaves behind: the new config
/// (so the Settings window can redraw) and, for an install, the install's
/// allowlist-collision warnings.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageResult {
    config: ConfigView,
    warnings: Vec<String>,
}

/// Install a package from a zip. The body is the raw zip bytes (a
/// `Uint8Array` passed as `invoke`'s args). Runs the package's
/// `install.solx` and grants its declared allowlist entries, which is why
/// this is `solx:config`, not `solx:default`.
#[command]
pub(crate) async fn config_install_package(
    state: State<'_, SolxState>,
    request: Request<'_>,
) -> Result<PackageResult> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(SolxError::Invalid(
            "config_install_package: expected the zip as a raw byte body".into(),
        ));
    };
    install_package_zip(&state, bytes.clone()).await
}

#[cfg(feature = "local")]
async fn install_package_zip(state: &SolxState, bytes: Vec<u8>) -> Result<PackageResult> {
    let app = state.app()?;
    host_config(state)?;
    let runner = solx_cli::AppRunner { app: app.clone() };
    let outcome = solx_packages::install_package_zip_bytes(&runner, &app.config, bytes).await?;
    Ok(PackageResult {
        config: config_view(&app.config),
        warnings: outcome.warnings,
    })
}

/// Uninstall a package by name: runs its `uninstall.solx`, revokes its
/// grants, and deletes its files if solx unpacked them.
#[command]
pub(crate) async fn config_uninstall_package(
    state: State<'_, SolxState>,
    name: String,
) -> Result<PackageResult> {
    uninstall_package_by_name(&state, &name).await
}

#[cfg(feature = "local")]
async fn uninstall_package_by_name(state: &SolxState, name: &str) -> Result<PackageResult> {
    let app = state.app()?;
    host_config(state)?;
    let runner = solx_cli::AppRunner { app: app.clone() };
    solx_packages::uninstall_package(&runner, &app.config, name).await?;
    Ok(PackageResult {
        config: config_view(&app.config),
        warnings: Vec::new(),
    })
}

#[cfg(not(feature = "local"))]
async fn install_package_zip(_: &SolxState, _: Vec<u8>) -> Result<PackageResult> {
    Err(no_local_packages())
}

#[cfg(not(feature = "local"))]
async fn uninstall_package_by_name(_: &SolxState, _: &str) -> Result<PackageResult> {
    Err(no_local_packages())
}

#[cfg(not(feature = "local"))]
fn no_local_packages() -> SolxError {
    SolxError::Invalid("this build can't manage packages; do it on the solx-server host".into())
}

/// The official package catalog (`solx_packages::catalog`) with what's
/// installed and what the release offers this platform. `refresh` skips the
/// few-minute cache of the release listing.
///
/// The `packages_*` commands are `solx:packages`, which the main window
/// holds: they only take catalog names, and the host resolves each to a zip
/// in the configured release, so a widget can't install arbitrary code.
#[command]
pub(crate) async fn packages_catalog(
    state: State<'_, SolxState>,
    refresh: Option<bool>,
) -> Result<Value> {
    catalog::view(&state, refresh.unwrap_or(false)).await
}

/// Install catalog package `name` (and any packages it requires) from the
/// release. Returns `{ catalog, warnings }`.
#[command]
pub(crate) async fn packages_install(state: State<'_, SolxState>, name: String) -> Result<Value> {
    catalog::install(&state, &name).await
}

/// Uninstall catalog package `name`. Returns `{ catalog, warnings }`; the
/// warnings name installed packages that require it.
#[command]
pub(crate) async fn packages_uninstall(state: State<'_, SolxState>, name: String) -> Result<Value> {
    catalog::uninstall(&state, &name).await
}

#[cfg(feature = "local")]
mod catalog {
    use super::*;
    use solx_packages::catalog::{
        catalog_entry, catalog_view, dependents, install_from_catalog, resolve_cached,
        PackageSource,
    };

    fn result(view: impl Serialize, warnings: Vec<String>) -> Result<Value> {
        Ok(json!({ "catalog": view, "warnings": warnings }))
    }

    pub(super) async fn view(state: &SolxState, refresh: bool) -> Result<Value> {
        let config = host_config(state)?;
        let source = PackageSource::from_env();
        let release = resolve_cached(&source, refresh).await?;
        Ok(serde_json::to_value(catalog_view(
            &config, &source, &release,
        ))?)
    }

    pub(super) async fn install(state: &SolxState, name: &str) -> Result<Value> {
        let app = state.app()?;
        host_config(state)?;
        let source = PackageSource::from_env();
        let release = resolve_cached(&source, false).await?;
        let runner = solx_cli::AppRunner { app: app.clone() };
        let outcome = install_from_catalog(&runner, &app.config, &release, name).await?;
        result(
            catalog_view(&app.config, &source, &release),
            outcome.warnings,
        )
    }

    pub(super) async fn uninstall(state: &SolxState, name: &str) -> Result<Value> {
        let app = state.app()?;
        host_config(state)?;
        if catalog_entry(name).is_none() {
            return Err(SolxError::Invalid(format!(
                "'{name}' is not in the package catalog; uninstall it from solx settings"
            )));
        }
        let runner = solx_cli::AppRunner { app: app.clone() };
        solx_packages::uninstall_package(&runner, &app.config, name).await?;
        let warnings = dependents(&app.config, name)
            .into_iter()
            .map(|d| format!("{d} requires {name} and may stop working"))
            .collect();
        let source = PackageSource::from_env();
        let release = resolve_cached(&source, false).await?;
        result(catalog_view(&app.config, &source, &release), warnings)
    }
}

#[cfg(not(feature = "local"))]
mod catalog {
    use super::*;

    pub(super) async fn view(_: &SolxState, _: bool) -> Result<Value> {
        Err(no_local_packages())
    }
    pub(super) async fn install(_: &SolxState, _: &str) -> Result<Value> {
        Err(no_local_packages())
    }
    pub(super) async fn uninstall(_: &SolxState, _: &str) -> Result<Value> {
        Err(no_local_packages())
    }
}

/// Build the client-mode `App` from a previously saved URL/token, if any.
/// A token an older build left in this private config's `server_token` is
/// moved to the OS credential manager first.
pub(crate) fn restore_client(config: &Arc<ConfigService>) -> Option<Arc<solx_manager::App>> {
    let snap = config.snapshot();
    let url = snap.server_url.filter(|s| !s.trim().is_empty())?;
    if let Err(e) = solx_config::credentials::adopt_legacy_client_token(config, &url) {
        eprintln!("solx: could not move the saved server token to the credential manager: {e}");
    }
    let token = solx_config::credentials::stored_client_token(config, &url)?;
    solx_manager::App::remote(config.clone(), url, token).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_normalizes_like_a_url_ref() {
        assert_eq!(entity("/", "note").unwrap(), ("/".into(), "note".into()));
        assert_eq!(
            entity("/research/ai/", "note").unwrap(),
            ("/research/ai".into(), "note".into())
        );
        assert_eq!(
            entity("research/ai", "note").unwrap(),
            ("/research/ai".into(), "note".into())
        );
        assert!(entity("/a", "").is_err());
        assert!(entity("/a", "b/c").is_err());
        assert!(entity("/a/../b", "c").is_err());
    }

    #[test]
    fn auth_exec_runs_only_the_auth_builtins() {
        for name in AUTH_ACTIONS {
            assert!(check_auth_action(name).is_ok(), "{name}");
        }
        for name in [
            "",
            "entity-save-action",
            "../console/print",
            "list-clients ",
        ] {
            assert!(check_auth_action(name).is_err(), "{name:?}");
        }
    }
}
