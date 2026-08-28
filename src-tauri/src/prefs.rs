//! Tiny persisted preferences (just the on/off toggle). Autostart state lives in
//! the OS via the autostart plugin, so it is not duplicated here.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}

fn enabled_default() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { enabled: true }
    }
}

pub fn load_prefs(path: &Path) -> Prefs {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_prefs(path: &Path, prefs: &Prefs) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(prefs) {
        let _ = std::fs::write(path, text);
    }
}
