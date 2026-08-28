//! Tauri commands invoked from the settings window.

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::controller::EngineStatus;
use crate::engine::config::{save_config as save_config_file, validate_structure, Config};
use crate::prefs::{save_prefs, Prefs};
use crate::state::AppState;
use crate::updater::{self, UpdateInfo};

#[tauri::command]
pub fn get_config(state: State<AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_config(state: State<AppState>, config: Config) -> Result<(), String> {
    validate_structure(&config).map_err(|e| e.to_string())?;
    save_config_file(&state.config_path, &config).map_err(|e| e.to_string())?;
    *state.config.lock().unwrap() = config.clone();
    state.control.reload(config);
    Ok(())
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> EngineStatus {
    state.control.status()
}

#[derive(Serialize)]
pub struct PrefsView {
    pub enabled: bool,
    pub autostart: bool,
}

#[tauri::command]
pub fn get_prefs(app: AppHandle, state: State<AppState>) -> PrefsView {
    PrefsView {
        enabled: state.control.enabled(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
    }
}

#[tauri::command]
pub fn set_enabled(state: State<AppState>, on: bool) {
    state.control.set_enabled(on);
    save_prefs(&state.prefs_path, &Prefs { enabled: on });
    if let Some(items) = state.tray.lock().unwrap().as_ref() {
        let _ = items.enabled.set_checked(on);
    }
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, state: State<AppState>, on: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    (if on { manager.enable() } else { manager.disable() }).map_err(|e| e.to_string())?;
    if let Some(items) = state.tray.lock().unwrap().as_ref() {
        let _ = items.autostart.set_checked(on);
    }
    Ok(())
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    updater::check(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    updater::install(&app).await
}
