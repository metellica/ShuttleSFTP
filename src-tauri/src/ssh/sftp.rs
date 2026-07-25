use std::sync::Arc;
use std::time::SystemTime;

use russh::keys::key::PrivateKeyWithHashAlg;
use russh::keys::*;
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::ssh::auth::AuthMethod;
use crate::ssh::session::ConnectParams;

/// File entry returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub permissions: Option<String>,
}

/// Wraps an SFTP session for file operations.
pub struct SftpClient {
    #[allow(dead_code)]
    session: russh::client::Handle<ClientHandler>,
    sftp: SftpSession,
}

/// Minimal russh client handler.
struct ClientHandler;

impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        // TODO: Verify against known_hosts in production
        async { Ok(true) }
    }
}

impl SftpClient {
    /// Establish an SSH connection and open an SFTP channel.
    pub async fn connect(params: &ConnectParams) -> AppResult<Self> {
        let config = Arc::new(russh::client::Config::default());
        let handler = ClientHandler;

        let mut session = russh::client::connect(config, (&*params.host, params.port), handler)
            .await
            .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;

        // Authenticate
        let auth_result = match &params.auth {
            AuthMethod::Password { password } => {
                session
                    .authenticate_password(&params.username, password)
                    .await
                    .map_err(|e| AppError::AuthFailed(e.to_string()))?
            }
            AuthMethod::PrivateKey { key_path, passphrase } => {
                let key_data = tokio::fs::read_to_string(key_path)
                    .await
                    .map_err(|e| AppError::AuthFailed(format!("Cannot read key file: {}", e)))?;

                let key_pair = if let Some(pass) = passphrase {
                    decode_secret_key(&key_data, Some(pass))
                        .map_err(|e| AppError::AuthFailed(format!("Key decode error: {}", e)))?
                } else {
                    decode_secret_key(&key_data, None)
                        .map_err(|e| AppError::AuthFailed(format!("Key decode error: {}", e)))?
                };

                let key_with_alg = PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);

                session
                    .authenticate_publickey(&params.username, key_with_alg)
                    .await
                    .map_err(|e| AppError::AuthFailed(e.to_string()))?
            }
            AuthMethod::Agent => {
                return Err(AppError::AuthFailed("Agent auth not yet implemented".into()));
            }
        };

        if !auth_result.success() {
            return Err(AppError::AuthFailed("Authentication rejected by server".into()));
        }

        // Open SFTP subsystem
        let channel = session
            .channel_open_session()
            .await
            .map_err(|e| AppError::SftpError(format!("Channel open failed: {}", e)))?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::SftpError(format!("SFTP subsystem request failed: {}", e)))?;

        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::SftpError(format!("SFTP session init failed: {}", e)))?;

        Ok(Self { session, sftp })
    }

    /// List directory contents.
    pub async fn list_dir(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        let entries = self
            .sftp
            .read_dir(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))?;

        let mut result = Vec::new();
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let full_path = if path == "/" {
                format!("/{}", name)
            } else {
                format!("{}/{}", path, name)
            };

            let attrs = entry.metadata();
            let is_dir = attrs.is_dir();
            let size = attrs.len();
            let modified = attrs.modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let permissions = Some(attrs.permissions().to_string());

            result.push(FileEntry {
                name,
                path: full_path,
                is_dir,
                size,
                modified,
                permissions,
            });
        }
        Ok(result)
    }

    /// Create a directory.
    pub async fn mkdir(&self, path: &str) -> AppResult<()> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))
    }

    /// Remove a file.
    pub async fn remove_file(&self, path: &str) -> AppResult<()> {
        self.sftp
            .remove_file(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))
    }

    /// Remove a directory.
    pub async fn remove_dir(&self, path: &str) -> AppResult<()> {
        self.sftp
            .remove_dir(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))
    }

    /// Remove a directory and all of its contents recursively.
    pub async fn remove_dir_all(&self, path: &str) -> AppResult<()> {
        // Breadth-first collect directories, deleting files along the way,
        // then remove directories deepest-first.
        let mut dirs = vec![path.to_string()];
        let mut i = 0;
        while i < dirs.len() {
            let dir = dirs[i].clone();
            for entry in self.list_dir(&dir).await? {
                if entry.is_dir {
                    dirs.push(entry.path);
                } else {
                    self.remove_file(&entry.path).await?;
                }
            }
            i += 1;
        }
        for dir in dirs.iter().rev() {
            self.remove_dir(dir).await?;
        }
        Ok(())
    }

    /// Rename/move a file or directory.
    pub async fn rename(&self, old_path: &str, new_path: &str) -> AppResult<()> {
        self.sftp
            .rename(old_path, new_path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))
    }

    /// Read up to `max_bytes` from the start of a remote file.
    pub async fn read_head(&self, path: &str, max_bytes: usize) -> AppResult<Vec<u8>> {
        use tokio::io::AsyncReadExt;

        let mut file = self
            .sftp
            .open(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))?;

        let mut buf = Vec::with_capacity(max_bytes.min(64 * 1024));
        let mut chunk = vec![0u8; 16 * 1024];
        while buf.len() < max_bytes {
            let n = file
                .read(&mut chunk)
                .await
                .map_err(|e| AppError::SftpError(e.to_string()))?;
            if n == 0 {
                break;
            }
            let take = n.min(max_bytes - buf.len());
            buf.extend_from_slice(&chunk[..take]);
        }
        Ok(buf)
    }

    /// Overwrite a remote file with the given bytes (create if missing).
    pub async fn write_file(&self, path: &str, data: &[u8]) -> AppResult<()> {
        use tokio::io::AsyncWriteExt;

        let mut file = self
            .sftp
            .create(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))?;
        file.write_all(data)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))?;
        file.shutdown()
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))?;
        Ok(())
    }

    /// Get the underlying SFTP session for transfer operations.
    pub fn sftp_session(&self) -> &SftpSession {
        &self.sftp
    }
}
