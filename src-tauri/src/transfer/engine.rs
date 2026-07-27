use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, Semaphore};

use crate::error::{AppError, AppResult};
use crate::fs::local::LocalFs;
use crate::fs::RemoteFs;
use crate::ssh::session::RemoteSession;
use crate::transfer::progress::{
    TaskGroup, TransferDirection, TransferProgress, TransferStatus, TransferTask,
};

const CHUNK_SIZE: usize = 64 * 1024;
const PROGRESS_EMIT_THRESHOLD: u64 = 256 * 1024;
const SPEED_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// Sliding-window transfer rate estimator: reports the rate over the
/// last few seconds instead of a whole-transfer average, so the shown
/// speed tracks the current throughput.
struct SpeedMeter {
    samples: std::collections::VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    fn new(initial_bytes: u64) -> Self {
        let mut samples = std::collections::VecDeque::new();
        samples.push_back((Instant::now(), initial_bytes));
        Self { samples }
    }

    /// Record the current absolute byte count; returns bytes/sec over
    /// the recent window.
    fn update(&mut self, bytes: u64) -> f64 {
        let now = Instant::now();
        self.samples.push_back((now, bytes));
        while self.samples.len() > 2 && now.duration_since(self.samples[0].0) > SPEED_WINDOW {
            self.samples.pop_front();
        }
        let (t0, b0) = self.samples[0];
        let dt = now.duration_since(t0).as_secs_f64();
        if dt > 0.0 {
            bytes.saturating_sub(b0) as f64 / dt
        } else {
            0.0
        }
    }
}

/// One side of a transfer: the local machine or a live session.
#[derive(Clone)]
pub enum Endpoint {
    Local,
    Session {
        id: String,
        session: Arc<Mutex<RemoteSession>>,
    },
}

/// One file of a batch queue operation (shared endpoints).
pub struct BatchItem {
    pub src_path: String,
    pub dst_path: String,
    pub group: Option<TaskGroup>,
    pub total_bytes: u64,
}

impl Endpoint {
    async fn fs(&self) -> Arc<dyn RemoteFs> {
        match self {
            Endpoint::Local => Arc::new(LocalFs),
            Endpoint::Session { session, .. } => session.lock().await.fs.clone(),
        }
    }

    fn session_id(&self) -> Option<String> {
        match self {
            Endpoint::Local => None,
            Endpoint::Session { id, .. } => Some(id.clone()),
        }
    }

    async fn host_user(&self) -> (String, String) {
        match self {
            Endpoint::Local => ("local".into(), String::new()),
            Endpoint::Session { session, .. } => {
                let s = session.lock().await;
                (s.params.host.clone(), s.params.username.clone())
            }
        }
    }
}

/// How a transfer run ended (other than by error).
enum Outcome {
    Completed,
    Paused,
    Cancelled,
}

/// Status change event emitted to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    task_id: String,
    status: TransferStatus,
}

fn transfers_path() -> PathBuf {
    crate::config::app_config_dir().join("transfers.json")
}

/// Load persisted tasks; interrupted (queued/active) tasks come back as paused.
fn load_persisted_tasks() -> HashMap<String, TransferTask> {
    let path = transfers_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    let list: Vec<TransferTask> = match serde_json::from_str(&content) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("Ignoring invalid transfers.json: {}", e);
            return HashMap::new();
        }
    };
    list.into_iter()
        .map(|mut t| {
            if matches!(t.status, TransferStatus::Queued | TransferStatus::Active) {
                t.status = TransferStatus::Paused;
            }
            (t.id.clone(), t)
        })
        .collect()
}

/// Serialize and write the whole task list off the async runtime.
/// Compact JSON: a large queue would take tens of MB pretty-printed.
async fn persist_tasks(tasks: &TaskMap) {
    let list: Vec<TransferTask> = {
        let map = tasks.lock().await;
        map.values().cloned().collect()
    };
    let write = tokio::task::spawn_blocking(move || {
        let json = serde_json::to_string(&list).map_err(|e| e.to_string())?;
        std::fs::write(transfers_path(), json).map_err(|e| e.to_string())
    })
    .await;
    match write {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::warn!("Cannot persist transfers: {}", e),
        Err(e) => log::warn!("Persist task panicked: {}", e),
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Set while a debounced persist of the task list is scheduled. Global:
/// per-task status flips during a large transfer must coalesce, or the
/// full-queue rewrite runs once per completed file.
static PERSIST_PENDING: AtomicBool = AtomicBool::new(false);

/// Persist soon, coalescing bursts (queueing, per-file completions,
/// bulk cancels): at most one full rewrite per 500ms.
fn schedule_persist(tasks: &TaskMap) {
    if PERSIST_PENDING.swap(true, Ordering::SeqCst) {
        return; // a persist is already scheduled
    }
    let tasks = tasks.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        PERSIST_PENDING.store(false, Ordering::SeqCst);
        persist_tasks(&tasks).await;
    });
}

/// Manages the transfer queue and executes copy tasks between any two
/// endpoints (local machine, SSH hosts, containers, pods).
pub struct TransferEngine {
    tasks: Arc<Mutex<HashMap<String, TransferTask>>>,
    semaphore: Arc<Semaphore>,
    /// Task ids whose transfer loop is currently running (file handles open).
    running: RunningSet,
    /// group_id -> local root dir to delete once no group member is running.
    pending_group_deletes: PendingDeletes,
}

impl TransferEngine {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(load_persisted_tasks())),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            running: Arc::new(Mutex::new(HashSet::new())),
            pending_group_deletes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Queue a copy between two endpoints and start it in the background.
    /// `total_bytes` is the source size when already known (e.g. from a
    /// directory listing), so queued tasks show correct totals before
    /// they start; 0 means unknown, filled in by stat at transfer start.
    pub async fn queue_transfer(
        &self,
        app: tauri::AppHandle,
        src: Endpoint,
        src_path: &str,
        dst: Endpoint,
        dst_path: &str,
        group: Option<TaskGroup>,
        total_bytes: u64,
    ) -> AppResult<String> {
        let task_id = uuid::Uuid::new_v4().to_string();

        let direction = match (&src, &dst) {
            (Endpoint::Local, _) => TransferDirection::Upload,
            (_, Endpoint::Local) => TransferDirection::Download,
            _ => TransferDirection::Remote,
        };
        // The "primary" session (kept in session_id for rebinding) is the
        // remote side of up/downloads, the source for remote-remote copies.
        let primary = match direction {
            TransferDirection::Upload => &dst,
            _ => &src,
        };
        let (host, username) = primary.host_user().await;
        let (dest_host, dest_username) = dst.host_user().await;

        let task = TransferTask {
            id: task_id.clone(),
            session_id: primary.session_id().unwrap_or_default(),
            dest_session_id: dst.session_id(),
            host,
            username,
            dest_host,
            dest_username,
            group_id: group.as_ref().map(|g| g.id.clone()),
            group_name: group.as_ref().map(|g| g.name.clone()),
            rel_path: group.map(|g| g.rel_path),
            created_at: now_millis(),
            delete_on_cancel: false,
            direction,
            source_path: src_path.to_string(),
            dest_path: dst_path.to_string(),
            total_bytes,
            transferred_bytes: 0,
            status: TransferStatus::Queued,
        };
        self.tasks.lock().await.insert(task_id.clone(), task.clone());
        schedule_persist(&self.tasks);
        // Tell the frontend right away so the queue UI shows the task even
        // while a long multi-file queueing call is still in progress.
        let _ = app.emit("transfer:queued", &task);
        self.spawn_transfer(app, src, dst, task_id.clone());
        Ok(task_id)
    }

    /// Queue many copies between the same two endpoints in one shot:
    /// one host/user lookup, one map lock, one frontend event. Use for
    /// directory transfers, where per-file queue_transfer's IPC event
    /// (and the frontend re-render it triggers) dominates queueing time.
    pub async fn queue_transfer_batch(
        &self,
        app: tauri::AppHandle,
        src: Endpoint,
        dst: Endpoint,
        items: Vec<BatchItem>,
    ) -> AppResult<Vec<String>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let direction = match (&src, &dst) {
            (Endpoint::Local, _) => TransferDirection::Upload,
            (_, Endpoint::Local) => TransferDirection::Download,
            _ => TransferDirection::Remote,
        };
        let primary = match direction {
            TransferDirection::Upload => &dst,
            _ => &src,
        };
        let (host, username) = primary.host_user().await;
        let (dest_host, dest_username) = dst.host_user().await;
        let session_id = primary.session_id().unwrap_or_default();
        let dest_session_id = dst.session_id();
        let created_at = now_millis();

        let few = items.len() <= 20;
        let mut task_ids = Vec::with_capacity(items.len());
        let mut tasks_out = Vec::with_capacity(items.len());
        for item in items {
            let task_id = uuid::Uuid::new_v4().to_string();
            let task = TransferTask {
                id: task_id.clone(),
                session_id: session_id.clone(),
                dest_session_id: dest_session_id.clone(),
                host: host.clone(),
                username: username.clone(),
                dest_host: dest_host.clone(),
                dest_username: dest_username.clone(),
                group_id: item.group.as_ref().map(|g| g.id.clone()),
                group_name: item.group.as_ref().map(|g| g.name.clone()),
                rel_path: item.group.map(|g| g.rel_path),
                created_at,
                delete_on_cancel: false,
                direction: direction.clone(),
                source_path: item.src_path,
                dest_path: item.dst_path,
                total_bytes: item.total_bytes,
                transferred_bytes: 0,
                status: TransferStatus::Queued,
            };
            task_ids.push(task_id);
            tasks_out.push(task);
        }
        {
            let mut map = self.tasks.lock().await;
            for task in &tasks_out {
                map.insert(task.id.clone(), task.clone());
            }
        }
        schedule_persist(&self.tasks);
        if few {
            for task in &tasks_out {
                let _ = app.emit("transfer:queued", task);
            }
        } else {
            // One bulk event: the frontend re-syncs the whole list once
            let _ = app.emit("transfer:bulk-update", ());
        }
        for task in tasks_out {
            self.spawn_transfer(app.clone(), src.clone(), dst.clone(), task.id);
        }
        Ok(task_ids)
    }

    fn spawn_transfer(&self, app: tauri::AppHandle, src: Endpoint, dst: Endpoint, task_id: String) {
        let tasks = self.tasks.clone();
        let semaphore = self.semaphore.clone();
        let running = self.running.clone();
        let pending_deletes = self.pending_group_deletes.clone();

        tokio::spawn(async move {
            let _permit = match semaphore.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return,
            };

            let (source, dest, group_id) = {
                let map = tasks.lock().await;
                match map.get(&task_id) {
                    Some(t) if t.status == TransferStatus::Queued => (
                        t.source_path.clone(),
                        t.dest_path.clone(),
                        t.group_id.clone(),
                    ),
                    _ => return, // cancelled before start, or missing
                }
            };

            set_status(&tasks, &app, &task_id, TransferStatus::Active).await;
            running.lock().await.insert(task_id.clone());

            let result = run_copy(&tasks, &app, &task_id, &src, &source, &dst, &dest).await;

            // The transfer loop has returned: its file handles are dropped.
            running.lock().await.remove(&task_id);

            match result {
                Ok(Outcome::Completed) => {
                    set_status(&tasks, &app, &task_id, TransferStatus::Completed).await;
                    log::info!("Transfer {} completed: {} -> {}", task_id, source, dest);
                }
                Ok(Outcome::Paused) => {
                    // Re-read the status: a cancel may have landed while the
                    // loop was exiting from the pause.
                    let status = {
                        let map = tasks.lock().await;
                        map.get(&task_id).map(|t| t.status.clone())
                    };
                    if status == Some(TransferStatus::Cancelled) {
                        let _ = app.emit(
                            "transfer:status",
                            StatusEvent {
                                task_id: task_id.clone(),
                                status: TransferStatus::Cancelled,
                            },
                        );
                        cleanup_cancelled(&tasks, &task_id).await;
                        schedule_persist(&tasks);
                        log::info!("Transfer {} cancelled", task_id);
                    } else {
                        // Re-emit now that the byte position is final, and
                        // persist it for restart-resume.
                        let _ = app.emit(
                            "transfer:status",
                            StatusEvent {
                                task_id: task_id.clone(),
                                status: TransferStatus::Paused,
                            },
                        );
                        schedule_persist(&tasks);
                        log::info!("Transfer {} paused", task_id);
                    }
                }
                Ok(Outcome::Cancelled) => {
                    let _ = app.emit(
                        "transfer:status",
                        StatusEvent {
                            task_id: task_id.clone(),
                            status: TransferStatus::Cancelled,
                        },
                    );
                    cleanup_cancelled(&tasks, &task_id).await;
                    schedule_persist(&tasks);
                    log::info!("Transfer {} cancelled", task_id);
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
                        cleanup_cancelled(&tasks, &task_id).await;
                        schedule_persist(&tasks);
                        log::info!("Transfer {} cancelled", task_id);
                    } else {
                        set_status(&tasks, &app, &task_id, TransferStatus::Failed).await;
                        log::error!("Transfer {} failed: {}", task_id, e);
                    }
                }
            }

            // If a group deletion is pending and this was its last running
            // member, the local root dir can now be removed safely.
            maybe_delete_group(&tasks, &running, &pending_deletes, group_id.as_deref()).await;
        });
    }

    /// Pause a queued or active transfer task.
    pub async fn pause(&self, app: &tauri::AppHandle, task_id: &str) -> AppResult<()> {
        let changed = {
            let mut tasks = self.tasks.lock().await;
            match tasks.get_mut(task_id) {
                Some(t) if matches!(t.status, TransferStatus::Queued | TransferStatus::Active) => {
                    t.status = TransferStatus::Paused;
                    true
                }
                _ => false,
            }
        };
        if changed {
            let _ = app.emit(
                "transfer:status",
                StatusEvent {
                    task_id: task_id.to_string(),
                    status: TransferStatus::Paused,
                },
            );
            schedule_persist(&self.tasks);
        }
        Ok(())
    }

    /// Pause all queued/active transfers. Returns the ids that were paused.
    pub async fn pause_all(&self, app: &tauri::AppHandle) -> AppResult<Vec<String>> {
        let paused: Vec<String> = {
            let mut tasks = self.tasks.lock().await;
            tasks
                .values_mut()
                .filter(|t| matches!(t.status, TransferStatus::Queued | TransferStatus::Active))
                .map(|t| {
                    t.status = TransferStatus::Paused;
                    t.id.clone()
                })
                .collect()
        };
        if paused.len() > 20 {
            // Large backlogs: one bulk event instead of flooding the UI
            let _ = app.emit("transfer:bulk-update", ());
        } else {
            for id in &paused {
                let _ = app.emit(
                    "transfer:status",
                    StatusEvent {
                        task_id: id.clone(),
                        status: TransferStatus::Paused,
                    },
                );
            }
        }
        if !paused.is_empty() {
            schedule_persist(&self.tasks);
        }
        Ok(paused)
    }

    /// Pause every queued/active task of a directory transfer group.
    pub async fn pause_group(
        &self,
        app: &tauri::AppHandle,
        group_id: &str,
    ) -> AppResult<Vec<String>> {
        let paused: Vec<String> = {
            let mut tasks = self.tasks.lock().await;
            tasks
                .values_mut()
                .filter(|t| {
                    t.group_id.as_deref() == Some(group_id)
                        && matches!(t.status, TransferStatus::Queued | TransferStatus::Active)
                })
                .map(|t| {
                    t.status = TransferStatus::Paused;
                    t.id.clone()
                })
                .collect()
        };
        if paused.len() > 20 {
            // Large groups: one bulk event instead of flooding the UI
            let _ = app.emit("transfer:bulk-update", ());
        } else {
            for id in &paused {
                let _ = app.emit(
                    "transfer:status",
                    StatusEvent {
                        task_id: id.clone(),
                        status: TransferStatus::Paused,
                    },
                );
            }
        }
        if !paused.is_empty() {
            schedule_persist(&self.tasks);
        }
        Ok(paused)
    }

    /// Resume a paused (or failed) transfer on the given endpoints.
    /// The transfer picks up from the destination file's current size
    /// when both endpoints support it. With `quiet`, no per-task status
    /// event is emitted (bulk callers send one bulk event instead).
    pub async fn resume(
        &self,
        app: tauri::AppHandle,
        task_id: &str,
        src: Endpoint,
        dst: Endpoint,
        quiet: bool,
    ) -> AppResult<()> {
        {
            let mut tasks = self.tasks.lock().await;
            let task = tasks
                .get_mut(task_id)
                .ok_or_else(|| AppError::TransferError(format!("Task not found: {}", task_id)))?;
            if !matches!(task.status, TransferStatus::Paused | TransferStatus::Failed) {
                return Err(AppError::TransferError(
                    "Only paused or failed transfers can be resumed".into(),
                ));
            }
            task.status = TransferStatus::Queued;
            let primary = match task.direction {
                TransferDirection::Upload => &dst,
                _ => &src,
            };
            task.session_id = primary.session_id().unwrap_or_default();
            task.dest_session_id = dst.session_id();
        }
        if !quiet {
            let _ = app.emit(
                "transfer:status",
                StatusEvent {
                    task_id: task_id.to_string(),
                    status: TransferStatus::Queued,
                },
            );
        }
        // Debounced: resume-all over a large backlog would otherwise
        // rewrite the whole task file once per task.
        schedule_persist(&self.tasks);
        self.spawn_transfer(app, src, dst, task_id.to_string());
        Ok(())
    }

    /// Cancel a transfer task. For downloads, `delete_local` removes the
    /// partially downloaded local file.
    pub async fn cancel(
        &self,
        app: &tauri::AppHandle,
        task_id: &str,
        delete_local: bool,
    ) -> AppResult<()> {
        let (changed, was_active) = {
            let mut tasks = self.tasks.lock().await;
            match tasks.get_mut(task_id) {
                Some(t) if matches!(
                    t.status,
                    TransferStatus::Queued | TransferStatus::Active | TransferStatus::Paused
                ) =>
                {
                    let was_active = t.status == TransferStatus::Active;
                    t.status = TransferStatus::Cancelled;
                    t.delete_on_cancel =
                        delete_local && matches!(t.direction, TransferDirection::Download);
                    (true, was_active)
                }
                _ => (false, false),
            }
        };
        if changed {
            let _ = app.emit(
                "transfer:status",
                StatusEvent {
                    task_id: task_id.to_string(),
                    status: TransferStatus::Cancelled,
                },
            );
            // Active tasks clean up in their transfer loop once the file
            // handle is released; others have no loop, so do it here.
            if !was_active {
                cleanup_cancelled(&self.tasks, task_id).await;
            }
            schedule_persist(&self.tasks);
        }
        Ok(())
    }

    /// Cancel all queued/active/paused transfers. For downloads,
    /// `delete_local` removes partially downloaded local files; directory
    /// transfer groups have their whole local root dir removed instead.
    pub async fn cancel_all(&self, app: &tauri::AppHandle, delete_local: bool) -> AppResult<()> {
        let mut group_roots: HashMap<String, String> = HashMap::new();
        let cancelled: Vec<(String, bool, bool)> = {
            let mut tasks = self.tasks.lock().await;
            tasks
                .values_mut()
                .filter(|t| {
                    matches!(
                        t.status,
                        TransferStatus::Queued | TransferStatus::Active | TransferStatus::Paused
                    )
                })
                .map(|t| {
                    let was_active = t.status == TransferStatus::Active;
                    t.status = TransferStatus::Cancelled;
                    let is_download = matches!(t.direction, TransferDirection::Download);
                    if delete_local && is_download {
                        if let Some(gid) = &t.group_id {
                            // Whole group root is deleted once handles close
                            t.delete_on_cancel = false;
                            group_roots.entry(gid.clone()).or_insert_with(|| {
                                group_root(&t.dest_path, t.rel_path.as_deref().unwrap_or(""))
                            });
                        } else {
                            t.delete_on_cancel = true;
                        }
                    } else {
                        t.delete_on_cancel = false;
                    }
                    (t.id.clone(), was_active, t.delete_on_cancel)
                })
                .collect()
        };
        // Per-task events make the frontend re-render the queue thousands
        // of times for large backlogs: send one bulk event instead.
        if cancelled.len() > 20 {
            let _ = app.emit("transfer:bulk-update", ());
        } else {
            for (id, _, _) in &cancelled {
                let _ = app.emit(
                    "transfer:status",
                    StatusEvent {
                        task_id: id.clone(),
                        status: TransferStatus::Cancelled,
                    },
                );
            }
        }
        for (id, was_active, delete) in &cancelled {
            if !was_active && *delete {
                cleanup_cancelled(&self.tasks, id).await;
            }
        }
        if !group_roots.is_empty() {
            let gids: Vec<String> = group_roots.keys().cloned().collect();
            self.pending_group_deletes.lock().await.extend(group_roots);
            for gid in gids {
                maybe_delete_group(
                    &self.tasks,
                    &self.running,
                    &self.pending_group_deletes,
                    Some(&gid),
                )
                .await;
            }
        }
        if !cancelled.is_empty() {
            schedule_persist(&self.tasks);
        }
        Ok(())
    }

    /// Cancel every cancellable task of a directory transfer group.
    /// For download groups, `delete_local` removes the whole local root
    /// directory once every member's file handle is closed.
    pub async fn cancel_group(
        &self,
        app: &tauri::AppHandle,
        group_id: &str,
        delete_local: bool,
    ) -> AppResult<()> {
        let mut cancelled_ids = Vec::new();
        let mut root: Option<String> = None;
        {
            let mut tasks = self.tasks.lock().await;
            for t in tasks
                .values_mut()
                .filter(|t| t.group_id.as_deref() == Some(group_id))
            {
                if matches!(t.direction, TransferDirection::Download) && root.is_none() {
                    root = Some(group_root(&t.dest_path, t.rel_path.as_deref().unwrap_or("")));
                }
                if matches!(
                    t.status,
                    TransferStatus::Queued | TransferStatus::Active | TransferStatus::Paused
                ) {
                    t.status = TransferStatus::Cancelled;
                    t.delete_on_cancel = false;
                    cancelled_ids.push(t.id.clone());
                }
            }
        }
        if cancelled_ids.len() > 20 {
            // Large groups: one bulk event instead of flooding the UI
            let _ = app.emit("transfer:bulk-update", ());
        } else {
            for id in &cancelled_ids {
                let _ = app.emit(
                    "transfer:status",
                    StatusEvent {
                        task_id: id.clone(),
                        status: TransferStatus::Cancelled,
                    },
                );
            }
        }
        if delete_local {
            if let Some(root) = root {
                self.pending_group_deletes
                    .lock()
                    .await
                    .insert(group_id.to_string(), root);
                maybe_delete_group(
                    &self.tasks,
                    &self.running,
                    &self.pending_group_deletes,
                    Some(group_id),
                )
                .await;
            }
        }
        if !cancelled_ids.is_empty() {
            schedule_persist(&self.tasks);
        }
        Ok(())
    }

    /// Get a single task by id.
    pub async fn get_task(&self, task_id: &str) -> Option<TransferTask> {
        let tasks = self.tasks.lock().await;
        tasks.get(task_id).cloned()
    }

    /// Remove completed and cancelled tasks from the queue.
    pub async fn clear_finished(&self) -> AppResult<()> {
        {
            let mut tasks = self.tasks.lock().await;
            tasks.retain(|_, t| {
                !matches!(
                    t.status,
                    TransferStatus::Completed | TransferStatus::Cancelled
                )
            });
        }
        schedule_persist(&self.tasks);
        Ok(())
    }

    /// Get all tasks.
    pub async fn list_tasks(&self) -> Vec<TransferTask> {
        let tasks = self.tasks.lock().await;
        tasks.values().cloned().collect()
    }
}

type TaskMap = Arc<Mutex<HashMap<String, TransferTask>>>;
type RunningSet = Arc<Mutex<HashSet<String>>>;
type PendingDeletes = Arc<Mutex<HashMap<String, String>>>;

/// Local root directory of a directory download, derived from a member
/// task's destination path by stripping its relative path components.
fn group_root(dest_path: &str, rel_path: &str) -> String {
    let mut p = PathBuf::from(dest_path);
    for _ in rel_path.split('/').filter(|s| !s.is_empty()) {
        p.pop();
    }
    p.to_string_lossy().to_string()
}

/// Delete a cancelled download group's local root dir, but only when no
/// group member's transfer loop is still running (open file handles).
async fn maybe_delete_group(
    tasks: &TaskMap,
    running: &RunningSet,
    pending: &PendingDeletes,
    group_id: Option<&str>,
) {
    let Some(gid) = group_id else { return };
    {
        let p = pending.lock().await;
        if !p.contains_key(gid) {
            return;
        }
    }
    let any_running = {
        let map = tasks.lock().await;
        let run = running.lock().await;
        map.values()
            .any(|t| t.group_id.as_deref() == Some(gid) && run.contains(&t.id))
    };
    if any_running {
        return;
    }
    let Some(root) = pending.lock().await.remove(gid) else {
        return;
    };
    tokio::spawn(async move {
        // Dropped tokio file handles are closed on a blocking thread, so the
        // OS may still hold them for a moment: retry briefly.
        for attempt in 0..5 {
            match tokio::fs::remove_dir_all(&root).await {
                Ok(()) => {
                    log::info!("Deleted cancelled download folder {}", root);
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => {
                    if attempt == 4 {
                        log::warn!("Cannot delete download folder {}: {}", root, e);
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
            }
        }
    });
}

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
    schedule_persist(tasks);
}

/// Check whether the running transfer should stop (paused or cancelled).
async fn check_control(tasks: &TaskMap, task_id: &str) -> Option<Outcome> {
    let map = tasks.lock().await;
    match map.get(task_id).map(|t| t.status.clone()) {
        None | Some(TransferStatus::Cancelled) => Some(Outcome::Cancelled),
        Some(TransferStatus::Paused) => Some(Outcome::Paused),
        _ => None,
    }
}

/// Delete the partial local file of a cancelled download when requested.
async fn cleanup_cancelled(tasks: &TaskMap, task_id: &str) {
    let target = {
        let map = tasks.lock().await;
        map.get(task_id).and_then(|t| {
            if t.delete_on_cancel && matches!(t.direction, TransferDirection::Download) {
                Some(t.dest_path.clone())
            } else {
                None
            }
        })
    };
    if let Some(path) = target {
        match tokio::fs::remove_file(&path).await {
            Ok(()) => log::info!("Deleted partial download {}", path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("Cannot delete partial download {}: {}", path, e),
        }
    }
}

async fn record_progress(
    tasks: &TaskMap,
    app: &tauri::AppHandle,
    task_id: &str,
    transferred: u64,
    total: u64,
    meter: &mut SpeedMeter,
) {
    {
        let mut map = tasks.lock().await;
        if let Some(t) = map.get_mut(task_id) {
            t.transferred_bytes = transferred;
            t.total_bytes = total;
        }
    }
    let speed = meter.update(transferred);
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

/// Server-side copy command when both endpoints live on the same host
/// machine (same SSH connection, or both local): the data never travels
/// through this app's relay loop. Returns None when no shortcut applies.
async fn server_side_copy_script(
    src: &Endpoint,
    src_path: &str,
    dst: &Endpoint,
    dst_path: &str,
) -> Option<(Arc<dyn crate::exec::CommandRunner>, String)> {
    let (Endpoint::Session { session: s, .. }, Endpoint::Session { session: d, .. }) = (src, dst)
    else {
        return None;
    };
    // Lock one session at a time (never both) to avoid deadlocks between
    // concurrent transfers locking in opposite order.
    let (src_ssh, runner, read_cmd) = {
        let s = s.lock().await;
        let read_cmd = s.fs.server_read_cmd(src_path)?;
        (s.ssh.clone(), s.runner.clone(), read_cmd)
    };
    let write_cmd = {
        let d = d.lock().await;
        // Same machine = same SSH connection, or both sessions local.
        let same_machine = match (&src_ssh, &d.ssh) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same_machine {
            return None;
        }
        d.fs.server_write_cmd(dst_path)?
    };
    Some((runner, format!("{} | {}", read_cmd, write_cmd)))
}

/// Generic streaming copy between two endpoints, with an opportunistic
/// same-host server-side fast path.
async fn run_copy(
    tasks: &TaskMap,
    app: &tauri::AppHandle,
    task_id: &str,
    src: &Endpoint,
    src_path: &str,
    dst: &Endpoint,
    dst_path: &str,
) -> AppResult<Outcome> {
    let src_fs = src.fs().await;
    let dst_fs = dst.fs().await;

    let total = src_fs.stat(src_path).await?.size;

    // A non-zero transferred count means this task is being resumed.
    let resume_hint = {
        let map = tasks.lock().await;
        map.get(task_id).map(|t| t.transferred_bytes).unwrap_or(0)
    };
    let mut offset = 0u64;
    if resume_hint > 0
        && src_fs.supports_resume_at(src_path)
        && dst_fs.supports_resume_at(dst_path)
    {
        if let Ok(meta) = dst_fs.stat(dst_path).await {
            offset = meta.size.min(total);
        }
    }

    // Same-host fast path: run the copy remotely, no local relay. Only
    // for fresh transfers — it cannot resume. Progress is tracked by
    // polling the destination file size.
    if offset == 0 {
        if let Some((runner, script)) = server_side_copy_script(src, src_path, dst, dst_path).await
        {
            if let Some(outcome) = check_control(tasks, task_id).await {
                return Ok(outcome);
            }
            let mut meter = SpeedMeter::new(0);
            record_progress(tasks, app, task_id, 0, total, &mut meter).await;
            log::info!("Transfer {} using server-side copy: {}", task_id, script);
            let argv = vec!["sh".to_string(), "-c".into(), script];
            // Spawn instead of run-to-completion so pause/cancel can
            // interrupt: dropping the stream closes the channel and
            // kills the remote pipeline.
            let mut stream = runner.spawn(&argv).await?;
            let mut ticks: u32 = 0;
            let done = loop {
                tokio::select! {
                    done = &mut stream.done => {
                        break done.map_err(|_| {
                            AppError::TransferError("Server-side copy aborted".into())
                        })?;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {
                        if let Some(outcome) = check_control(tasks, task_id).await {
                            drop(stream);
                            return Ok(outcome);
                        }
                        // Poll the destination size (every other tick) so
                        // the frontend sees live progress and speed.
                        ticks += 1;
                        if ticks % 2 == 0 {
                            if let Ok(meta) = dst_fs.stat(dst_path).await {
                                let written = meta.size.min(total);
                                record_progress(tasks, app, task_id, written, total, &mut meter)
                                    .await;
                            }
                        }
                    }
                }
            };
            if done.exit.unwrap_or(0) != 0 {
                return Err(AppError::TransferError(format!(
                    "Server-side copy failed (exit {}): {}",
                    done.exit.unwrap_or(0),
                    done.stderr.trim()
                )));
            }
            // Verify the destination size matches the source.
            let written = dst_fs.stat(dst_path).await?.size;
            if written != total {
                return Err(AppError::TransferError(format!(
                    "Server-side copy size mismatch: {} of {} bytes",
                    written, total
                )));
            }
            record_progress(tasks, app, task_id, total, total, &mut meter).await;
            return Ok(Outcome::Completed);
        }
    }

    let reader = src_fs.open_read(src_path, offset).await?;
    let mut remote_reader = reader.reader;
    let mut writer = dst_fs.open_write(dst_path, offset).await?;

    let mut meter = SpeedMeter::new(offset);
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut transferred: u64 = offset;
    let mut last_emit: u64 = offset;

    record_progress(tasks, app, task_id, transferred, total, &mut meter).await;

    loop {
        if let Some(outcome) = check_control(tasks, task_id).await {
            let _ = writer.flush().await;
            record_progress(tasks, app, task_id, transferred, total, &mut meter).await;
            return Ok(outcome);
        }
        let n = remote_reader
            .read(&mut buf)
            .await
            .map_err(|e| AppError::TransferError(format!("Read error: {}", e)))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .await
            .map_err(|e| AppError::TransferError(format!("Write error: {}", e)))?;
        transferred += n as u64;
        if transferred - last_emit >= PROGRESS_EMIT_THRESHOLD {
            last_emit = transferred;
            record_progress(tasks, app, task_id, transferred, total, &mut meter).await;
        }
    }

    writer.finish().await?;

    // Streams without an out-of-band error channel (exec-based backends)
    // surface failures as truncated data: verify the byte count.
    if transferred < total {
        return Err(AppError::TransferError(format!(
            "Source stream ended early: {} of {} bytes",
            transferred, total
        )));
    }

    record_progress(tasks, app, task_id, transferred, total, &mut meter).await;
    Ok(Outcome::Completed)
}
