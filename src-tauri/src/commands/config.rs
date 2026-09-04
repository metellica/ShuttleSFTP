use crate::config::bookmarks::{self, Bookmark};
use crate::config::profiles::{self, ConnectionProfile};
use crate::config::ssh_config::{self, SshHostEntry};
use crate::error::AppResult;

#[tauri::command]
pub fn load_ssh_config() -> AppResult<Vec<SshHostEntry>> {
    ssh_config::load_ssh_config()
}

#[tauri::command]
pub fn list_imported_ssh_hosts() -> AppResult<Vec<String>> {
    ssh_config::load_imported_hosts()
}

#[tauri::command]
pub fn set_imported_ssh_hosts(names: Vec<String>) -> AppResult<()> {
    ssh_config::save_imported_hosts(&names)
}

#[tauri::command]
pub fn list_profiles() -> AppResult<Vec<ConnectionProfile>> {
    profiles::load_profiles()
}

#[tauri::command]
pub fn save_profile(profile: ConnectionProfile) -> AppResult<()> {
    let mut profiles = profiles::load_profiles().unwrap_or_default();
    // Update existing or add new
    if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
        *existing = profile;
    } else {
        profiles.push(profile);
    }
    profiles::save_profiles(&profiles)
}

#[tauri::command]
pub fn delete_profile(profile_id: String) -> AppResult<()> {
    let mut profiles = profiles::load_profiles().unwrap_or_default();
    profiles.retain(|p| p.id != profile_id);
    profiles::save_profiles(&profiles)
}

#[tauri::command]
pub fn list_bookmarks() -> AppResult<Vec<Bookmark>> {
    bookmarks::load_bookmarks()
}

#[tauri::command]
pub fn save_bookmark(bookmark: Bookmark) -> AppResult<()> {
    let mut bookmarks = bookmarks::load_bookmarks().unwrap_or_default();
    // Same id, or same target (host+port+user+path) counts as a duplicate: update in place
    if let Some(existing) = bookmarks.iter_mut().find(|b| {
        b.id == bookmark.id
            || (b.host == bookmark.host
                && b.port == bookmark.port
                && b.username == bookmark.username
                && b.path == bookmark.path
                && same_jump_route(&b.jump_hosts, &bookmark.jump_hosts))
    }) {
        let id = existing.id.clone();
        *existing = bookmark;
        existing.id = id;
    } else {
        bookmarks.push(bookmark);
    }
    bookmarks::save_bookmarks(&bookmarks)
}

fn same_jump_route(
    left: &[crate::ssh::session::JumpHost],
    right: &[crate::ssh::session::JumpHost],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.host == right.host
                && left.port == right.port
                && left.username == right.username
                && left.identity_file == right.identity_file
        })
}

#[tauri::command]
pub fn delete_bookmark(bookmark_id: String) -> AppResult<()> {
    let mut bookmarks = bookmarks::load_bookmarks().unwrap_or_default();
    bookmarks.retain(|b| b.id != bookmark_id);
    bookmarks::save_bookmarks(&bookmarks)
}
