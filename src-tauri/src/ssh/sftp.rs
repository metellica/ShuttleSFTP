use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use tokio::io::{AsyncSeekExt, AsyncWrite, AsyncWriteExt};

use crate::error::{AppError, AppResult};
use crate::fs::{FileEntry, FileStat, FsReader, FsWriter, RemoteFs};
use crate::ssh::client::SshHandle;
use crate::ssh::session::ConnectParams;

/// Wraps an SFTP session for file operations. Holds the underlying SSH
/// connection handle so exec channels (containers, tunnels) can be derived
/// from the same connection.
pub struct SftpClient {
    session: Arc<SshHandle>,
    sftp: SftpSession,
}

impl SftpClient {
    /// Establish an SSH connection and open an SFTP channel.
    pub async fn connect(params: &ConnectParams) -> AppResult<Self> {
        let session = crate::ssh::client::connect_ssh(params).await?;
        Self::open(session).await
    }

    /// Open an SFTP channel on an existing SSH connection.
    pub async fn open(session: Arc<SshHandle>) -> AppResult<Self> {
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

    /// The underlying SSH connection, for deriving exec channels.
    pub fn ssh_handle(&self) -> Arc<SshHandle> {
        self.session.clone()
    }

    /// Get the underlying SFTP session for transfer operations.
    pub fn sftp_session(&self) -> &SftpSession {
        &self.sftp
    }
}

struct SftpWriter(russh_sftp::client::fs::File);

impl AsyncWrite for SftpWriter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[async_trait]
impl FsWriter for SftpWriter {
    async fn finish(mut self: Box<Self>) -> AppResult<()> {
        self.0
            .shutdown()
            .await
            .map_err(|e| AppError::SftpError(format!("Remote close error: {}", e)))
    }
}

#[async_trait]
impl RemoteFs for SftpClient {
    fn kind(&self) -> &'static str {
        "sftp"
    }

    fn server_read_cmd(&self, path: &str) -> Option<String> {
        Some(format!("cat -- {}", crate::exec::shell_quote(path)))
    }

    fn server_write_cmd(&self, path: &str) -> Option<String> {
        Some(format!("cat > {}", crate::exec::shell_quote(path)))
    }

    fn server_scan_cmd(&self, dir: &str) -> Option<String> {
        Some(format!(
            r"find {} -mindepth 1 -type f -printf 'f\t%s\t%P\0' -o -type d -printf 'd\t0\t%P\0'",
            crate::exec::shell_quote(dir)
        ))
    }

    async fn stat(&self, path: &str) -> AppResult<FileStat> {
        let meta = self
            .sftp
            .metadata(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))?;
        Ok(FileStat {
            size: meta.size.unwrap_or(0),
            is_dir: meta.is_dir(),
        })
    }

    async fn list_dir(&self, path: &str) -> AppResult<Vec<FileEntry>> {
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
            let full_path = crate::fs::join_path(path, &name);

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

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        self.sftp
            .create_dir(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        self.sftp
            .remove_file(path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))
    }

    async fn remove_dir_all(&self, path: &str) -> AppResult<()> {
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
            self.sftp
                .remove_dir(dir)
                .await
                .map_err(|e| AppError::SftpError(e.to_string()))?;
        }
        Ok(())
    }

    fn fast_remove_dir(&self, _path: &str) -> bool {
        // Client-side recursion (above): callers may prefer the
        // progress-reporting variant.
        false
    }

    async fn rename(&self, old_path: &str, new_path: &str) -> AppResult<()> {
        self.sftp
            .rename(old_path, new_path)
            .await
            .map_err(|e| AppError::SftpError(e.to_string()))
    }

    /// Read up to `max_bytes` from the start of a remote file.
    async fn read_head(&self, path: &str, max_bytes: usize) -> AppResult<Vec<u8>> {
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
    async fn write_file(&self, path: &str, data: &[u8]) -> AppResult<()> {
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

    async fn open_read(&self, path: &str, offset: u64) -> AppResult<FsReader> {
        let meta = self
            .sftp
            .metadata(path)
            .await
            .map_err(|e| AppError::SftpError(format!("Cannot stat remote file: {}", e)))?;
        let total = meta.size.unwrap_or(0);
        let mut file = self
            .sftp
            .open(path)
            .await
            .map_err(|e| AppError::SftpError(format!("Cannot open remote file: {}", e)))?;
        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| AppError::SftpError(format!("Remote seek error: {}", e)))?;
        }
        Ok(FsReader {
            total,
            reader: Box::new(file),
        })
    }

    async fn open_write(&self, path: &str, offset: u64) -> AppResult<Box<dyn FsWriter>> {
        let mut file = if offset > 0 {
            self.sftp
                .open_with_flags(path, OpenFlags::WRITE)
                .await
                .map_err(|e| AppError::SftpError(format!("Cannot open remote file: {}", e)))?
        } else {
            self.sftp
                .create(path)
                .await
                .map_err(|e| AppError::SftpError(format!("Cannot create remote file: {}", e)))?
        };
        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| AppError::SftpError(format!("Remote seek error: {}", e)))?;
        }
        Ok(Box::new(SftpWriter(file)))
    }
}
