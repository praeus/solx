//! System tray: the only way to quit solx. Closing the main window just hides
//! it (see `lib.rs`), so the embedded server keeps running.

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};

const MAIN: &str = "main";

pub(crate) fn init(app: &tauri::App) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show/Hide", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart", "Restart", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &restart, &exit])?;

    let mut tray = TrayIconBuilder::<Wry>::with_id("main")
        .tooltip("solx")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle" => toggle_main(app),
            "restart" => crate::restart(app),
            "exit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Bring the main window to the front, restoring it from the tray if hidden.
pub(crate) fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN) else { return };
    let visible = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if visible {
        let _ = window.hide();
    } else {
        show_main(app);
    }
}
