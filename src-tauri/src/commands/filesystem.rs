use serde::Serialize;
use tauri::State;

use crate::commands::prepare::{Prepare, PrepareRegistry};
use crate::commands::scan::scan_tree;
use crate::error::AppResult;
use crate::ssh::session::SessionManager;
use crate::fs::FileEntry;

/// Preview of a remote file: text content if it looks like text.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub is_text: bool,
    pub content: Option<String>,
    pub truncated: bool,
}

const PREVIEW_MAX_BYTES: usize = 256 * 1024;

#[tauri::command]
pub async fn preview_file(
    session_id: String,
    path: String,
    full: Option<bool>,
    session_manager: State<'_, SessionManager>,
) -> AppResult<FilePreview> {
    let cap = if full.unwrap_or(false) {
        usize::MAX
    } else {
        PREVIEW_MAX_BYTES
    };
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    // Read one extra byte to detect truncation
    let bytes = session.fs.read_head(&path, cap.saturating_add(1)).await?;

    let truncated = bytes.len() > cap;
    let data = &bytes[..bytes.len().min(cap)];

    // Binary sniffing: NUL byte or too many non-UTF8 sequences => binary
    if data.contains(&0u8) {
        return Ok(FilePreview {
            is_text: false,
            content: None,
            truncated,
        });
    }

    match std::str::from_utf8(data) {
        Ok(s) => Ok(FilePreview {
            is_text: true,
            content: Some(s.to_string()),
            truncated,
        }),
        Err(e) => {
            // Allow a trailing multi-byte char cut off by the read window
            let valid = e.valid_up_to();
            if valid > 0 && data.len() - valid < 4 {
                let s = String::from_utf8_lossy(&data[..valid]).to_string();
                Ok(FilePreview {
                    is_text: true,
                    content: Some(s),
                    truncated: true,
                })
            } else {
                Ok(FilePreview {
                    is_text: false,
                    content: None,
                    truncated,
                })
            }
        }
    }
}

#[tauri::command]
pub async fn save_file(
    session_id: String,
    path: String,
    content: String,
    session_manager: State<'_, SessionManager>,
) -> AppResult<()> {
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    session.fs.write_file(&path, content.as_bytes()).await
}

#[tauri::command]
pub async fn list_dir(
    session_id: String,
    path: String,
    session_manager: State<'_, SessionManager>,
) -> AppResult<Vec<FileEntry>> {
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    session.fs.list_dir(&path).await
}

#[tauri::command]
pub async fn mkdir(
    session_id: String,
    path: String,
    session_manager: State<'_, SessionManager>,
) -> AppResult<()> {
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    session.fs.mkdir(&path).await
}

#[tauri::command]
pub async fn remove(
    session_id: String,
    path: String,
    is_dir: bool,
    prepare_id: Option<String>,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    prepare_registry: State<'_, PrepareRegistry>,
) -> AppResult<()> {
    let session = session_manager.get_session(&session_id).await?;
    let (fs, runner) = {
        let s = session.lock().await;
        (s.fs.clone(), s.runner.clone())
    };
    if !is_dir {
        return fs.remove_file(&path).await;
    }
    let prep = Prepare::new(&app, &prepare_registry, prepare_id);
    prep.check()?;
    if fs.fast_remove_dir(&path) {
        // One fast server-side call (rm -rf / native recursive delete):
        // indeterminate progress, not cancellable mid-flight.
        prep.set_phase("deleting", 0);
        return fs.remove_dir_all(&path).await;
    }
    // Client-side recursion (SFTP): scan first for a file count, then
    // delete per file with progress and cancellation.
    prep.set_phase("scanning", 0);
    let (dirs, files) = scan_tree(&fs, &runner, &path, &prep).await?;
    prep.set_phase("deleting", files.len() as u64);
    let root = path.trim_end_matches('/');
    for (rel, _) in &files {
        prep.check()?;
        fs.remove_file(&format!("{}/{}", root, rel)).await?;
        prep.tick();
    }
    // Dirs are empty now; children sort after their parent, so reverse
    // lexicographic order removes deepest-first.
    for d in dirs.iter().rev() {
        prep.check()?;
        fs.remove_dir_all(&format!("{}/{}", root, d)).await?;
    }
    fs.remove_dir_all(&path).await
}

#[tauri::command]
pub async fn rename(
    session_id: String,
    old_path: String,
    new_path: String,
    session_manager: State<'_, SessionManager>,
) -> AppResult<()> {
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    session.fs.rename(&old_path, &new_path).await
}
