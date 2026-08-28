pub mod engine;

mod commands;
mod controller;
mod prefs;
mod state;
mod tray;
mod updater;

use std::sync::Mutex;

use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use crate::engine::config::{default_config, load_config, save_config};
use crate::prefs::load_prefs;
use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        // single-instance must be registered first so a second launch surfaces
        // the existing window instead of starting a rival engine.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::show_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build());

    if cfg!(debug_assertions) {
        builder = builder.plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        );
    }

    builder
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&config_dir)?;
            let config_path = config_dir.join("config.toml");
            let prefs_path = config_dir.join("prefs.json");

            // First run: write a starter config the user can edit in the window.
            if !config_path.exists() {
                let _ = save_config(&config_path, &default_config());
            }
            let config = load_config(&config_path).unwrap_or_else(|e| {
                log::warn!("config load failed, using defaults: {e}");
                default_config()
            });
            let enabled = load_prefs(&prefs_path).enabled;

            let control = controller::spawn(app.handle().clone(), config.clone(), enabled);
            app.manage(AppState {
                config_path,
                prefs_path,
                config: Mutex::new(config),
                control,
                tray: Mutex::new(None),
            });

            let autostart_on = {
                use tauri_plugin_autostart::ManagerExt;
                app.autolaunch().is_enabled().unwrap_or(false)
            };
            let items = tray::build(app.handle(), enabled, autostart_on)?;
            *app.state::<AppState>().tray.lock().unwrap() = Some(items);

            let launched_minimized = std::env::args().any(|a| a == "--minimized");
            if !launched_minimized {
                tray::show_window(app.handle());
            }

            // Check for updates on startup, then every 6 hours while running.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                tauri::async_runtime::block_on(updater::check_and_notify(&handle));
                std::thread::sleep(std::time::Duration::from_secs(6 * 60 * 60));
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides it to the tray; the app keeps ducking.
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::get_status,
            commands::get_prefs,
            commands::set_enabled,
            commands::set_autostart,
            commands::check_update,
            commands::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Duck")
        .run(|_app_handle, event| {
            // Last window closing must not exit the app - it lives in the tray.
            if let tauri::RunEvent::ExitRequested { code, api, .. } = &event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
