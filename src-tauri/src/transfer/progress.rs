use serde::{Deserialize, Serialize};

/// Transfer progress event emitted to frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub task_id: String,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub speed: f64, // bytes per second
}

/// Transfer task status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Queued,
    Active,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

/// A single transfer task descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferTask {
    pub id: String,
    pub session_id: String,
    /// Remote host/username, used to rebind the task to a live session
    /// when resuming after a restart or reconnect.
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub username: String,
    /// Set when this task is part of a directory transfer (tree display).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// Name of the directory the group transfers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
    /// Path of this file relative to the directory root, '/'-separated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel_path: Option<String>,
    /// Queue time in epoch milliseconds, for stable display ordering.
    #[serde(default)]
    pub created_at: u64,
    /// When true, the partially downloaded local file is deleted once the
    /// task is cancelled. Runtime-only, not persisted.
    #[serde(skip)]
    pub delete_on_cancel: bool,
    pub direction: TransferDirection,
    pub source_path: String,
    pub dest_path: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub status: TransferStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferDirection {
    Upload,
    Download,
}

/// Group membership passed when queueing files of a directory transfer.
#[derive(Debug, Clone)]
pub struct TaskGroup {
    pub id: String,
    pub name: String,
    pub rel_path: String,
}
