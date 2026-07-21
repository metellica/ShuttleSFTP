use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::ssh::auth::AuthMethod;
use crate::ssh::sftp::SftpClient;

/// Connection parameters from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
}

/// Represents an active SSH session with its SFTP channel.
pub struct SshSession {
    pub id: String,
    pub params: ConnectParams,
    pub sftp: SftpClient,
}

/// Manages all active SSH sessions (one per tab).
pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<SshSession>>>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Connect to a remote host and store the session.
    pub async fn connect(&self, params: ConnectParams) -> AppResult<String> {
        let session_id = uuid::Uuid::new_v4().to_string();

        let sftp = SftpClient::connect(&params).await?;

        let session = SshSession {
            id: session_id.clone(),
            params: params.clone(),
            sftp,
        };

        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_id.clone(), Arc::new(Mutex::new(session)));

        log::info!("Connected to {}@{}:{}", params.username, params.host, params.port);
        Ok(session_id)
    }

    /// Disconnect and remove a session.
    pub async fn disconnect(&self, session_id: &str) -> AppResult<()> {
        let mut sessions = self.sessions.lock().await;
        if sessions.remove(session_id).is_some() {
            log::info!("Disconnected session {}", session_id);
            Ok(())
        } else {
            Err(AppError::SessionNotFound(session_id.to_string()))
        }
    }

    /// Get a session by ID for operations.
    pub async fn get_session(&self, session_id: &str) -> AppResult<Arc<Mutex<SshSession>>> {
        let sessions = self.sessions.lock().await;
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| AppError::SessionNotFound(session_id.to_string()))
    }
}
