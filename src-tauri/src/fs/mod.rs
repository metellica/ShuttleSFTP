pub mod host;
pub mod local;
pub mod prefix;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::AppResult;

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

/// Minimal metadata for a single path.
#[derive(Debug, Clone)]
pub struct FileStat {
    pub size: u64,
    pub is_dir: bool,
}

/// A streaming reader plus the total size of the file being read.
pub struct FsReader {
    pub total: u64,
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
}

/// A streaming write sink. `finish` must be called to flush, close and
/// verify the write (e.g. await the remote command's exit status).
#[async_trait]
pub trait FsWriter: AsyncWrite + Send + Unpin {
    async fn finish(self: Box<Self>) -> AppResult<()>;
}

/// Abstraction over any browsable/transferable file system endpoint:
/// SFTP, the local machine, a container (exec-based) or a container
/// rootfs exposed through the host.
#[async_trait]
pub trait RemoteFs: Send + Sync {
    /// Short backend identifier ("sftp", "local", "exec", "rootfs").
    fn kind(&self) -> &'static str;

    /// Whether reads/writes can start at a byte offset (resume support).
    fn supports_resume(&self) -> bool {
        true
    }

    /// Per-path resume capability (virtual-dir backends vary by path).
    fn supports_resume_at(&self, _path: &str) -> bool {
        self.supports_resume()
    }

    /// Shell command that streams this file to stdout when run on the
    /// endpoint's host machine. Enables server-side copies that bypass
    /// the local relay. None = not expressible as a host-side command.
    fn server_read_cmd(&self, _path: &str) -> Option<String> {
        None
    }

    /// Shell command that writes stdin to this file when run on the
    /// endpoint's host machine.
    fn server_write_cmd(&self, _path: &str) -> Option<String> {
        None
    }

    async fn stat(&self, path: &str) -> AppResult<FileStat>;
    async fn list_dir(&self, path: &str) -> AppResult<Vec<FileEntry>>;
    async fn mkdir(&self, path: &str) -> AppResult<()>;
    async fn remove_file(&self, path: &str) -> AppResult<()>;
    async fn remove_dir_all(&self, path: &str) -> AppResult<()>;
    async fn rename(&self, old_path: &str, new_path: &str) -> AppResult<()>;
    async fn read_head(&self, path: &str, max_bytes: usize) -> AppResult<Vec<u8>>;
    async fn write_file(&self, path: &str, data: &[u8]) -> AppResult<()>;
    async fn open_read(&self, path: &str, offset: u64) -> AppResult<FsReader>;
    async fn open_write(&self, path: &str, offset: u64) -> AppResult<Box<dyn FsWriter>>;
}

/// Join a base dir and a child name with '/' (POSIX-style remote paths).
pub fn join_path(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{}", name)
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), name)
    }
}

/// Render a Unix mode's permission bits as "rwxr-xr-x".
pub fn mode_to_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    let flags = [
        (0o400, 'r'), (0o200, 'w'), (0o100, 'x'),
        (0o040, 'r'), (0o020, 'w'), (0o010, 'x'),
        (0o004, 'r'), (0o002, 'w'), (0o001, 'x'),
    ];
    for (bit, ch) in flags {
        s.push(if mode & bit != 0 { ch } else { '-' });
    }
    s
}

/// Walk a directory tree of any RemoteFs. Returns (dirs, files) as
/// '/'-separated paths relative to `root`, both sorted shallow-first.
pub async fn walk_fs_dir(fs: &dyn RemoteFs, root: &str) -> AppResult<(Vec<String>, Vec<String>)> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut stack: Vec<(String, String)> = vec![(root.to_string(), String::new())];
    while let Some((path, rel)) = stack.pop() {
        for entry in fs.list_dir(&path).await? {
            let child_rel = if rel.is_empty() {
                entry.name
            } else {
                format!("{}/{}", rel, entry.name)
            };
            if entry.is_dir {
                dirs.push(child_rel.clone());
                stack.push((entry.path, child_rel));
            } else {
                files.push(child_rel);
            }
        }
    }
    dirs.sort();
    files.sort();
    Ok((dirs, files))
}
