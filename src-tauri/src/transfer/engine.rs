use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, Semaphore};

use crate::error::{AppError, AppResult};
use crate::ssh::session::SshSession;
use crate::transfer::progress::{
    TransferDirection, TransferProgress, TransferStatus, TransferTask,
};

const CHUNK_SIZE: usize = 64 * 1024;
const PROGRESS_EMIT_THRESHOLD: u64 = 256 * 1024;

/// Status change event emitted to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    task_id: String,
    status: TransferStatus,
}

/// Manages the transfer queue and executes upload/download tasks.
pub struct TransferEngine {
    tasks: Arc<Mutex<HashMap<String, TransferTask>>>,
    semaphore: Arc<Semaphore>,
}

impl TransferEngine {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Queue an upload task and start it in the background.
    pub async fn queue_upload(
        &self,
        app: tauri::AppHandle,
        session: Arc<Mutex<SshSession>>,
        session_id: &str,
        local_path: &str,
        remote_path: &str,
    ) -> AppResult<String> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let task = TransferTask {
            id: task_id.clone(),
            session_id: session_id.to_string(),
            direction: TransferDirection::Upload,
            source_path: local_path.to_string(),
            dest_path: remote_path.to_string(),
            total_bytes: 0,
            transferred_bytes: 0,
            status: TransferStatus::Queued,
        };
        self.tasks.lock().await.insert(task_id.clone(), task);
        self.spawn_transfer(app, session, task_id.clone());
        Ok(task_id)
    }

    /// Queue a download task and start it in the background.
    pub async fn queue_download(
        &self,
        app: tauri::AppHandle,
        session: Arc<Mutex<SshSession>>,
        session_id: &str,
        remote_path: &str,
        local_path: &str,
    ) -> AppResult<String> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let task = TransferTask {
            id: task_id.clone(),
            session_id: session_id.to_string(),
            direction: TransferDirection::Download,
            source_path: remote_path.to_string(),
            dest_path: local_path.to_string(),
            total_bytes: 0,
            transferred_bytes: 0,
            status: TransferStatus::Queued,
        };
        self.tasks.lock().await.insert(task_id.clone(), task);
        self.spawn_transfer(app, session, task_id.clone());
        Ok(task_id)
    }

    fn spawn_transfer(
        &self,
        app: tauri::AppHandle,
        session: Arc<Mutex<SshSession>>,
        task_id: String,
    ) {
        let tasks = self.tasks.clone();
        let semaphore = self.semaphore.clone();

        tokio::spawn(async move {
            let _permit = match semaphore.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };

            let (direction, source, dest) = {
                let map = tasks.lock().await;
                match map.get(&task_id) {
                    Some(t) if t.status == TransferStatus::Queued => (
                        t.direction.clone(),
                        t.source_path.clone(),
                        t.dest_path.clone(),
                    ),
                    _ => return, // cancelled before start, or missing
                }
            };

            set_status(&tasks, &app, &task_id, TransferStatus::Active).await;

            let result = match direction {
                TransferDirection::Upload => {
                    run_upload(&tasks, &app, &task_id, session, &source, &dest).await
                }
                TransferDirection::Download => {
                    run_download(&tasks, &app, &task_id, session, &source, &dest).await
                }
            };

            match result {
                Ok(()) => {
                    set_status(&tasks, &app, &task_id, TransferStatus::Completed).await;
                    log::info!("Transfer {} completed: {} -> {}", task_id, source, dest);
                }
                Err(e) => {
                    let cancelled = {
                        let map = tasks.lock().await;
                        map.get(&task_id)
                            .map(|t| t.status == TransferStatus::Cancelled)
                            .unwrap_or(false)
                    };
                    if cancelled {
                        // keep Cancelled status, just notify
                        let _ = app.emit(
                            "transfer:status",
                            StatusEvent {
                                task_id: task_id.clone(),
                                status: TransferStatus::Cancelled,
                            },
                        );
                        log::info!("Transfer {} cancelled", task_id);
                    } else {
                        set_status(&tasks, &app, &task_id, TransferStatus::Failed).await;
                        log::error!("Transfer {} failed: {}", task_id, e);
                    }
                }
            }
        });
    }

    /// Cancel a transfer task.
    pub async fn cancel(&self, task_id: &str) -> AppResult<()> {
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TransferStatus::Cancelled;
        }
        Ok(())
    }

    /// Get all tasks.
    pub async fn list_tasks(&self) -> Vec<TransferTask> {
        let tasks = self.tasks.lock().await;
        tasks.values().cloned().collect()
    }
}

type TaskMap = Arc<Mutex<HashMap<String, TransferTask>>>;

async fn set_status(
    tasks: &TaskMap,
    app: &tauri::AppHandle,
    task_id: &str,
    status: TransferStatus,
) {
    {
        let mut map = tasks.lock().await;
        if let Some(t) = map.get_mut(task_id) {
            t.status = status.clone();
        }
    }
    let _ = app.emit(
        "transfer:status",
        StatusEvent {
            task_id: task_id.to_string(),
            status,
        },
    );
}

async fn is_cancelled(tasks: &TaskMap, task_id: &str) -> bool {
    let map = tasks.lock().await;
    map.get(task_id)
        .map(|t| t.status == TransferStatus::Cancelled)
        .unwrap_or(true)
}

async fn record_progress(
    tasks: &TaskMap,
    app: &tauri::AppHandle,
    task_id: &str,
    transferred: u64,
    total: u64,
    started: Instant,
) {
    {
        let mut map = tasks.lock().await;
        if let Some(t) = map.get_mut(task_id) {
            t.transferred_bytes = transferred;
            t.total_bytes = total;
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let speed = if elapsed > 0.0 {
        transferred as f64 / elapsed
    } else {
        0.0
    };
    let _ = app.emit(
        "transfer:progress",
        TransferProgress {
            task_id: task_id.to_string(),
            transferred_bytes: transferred,
            total_bytes: total,
            speed,
        },
    );
}

async fn run_upload(
    tasks: &TaskMap,
    app: &tauri::AppHandle,
    task_id: &str,
    session: Arc<Mutex<SshSession>>,
    local_path: &str,
    remote_path: &str,
) -> AppResult<()> {
    let total = tokio::fs::metadata(local_path)
        .await
        .map_err(|e| AppError::TransferError(format!("Cannot stat local file: {}", e)))?
        .len();

    let mut local = tokio::fs::File::open(local_path)
        .await
        .map_err(|e| AppError::TransferError(format!("Cannot open local file: {}", e)))?;

    // Open remote file while briefly holding the session lock; the returned
    // handle operates over the SFTP channel independently afterwards.
    let mut remote = {
        let sess = session.lock().await;
        sess.sftp
            .sftp_session()
            .create(remote_path)
            .await
            .map_err(|e| AppError::TransferError(format!("Cannot create remote file: {}", e)))?
    };

    let started = Instant::now();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;

    record_progress(tasks, app, task_id, 0, total, started).await;

    loop {
        if is_cancelled(tasks, task_id).await {
            return Err(AppError::TransferError("Cancelled".into()));
        }
        let n = local
            .read(&mut buf)
            .await
            .map_err(|e| AppError::TransferError(format!("Local read error: {}", e)))?;
        if n == 0 {
            break;
        }
        remote
            .write_all(&buf[..n])
            .await
            .map_err(|e| AppError::TransferError(format!("Remote write error: {}", e)))?;
        transferred += n as u64;
        if transferred - last_emit >= PROGRESS_EMIT_THRESHOLD {
            last_emit = transferred;
            record_progress(tasks, app, task_id, transferred, total, started).await;
        }
    }

    remote
        .shutdown()
        .await
        .map_err(|e| AppError::TransferError(format!("Remote close error: {}", e)))?;

    record_progress(tasks, app, task_id, transferred, total, started).await;
    Ok(())
}

async fn run_download(
    tasks: &TaskMap,
    app: &tauri::AppHandle,
    task_id: &str,
    session: Arc<Mutex<SshSession>>,
    remote_path: &str,
    local_path: &str,
) -> AppResult<()> {
    let (total, mut remote) = {
        let sess = session.lock().await;
        let sftp = sess.sftp.sftp_session();
        let meta = sftp
            .metadata(remote_path)
            .await
            .map_err(|e| AppError::TransferError(format!("Cannot stat remote file: {}", e)))?;
        let total = meta.size.unwrap_or(0);
        let remote = sftp
            .open(remote_path)
            .await
            .map_err(|e| AppError::TransferError(format!("Cannot open remote file: {}", e)))?;
        (total, remote)
    };

    let mut local = tokio::fs::File::create(local_path)
        .await
        .map_err(|e| AppError::TransferError(format!("Cannot create local file: {}", e)))?;

    let started = Instant::now();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred: u64 = 0;
    let mut last_emit: u64 = 0;

    record_progress(tasks, app, task_id, 0, total, started).await;

    loop {
        if is_cancelled(tasks, task_id).await {
            return Err(AppError::TransferError("Cancelled".into()));
        }
        let n = remote
            .read(&mut buf)
            .await
            .map_err(|e| AppError::TransferError(format!("Remote read error: {}", e)))?;
        if n == 0 {
            break;
        }
        local
            .write_all(&buf[..n])
            .await
            .map_err(|e| AppError::TransferError(format!("Local write error: {}", e)))?;
        transferred += n as u64;
        if transferred - last_emit >= PROGRESS_EMIT_THRESHOLD {
            last_emit = transferred;
            record_progress(tasks, app, task_id, transferred, total, started).await;
        }
    }

    local
        .flush()
        .await
        .map_err(|e| AppError::TransferError(format!("Local flush error: {}", e)))?;

    record_progress(tasks, app, task_id, transferred, total, started).await;
    Ok(())
}
