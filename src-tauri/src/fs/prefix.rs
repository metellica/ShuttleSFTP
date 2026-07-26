use std::sync::Arc;

use async_trait::async_trait;

use crate::error::AppResult;
use crate::fs::{FileEntry, FileStat, FsReader, FsWriter, RemoteFs};

/// Exposes a subtree of another RemoteFs as its root ("chroot view").
/// Used for containerd/docker rootfs access: the container's file system
/// lives under a prefix directory on the host, browsed over SFTP.
pub struct PrefixFs {
    inner: Arc<dyn RemoteFs>,
    prefix: String,
}

impl PrefixFs {
    pub fn new(inner: Arc<dyn RemoteFs>, prefix: impl Into<String>) -> Self {
        let mut prefix = prefix.into();
        while prefix.ends_with('/') && prefix.len() > 1 {
            prefix.pop();
        }
        Self { inner, prefix }
    }

    /// The host-side path a view path maps to.
    pub fn to_inner(&self, path: &str) -> String {
        let p = path.trim_start_matches('/');
        if p.is_empty() {
            self.prefix.clone()
        } else {
            format!("{}/{}", self.prefix, p)
        }
    }

    fn to_view(&self, inner_path: &str) -> String {
        let stripped = inner_path
            .strip_prefix(&self.prefix)
            .unwrap_or(inner_path);
        if stripped.is_empty() {
            "/".to_string()
        } else {
            stripped.to_string()
        }
    }
}

#[async_trait]
impl RemoteFs for PrefixFs {
    fn kind(&self) -> &'static str {
        "rootfs"
    }

    fn supports_resume(&self) -> bool {
        self.inner.supports_resume()
    }

    fn server_read_cmd(&self, path: &str) -> Option<String> {
        self.inner.server_read_cmd(&self.to_inner(path))
    }

    fn server_write_cmd(&self, path: &str) -> Option<String> {
        self.inner.server_write_cmd(&self.to_inner(path))
    }

    fn server_scan_cmd(&self, dir: &str) -> Option<String> {
        self.inner.server_scan_cmd(&self.to_inner(dir))
    }

    async fn stat(&self, path: &str) -> AppResult<FileStat> {
        self.inner.stat(&self.to_inner(path)).await
    }

    async fn list_dir(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        let mut entries = self.inner.list_dir(&self.to_inner(path)).await?;
        for e in &mut entries {
            e.path = self.to_view(&e.path);
        }
        Ok(entries)
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        self.inner.mkdir(&self.to_inner(path)).await
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        self.inner.remove_file(&self.to_inner(path)).await
    }

    async fn remove_dir_all(&self, path: &str) -> AppResult<()> {
        self.inner.remove_dir_all(&self.to_inner(path)).await
    }

    fn fast_remove_dir(&self, path: &str) -> bool {
        self.inner.fast_remove_dir(&self.to_inner(path))
    }

    async fn rename(&self, old_path: &str, new_path: &str) -> AppResult<()> {
        self.inner
            .rename(&self.to_inner(old_path), &self.to_inner(new_path))
            .await
    }

    async fn read_head(&self, path: &str, max_bytes: usize) -> AppResult<Vec<u8>> {
        self.inner.read_head(&self.to_inner(path), max_bytes).await
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> AppResult<()> {
        self.inner.write_file(&self.to_inner(path), data).await
    }

    async fn open_read(&self, path: &str, offset: u64) -> AppResult<FsReader> {
        self.inner.open_read(&self.to_inner(path), offset).await
    }

    async fn open_write(&self, path: &str, offset: u64) -> AppResult<Box<dyn FsWriter>> {
        self.inner.open_write(&self.to_inner(path), offset).await
    }
}
