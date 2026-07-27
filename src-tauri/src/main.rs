// Hide the console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use shuttle_sftp::commands::prepare::PrepareRegistry;
use shuttle_sftp::ssh::SessionManager;
use shuttle_sftp::terminal::TerminalManager;
use shuttle_sftp::transfer::TransferEngine;

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(SessionManager::new())
        .manage(TransferEngine::new(3))
        .manage(PrepareRegistry::default())
        .manage(TerminalManager::default())
        .invoke_handler(tauri::generate_handler![
            shuttle_sftp::commands::connection::connect,
            shuttle_sftp::commands::connection::connect_local,
            shuttle_sftp::commands::connection::disconnect,
            shuttle_sftp::commands::filesystem::list_dir,
            shuttle_sftp::commands::filesystem::mkdir,
            shuttle_sftp::commands::filesystem::remove,
            shuttle_sftp::commands::filesystem::rename,
            shuttle_sftp::commands::filesystem::preview_file,
            shuttle_sftp::commands::filesystem::save_file,
            shuttle_sftp::commands::transfer::upload,
            shuttle_sftp::commands::transfer::download,
            shuttle_sftp::commands::transfer::download_as,
            shuttle_sftp::commands::transfer::transfer_remote,
            shuttle_sftp::commands::transfer::cancel_transfer,
            shuttle_sftp::commands::transfer::cancel_all_transfers,
            shuttle_sftp::commands::transfer::cancel_transfer_group,
            shuttle_sftp::commands::transfer::pause_transfer,
            shuttle_sftp::commands::transfer::pause_all_transfers,
            shuttle_sftp::commands::transfer::pause_transfer_group,
            shuttle_sftp::commands::transfer::resume_transfer,
            shuttle_sftp::commands::transfer::resume_all_transfers,
            shuttle_sftp::commands::transfer::resume_transfer_group,
            shuttle_sftp::commands::transfer::clear_finished_transfers,
            shuttle_sftp::commands::transfer::show_in_folder,
            shuttle_sftp::commands::transfer::list_transfers,
            shuttle_sftp::commands::prepare::cancel_prepare,
            shuttle_sftp::commands::terminal::terminal_open,
            shuttle_sftp::commands::terminal::terminal_input,
            shuttle_sftp::commands::terminal::terminal_resize,
            shuttle_sftp::commands::terminal::terminal_close,
            shuttle_sftp::commands::config::load_ssh_config,
            shuttle_sftp::commands::config::list_imported_ssh_hosts,
            shuttle_sftp::commands::config::set_imported_ssh_hosts,
            shuttle_sftp::commands::config::list_profiles,
            shuttle_sftp::commands::config::save_profile,
            shuttle_sftp::commands::config::delete_profile,
            shuttle_sftp::commands::config::list_bookmarks,
            shuttle_sftp::commands::config::save_bookmark,
            shuttle_sftp::commands::config::delete_bookmark,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ShuttleSFTP");
}
