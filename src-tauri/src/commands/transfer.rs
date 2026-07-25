use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::State;
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::fs::{walk_fs_dir, RemoteFs};
use crate::ssh::session::{RemoteSession, SessionManager};
use crate::transfer::engine::{Endpoint, TransferEngine};
use crate::transfer::progress::{TaskGroup, TransferDirection, TransferStatus, TransferTask};

/// Walk a local directory tree. Returns (dirs, files) as '/'-separated
/// paths relative to `root`, dirs sorted shallow-first.
async fn walk_local_dir(root: &Path) -> AppResult<(Vec<String>, Vec<String>)> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| AppError::TransferError(format!("Cannot read local dir: {}", e)))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| AppError::TransferError(format!("Cannot read local dir: {}", e)))?
        {
            let name = entry.file_name().to_string_lossy().to_string();
            let child_rel = if rel.is_empty() {
                name
            } else {
                format!("{}/{}", rel, name)
            };
            let ft = entry
                .file_type()
                .await
                .map_err(|e| AppError::TransferError(format!("Cannot stat local entry: {}", e)))?;
            if ft.is_dir() {
                dirs.push(child_rel.clone());
                stack.push((entry.path(), child_rel));
            } else if ft.is_file() {
                files.push(child_rel);
            }
            // symlinks and special files are skipped
        }
    }
    dirs.sort();
    files.sort();
    Ok((dirs, files))
}

fn rel_to_local(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in rel.split('/') {
        p.push(part);
    }
    p
}

async fn session_fs(session: &Arc<Mutex<RemoteSession>>) -> Arc<dyn RemoteFs> {
    session.lock().await.fs.clone()
}

fn endpoint(session_id: &str, session: Arc<Mutex<RemoteSession>>) -> Endpoint {
    Endpoint::Session {
        id: session_id.to_string(),
        session,
    }
}

/// Queue every file of a local directory as one transfer group,
/// creating the remote directory structure first.
async fn queue_dir_upload(
    app: &tauri::AppHandle,
    transfer_engine: &TransferEngine,
    session: Arc<Mutex<RemoteSession>>,
    session_id: &str,
    local_dir: &Path,
    remote_parent: &str,
) -> AppResult<Vec<String>> {
    let dir_name = local_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let remote_root = format!("{}/{}", remote_parent.trim_end_matches('/'), dir_name);
    let (dirs, files) = walk_local_dir(local_dir).await?;

    {
        let fs = session_fs(&session).await;
        // Ignore "already exists" failures
        let _ = fs.mkdir(&remote_root).await;
        for d in &dirs {
            let _ = fs.mkdir(&format!("{}/{}", remote_root, d)).await;
        }
    }

    let group_id = uuid::Uuid::new_v4().to_string();
    let mut task_ids = Vec::new();
    for rel in files {
        let local = rel_to_local(local_dir, &rel);
        let remote = format!("{}/{}", remote_root, rel);
        let group = TaskGroup {
            id: group_id.clone(),
            name: dir_name.clone(),
            rel_path: rel,
        };
        let id = transfer_engine
            .queue_transfer(
                app.clone(),
                Endpoint::Local,
                &local.to_string_lossy(),
                endpoint(session_id, session.clone()),
                &remote,
                Some(group),
            )
            .await?;
        task_ids.push(id);
    }
    Ok(task_ids)
}

/// Queue every file of a remote directory as one transfer group,
/// creating the local directory structure first.
async fn queue_dir_download(
    app: &tauri::AppHandle,
    transfer_engine: &TransferEngine,
    session: Arc<Mutex<RemoteSession>>,
    session_id: &str,
    remote_dir: &str,
    local_root: &Path,
) -> AppResult<Vec<String>> {
    let dir_name = local_root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let (dirs, files) = {
        let fs = session_fs(&session).await;
        walk_fs_dir(fs.as_ref(), remote_dir).await?
    };

    tokio::fs::create_dir_all(local_root)
        .await
        .map_err(|e| AppError::TransferError(format!("Cannot create local dir: {}", e)))?;
    for d in &dirs {
        tokio::fs::create_dir_all(rel_to_local(local_root, d))
            .await
            .map_err(|e| AppError::TransferError(format!("Cannot create local dir: {}", e)))?;
    }

    let group_id = uuid::Uuid::new_v4().to_string();
    let mut task_ids = Vec::new();
    for rel in files {
        let remote = format!("{}/{}", remote_dir.trim_end_matches('/'), rel);
        let local = rel_to_local(local_root, &rel);
        let group = TaskGroup {
            id: group_id.clone(),
            name: dir_name.clone(),
            rel_path: rel,
        };
        let id = transfer_engine
            .queue_transfer(
                app.clone(),
                endpoint(session_id, session.clone()),
                &remote,
                Endpoint::Local,
                &local.to_string_lossy(),
                Some(group),
            )
            .await?;
        task_ids.push(id);
    }
    Ok(task_ids)
}

async fn is_remote_dir(session: &Arc<Mutex<RemoteSession>>, path: &str) -> bool {
    let fs = session_fs(session).await;
    fs.stat(path).await.map(|m| m.is_dir).unwrap_or(false)
}

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
        let session = session_manager.get_session(&session_id).await?;
        let path = Path::new(local_path);
        let is_dir = tokio::fs::metadata(path)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if is_dir {
            let ids = queue_dir_upload(
                &app,
                transfer_engine.inner(),
                session,
                &session_id,
                path,
                &remote_dir,
            )
            .await?;
            task_ids.extend(ids);
        } else {
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            let remote_path = format!("{}/{}", remote_dir.trim_end_matches('/'), filename);
            let task_id = transfer_engine
                .queue_transfer(
                    app.clone(),
                    Endpoint::Local,
                    local_path,
                    endpoint(&session_id, session),
                    &remote_path,
                    None,
                )
                .await?;
            task_ids.push(task_id);
        }
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
        let local_path = Path::new(&local_dir).join(filename);
        let session = session_manager.get_session(&session_id).await?;
        if is_remote_dir(&session, remote_path).await {
            let ids = queue_dir_download(
                &app,
                transfer_engine.inner(),
                session,
                &session_id,
                remote_path,
                &local_path,
            )
            .await?;
            task_ids.extend(ids);
        } else {
            let task_id = transfer_engine
                .queue_transfer(
                    app.clone(),
                    endpoint(&session_id, session),
                    remote_path,
                    Endpoint::Local,
                    &local_path.to_string_lossy(),
                    None,
                )
                .await?;
            task_ids.push(task_id);
        }
    }
    Ok(task_ids)
}

/// Download a single remote file or directory to an exact local path (Save As).
#[tauri::command]
pub async fn download_as(
    session_id: String,
    remote_path: String,
    local_path: String,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<String>> {
    let session = session_manager.get_session(&session_id).await?;
    if is_remote_dir(&session, &remote_path).await {
        queue_dir_download(
            &app,
            transfer_engine.inner(),
            session,
            &session_id,
            &remote_path,
            Path::new(&local_path),
        )
        .await
    } else {
        let task_id = transfer_engine
            .queue_transfer(
                app,
                endpoint(&session_id, session),
                &remote_path,
                Endpoint::Local,
                &local_path,
                None,
            )
            .await?;
        Ok(vec![task_id])
    }
}

/// Copy files/directories from one live session to another (host,
/// container or pod — any combination). Data is relayed through this
/// machine unless both sides live on the same host, in which case the
/// engine copies server-side.
#[tauri::command]
pub async fn transfer_remote(
    src_session_id: String,
    src_paths: Vec<String>,
    dst_session_id: String,
    dst_dir: String,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<String>> {
    if src_session_id == dst_session_id {
        return Err(AppError::TransferError(
            "Source and destination are the same session".into(),
        ));
    }
    let src_session = session_manager.get_session(&src_session_id).await?;
    let dst_session = session_manager.get_session(&dst_session_id).await?;
    let mut task_ids = Vec::new();

    for src_path in &src_paths {
        let name = src_path.rsplit('/').next().unwrap_or("file");
        let dst_path = format!("{}/{}", dst_dir.trim_end_matches('/'), name);

        if is_remote_dir(&src_session, src_path).await {
            // Recreate the tree on the destination, then queue each file.
            let (dirs, files) = {
                let fs = session_fs(&src_session).await;
                walk_fs_dir(fs.as_ref(), src_path).await?
            };
            {
                let fs = session_fs(&dst_session).await;
                let _ = fs.mkdir(&dst_path).await;
                for d in &dirs {
                    let _ = fs.mkdir(&format!("{}/{}", dst_path, d)).await;
                }
            }
            let group_id = uuid::Uuid::new_v4().to_string();
            for rel in files {
                let group = TaskGroup {
                    id: group_id.clone(),
                    name: name.to_string(),
                    rel_path: rel.clone(),
                };
                let id = transfer_engine
                    .queue_transfer(
                        app.clone(),
                        endpoint(&src_session_id, src_session.clone()),
                        &format!("{}/{}", src_path.trim_end_matches('/'), rel),
                        endpoint(&dst_session_id, dst_session.clone()),
                        &format!("{}/{}", dst_path, rel),
                        Some(group),
                    )
                    .await?;
                task_ids.push(id);
            }
        } else {
            let id = transfer_engine
                .queue_transfer(
                    app.clone(),
                    endpoint(&src_session_id, src_session.clone()),
                    src_path,
                    endpoint(&dst_session_id, dst_session.clone()),
                    &dst_path,
                    None,
                )
                .await?;
            task_ids.push(id);
        }
    }
    Ok(task_ids)
}

#[tauri::command]
pub async fn cancel_transfer(
    task_id: String,
    delete_local: Option<bool>,
    app: tauri::AppHandle,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<()> {
    transfer_engine
        .cancel(&app, &task_id, delete_local.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn cancel_all_transfers(
    delete_local: Option<bool>,
    app: tauri::AppHandle,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<()> {
    transfer_engine
        .cancel_all(&app, delete_local.unwrap_or(false))
        .await
}

/// Cancel a directory transfer group. For downloads, `delete_local`
/// removes the whole local root directory of the group.
#[tauri::command]
pub async fn cancel_transfer_group(
    group_id: String,
    delete_local: Option<bool>,
    app: tauri::AppHandle,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<()> {
    transfer_engine
        .cancel_group(&app, &group_id, delete_local.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn pause_transfer(
    task_id: String,
    app: tauri::AppHandle,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<()> {
    transfer_engine.pause(&app, &task_id).await
}

#[tauri::command]
pub async fn pause_all_transfers(
    app: tauri::AppHandle,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<String>> {
    transfer_engine.pause_all(&app).await
}

/// Find a live session to run this task on: prefer its original session,
/// otherwise any session connected to the same host as the same user.
async fn resolve_session_for_task(
    session_manager: &SessionManager,
    task: &TransferTask,
) -> AppResult<(String, Arc<Mutex<RemoteSession>>)> {
    if let Ok(session) = session_manager.get_session(&task.session_id).await {
        return Ok((task.session_id.clone(), session));
    }
    session_manager
        .find_session_for(&task.host, &task.username)
        .await
        .ok_or_else(|| {
            AppError::SessionNotFound(format!(
                "No active session for {}@{}; connect first, then resume",
                task.username, task.host
            ))
        })
}

/// Rebuild the two transfer endpoints of a persisted task.
async fn endpoints_for_task(
    session_manager: &SessionManager,
    task: &TransferTask,
    session_override: Option<String>,
) -> AppResult<(Endpoint, Endpoint)> {
    match task.direction {
        TransferDirection::Upload => {
            let (sid, session) = match session_override {
                Some(sid) => {
                    let s = session_manager.get_session(&sid).await?;
                    (sid, s)
                }
                None => resolve_session_for_task(session_manager, task).await?,
            };
            Ok((Endpoint::Local, endpoint(&sid, session)))
        }
        TransferDirection::Download => {
            let (sid, session) = match session_override {
                Some(sid) => {
                    let s = session_manager.get_session(&sid).await?;
                    (sid, s)
                }
                None => resolve_session_for_task(session_manager, task).await?,
            };
            Ok((endpoint(&sid, session), Endpoint::Local))
        }
        TransferDirection::Remote => {
            // Both sessions must still be alive: container/pod endpoints
            // cannot be rebound by host name.
            let src = session_manager.get_session(&task.session_id).await.map_err(|_| {
                AppError::SessionNotFound(
                    "Source session of this copy is closed; reconnect and start it again".into(),
                )
            })?;
            let dst_id = task.dest_session_id.clone().ok_or_else(|| {
                AppError::TransferError("Copy task has no destination session".into())
            })?;
            let dst = session_manager.get_session(&dst_id).await.map_err(|_| {
                AppError::SessionNotFound(
                    "Destination session of this copy is closed; reconnect and start it again"
                        .into(),
                )
            })?;
            Ok((endpoint(&task.session_id, src), endpoint(&dst_id, dst)))
        }
    }
}

/// Resume a paused/failed transfer. If `session_id` is given, run it on that
/// session; otherwise rebind to the original or a matching live session.
#[tauri::command]
pub async fn resume_transfer(
    task_id: String,
    session_id: Option<String>,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<()> {
    let task = transfer_engine
        .get_task(&task_id)
        .await
        .ok_or_else(|| AppError::TransferError(format!("Task not found: {}", task_id)))?;
    let (src, dst) = endpoints_for_task(session_manager.inner(), &task, session_id).await?;
    transfer_engine.resume(app, &task_id, src, dst).await
}

/// Resume all paused transfers that can be bound to a live session.
/// Returns the ids of the transfers that were resumed.
#[tauri::command]
pub async fn resume_all_transfers(
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<String>> {
    let mut resumed = Vec::new();
    for task in transfer_engine.list_tasks().await {
        if task.status != TransferStatus::Paused {
            continue;
        }
        let Ok((src, dst)) = endpoints_for_task(session_manager.inner(), &task, None).await
        else {
            continue;
        };
        if transfer_engine
            .resume(app.clone(), &task.id, src, dst)
            .await
            .is_ok()
        {
            resumed.push(task.id);
        }
    }
    Ok(resumed)
}

/// Reveal a local path in the system file manager: directories open
/// directly, files are selected in their containing folder.
#[tauri::command]
pub async fn show_in_folder(path: String) -> AppResult<()> {
    let p = PathBuf::from(&path);
    let target = if p.exists() {
        p
    } else {
        // e.g. file already deleted: fall back to the containing folder
        p.parent()
            .filter(|d| d.exists())
            .map(|d| d.to_path_buf())
            .ok_or_else(|| AppError::IoError(format!("Path not found: {}", path)))?
    };

    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("explorer");
        if target.is_dir() {
            cmd.arg(&target);
        } else {
            cmd.arg("/select,").arg(&target);
        }
        // explorer.exe reports a nonzero exit code even on success
        cmd.spawn().map_err(|e| AppError::IoError(e.to_string()))?;
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if !target.is_dir() {
            cmd.arg("-R");
        }
        cmd.arg(&target);
        cmd.spawn().map_err(|e| AppError::IoError(e.to_string()))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = if target.is_dir() {
            target.clone()
        } else {
            target.parent().map(|d| d.to_path_buf()).unwrap_or(target)
        };
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| AppError::IoError(e.to_string()))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn clear_finished_transfers(
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<()> {
    transfer_engine.clear_finished().await
}

#[tauri::command]
pub async fn list_transfers(
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<TransferTask>> {
    Ok(transfer_engine.list_tasks().await)
}
