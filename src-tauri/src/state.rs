//! Shared application state managed by Tauri.

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Wry;

use crate::controller::EngineControl;
use crate::engine::Config;
use crate::tray::TrayItems;

pub struct AppState {
    pub config_path: PathBuf,
    pub prefs_path: PathBuf,
    pub config: Mutex<Config>,
    pub control: EngineControl,
    pub tray: Mutex<Option<TrayItems<Wry>>>,
}
