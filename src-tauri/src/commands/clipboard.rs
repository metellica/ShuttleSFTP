//! Commands bridging the frontend to the system clipboard's real file
//! list (see `crate::clipboard`), plus the eager-download step that
//! makes "copy remote files onto the system clipboard" possible: the
//! selection is downloaded into a fresh temp directory first, then that
//! directory's paths are written as the clipboard's file list.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::State;
use tokio::io::AsyncWriteExt;

use crate::clipboard;
use crate::commands::prepare::{Prepare, PrepareRegistry};
use crate::commands::scan::scan_tree;
use crate::error::{AppError, AppResult};
use crate::fs::RemoteFs;
use crate::ssh::session::SessionManager;

/// Base temp directory for eagerly-downloaded clipboard files; each
/// copy gets its own fresh subfolder so concurrent/successive copies
/// never clash and stay valid until pasted elsewhere.
fn clipboard_temp_root() -> PathBuf {
    std::env::temp_dir().join("shuttle-sftp-clipboard")
}

/// Delete clipboard temp subfolders older than a day, best-effort, so
/// eagerly-downloaded files from past sessions don't accumulate. Meant
/// to be called once at startup off the main thread.
pub fn prune_stale_temp_dirs() {
    let root = clipboard_temp_root();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);
    for entry in entries.flatten() {
        let is_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|m| m < cutoff)
            .unwrap_or(true);
        if is_old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

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

async fn download_one(
    fs: &Arc<dyn RemoteFs>,
    remote_path: &str,
    local_path: &Path,
) -> AppResult<()> {
    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::IoError(format!("Cannot create temp dir: {}", e)))?;
    }
    let reader = fs.open_read(remote_path, 0).await?;
    let mut src = reader.reader;
    let mut dst = tokio::fs::File::create(local_path)
        .await
        .map_err(|e| AppError::IoError(format!("Cannot create temp file: {}", e)))?;
    tokio::io::copy(&mut src, &mut dst)
        .await
        .map_err(|e| AppError::IoError(format!("Cannot download to temp file: {}", e)))?;
    dst.flush().await.ok();
    Ok(())
}

/// Download the given remote paths into a fresh temp directory, then
/// place them on the system clipboard as a real file list so they can
/// be pasted into Explorer (or any other app). Returns the resulting
/// clipboard sequence number, which the frontend records as the point
/// its virtual and system clipboards were last known to agree.
#[tauri::command]
pub async fn copy_files_to_system_clipboard(
    session_id: String,
    remote_paths: Vec<String>,
    prepare_id: Option<String>,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    prepare_registry: State<'_, PrepareRegistry>,
) -> AppResult<u64> {
    if !clipboard::supports_files() {
        return Err(AppError::TransferError(
            "System clipboard file transfer is not supported on this platform".into(),
        ));
    }
    let prep = Prepare::new(&app, &prepare_registry, prepare_id);
    let session = session_manager.get_session(&session_id).await?;
    let (fs, runner) = {
        let s = session.lock().await;
        (s.fs.clone(), s.runner.clone())
    };

    let dest_root = clipboard_temp_root().join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&dest_root)
        .await
        .map_err(|e| AppError::IoError(format!("Cannot create temp dir: {}", e)))?;

    let mut local_paths = Vec::with_capacity(remote_paths.len());
    for remote_path in &remote_paths {
        prep.check()?;
        let name = remote_path.rsplit('/').next().unwrap_or("file");
        let local_root = dest_root.join(name);
        let stat = fs.stat(remote_path).await.ok();

        if stat.as_ref().map(|s| s.is_dir).unwrap_or(false) {
            prep.set_phase("scanning", 0);
            let (dirs, files) = scan_tree(&fs, &runner, remote_path, &prep).await?;
            tokio::fs::create_dir_all(&local_root).await.ok();
            for d in &dirs {
                let mut p = local_root.clone();
                for part in d.split('/') {
                    p.push(part);
                }
                tokio::fs::create_dir_all(&p).await.ok();
            }
            prep.set_phase("downloading", files.len() as u64);
            for (rel, _size) in files {
                prep.check()?;
                let remote_file = format!("{}/{}", remote_path.trim_end_matches('/'), rel);
                let mut local_file = local_root.clone();
                for part in rel.split('/') {
                    local_file.push(part);
                }
                download_one(&fs, &remote_file, &local_file).await?;
                prep.tick();
            }
        } else {
            prep.set_phase("downloading", remote_paths.len() as u64);
            download_one(&fs, remote_path, &local_root).await?;
            prep.tick();
        }
        local_paths.push(local_root.to_string_lossy().to_string());
    }

    clipboard::write_file_list(&local_paths)?;
    Ok(clipboard::seq_num())
}
