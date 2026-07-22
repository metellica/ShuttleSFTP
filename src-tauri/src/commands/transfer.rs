use tauri::State;

use crate::error::AppResult;
use crate::ssh::session::SessionManager;
use crate::transfer::engine::TransferEngine;
use crate::transfer::progress::TransferTask;

#[tauri::command]
pub async fn upload(
    session_id: String,
    local_paths: Vec<String>,
    remote_dir: String,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<String>> {
    let mut task_ids = Vec::new();
    for local_path in &local_paths {
        let filename = std::path::Path::new(local_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let remote_path = format!("{}/{}", remote_dir.trim_end_matches('/'), filename);
        let session = session_manager.get_session(&session_id).await?;
        let task_id = transfer_engine
            .queue_upload(app.clone(), session, &session_id, local_path, &remote_path)
            .await?;
        task_ids.push(task_id);
    }
    Ok(task_ids)
}

#[tauri::command]
pub async fn download(
    session_id: String,
    remote_paths: Vec<String>,
    local_dir: String,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<String>> {
    let mut task_ids = Vec::new();
    for remote_path in &remote_paths {
        let filename = remote_path
            .rsplit('/')
            .next()
            .unwrap_or("file");
        let local_path = std::path::Path::new(&local_dir)
            .join(filename)
            .to_string_lossy()
            .to_string();
        let session = session_manager.get_session(&session_id).await?;
        let task_id = transfer_engine
            .queue_download(app.clone(), session, &session_id, remote_path, &local_path)
            .await?;
        task_ids.push(task_id);
    }
    Ok(task_ids)
}

/// Download a single remote file to an exact local path (Save As).
#[tauri::command]
pub async fn download_as(
    session_id: String,
    remote_path: String,
    local_path: String,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<String> {
    let session = session_manager.get_session(&session_id).await?;
    transfer_engine
        .queue_download(app, session, &session_id, &remote_path, &local_path)
        .await
}

#[tauri::command]
pub async fn cancel_transfer(
    task_id: String,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<()> {
    transfer_engine.cancel(&task_id).await
}

#[tauri::command]
pub async fn list_transfers(
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<TransferTask>> {
    Ok(transfer_engine.list_tasks().await)
}
