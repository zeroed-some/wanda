//! WANDA GUI - Tauri application

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use state::AppState;
use std::sync::Arc;
use tokio::sync::Mutex;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Arc::new(Mutex::new(AppState::new())))
        .invoke_handler(tauri::generate_handler![
            commands::get_games,
            commands::get_game,
            commands::launch_game,
            commands::get_prefixes,
            commands::get_prefix_health,
            commands::repair_prefix,
            commands::init_wanda,
            commands::get_init_status,
            commands::get_config,
            commands::update_config,
            commands::get_wemod_status,
            commands::update_wemod,
            commands::get_proton_versions,
            commands::run_doctor,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
