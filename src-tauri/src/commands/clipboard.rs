//! Commands bridging the frontend to the system clipboard's real file
//! list (see `crate::clipboard`). Only the read direction is exposed:
//! files copied in Explorer (or another app) can be pasted into a
//! remote session to upload them. Remote files copied in the app are
//! never eagerly written back to the system clipboard — that required
//! downloading the whole selection to a temp dir up front, which was a
//! poor user experience for large/slow remote-to-remote copies.

use crate::clipboard;
use crate::error::AppResult;

/// Whether this platform can carry real files (not just text) on the
/// system clipboard.
#[tauri::command]
pub fn clipboard_supports_files() -> bool {
    clipboard::supports_files()
}

/// Current system clipboard change counter, so the frontend can tell
/// whether the clipboard changed behind its back (e.g. a copy in
/// Explorer) since it last synced its own state.
#[tauri::command]
pub fn clipboard_seq_num() -> u64 {
    clipboard::seq_num()
}

/// Local file paths currently on the system clipboard (Windows
/// `CF_HDROP`); empty when unsupported or it holds something else.
#[tauri::command]
pub fn read_system_clipboard_files() -> AppResult<Vec<String>> {
    clipboard::read_file_list()
}
