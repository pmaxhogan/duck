//! System-tray icon + menu. Every action lives in the menu (left-click just
//! surfaces the window); toggles mirror the app state and are kept in sync with
//! the settings window.

use tauri::menu::{CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;

use crate::prefs::{save_prefs, Prefs};
use crate::state::AppState;

pub struct TrayItems<R: Runtime> {
    pub enabled: CheckMenuItem<R>,
    pub autostart: CheckMenuItem<R>,
}

pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn build(app: &AppHandle, enabled: bool, autostart: bool) -> tauri::Result<TrayItems<tauri::Wry>> {
    let settings = MenuItemBuilder::with_id("settings", "Settings").build(app)?;
    let enabled_item = CheckMenuItemBuilder::with_id("enabled", "Enabled")
        .checked(enabled)
        .build(app)?;
    let autostart_item = CheckMenuItemBuilder::with_id("autostart", "Start on login")
        .checked(autostart)
        .build(app)?;
    let update = MenuItemBuilder::with_id("update", "Check for updates").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&settings)
        .separator()
        .item(&enabled_item)
        .item(&autostart_item)
        .separator()
        .item(&update)
        .item(&quit)
        .build()?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundled default window icon");

    TrayIconBuilder::with_id("duck")
        .icon(icon)
        .tooltip("Duck")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu_event(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(TrayItems {
        enabled: enabled_item,
        autostart: autostart_item,
    })
}

fn on_menu_event(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    match id {
        "settings" => show_window(app),
        "enabled" => {
            let on = !state.control.enabled();
            state.control.set_enabled(on);
            save_prefs(&state.prefs_path, &Prefs { enabled: on });
            if let Some(items) = state.tray.lock().unwrap().as_ref() {
                let _ = items.enabled.set_checked(on);
            }
        }
        "autostart" => {
            let manager = app.autolaunch();
            let on = !manager.is_enabled().unwrap_or(false);
            let _ = if on { manager.enable() } else { manager.disable() };
            if let Some(items) = state.tray.lock().unwrap().as_ref() {
                let _ = items.autostart.set_checked(on);
            }
        }
        "update" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                crate::updater::check_and_notify(&app).await;
            });
        }
        "quit" => {
            state.control.shutdown();
            app.exit(0);
        }
        _ => {}
    }
}
