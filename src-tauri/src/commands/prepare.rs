//! Cancellable "preparing" phase for long-running bulk operations
//! (directory transfer queueing, recursive deletes). The frontend shows
//! a blocking modal driven by `prepare:progress` events and can abort
//! through `cancel_prepare`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

use crate::error::{AppError, AppResult};

/// Registry of in-flight preparations, keyed by frontend-generated id.
/// Cancelled ids are remembered so an id shared by several sequential
/// commands (e.g. a multi-selection delete) stays cancelled.
#[derive(Default)]
pub struct PrepareRegistry {
    flags: StdMutex<HashMap<String, Arc<AtomicBool>>>,
    cancelled: StdMutex<HashSet<String>>,
}

impl PrepareRegistry {
    fn register(&self, id: &str) -> Arc<AtomicBool> {
        let already = self.cancelled.lock().unwrap().contains(id);
        let flag = Arc::new(AtomicBool::new(already));
        self.flags
            .lock()
            .unwrap()
            .insert(id.to_string(), flag.clone());
        flag
    }

    fn unregister(&self, id: &str) {
        self.flags.lock().unwrap().remove(id);
    }

    pub fn cancel(&self, id: &str) {
        {
            let mut cancelled = self.cancelled.lock().unwrap();
            cancelled.insert(id.to_string());
            // Ids are per user gesture: keep the memory bounded
            if cancelled.len() > 256 {
                cancelled.clear();
                cancelled.insert(id.to_string());
            }
        }
        if let Some(flag) = self.flags.lock().unwrap().get(id) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepareProgress<'a> {
    prepare_id: &'a str,
    /// "scanning" | "queueing" | "deleting"
    phase: &'a str,
    /// Files processed in the current phase.
    done: u64,
    /// Total files when known (0 while scanning).
    total: u64,
}

/// Error message used when a preparation is cancelled by the user.
/// The frontend matches on this string to swallow the error silently.
pub const CANCELLED_MSG: &str = "Preparation cancelled";

const EMIT_INTERVAL: Duration = Duration::from_millis(80);

/// Progress/cancellation context threaded through a bulk operation.
/// Emits throttled `prepare:progress` events. `None`-safe: every method
/// is a no-op when constructed without an id.
pub struct Prepare<'r> {
    inner: Option<PrepareInner<'r>>,
}

struct PrepareInner<'r> {
    id: String,
    app: tauri::AppHandle,
    registry: &'r PrepareRegistry,
    cancelled: Arc<AtomicBool>,
    done: AtomicU64,
    total: AtomicU64,
    phase: StdMutex<&'static str>,
    last_emit: StdMutex<Instant>,
}

impl<'r> Prepare<'r> {
    pub fn new(
        app: &tauri::AppHandle,
        registry: &'r PrepareRegistry,
        id: Option<String>,
    ) -> Self {
        let inner = id.map(|id| {
            let cancelled = registry.register(&id);
            PrepareInner {
                id,
                app: app.clone(),
                registry,
                cancelled,
                done: AtomicU64::new(0),
                total: AtomicU64::new(0),
                phase: StdMutex::new("scanning"),
                // Backdated so the first tick emits immediately
                last_emit: StdMutex::new(Instant::now() - EMIT_INTERVAL),
            }
        });
        Self { inner }
    }

    /// Err(TransferError("Preparation cancelled")) once cancel was requested.
    pub fn check(&self) -> AppResult<()> {
        if let Some(inner) = &self.inner {
            if inner.cancelled.load(Ordering::Relaxed) {
                return Err(AppError::TransferError(CANCELLED_MSG.into()));
            }
        }
        Ok(())
    }

    /// Shared cancellation flag, for handing to blocking threads.
    /// Always-false dummy when this preparation is untracked.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        match &self.inner {
            Some(inner) => inner.cancelled.clone(),
            None => Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set the absolute done count (from an external counter) and maybe
    /// emit a throttled progress event.
    pub fn set_done(&self, n: u64) {
        if let Some(inner) = &self.inner {
            inner.done.store(n, Ordering::Relaxed);
            inner.emit(false);
        }
    }

    /// Switch phase and reset the done counter. Sets total for counted
    /// phases (queueing/deleting with a known file count).
    pub fn set_phase(&self, phase: &'static str, total: u64) {
        if let Some(inner) = &self.inner {
            *inner.phase.lock().unwrap() = phase;
            inner.done.store(0, Ordering::Relaxed);
            inner.total.store(total, Ordering::Relaxed);
            inner.emit(true);
        }
    }

    /// Count one processed file and maybe emit a throttled progress event.
    pub fn tick(&self) {
        if let Some(inner) = &self.inner {
            inner.done.fetch_add(1, Ordering::Relaxed);
            inner.emit(false);
        }
    }

    /// tick() + check() in one call, for per-item loops.
    pub fn step(&self) -> AppResult<()> {
        self.tick();
        self.check()
    }
}

impl PrepareInner<'_> {
    fn emit(&self, force: bool) {
        {
            let mut last = self.last_emit.lock().unwrap();
            if !force && last.elapsed() < EMIT_INTERVAL {
                return;
            }
            *last = Instant::now();
        }
        let _ = self.app.emit(
            "prepare:progress",
            PrepareProgress {
                prepare_id: &self.id,
                phase: *self.phase.lock().unwrap(),
                done: self.done.load(Ordering::Relaxed),
                total: self.total.load(Ordering::Relaxed),
            },
        );
    }
}

impl Drop for Prepare<'_> {
    fn drop(&mut self) {
        if let Some(inner) = &self.inner {
            inner.registry.unregister(&inner.id);
        }
    }
}

/// Abort an in-flight preparation. The running command notices the flag
/// at its next per-item check and returns "Preparation cancelled".
#[tauri::command]
pub async fn cancel_prepare(
    prepare_id: String,
    registry: tauri::State<'_, PrepareRegistry>,
) -> AppResult<()> {
    registry.cancel(&prepare_id);
    Ok(())
}
