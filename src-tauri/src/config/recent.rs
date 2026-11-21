//! Visit history of remote folders, ranked by frecency and grouped per
//! server. Unlike bookmarks it needs no curating: what was busy a week
//! ago fades by itself, and only pinned folders stay.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

/// Keeping everything would grow unbounded; the tail is never shown.
const TOTAL_LIMIT: usize = 800;
/// Stops one busy server from pushing every other server's folders out.
const PER_SERVER_LIMIT: usize = 80;
/// A folder's weight halves every week.
const HALF_LIFE_SECS: f64 = 7.0 * 86_400.0;
/// Unpinned folders untouched this long are forgotten.
const STALE_SECS: u64 = 60 * 86_400;

/// Which endpoint a folder lives on. Carries no credentials: reopening a
/// folder looks them up in profiles, bookmarks and `~/.ssh/config`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRef {
    /// `local` or `user@host:port`; groups entries and finds a live tab.
    pub server_key: String,
    /// Tab label (`user@alias`) shown as the group title.
    pub label: String,
    /// "ssh" or "local".
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentDir {
    #[serde(flatten)]
    pub server: ServerRef,
    pub path: String,
    pub name: String,
    pub visits: u32,
    /// Unix seconds.
    pub last_visit: u64,
    /// Exempt from fading and eviction; listed first on its server.
    #[serde(default)]
    pub pinned: bool,
    /// Visits weighted by recency. Derived on every read.
    #[serde(default)]
    pub score: f64,
}

fn recent_path() -> PathBuf {
    super::app_config_dir().join("recent.json")
}

fn load() -> AppResult<Vec<RecentDir>> {
    let path = recent_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| AppError::ConfigError(format!("Cannot read recent folders: {}", e)))?;
    // A damaged history is not worth refusing to start over.
    Ok(serde_json::from_str(&content).unwrap_or_default())
}

fn save(list: &[RecentDir]) -> AppResult<()> {
    let content = serde_json::to_string_pretty(list)
        .map_err(|e| AppError::ConfigError(format!("Serialize error: {}", e)))?;
    std::fs::write(recent_path(), content)
        .map_err(|e| AppError::ConfigError(format!("Cannot write recent folders: {}", e)))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn frecency(entry: &RecentDir, now: u64) -> f64 {
    let age = now.saturating_sub(entry.last_visit) as f64;
    entry.visits as f64 * 0.5_f64.powf(age / HALF_LIFE_SECS)
}

/// Fill the derived score and order the list best-first.
fn rank(list: &mut [RecentDir], now: u64) {
    for e in list.iter_mut() {
        e.score = frecency(e, now);
    }
    list.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
            .then(b.last_visit.cmp(&a.last_visit))
    });
}

/// Drop what has faded and enforce the caps. Expects a ranked list.
fn prune(list: Vec<RecentDir>, now: u64) -> Vec<RecentDir> {
    let mut per_server: HashMap<String, usize> = HashMap::new();
    let mut kept = Vec::with_capacity(list.len().min(TOTAL_LIMIT));
    for e in list {
        if !e.pinned && now.saturating_sub(e.last_visit) > STALE_SECS {
            continue;
        }
        let n = per_server.entry(e.server.server_key.clone()).or_insert(0);
        if !e.pinned && *n >= PER_SERVER_LIMIT {
            continue;
        }
        *n += 1;
        kept.push(e);
        if kept.len() >= TOTAL_LIMIT {
            break;
        }
    }
    kept
}

fn dir_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => "/".to_string(),
    }
}

fn is_entry(e: &RecentDir, server_key: &str, path: &str) -> bool {
    e.server.server_key == server_key && e.path == path
}

pub fn list_recent() -> AppResult<Vec<RecentDir>> {
    let mut list = load()?;
    rank(&mut list, now_secs());
    Ok(list)
}

/// Record a visit. Called on every navigation, so it must stay cheap.
pub fn record_visit(server: ServerRef, path: &str) -> AppResult<()> {
    let mut list = load()?;
    let now = now_secs();
    match list.iter_mut().find(|e| is_entry(e, &server.server_key, path)) {
        Some(entry) => {
            // Fold the decay into the count so a revived folder starts
            // from what it is worth today, not from its old peak.
            entry.visits = (frecency(entry, now).round() as u32).max(1) + 1;
            entry.last_visit = now;
            // The tab label may have changed (alias renamed).
            entry.server = server;
        }
        None => list.push(RecentDir {
            server,
            path: path.to_string(),
            name: dir_name(path),
            visits: 1,
            last_visit: now,
            pinned: false,
            score: 0.0,
        }),
    }
    rank(&mut list, now);
    save(&prune(list, now))
}

/// Pin or unpin a folder; pinning one with no history yet adds it.
pub fn toggle_pin(server: ServerRef, path: &str) -> AppResult<Vec<RecentDir>> {
    let mut list = load()?;
    let now = now_secs();
    match list.iter_mut().find(|e| is_entry(e, &server.server_key, path)) {
        Some(entry) => entry.pinned = !entry.pinned,
        None => list.push(RecentDir {
            server,
            path: path.to_string(),
            name: dir_name(path),
            visits: 1,
            last_visit: now,
            pinned: true,
            score: 0.0,
        }),
    }
    rank(&mut list, now);
    save(&list)?;
    Ok(list)
}

pub fn remove_recent(server_key: &str, path: &str) -> AppResult<Vec<RecentDir>> {
    let mut list = load()?;
    list.retain(|e| !is_entry(e, server_key, path));
    rank(&mut list, now_secs());
    save(&list)?;
    Ok(list)
}

/// Forget the history but keep what the user pinned on purpose.
pub fn clear_recent() -> AppResult<Vec<RecentDir>> {
    let mut list = load()?;
    list.retain(|e| e.pinned);
    rank(&mut list, now_secs());
    save(&list)?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, path: &str, visits: u32, age_days: u64, pinned: bool, now: u64) -> RecentDir {
        RecentDir {
            server: ServerRef {
                server_key: key.into(),
                label: key.into(),
                kind: "ssh".into(),
                host: "h".into(),
                port: 22,
                username: "u".into(),
            },
            path: path.into(),
            name: dir_name(path),
            visits,
            last_visit: now - age_days * 86_400,
            pinned,
            score: 0.0,
        }
    }

    #[test]
    fn recent_use_beats_old_heavy_use() {
        let now = 100 * 86_400;
        let mut list = vec![
            entry("a", "/old", 20, 30, false, now),
            entry("a", "/new", 3, 0, false, now),
        ];
        rank(&mut list, now);
        assert_eq!(list[0].path, "/new");
    }

    #[test]
    fn pinned_first_and_survives_pruning() {
        let now = 100 * 86_400;
        let mut list = vec![
            entry("a", "/busy", 50, 0, false, now),
            entry("a", "/ancient", 1, 90, true, now),
            entry("a", "/stale", 9, 90, false, now),
        ];
        rank(&mut list, now);
        let paths: Vec<_> = prune(list, now).iter().map(|e| e.path.clone()).collect();
        assert_eq!(paths, ["/ancient", "/busy"]);
    }

    #[test]
    fn per_server_cap_leaves_room_for_other_servers() {
        let now = 100 * 86_400;
        let mut list: Vec<_> = (0..PER_SERVER_LIMIT + 20)
            .map(|i| entry("busy", &format!("/d{i}"), 5, 0, false, now))
            .collect();
        list.push(entry("quiet", "/x", 1, 1, false, now));
        rank(&mut list, now);
        let kept = prune(list, now);
        assert_eq!(
            kept.iter().filter(|e| e.server.server_key == "busy").count(),
            PER_SERVER_LIMIT
        );
        assert!(kept.iter().any(|e| e.server.server_key == "quiet"));
    }

    #[test]
    fn names_come_from_the_last_segment() {
        assert_eq!(dir_name("/var/log/"), "log");
        assert_eq!(dir_name("/"), "/");
    }
}
