//! System clipboard file interop, layered above the frontend's virtual
//! (cross-tab) clipboard so the two can mix: whichever was touched most
//! recently — an in-app Copy or an external file manager copy — wins on
//! the next Paste. See `stores/clipboard.ts` for the priority logic.
//!
//! Real file lists (Windows `CF_HDROP`) are Windows-only for now; other
//! platforms report `supports_files() == false` and the frontend falls
//! back to the virtual-clipboard-only behaviour it always had.

use crate::error::{AppError, AppResult};

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

/// Place `paths` (absolute, local) on the system clipboard as a real
/// file list, pasteable into Explorer or any other app.
#[cfg(windows)]
pub fn write_file_list(paths: &[String]) -> AppResult<()> {
    use clipboard_win::{formats, Clipboard, Setter};
    let _clip = Clipboard::new_attempts(10)
        .map_err(|e| AppError::IoError(format!("Cannot open system clipboard: {}", e)))?;
    formats::FileList
        .write_clipboard(paths)
        .map_err(|e| AppError::IoError(format!("Cannot write file list to clipboard: {}", e)))
}

#[cfg(not(windows))]
pub fn write_file_list(_paths: &[String]) -> AppResult<()> {
    Err(AppError::TransferError(
        "System clipboard file transfer is not supported on this platform".into(),
    ))
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
