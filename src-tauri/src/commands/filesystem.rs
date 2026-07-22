use serde::Serialize;
use tauri::State;

use crate::error::AppResult;
use crate::ssh::session::SessionManager;
use crate::ssh::sftp::FileEntry;

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
    session_manager: State<'_, SessionManager>,
) -> AppResult<FilePreview> {
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    // Read one extra byte to detect truncation
    let bytes = session.sftp.read_head(&path, PREVIEW_MAX_BYTES + 1).await?;

    let truncated = bytes.len() > PREVIEW_MAX_BYTES;
    let data = &bytes[..bytes.len().min(PREVIEW_MAX_BYTES)];

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
pub async fn list_dir(
    session_id: String,
    path: String,
    session_manager: State<'_, SessionManager>,
) -> AppResult<Vec<FileEntry>> {
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    session.sftp.list_dir(&path).await
}

#[tauri::command]
pub async fn mkdir(
    session_id: String,
    path: String,
    session_manager: State<'_, SessionManager>,
) -> AppResult<()> {
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    session.sftp.mkdir(&path).await
}

#[tauri::command]
pub async fn remove(
    session_id: String,
    path: String,
    is_dir: bool,
    session_manager: State<'_, SessionManager>,
) -> AppResult<()> {
    let session = session_manager.get_session(&session_id).await?;
    let session = session.lock().await;
    if is_dir {
        session.sftp.remove_dir(&path).await
    } else {
        session.sftp.remove_file(&path).await
    }
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
    session.sftp.rename(&old_path, &new_path).await
}
