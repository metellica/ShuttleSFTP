use tauri::State;

use crate::error::AppResult;
use crate::ssh::session::SessionManager;
use crate::ssh::sftp::FileEntry;

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
