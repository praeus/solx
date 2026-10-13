//! The solx desktop app: solx-web in a Tauri webview, talking to solx over
//! IPC through `solx-tauri-plugin` (`@solx/tauri` on the JS side).
//!
//! Two builds from one crate, chosen by cargo feature:
//!
//! - `server` (default) — solx-core runs in-process with local managers.
//!   The UI uses them over IPC, and unless `server_embedded` is off in
//!   solx-config.json, an embedded solx-server serves the *same* `App` over
//!   HTTP for other clients (browsers, solx-cli, desktop clients, and Command
//!   actions that call back into solx-server).
//! - `client` — no local managers at all; the plugin proxies to an existing
//!   solx-server whose URL/token the user enters in Settings, persisted in
//!   this app's own config dir.

#[cfg(all(feature = "server", feature = "client"))]
compile_error!("enable exactly one of the `server` and `client` features (client: --no-default-features --features client)");
#[cfg(not(any(feature = "server", feature = "client")))]
compile_error!("enable one of the `server` (default) or `client` features");

#[cfg(feature = "server")]
mod server;

use tauri::{App, AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// Set across `restart_app` so the restarted process knows the previous one
/// may still be releasing its port (see `server::setup`).
pub(crate) const RESTART_ENV: &str = "SOLX_DESKTOP_RESTARTED";

/// Open (or focus) the Settings window, where solx-config.json is edited.
///
/// It is the only window granted `solx:config` (capabilities/settings.json):
/// package widgets run in the main window, and must not be able to edit the
/// allowlists. The URL is fixed here rather than taken from the caller, so
/// whatever asks for the window can't choose what loads in it. Async because
/// creating a window from a sync command deadlocks on Windows.
#[tauri::command]
async fn open_settings(app: AppHandle) -> Result<(), String> {
    if cfg!(feature = "client") {
        return Err("solx config is edited on the solx-server host, not from a client".into());
    }
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        return window.set_focus().map_err(|e| e.to_string());
    }
    WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("index.html?view=settings".into()))
        .title("solx settings")
        .inner_size(960.0, 720.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Restart the app, e.g. after changing a startup-only config key.
#[tauri::command]
fn restart_app(app: AppHandle) {
    std::env::set_var(RESTART_ENV, "1");
    app.restart();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![open_settings, restart_app])
        .setup(|app| {
            setup(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_handle, event| {
            if let RunEvent::Exit = event {
                #[cfg(feature = "server")]
                server::shutdown(_handle);
            }
        });
}

#[cfg(feature = "server")]
fn setup(app: &mut App) -> anyhow::Result<()> {
    server::setup(app)
}

/// Client build: the plugin's connection lives in an app-private
/// `ConfigService`, never the shared solx appdata dir — setting
/// `server_url` there would flip every other solx tool on this machine
/// into remote mode too.
#[cfg(feature = "client")]
fn setup(app: &mut App) -> anyhow::Result<()> {
    use std::sync::Arc;

    use anyhow::Context;
    use tauri::Manager;

    let dir = app.path().app_config_dir().context("resolve app config dir")?;
    let config = solx_config::ConfigService::open_in(&dir)
        .with_context(|| format!("open client config in {}", dir.display()))?;
    app.handle().plugin(solx_tauri_plugin::client(Arc::new(config)))?;
    Ok(())
}
