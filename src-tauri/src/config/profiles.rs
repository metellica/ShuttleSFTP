use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

/// A saved connection profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String, // "password" | "key" | "agent"
    pub private_key_path: Option<String>,
    pub default_remote_path: Option<String>,
    /// Stored only when the user opts in to saving the password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Stored only when the user opts in (key passphrase).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

/// Get the app config directory for storing profiles.
fn profiles_path() -> PathBuf {
    super::app_config_dir().join("profiles.json")
}

/// Load saved profiles.
pub fn load_profiles() -> AppResult<Vec<ConnectionProfile>> {
    let path = profiles_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| AppError::ConfigError(format!("Cannot read profiles: {}", e)))?;
    let profiles: Vec<ConnectionProfile> = serde_json::from_str(&content)
        .map_err(|e| AppError::ConfigError(format!("Invalid profiles JSON: {}", e)))?;
    Ok(profiles)
}

/// Save profiles to disk.
pub fn save_profiles(profiles: &[ConnectionProfile]) -> AppResult<()> {
    let path = profiles_path();
    let content = serde_json::to_string_pretty(profiles)
        .map_err(|e| AppError::ConfigError(format!("Serialize error: {}", e)))?;
    std::fs::write(&path, content)
        .map_err(|e| AppError::ConfigError(format!("Cannot write profiles: {}", e)))?;
    Ok(())
}
