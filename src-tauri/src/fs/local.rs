use std::path::PathBuf;
use std::time::SystemTime;

use async_trait::async_trait;
use tokio::io::{AsyncSeekExt, AsyncWrite, AsyncWriteExt};

use crate::error::{AppError, AppResult};
use crate::fs::{FileEntry, FileStat, FsReader, FsWriter, RemoteFs};

/// The local machine's file system as a transfer endpoint.
pub struct LocalFs;

struct LocalWriter(tokio::fs::File);

impl AsyncWrite for LocalWriter {
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
impl FsWriter for LocalWriter {
    async fn finish(mut self: Box<Self>) -> AppResult<()> {
        self.0
            .flush()
            .await
            .map_err(|e| AppError::IoError(format!("Local flush error: {}", e)))?;
        Ok(())
    }
}

fn to_entry(dir: &str, name: String, meta: &std::fs::Metadata) -> FileEntry {
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    FileEntry {
        path: crate::fs::join_path(dir, &name),
        name,
        is_dir: meta.is_dir(),
        size: meta.len(),
        modified,
        permissions: None,
    }
}

/// Map a browse path to a native file system path.
///
/// On Windows the virtual root "/" lists drives, and drive paths are
/// represented as "/C:/Users/...". Native Windows paths ("C:\...") and
/// current-drive paths pass through unchanged.
#[cfg(windows)]
fn native_path(path: &str) -> PathBuf {
    let p = path.strip_prefix('/').unwrap_or(path);
    let bytes = p.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let mut s = p.replace('/', "\\");
        if s.len() == 2 {
            s.push('\\'); // "C:" -> "C:\" (drive root, not drive-relative)
        }
        PathBuf::from(s)
    } else {
        PathBuf::from(path)
    }
}

#[cfg(not(windows))]
fn native_path(path: &str) -> PathBuf {
    PathBuf::from(path)
}

#[cfg(windows)]
fn is_virtual_root(path: &str) -> bool {
    path.trim_matches('/').is_empty()
}

/// List available drive letters as directory entries under "/".
#[cfg(windows)]
async fn list_drives() -> Vec<FileEntry> {
    let mut out = Vec::new();
    for letter in 'A'..='Z' {
        let root = format!("{}:\\", letter);
        if tokio::fs::metadata(&root).await.is_ok() {
            out.push(FileEntry {
                name: format!("{}:", letter),
                path: format!("/{}:", letter),
                is_dir: true,
                size: 0,
                modified: 0,
                permissions: None,
            });
        }
    }
    out
}

#[async_trait]
impl RemoteFs for LocalFs {
    fn kind(&self) -> &'static str {
        "local"
    }

    async fn stat(&self, path: &str) -> AppResult<FileStat> {
        #[cfg(windows)]
        if is_virtual_root(path) {
            return Ok(FileStat { size: 0, is_dir: true });
        }
        let meta = tokio::fs::metadata(native_path(path))
            .await
            .map_err(|e| AppError::IoError(format!("Cannot stat {}: {}", path, e)))?;
        Ok(FileStat {
            size: meta.len(),
            is_dir: meta.is_dir(),
        })
    }

    async fn list_dir(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        #[cfg(windows)]
        if is_virtual_root(path) {
            return Ok(list_drives().await);
        }
        let dir = native_path(path);
        let mut rd = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| AppError::IoError(format!("Cannot read dir {}: {}", path, e)))?;
        let mut result = Vec::new();
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| AppError::IoError(e.to_string()))?
        {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(meta) = entry.metadata().await {
                result.push(to_entry(path, name, &meta));
            }
        }
        Ok(result)
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        tokio::fs::create_dir_all(native_path(path))
            .await
            .map_err(|e| AppError::IoError(format!("Cannot create dir {}: {}", path, e)))
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        tokio::fs::remove_file(native_path(path))
            .await
            .map_err(|e| AppError::IoError(format!("Cannot remove {}: {}", path, e)))
    }

    async fn remove_dir_all(&self, path: &str) -> AppResult<()> {
        tokio::fs::remove_dir_all(native_path(path))
            .await
            .map_err(|e| AppError::IoError(format!("Cannot remove dir {}: {}", path, e)))
    }

    async fn rename(&self, old_path: &str, new_path: &str) -> AppResult<()> {
        tokio::fs::rename(native_path(old_path), native_path(new_path))
            .await
            .map_err(|e| AppError::IoError(format!("Cannot rename {}: {}", old_path, e)))
    }

    async fn read_head(&self, path: &str, max_bytes: usize) -> AppResult<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        let mut file = tokio::fs::File::open(native_path(path))
            .await
            .map_err(|e| AppError::IoError(format!("Cannot open {}: {}", path, e)))?;
        let mut buf = Vec::new();
        let mut chunk = vec![0u8; 16 * 1024];
        while buf.len() < max_bytes {
            let n = file
                .read(&mut chunk)
                .await
                .map_err(|e| AppError::IoError(e.to_string()))?;
            if n == 0 {
                break;
            }
            let take = n.min(max_bytes - buf.len());
            buf.extend_from_slice(&chunk[..take]);
        }
        Ok(buf)
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> AppResult<()> {
        tokio::fs::write(native_path(path), data)
            .await
            .map_err(|e| AppError::IoError(format!("Cannot write {}: {}", path, e)))
    }

    async fn open_read(&self, path: &str, offset: u64) -> AppResult<FsReader> {
        let native = native_path(path);
        let meta = tokio::fs::metadata(&native)
            .await
            .map_err(|e| AppError::IoError(format!("Cannot stat {}: {}", path, e)))?;
        let mut file = tokio::fs::File::open(&native)
            .await
            .map_err(|e| AppError::IoError(format!("Cannot open {}: {}", path, e)))?;
        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| AppError::IoError(format!("Seek error: {}", e)))?;
        }
        Ok(FsReader {
            total: meta.len(),
            reader: Box::new(file),
        })
    }

    async fn open_write(&self, path: &str, offset: u64) -> AppResult<Box<dyn FsWriter>> {
        let native = native_path(path);
        let mut file = if offset > 0 {
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(&native)
                .await
                .map_err(|e| AppError::IoError(format!("Cannot open {}: {}", path, e)))?
        } else {
            tokio::fs::File::create(&native)
                .await
                .map_err(|e| AppError::IoError(format!("Cannot create {}: {}", path, e)))?
        };
        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| AppError::IoError(format!("Seek error: {}", e)))?;
        }
        Ok(Box::new(LocalWriter(file)))
    }
}
