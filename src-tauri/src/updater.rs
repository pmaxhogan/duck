//! GitHub-releases auto-update via tauri-plugin-updater. Checks are triggered on
//! startup and from the tray; installs are user-initiated from the window.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

pub const UPDATE_EVENT: &str = "updater:available";

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
}

pub async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| e.to_string())?;
    Ok(found.map(|u| UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone(),
    }))
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    // Never returns.
    app.restart()
}

/// Check on startup: if an update exists, post a tray notification and let the
/// window know so it can offer an Install button.
pub async fn check_and_notify(app: &AppHandle) {
    if let Ok(Some(info)) = check(app).await {
        let _ = app
            .notification()
            .builder()
            .title("Duck update available")
            .body(format!(
                "Version {} is available. Open Duck to install.",
                info.version
            ))
            .show();
        let _ = app.emit(UPDATE_EVENT, &info);
    }
}
