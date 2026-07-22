// Hide the console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use shuttle_sftp::ssh::SessionManager;
use shuttle_sftp::transfer::TransferEngine;

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(SessionManager::new())
        .manage(TransferEngine::new(3))
        .invoke_handler(tauri::generate_handler![
            shuttle_sftp::commands::connection::connect,
            shuttle_sftp::commands::connection::disconnect,
            shuttle_sftp::commands::filesystem::list_dir,
            shuttle_sftp::commands::filesystem::mkdir,
            shuttle_sftp::commands::filesystem::remove,
            shuttle_sftp::commands::filesystem::rename,
            shuttle_sftp::commands::transfer::upload,
            shuttle_sftp::commands::transfer::download,
            shuttle_sftp::commands::transfer::download_as,
            shuttle_sftp::commands::transfer::cancel_transfer,
            shuttle_sftp::commands::transfer::list_transfers,
            shuttle_sftp::commands::config::load_ssh_config,
            shuttle_sftp::commands::config::list_profiles,
            shuttle_sftp::commands::config::save_profile,
            shuttle_sftp::commands::config::delete_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ShuttleSFTP");
}
