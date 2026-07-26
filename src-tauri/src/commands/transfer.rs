use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use tauri::{Emitter, State};
use tokio::sync::Mutex;

use crate::commands::prepare::{Prepare, PrepareRegistry, CANCELLED_MSG};
use crate::commands::scan::scan_tree;
use crate::error::{AppError, AppResult};
use crate::exec::CommandRunner;
use crate::fs::RemoteFs;
use crate::ssh::session::{RemoteSession, SessionManager};
use crate::transfer::engine::{BatchItem, Endpoint, TransferEngine};
use crate::transfer::progress::{TaskGroup, TransferDirection, TransferStatus, TransferTask};

/// Synchronous local tree walk on std::fs, for one spawn_blocking call:
/// avoids tokio::fs's per-entry thread-pool dispatch. Progress goes to
/// `count`; `cancel` aborts between entries.
fn walk_local_sync(
    root: &Path,
    count: &AtomicU64,
    cancel: &AtomicBool,
) -> AppResult<(Vec<String>, Vec<(String, u64)>)> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let rd = std::fs::read_dir(&dir)
            .map_err(|e| AppError::TransferError(format!("Cannot read local dir: {}", e)))?;
        for entry in rd {
            if cancel.load(Ordering::Relaxed) {
                return Err(AppError::TransferError(CANCELLED_MSG.into()));
            }
            let entry = entry
                .map_err(|e| AppError::TransferError(format!("Cannot read local dir: {}", e)))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let child_rel = if rel.is_empty() {
                name
            } else {
                format!("{}/{}", rel, name)
            };
            let meta = entry
                .metadata()
                .map_err(|e| AppError::TransferError(format!("Cannot stat local entry: {}", e)))?;
            if meta.is_dir() {
                dirs.push(child_rel.clone());
                stack.push((entry.path(), child_rel));
            } else if meta.is_file() {
                count.fetch_add(1, Ordering::Relaxed);
                files.push((child_rel, meta.len()));
            }
            // symlinks and special files are skipped
        }
    }
    dirs.sort();
    files.sort();
    Ok((dirs, files))
}

/// Walk a local directory tree on a blocking thread, relaying live
/// progress and cancellation through `prep`. Returns dirs as
/// '/'-separated paths relative to `root` and files as (relative path,
/// size) pairs, sorted shallow-first.
async fn walk_local_dir(
    root: &Path,
    prep: &Prepare<'_>,
) -> AppResult<(Vec<String>, Vec<(String, u64)>)> {
    let count = Arc::new(AtomicU64::new(0));
    let cancel = prep.cancel_flag();
    let (root2, count2) = (root.to_path_buf(), count.clone());
    let mut task = tokio::task::spawn_blocking(move || walk_local_sync(&root2, &count2, &cancel));
    let result = loop {
        tokio::select! {
            joined = &mut task => {
                break joined
                    .map_err(|e| AppError::TransferError(format!("Scan thread failed: {}", e)))??;
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(80)) => {
                prep.set_done(count.load(Ordering::Relaxed));
            }
        }
    };
    prep.set_done(count.load(Ordering::Relaxed));
    Ok(result)
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

async fn session_fs_runner(
    session: &Arc<Mutex<RemoteSession>>,
) -> (Arc<dyn RemoteFs>, Arc<dyn CommandRunner>) {
    let s = session.lock().await;
    (s.fs.clone(), s.runner.clone())
}

/// Create a directory tree, level by level: parents strictly before
/// children, dirs within one level concurrently (hides per-mkdir
/// latency). "Already exists" failures are ignored, like before.
async fn mkdir_tree(
    fs: &Arc<dyn RemoteFs>,
    root: &str,
    dirs: &[String],
    prep: &Prepare<'_>,
) -> AppResult<()> {
    let _ = fs.mkdir(root).await;
    let mut levels: Vec<Vec<String>> = Vec::new();
    for d in dirs {
        let depth = d.matches('/').count();
        while levels.len() <= depth {
            levels.push(Vec::new());
        }
        levels[depth].push(format!("{}/{}", root, d));
    }
    for level in levels {
        prep.check()?;
        let mut inflight = tokio::task::JoinSet::new();
        for path in level {
            let fs = fs.clone();
            inflight.spawn(async move {
                let _ = fs.mkdir(&path).await;
            });
            if inflight.len() >= 16 {
                let _ = inflight.join_next().await;
            }
        }
        while inflight.join_next().await.is_some() {}
    }
    Ok(())
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
    prep: &Prepare<'_>,
) -> AppResult<Vec<String>> {
    let dir_name = local_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let remote_root = format!("{}/{}", remote_parent.trim_end_matches('/'), dir_name);
    prep.set_phase("scanning", 0);
    let (dirs, files) = walk_local_dir(local_dir, prep).await?;

    {
        let fs = session_fs(&session).await;
        mkdir_tree(&fs, &remote_root, &dirs, prep).await?;
    }

    prep.set_phase("queueing", files.len() as u64);
    let group_id = uuid::Uuid::new_v4().to_string();
    let mut items = Vec::with_capacity(files.len());
    for (rel, size) in files {
        prep.check()?;
        let local = rel_to_local(local_dir, &rel);
        items.push(BatchItem {
            src_path: local.to_string_lossy().to_string(),
            dst_path: format!("{}/{}", remote_root, rel),
            group: Some(TaskGroup {
                id: group_id.clone(),
                name: dir_name.clone(),
                rel_path: rel,
            }),
            total_bytes: size,
        });
        prep.tick();
    }
    transfer_engine
        .queue_transfer_batch(
            app.clone(),
            Endpoint::Local,
            endpoint(session_id, session.clone()),
            items,
        )
        .await
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
    prep: &Prepare<'_>,
) -> AppResult<Vec<String>> {
    let dir_name = local_root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    prep.set_phase("scanning", 0);
    let (dirs, files) = {
        let (fs, runner) = session_fs_runner(&session).await;
        scan_tree(&fs, &runner, remote_dir, prep).await?
    };

    tokio::fs::create_dir_all(local_root)
        .await
        .map_err(|e| AppError::TransferError(format!("Cannot create local dir: {}", e)))?;
    for d in &dirs {
        prep.check()?;
        tokio::fs::create_dir_all(rel_to_local(local_root, d))
            .await
            .map_err(|e| AppError::TransferError(format!("Cannot create local dir: {}", e)))?;
    }

    prep.set_phase("queueing", files.len() as u64);
    let group_id = uuid::Uuid::new_v4().to_string();
    let mut items = Vec::with_capacity(files.len());
    for (rel, size) in files {
        prep.check()?;
        let local = rel_to_local(local_root, &rel);
        items.push(BatchItem {
            src_path: format!("{}/{}", remote_dir.trim_end_matches('/'), rel),
            dst_path: local.to_string_lossy().to_string(),
            group: Some(TaskGroup {
                id: group_id.clone(),
                name: dir_name.clone(),
                rel_path: rel,
            }),
            total_bytes: size,
        });
        prep.tick();
    }
    transfer_engine
        .queue_transfer_batch(
            app.clone(),
            endpoint(session_id, session.clone()),
            Endpoint::Local,
            items,
        )
        .await
}

/// Stat a remote path, returning None when it does not exist.
async fn remote_stat(
    session: &Arc<Mutex<RemoteSession>>,
    path: &str,
) -> Option<crate::fs::FileStat> {
    let fs = session_fs(session).await;
    fs.stat(path).await.ok()
}

#[tauri::command]
pub async fn upload(
    session_id: String,
    local_paths: Vec<String>,
    remote_dir: String,
    prepare_id: Option<String>,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
    prepare_registry: State<'_, PrepareRegistry>,
) -> AppResult<Vec<String>> {
    let prep = Prepare::new(&app, &prepare_registry, prepare_id);
    let mut task_ids = Vec::new();
    for local_path in &local_paths {
        prep.check()?;
        let session = session_manager.get_session(&session_id).await?;
        let path = Path::new(local_path);
        let meta = tokio::fs::metadata(path).await.ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        if is_dir {
            let ids = queue_dir_upload(
                &app,
                transfer_engine.inner(),
                session,
                &session_id,
                path,
                &remote_dir,
                &prep,
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
                    meta.map(|m| m.len()).unwrap_or(0),
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
    prepare_id: Option<String>,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
    prepare_registry: State<'_, PrepareRegistry>,
) -> AppResult<Vec<String>> {
    let prep = Prepare::new(&app, &prepare_registry, prepare_id);
    let mut task_ids = Vec::new();
    for remote_path in &remote_paths {
        prep.check()?;
        let filename = remote_path
            .rsplit('/')
            .next()
            .unwrap_or("file");
        let local_path = Path::new(&local_dir).join(filename);
        let session = session_manager.get_session(&session_id).await?;
        let stat = remote_stat(&session, remote_path).await;
        if stat.as_ref().map(|s| s.is_dir).unwrap_or(false) {
            let ids = queue_dir_download(
                &app,
                transfer_engine.inner(),
                session,
                &session_id,
                remote_path,
                &local_path,
                &prep,
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
                    stat.map(|s| s.size).unwrap_or(0),
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
    prepare_id: Option<String>,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
    prepare_registry: State<'_, PrepareRegistry>,
) -> AppResult<Vec<String>> {
    let prep = Prepare::new(&app, &prepare_registry, prepare_id);
    let session = session_manager.get_session(&session_id).await?;
    let stat = remote_stat(&session, &remote_path).await;
    if stat.as_ref().map(|s| s.is_dir).unwrap_or(false) {
        queue_dir_download(
            &app,
            transfer_engine.inner(),
            session,
            &session_id,
            &remote_path,
            Path::new(&local_path),
            &prep,
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
                stat.map(|s| s.size).unwrap_or(0),
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
    prepare_id: Option<String>,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
    prepare_registry: State<'_, PrepareRegistry>,
) -> AppResult<Vec<String>> {
    if src_session_id == dst_session_id && src_paths.iter().any(|p| {
        let name = p.rsplit('/').next().unwrap_or("");
        format!("{}/{}", dst_dir.trim_end_matches('/'), name) == *p
    }) {
        return Err(AppError::TransferError(
            "Source and destination are the same file".into(),
        ));
    }
    let prep = Prepare::new(&app, &prepare_registry, prepare_id);
    let src_session = session_manager.get_session(&src_session_id).await?;
    let dst_session = session_manager.get_session(&dst_session_id).await?;
    let mut task_ids = Vec::new();

    for src_path in &src_paths {
        prep.check()?;
        let name = src_path.rsplit('/').next().unwrap_or("file");
        let dst_path = format!("{}/{}", dst_dir.trim_end_matches('/'), name);

        let stat = remote_stat(&src_session, src_path).await;
        if stat.as_ref().map(|s| s.is_dir).unwrap_or(false) {
            // Recreate the tree on the destination, then queue each file.
            prep.set_phase("scanning", 0);
            let (dirs, files) = {
                let (fs, runner) = session_fs_runner(&src_session).await;
                scan_tree(&fs, &runner, src_path, &prep).await?
            };
            {
                let fs = session_fs(&dst_session).await;
                mkdir_tree(&fs, &dst_path, &dirs, &prep).await?;
            }
            prep.set_phase("queueing", files.len() as u64);
            let group_id = uuid::Uuid::new_v4().to_string();
            let mut items = Vec::with_capacity(files.len());
            for (rel, size) in files {
                prep.check()?;
                items.push(BatchItem {
                    src_path: format!("{}/{}", src_path.trim_end_matches('/'), rel),
                    dst_path: format!("{}/{}", dst_path, rel),
                    group: Some(TaskGroup {
                        id: group_id.clone(),
                        name: name.to_string(),
                        rel_path: rel,
                    }),
                    total_bytes: size,
                });
                prep.tick();
            }
            let ids = transfer_engine
                .queue_transfer_batch(
                    app.clone(),
                    endpoint(&src_session_id, src_session.clone()),
                    endpoint(&dst_session_id, dst_session.clone()),
                    items,
                )
                .await?;
            task_ids.extend(ids);
        } else {
            let id = transfer_engine
                .queue_transfer(
                    app.clone(),
                    endpoint(&src_session_id, src_session.clone()),
                    src_path,
                    endpoint(&dst_session_id, dst_session.clone()),
                    &dst_path,
                    None,
                    stat.map(|s| s.size).unwrap_or(0),
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

/// Pause every queued/active task of a directory transfer group.
#[tauri::command]
pub async fn pause_transfer_group(
    group_id: String,
    app: tauri::AppHandle,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<String>> {
    transfer_engine.pause_group(&app, &group_id).await
}

/// Resume every paused/failed task of a directory transfer group that can
/// be bound to a live session. Returns the ids of the resumed transfers.
#[tauri::command]
pub async fn resume_transfer_group(
    group_id: String,
    session_id: Option<String>,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    transfer_engine: State<'_, TransferEngine>,
) -> AppResult<Vec<String>> {
    let mut resumed = Vec::new();
    for task in transfer_engine.list_tasks().await {
        if task.group_id.as_deref() != Some(group_id.as_str()) {
            continue;
        }
        if !matches!(task.status, TransferStatus::Paused | TransferStatus::Failed) {
            continue;
        }
        let Ok((src, dst)) =
            endpoints_for_task(session_manager.inner(), &task, session_id.clone()).await
        else {
            continue;
        };
        if transfer_engine
            .resume(app.clone(), &task.id, src, dst, true)
            .await
            .is_ok()
        {
            resumed.push(task.id);
        }
    }
    // One event for the whole batch instead of one per resumed task
    if !resumed.is_empty() {
        let _ = app.emit("transfer:bulk-update", ());
    }
    Ok(resumed)
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
            // Rebind each side: prefer the original session, else any live
            // session on the same host (virtual container/pod paths are
            // served by the host session).
            let (src_id, src) = match session_manager.get_session(&task.session_id).await {
                Ok(s) => (task.session_id.clone(), s),
                Err(_) => session_manager
                    .find_session_for(&task.host, &task.username)
                    .await
                    .ok_or_else(|| {
                        AppError::SessionNotFound(format!(
                            "No active session for {}@{}; connect first, then resume",
                            task.username, task.host
                        ))
                    })?,
            };
            let dst_live = match &task.dest_session_id {
                Some(id) => session_manager
                    .get_session(id)
                    .await
                    .ok()
                    .map(|s| (id.clone(), s)),
                None => None,
            };
            let (dst_id, dst) = match dst_live {
                Some(pair) => pair,
                None => session_manager
                    .find_session_for(&task.dest_host, &task.dest_username)
                    .await
                    .ok_or_else(|| {
                        AppError::SessionNotFound(format!(
                            "No active session for destination {}; connect first, then resume",
                            task.dest_host
                        ))
                    })?,
            };
            Ok((endpoint(&src_id, src), endpoint(&dst_id, dst)))
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
    transfer_engine.resume(app, &task_id, src, dst, false).await
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
            .resume(app.clone(), &task.id, src, dst, true)
            .await
            .is_ok()
        {
            resumed.push(task.id);
        }
    }
    // One event for the whole batch instead of one per resumed task
    if !resumed.is_empty() {
        let _ = app.emit("transfer:bulk-update", ());
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
