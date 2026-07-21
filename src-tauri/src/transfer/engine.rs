use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::AppResult;
use crate::transfer::progress::{TransferDirection, TransferStatus, TransferTask};

/// Manages the transfer queue and executes upload/download tasks.
pub struct TransferEngine {
    tasks: Arc<Mutex<HashMap<String, TransferTask>>>,
    #[allow(dead_code)]
    max_concurrent: usize,
}

impl TransferEngine {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            max_concurrent,
        }
    }

    /// Queue an upload task.
    pub async fn queue_upload(
        &self,
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

        let mut tasks = self.tasks.lock().await;
        tasks.insert(task_id.clone(), task);
        Ok(task_id)
    }

    /// Queue a download task.
    pub async fn queue_download(
        &self,
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

        let mut tasks = self.tasks.lock().await;
        tasks.insert(task_id.clone(), task);
        Ok(task_id)
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
