use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::container::{resolve_rootfs, ExecFs, ExecTarget, RuntimeKind};
use crate::error::{AppError, AppResult};
use crate::exec::{CommandRunner, LocalRunner, SshRunner};
use crate::fs::prefix::PrefixFs;
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

/// What kind of endpoint a session browses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Ssh,
    Container,
    Pod,
}

/// Container connection request from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerConnectSpec {
    pub runtime: RuntimeKind,
    pub container_id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Reuse the SSH connection of an existing session (remote engine).
    #[serde(default)]
    pub via_session_id: Option<String>,
    /// Or open a dedicated SSH connection (bookmark reconnects).
    #[serde(default)]
    pub via: Option<ConnectParams>,
    /// Try direct rootfs access through the host before exec+shell.
    #[serde(default)]
    pub prefer_rootfs: bool,
}

/// K8s pod connection request from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PodConnectSpec {
    #[serde(default)]
    pub context: Option<String>,
    pub namespace: String,
    pub pod: String,
    #[serde(default)]
    pub container: Option<String>,
    /// Where kubectl runs: an existing session's host, or local when None.
    #[serde(default)]
    pub via_session_id: Option<String>,
    #[serde(default)]
    pub via: Option<ConnectParams>,
}

/// How the session accesses files — used to pick transfer fast paths.
pub enum SessionAccess {
    /// Plain SFTP on the session's own SSH connection.
    Sftp,
    /// Rootfs subtree browsed over the host's SFTP; prefix is the host path.
    Rootfs { prefix: String },
    /// exec+shell through a container runtime CLI.
    Exec { target: ExecTarget },
}

/// An active session: SSH host, container or pod.
pub struct RemoteSession {
    pub id: String,
    pub kind: SessionKind,
    /// SSH leg parameters (for rebinding and bookmarks). For local
    /// containers host is "local".
    pub params: ConnectParams,
    /// File system endpoint used by all browse/transfer operations.
    pub fs: Arc<dyn RemoteFs>,
    /// SSH connection of the host leg, when there is one.
    pub ssh: Option<Arc<SshHandle>>,
    /// Command runner on the host machine (local or over SSH).
    pub runner: Option<Arc<dyn CommandRunner>>,
    pub access: SessionAccess,
}

impl RemoteSession {
    /// Host-side path of a session path, when the files are directly
    /// reachable on the host machine (SFTP or rootfs view).
    pub fn host_side_path(&self, path: &str) -> Option<String> {
        match &self.access {
            SessionAccess::Sftp => Some(path.to_string()),
            SessionAccess::Rootfs { prefix } => Some(format!(
                "{}/{}",
                prefix.trim_end_matches('/'),
                path.trim_start_matches('/')
            )),
            SessionAccess::Exec { .. } => None,
        }
    }
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

        log::info!("Connected to {}@{}:{}", params.username, params.host, params.port);
        self.insert(RemoteSession {
            id: session_id.clone(),
            kind: SessionKind::Ssh,
            params,
            fs: Arc::new(sftp),
            ssh: Some(ssh),
            runner: Some(runner),
            access: SessionAccess::Sftp,
        })
        .await;
        Ok(session_id)
    }

    /// Resolve the host runner (and its SSH handle) for container/pod
    /// operations: local machine, an existing session's host, or a fresh
    /// SSH connection.
    pub async fn host_runner(
        &self,
        via_session_id: Option<&str>,
        via: Option<&ConnectParams>,
    ) -> AppResult<(Arc<dyn CommandRunner>, Option<Arc<SshHandle>>, ConnectParams)> {
        if let Some(sid) = via_session_id {
            let session = self.get_session(sid).await?;
            let sess = session.lock().await;
            let ssh = sess.ssh.clone().ok_or_else(|| {
                AppError::ConnectionFailed("Selected session has no SSH connection".into())
            })?;
            let runner: Arc<dyn CommandRunner> = Arc::new(SshRunner::new(
                ssh.clone(),
                format!("{}@{}", sess.params.username, sess.params.host),
            ));
            return Ok((runner, Some(ssh), sess.params.clone()));
        }
        if let Some(params) = via {
            let ssh = crate::ssh::client::connect_ssh(params).await?;
            let runner: Arc<dyn CommandRunner> = Arc::new(SshRunner::new(
                ssh.clone(),
                format!("{}@{}", params.username, params.host),
            ));
            return Ok((runner, Some(ssh), params.clone()));
        }
        Ok((Arc::new(LocalRunner), None, local_params()))
    }

    /// Connect to a container (local engine or on a remote host).
    pub async fn connect_container(&self, spec: ContainerConnectSpec) -> AppResult<String> {
        let (runner, ssh, host_params) = self
            .host_runner(spec.via_session_id.as_deref(), spec.via.as_ref())
            .await?;

        // Fast, distroless-proof path: browse the container rootfs on the
        // host over SFTP (needs an SSH host leg and typically root).
        if spec.prefer_rootfs {
            if let Some(ssh) = &ssh {
                if let Some(prefix) =
                    resolve_rootfs(runner.as_ref(), spec.runtime, &spec.container_id).await
                {
                    match SftpClient::open(ssh.clone()).await {
                        Ok(sftp) => {
                            // Confirm SFTP can actually read the rootfs (permissions)
                            let view = PrefixFs::new(Arc::new(sftp), prefix.clone());
                            if view.list_dir("/").await.is_ok() {
                                log::info!(
                                    "Container {} via rootfs {}",
                                    spec.container_id,
                                    prefix
                                );
                                let session_id = uuid::Uuid::new_v4().to_string();
                                self.insert(RemoteSession {
                                    id: session_id.clone(),
                                    kind: SessionKind::Container,
                                    params: host_params,
                                    fs: Arc::new(view),
                                    ssh: Some(ssh.clone()),
                                    runner: Some(runner),
                                    access: SessionAccess::Rootfs { prefix },
                                })
                                .await;
                                return Ok(session_id);
                            }
                        }
                        Err(e) => log::warn!("Rootfs SFTP open failed: {}", e),
                    }
                }
            }
        }

        // Generic path: exec + shell tools inside the container.
        let target = ExecTarget::container(spec.runtime, spec.container_id.clone());
        let fs = ExecFs::new(runner.clone(), target.clone());
        fs.probe().await?;

        log::info!(
            "Container {} via {} exec on {}",
            spec.container_id,
            spec.runtime.as_str(),
            runner.location()
        );
        let session_id = uuid::Uuid::new_v4().to_string();
        self.insert(RemoteSession {
            id: session_id.clone(),
            kind: SessionKind::Container,
            params: host_params,
            fs: Arc::new(fs),
            ssh,
            runner: Some(runner),
            access: SessionAccess::Exec { target },
        })
        .await;
        Ok(session_id)
    }

    /// Connect to a K8s pod through kubectl exec (API path — no node
    /// access needed, only kubeconfig + pods/exec RBAC).
    pub async fn connect_pod(&self, spec: PodConnectSpec) -> AppResult<String> {
        let (runner, ssh, host_params) = self
            .host_runner(spec.via_session_id.as_deref(), spec.via.as_ref())
            .await?;

        let target = ExecTarget::pod(
            spec.context.clone(),
            spec.namespace.clone(),
            spec.pod.clone(),
            spec.container.clone(),
        );
        let fs = ExecFs::new(runner.clone(), target.clone());
        fs.probe().await?;

        log::info!(
            "Pod {}/{} via kubectl exec on {}",
            spec.namespace,
            spec.pod,
            runner.location()
        );
        let session_id = uuid::Uuid::new_v4().to_string();
        self.insert(RemoteSession {
            id: session_id.clone(),
            kind: SessionKind::Pod,
            params: host_params,
            fs: Arc::new(fs),
            ssh,
            runner: Some(runner),
            access: SessionAccess::Exec { target },
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

    /// Find any live SSH session connected to the given host as the given
    /// user. Used to rebind persisted transfers after a restart.
    pub async fn find_session_for(
        &self,
        host: &str,
        username: &str,
    ) -> Option<(String, Arc<Mutex<RemoteSession>>)> {
        let sessions = self.sessions.lock().await;
        for (id, session) in sessions.iter() {
            let (kind, params) = {
                let s = session.lock().await;
                (s.kind.clone(), s.params.clone())
            };
            if kind == SessionKind::Ssh && params.host == host && params.username == username {
                return Some((id.clone(), session.clone()));
            }
        }
        None
    }
}
