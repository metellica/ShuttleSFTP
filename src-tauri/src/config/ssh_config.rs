use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{AppError, AppResult};
use crate::ssh::session::JumpHost;

/// A host entry parsed from ~/.ssh/config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostEntry {
    pub name: String,
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_file: Option<String>,
    #[serde(default)]
    pub jump_hosts: Vec<JumpHost>,
}

#[derive(Default)]
struct HostOptions {
    hostname: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    identity_file: Option<String>,
    proxy_jump: Option<String>,
}

struct HostBlock {
    patterns: Vec<String>,
    options: HostOptions,
}

struct ResolvedHost {
    entry: SshHostEntry,
    proxy_jump: Option<String>,
}

/// Load and parse the user's SSH config file.
pub fn load_ssh_config() -> AppResult<Vec<SshHostEntry>> {
    let config_path = get_ssh_config_path();

    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| AppError::ConfigError(format!("Cannot read SSH config: {}", e)))?;

    parse_ssh_config(&content)
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

/// Parse the host fields used by the app and resolve ProxyJump aliases.
fn parse_ssh_config(content: &str) -> AppResult<Vec<SshHostEntry>> {
    let mut blocks = Vec::new();
    let mut current = HostBlock {
        patterns: vec!["*".into()],
        options: HostOptions::default(),
    };

    for raw_line in content.lines() {
        let line = strip_inline_comment(raw_line).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = match split_directive(line) {
            Some((key, value)) => (key.to_lowercase(), value.to_string()),
            None => continue,
        };

        match key.as_str() {
            "host" => {
                if should_keep_block(&current) {
                    blocks.push(current);
                }
                current = HostBlock {
                    patterns: value.split_whitespace().map(str::to_string).collect(),
                    options: HostOptions::default(),
                };
            }
            "hostname" => {
                current.options.hostname.get_or_insert(unquote(value));
            }
            "port" => {
                if current.options.port.is_none() {
                    current.options.port = value.parse().ok();
                }
            }
            "user" => {
                current.options.user.get_or_insert(unquote(value));
            }
            "identityfile" => {
                current.options.identity_file.get_or_insert(unquote(value));
            }
            "proxyjump" => {
                current.options.proxy_jump.get_or_insert(unquote(value));
            }
            _ => {}
        }
    }

    if should_keep_block(&current) {
        blocks.push(current);
    }

    let aliases = concrete_aliases(&blocks);
    let resolved: Vec<ResolvedHost> = aliases
        .iter()
        .map(|alias| resolve_host(alias, &blocks))
        .collect();
    let mut entries: Vec<SshHostEntry> = resolved.iter().map(|host| host.entry.clone()).collect();
    for (index, host) in resolved.iter().enumerate() {
        if let Some(spec) = &host.proxy_jump {
            entries[index].jump_hosts =
                resolve_proxy_jump(spec, &entries, &blocks).map_err(|message| {
                    AppError::ConfigError(format!(
                        "Invalid ProxyJump for Host {}: {}",
                        host.entry.name, message
                    ))
                })?;
        }
    }
    Ok(entries)
}

fn has_options(options: &HostOptions) -> bool {
    options.hostname.is_some()
        || options.port.is_some()
        || options.user.is_some()
        || options.identity_file.is_some()
        || options.proxy_jump.is_some()
}

fn should_keep_block(block: &HostBlock) -> bool {
    has_options(&block.options)
        || block.patterns.len() != 1
        || block.patterns.first().is_some_and(|pattern| pattern != "*")
}

fn concrete_aliases(blocks: &[HostBlock]) -> Vec<String> {
    let mut aliases = Vec::new();
    for pattern in blocks.iter().flat_map(|block| &block.patterns) {
        let pattern = pattern.strip_prefix('!').unwrap_or(pattern);
        if pattern.is_empty() || pattern.contains('*') || pattern.contains('?') {
            continue;
        }
        if !aliases
            .iter()
            .any(|alias: &String| alias.eq_ignore_ascii_case(pattern))
        {
            aliases.push(pattern.to_string());
        }
    }
    aliases
}

fn resolve_host(alias: &str, blocks: &[HostBlock]) -> ResolvedHost {
    let mut options = HostOptions::default();
    for block in blocks {
        if host_block_matches(&block.patterns, alias) {
            options.hostname = options.hostname.or_else(|| block.options.hostname.clone());
            options.port = options.port.or(block.options.port);
            options.user = options.user.or_else(|| block.options.user.clone());
            options.identity_file = options
                .identity_file
                .or_else(|| block.options.identity_file.clone());
            options.proxy_jump = options
                .proxy_jump
                .or_else(|| block.options.proxy_jump.clone());
        }
    }

    let expand_tokens = |value: String| value.replace("%h", alias);
    ResolvedHost {
        entry: SshHostEntry {
            name: alias.to_string(),
            hostname: options.hostname.map(&expand_tokens),
            port: options.port,
            user: options.user,
            identity_file: options.identity_file.map(expand_tokens).map(expand_home),
            jump_hosts: Vec::new(),
        },
        proxy_jump: options.proxy_jump,
    }
}

fn host_block_matches(patterns: &[String], alias: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        if let Some(negative) = pattern.strip_prefix('!') {
            if wildcard_matches(negative, alias) {
                return false;
            }
        } else if wildcard_matches(pattern, alias) {
            matched = true;
        }
    }
    matched
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let value: Vec<char> = value.to_lowercase().chars().collect();
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;

    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if token == '*' {
            current[0] = previous[0];
            for index in 1..=value.len() {
                current[index] = previous[index] || current[index - 1];
            }
        } else {
            for index in 1..=value.len() {
                current[index] = previous[index - 1] && (token == '?' || token == value[index - 1]);
            }
        }
        previous = current;
    }

    previous[value.len()]
}

fn strip_inline_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    let mut previous_was_whitespace = true;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            previous_was_whitespace = character.is_whitespace();
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        } else if character == '#' && quote.is_none() && previous_was_whitespace {
            return &line[..index];
        }
        previous_was_whitespace = character.is_whitespace();
    }
    line
}

fn unquote(value: String) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn split_directive(line: &str) -> Option<(&str, &str)> {
    let separator = line
        .char_indices()
        .find(|(_, character)| character.is_whitespace() || *character == '=')
        .map(|(index, _)| index)?;
    let key = line[..separator].trim();
    let value = line[separator..]
        .trim_start()
        .strip_prefix('=')
        .unwrap_or(&line[separator..])
        .trim();
    if key.is_empty() || value.is_empty() {
        None
    } else {
        Some((key, value))
    }
}

fn expand_home(value: String) -> String {
    if let Some(relative) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        dirs::home_dir()
            .unwrap_or_default()
            .join(relative)
            .to_string_lossy()
            .to_string()
    } else {
        value
    }
}

fn resolve_proxy_jump(
    spec: &str,
    hosts: &[SshHostEntry],
    blocks: &[HostBlock],
) -> Result<Vec<JumpHost>, String> {
    if spec.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }

    spec.split(',')
        .map(|destination| {
            let destination = destination.trim();
            let (explicit_user, host, explicit_port) = parse_jump_destination(destination)?;
            let entry = hosts
                .iter()
                .find(|entry| entry.name.eq_ignore_ascii_case(&host));
            let resolved = resolve_host(&host, blocks).entry;

            Ok(JumpHost {
                alias: entry.map(|entry| entry.name.clone()),
                host: resolved.hostname.unwrap_or_else(|| host.clone()),
                port: explicit_port.or(resolved.port).unwrap_or(22),
                username: explicit_user
                    .or(resolved.user)
                    .or_else(default_ssh_username),
                identity_file: resolved.identity_file,
                password: None,
                passphrase: None,
            })
        })
        .collect()
}

fn parse_jump_destination(value: &str) -> Result<(Option<String>, String, Option<u16>), String> {
    if value.is_empty() {
        return Err("empty jump host".into());
    }

    let (username, address) = match value.rsplit_once('@') {
        Some((username, address)) if !username.is_empty() && !address.is_empty() => {
            (Some(username.to_string()), address)
        }
        Some(_) => return Err(format!("invalid jump destination \"{}\"", value)),
        None => (None, value),
    };

    if let Some(address) = address.strip_prefix('[') {
        let (host, suffix) = address
            .split_once(']')
            .ok_or_else(|| format!("invalid bracketed jump destination \"{}\"", value))?;
        if host.is_empty() {
            return Err(format!("invalid jump destination \"{}\"", value));
        }
        let port = if suffix.is_empty() {
            None
        } else {
            let port = suffix
                .strip_prefix(':')
                .ok_or_else(|| format!("invalid jump destination \"{}\"", value))?;
            Some(parse_jump_port(port, value)?)
        };
        return Ok((username, host.to_string(), port));
    }

    if address.matches(':').count() == 1 {
        let (host, port) = address
            .rsplit_once(':')
            .ok_or_else(|| format!("invalid jump destination \"{}\"", value))?;
        if host.is_empty() {
            return Err(format!("invalid jump destination \"{}\"", value));
        }
        return Ok((
            username,
            host.to_string(),
            Some(parse_jump_port(port, value)?),
        ));
    }

    Ok((username, address.to_string(), None))
}

fn default_ssh_username() -> Option<String> {
    std::env::var("USER")
        .ok()
        .filter(|user| !user.is_empty())
        .or_else(|| {
            std::env::var("USERNAME")
                .ok()
                .filter(|user| !user.is_empty())
        })
}

fn parse_jump_port(port: &str, value: &str) -> Result<u16, String> {
    port.parse()
        .map_err(|_| format!("invalid port in jump destination \"{}\"", value))
}

#[cfg(test)]
mod tests {
    use super::parse_ssh_config;

    #[test]
    fn resolves_proxy_jump_alias_and_identity() {
        let hosts = parse_ssh_config(
            r#"
Host bastion
  HostName gateway.example.com
  Port 2222
  User jump-user
  IdentityFile ~/.ssh/jump_ed25519

Host internal
  HostName 10.0.0.10
  User deploy
  ProxyJump bastion
"#,
        )
        .unwrap();

        let internal = hosts.iter().find(|host| host.name == "internal").unwrap();
        assert_eq!(internal.jump_hosts.len(), 1);
        let jump = &internal.jump_hosts[0];
        assert_eq!(jump.alias.as_deref(), Some("bastion"));
        assert_eq!(jump.host, "gateway.example.com");
        assert_eq!(jump.port, 2222);
        assert_eq!(jump.username.as_deref(), Some("jump-user"));
        assert!(
            jump.identity_file
                .as_deref()
                .unwrap()
                .ends_with(".ssh\\jump_ed25519")
                || jump
                    .identity_file
                    .as_deref()
                    .unwrap()
                    .ends_with(".ssh/jump_ed25519")
        );
    }

    #[test]
    fn parses_multi_hop_overrides_and_ipv6() {
        let hosts = parse_ssh_config(
            r#"
Host first
  HostName first.example.com
  Port 2200
  User config-user

Host target
  ProxyJump explicit@first:2222,[2001:db8::1]:2201
"#,
        )
        .unwrap();

        let target = hosts.iter().find(|host| host.name == "target").unwrap();
        assert_eq!(target.jump_hosts.len(), 2);
        assert_eq!(target.jump_hosts[0].host, "first.example.com");
        assert_eq!(target.jump_hosts[0].port, 2222);
        assert_eq!(target.jump_hosts[0].username.as_deref(), Some("explicit"));
        assert_eq!(target.jump_hosts[1].host, "2001:db8::1");
        assert_eq!(target.jump_hosts[1].port, 2201);
    }

    #[test]
    fn proxy_jump_none_disables_jumps() {
        let hosts = parse_ssh_config(
            r#"
Host target
  ProxyJump none
"#,
        )
        .unwrap();

        assert!(hosts[0].jump_hosts.is_empty());
    }

    #[test]
    fn applies_wildcards_multi_aliases_and_inline_comments() {
        let hosts = parse_ssh_config(
            r#"
Host bastion gateway
  HostName gateway.example.com

Host target
  HostName internal.example.com
  ProxyJump bastion # route through the gateway

Host *
  User shared-user
  IdentityFile ~/.ssh/shared_ed25519
"#,
        )
        .unwrap();

        let target = hosts.iter().find(|host| host.name == "target").unwrap();
        assert_eq!(target.user.as_deref(), Some("shared-user"));
        assert_eq!(target.jump_hosts.len(), 1);
        assert_eq!(target.jump_hosts[0].alias.as_deref(), Some("bastion"));
        assert_eq!(target.jump_hosts[0].host, "gateway.example.com");
        assert_eq!(
            target.jump_hosts[0].username.as_deref(),
            Some("shared-user")
        );
        assert!(target.jump_hosts[0].identity_file.is_some());
        assert!(hosts.iter().any(|host| host.name == "gateway"));
    }

    #[test]
    fn applies_wildcards_to_literal_jump_destinations() {
        let hosts = parse_ssh_config(
            r#"
Host target
  ProxyJump gateway.corp

Host *.corp
  Port 2200
  User corp-user
  IdentityFile ~/.ssh/corp_ed25519
"#,
        )
        .unwrap();

        let jump = &hosts
            .iter()
            .find(|host| host.name == "target")
            .unwrap()
            .jump_hosts[0];
        assert_eq!(jump.host, "gateway.corp");
        assert_eq!(jump.port, 2200);
        assert_eq!(jump.username.as_deref(), Some("corp-user"));
        assert!(jump.identity_file.is_some());
    }

    #[test]
    fn parses_equals_separated_directives() {
        let hosts = parse_ssh_config(
            r#"
Host=target
  HostName=internal.example.com
  ProxyJump = bastion

Host=bastion
  HostName=gateway.example.com
"#,
        )
        .unwrap();

        let target = hosts.iter().find(|host| host.name == "target").unwrap();
        assert_eq!(target.hostname.as_deref(), Some("internal.example.com"));
        assert_eq!(target.jump_hosts[0].host, "gateway.example.com");
    }
}
