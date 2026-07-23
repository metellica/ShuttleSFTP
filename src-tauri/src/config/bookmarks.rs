use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

/// A bookmarked remote path with the connection info needed to reopen it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String, // "password" | "key" | "agent"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,
    /// Stored only when the user opts in to saving the password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Stored only when the user opts in (key passphrase).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
    /// Remote directory this bookmark opens.
    pub path: String,
}

fn bookmarks_path() -> PathBuf {
    super::app_config_dir().join("bookmarks.json")
}

/// Load saved bookmarks.
pub fn load_bookmarks() -> AppResult<Vec<Bookmark>> {
    let path = bookmarks_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| AppError::ConfigError(format!("Cannot read bookmarks: {}", e)))?;
    let bookmarks: Vec<Bookmark> = serde_json::from_str(&content)
        .map_err(|e| AppError::ConfigError(format!("Invalid bookmarks JSON: {}", e)))?;
    Ok(bookmarks)
}

/// Save bookmarks to disk.
pub fn save_bookmarks(bookmarks: &[Bookmark]) -> AppResult<()> {
    let path = bookmarks_path();
    let content = serde_json::to_string_pretty(bookmarks)
        .map_err(|e| AppError::ConfigError(format!("Serialize error: {}", e)))?;
    std::fs::write(&path, content)
        .map_err(|e| AppError::ConfigError(format!("Cannot write bookmarks: {}", e)))?;
    Ok(())
}
