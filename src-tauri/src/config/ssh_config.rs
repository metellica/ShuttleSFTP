use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

/// A host entry parsed from ~/.ssh/config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostEntry {
    pub name: String,
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_file: Option<String>,
}

/// Load and parse the user's SSH config file.
pub fn load_ssh_config() -> AppResult<Vec<SshHostEntry>> {
    let config_path = get_ssh_config_path();

    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| AppError::ConfigError(format!("Cannot read SSH config: {}", e)))?;

    Ok(parse_ssh_config(&content))
}

fn imported_hosts_path() -> PathBuf {
    super::app_config_dir().join("imported_ssh_hosts.json")
}

/// Host aliases the user chose to import from ~/.ssh/config. Only these
/// are offered in the connection UI; nothing is imported by default.
pub fn load_imported_hosts() -> AppResult<Vec<String>> {
    let path = imported_hosts_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| AppError::ConfigError(format!("Cannot read imported hosts: {}", e)))?;
    serde_json::from_str(&content)
        .map_err(|e| AppError::ConfigError(format!("Invalid imported hosts JSON: {}", e)))
}

pub fn save_imported_hosts(names: &[String]) -> AppResult<()> {
    let content = serde_json::to_string_pretty(names)
        .map_err(|e| AppError::ConfigError(format!("Serialize error: {}", e)))?;
    std::fs::write(imported_hosts_path(), content)
        .map_err(|e| AppError::ConfigError(format!("Cannot write imported hosts: {}", e)))
}

fn get_ssh_config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".ssh")
        .join("config")
}

/// Simple SSH config parser.
fn parse_ssh_config(content: &str) -> Vec<SshHostEntry> {
    let mut hosts = Vec::new();
    let mut current: Option<SshHostEntry> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = match line.split_once(char::is_whitespace) {
            Some((k, v)) => (k.to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        match key.as_str() {
            "host" => {
                if let Some(entry) = current.take() {
                    // Skip wildcard patterns
                    if !entry.name.contains('*') && !entry.name.contains('?') {
                        hosts.push(entry);
                    }
                }
                current = Some(SshHostEntry {
                    name: value,
                    hostname: None,
                    port: None,
                    user: None,
                    identity_file: None,
                });
            }
            "hostname" => {
                if let Some(ref mut entry) = current {
                    entry.hostname = Some(value);
                }
            }
            "port" => {
                if let Some(ref mut entry) = current {
                    entry.port = value.parse().ok();
                }
            }
            "user" => {
                if let Some(ref mut entry) = current {
                    entry.user = Some(value);
                }
            }
            "identityfile" => {
                if let Some(ref mut entry) = current {
                    // Expand ~ to home directory
                    let expanded = if value.starts_with("~/") {
                        dirs::home_dir()
                            .unwrap_or_default()
                            .join(&value[2..])
                            .to_string_lossy()
                            .to_string()
                    } else {
                        value
                    };
                    entry.identity_file = Some(expanded);
                }
            }
            _ => {}
        }
    }

    // Don't forget the last entry
    if let Some(entry) = current {
        if !entry.name.contains('*') && !entry.name.contains('?') {
            hosts.push(entry);
        }
    }

    hosts
}
