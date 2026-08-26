//! System clipboard file interop, layered above the frontend's virtual
//! (cross-tab) clipboard so the two can mix: whichever was touched most
//! recently — an in-app Copy or an external file manager copy — wins on
//! the next Paste. See `stores/clipboard.ts` for the priority logic.
//!
//! Only the read direction is supported: files copied in Explorer (or
//! another app) can be detected and pasted into a remote session to
//! upload them. Remote files copied in the app are never written back
//! to the system clipboard — doing so eagerly needed the whole
//! selection downloaded to a temp dir up front, which was a poor user
//! experience for large/slow remote-to-remote copies.
//!
//! Real file lists (Windows `CF_HDROP`) are Windows-only for now; other
//! platforms report `supports_files() == false` and the frontend falls
//! back to the virtual-clipboard-only behaviour it always had.

use crate::error::AppResult;

/// Whether this platform can carry a real file list (not just text) on
/// the system clipboard.
pub fn supports_files() -> bool {
    cfg!(windows)
}

/// Current system clipboard change counter (`GetClipboardSequenceNumber`
/// on Windows), so the frontend can tell whether the clipboard changed
/// behind its back (e.g. a copy in Explorer) since it last synced its
/// own state. Always 0 where unsupported.
#[cfg(windows)]
pub fn seq_num() -> u64 {
    clipboard_win::raw::seq_num()
        .map(|n| n.get() as u64)
        .unwrap_or(0)
}

#[cfg(not(windows))]
pub fn seq_num() -> u64 {
    0
}

/// Local file paths currently on the system clipboard, or an empty list
/// when unsupported or it currently holds something other than files.
#[cfg(windows)]
pub fn read_file_list() -> AppResult<Vec<String>> {
    use clipboard_win::{formats, Clipboard, Getter};
    let _clip = match Clipboard::new_attempts(10) {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    match formats::FileList.read_clipboard(&mut out) {
        Ok(_) => Ok(out),
        // Clipboard doesn't currently hold a file list (e.g. it's text) —
        // this is a normal, frequent case, not an error.
        Err(_) => Ok(Vec::new()),
    }
}

#[cfg(not(windows))]
pub fn read_file_list() -> AppResult<Vec<String>> {
    Ok(Vec::new())
}
