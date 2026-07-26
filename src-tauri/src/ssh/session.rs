use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::exec::{CommandRunner, LocalRunner, SshRunner};
use crate::fs::host::HostFs;
use crate::fs::local::LocalFs;
use crate::fs::RemoteFs;
use crate::ssh::auth::AuthMethod;
use crate::ssh::client::SshHandle;
use crate::ssh::sftp::SftpClient;

/// Connection parameters from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
}

/// What kind of endpoint a session browses. Containers and pods are
/// reached through the host's virtual /@containers and /@pods dirs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Ssh,
    Local,
}

/// An active session: an SSH host or this machine, with virtual
/// container/pod directories layered on top.
pub struct RemoteSession {
    pub id: String,
    pub kind: SessionKind,
    /// SSH leg parameters (host "local" for local sessions).
    pub params: ConnectParams,
    /// File system endpoint used by all browse/transfer operations.
    pub fs: Arc<dyn RemoteFs>,
    /// SSH connection of the host leg, when there is one.
    pub ssh: Option<Arc<SshHandle>>,
    /// Command runner on the host machine (local or over SSH).
    pub runner: Arc<dyn CommandRunner>,
}

fn local_params() -> ConnectParams {
    ConnectParams {
        host: "local".into(),
        port: 0,
        username: String::new(),
        auth: AuthMethod::Agent,
    }
}

/// Manages all active sessions (one per tab).
pub struct SessionManager {
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<RemoteSession>>>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn insert(&self, session: RemoteSession) -> String {
        let id = session.id.clone();
        let mut sessions = self.sessions.lock().await;
        sessions.insert(id.clone(), Arc::new(Mutex::new(session)));
        id
    }

    /// Connect to a remote host over SSH/SFTP and store the session.
    pub async fn connect(&self, params: ConnectParams) -> AppResult<String> {
        let session_id = uuid::Uuid::new_v4().to_string();

        let sftp = SftpClient::connect(&params).await?;
        let ssh = sftp.ssh_handle();
        let runner: Arc<dyn CommandRunner> = Arc::new(SshRunner::new(
            ssh.clone(),
            format!("{}@{}", params.username, params.host),
        ));
        let fs = HostFs::new(Arc::new(sftp), runner.clone(), Some(ssh.clone()));

        log::info!("Connected to {}@{}:{}", params.username, params.host, params.port);
        self.insert(RemoteSession {
            id: session_id.clone(),
            kind: SessionKind::Ssh,
            params,
            fs: Arc::new(fs),
            ssh: Some(ssh),
            runner,
        })
        .await;
        Ok(session_id)
    }

    /// Open a session on this machine (local fs + local container engines).
    pub async fn connect_local(&self) -> AppResult<String> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let runner: Arc<dyn CommandRunner> = Arc::new(LocalRunner);
        let fs = HostFs::new(Arc::new(LocalFs), runner.clone(), None);
        log::info!("Opened local session");
        self.insert(RemoteSession {
            id: session_id.clone(),
            kind: SessionKind::Local,
            params: local_params(),
            fs: Arc::new(fs),
            ssh: None,
            runner,
        })
        .await;
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
    pub async fn get_session(&self, session_id: &str) -> AppResult<Arc<Mutex<RemoteSession>>> {
        let sessions = self.sessions.lock().await;
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| AppError::SessionNotFound(session_id.to_string()))
    }

    /// Find any live session connected to the given host as the given
    /// user. An empty username matches any user on that host. Used to
    /// rebind persisted transfers after a restart.
    pub async fn find_session_for(
        &self,
        host: &str,
        username: &str,
    ) -> Option<(String, Arc<Mutex<RemoteSession>>)> {
        let sessions = self.sessions.lock().await;
        for (id, session) in sessions.iter() {
            let params = { session.lock().await.params.clone() };
            if params.host == host && (username.is_empty() || params.username == username) {
                return Some((id.clone(), session.clone()));
            }
        }
        None
    }
}
