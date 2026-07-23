pub mod ssh_config;
pub mod profiles;
pub mod bookmarks;

use std::path::PathBuf;

/// Unified config dir across all platforms: ~/.config/shuttle-sftp/
pub fn app_config_dir() -> PathBuf {
    let dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("shuttle-sftp");
    std::fs::create_dir_all(&dir).ok();
    dir
}
