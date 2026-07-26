//! Directory tree scanning for bulk operations, at three speed tiers:
//! 1. One server-side `find` command, streamed and parsed as it runs
//!    (single network round trip).
//! 2. Concurrent per-directory listing (latency hidden by pipelining).
//! The caller picks tier 1 when the endpoint supports it, falling back
//! to tier 2 on runtime failure (e.g. BusyBox find without -printf).

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::commands::prepare::Prepare;
use crate::error::{AppError, AppResult};
use crate::exec::CommandRunner;
use crate::fs::{walk_fs_dir_concurrent, RemoteFs};

/// Scanned tree: dirs and (file, size) pairs, '/'-separated paths
/// relative to the scan root, sorted shallow-first.
pub type ScanResult = (Vec<String>, Vec<(String, u64)>);

/// Enumerate everything under `root` on any RemoteFs endpoint,
/// reporting per-file progress and honouring cancellation via `prep`.
pub async fn scan_tree(
    fs: &Arc<dyn RemoteFs>,
    runner: &Arc<dyn CommandRunner>,
    root: &str,
    prep: &Prepare<'_>,
) -> AppResult<ScanResult> {
    if let Some(cmd) = fs.server_scan_cmd(root) {
        match scan_server_side(runner, &cmd, prep).await? {
            Some(result) => return Ok(result),
            None => log::info!("Server-side scan unavailable for {}, walking instead", root),
        }
    }
    walk_fs_dir_concurrent(fs, root, || prep.step()).await
}

/// Sentinel proving the find command ran to completion; without it the
/// output is discarded and the caller falls back to the walk.
const SENTINEL: &[u8] = b"__SCAN_OK__";

/// Run a server_scan_cmd and stream-parse its NUL-terminated records.
/// Ok(None) = command unsupported/failed there, caller should walk.
async fn scan_server_side(
    runner: &Arc<dyn CommandRunner>,
    cmd: &str,
    prep: &Prepare<'_>,
) -> AppResult<Option<ScanResult>> {
    let script = format!("{} && printf {}", cmd, String::from_utf8_lossy(SENTINEL));
    let argv = vec!["sh".to_string(), "-c".into(), script];
    let stream = match runner.spawn(&argv).await {
        Ok(s) => s,
        Err(e) => {
            log::info!("Cannot spawn scan command: {}", e);
            return Ok(None);
        }
    };
    drop(stream.stdin);

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut rd = BufReader::with_capacity(64 * 1024, stream.stdout);
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    loop {
        buf.clear();
        let n = rd
            .read_until(0, &mut buf)
            .await
            .map_err(|e| AppError::IoError(format!("Scan stream error: {}", e)))?;
        if n == 0 {
            // EOF before the sentinel: find failed or is not GNU find
            return Ok(None);
        }
        if buf.last() == Some(&0) {
            buf.pop();
        } else {
            // Last chunk has no terminator: complete iff it's the sentinel
            return Ok(if buf == SENTINEL {
                dirs.sort();
                files.sort();
                Some((dirs, files))
            } else {
                None
            });
        }
        // Record: <f|d> \t <size> \t <path relative to root>
        let mut fields = buf.splitn(3, |&b| b == b'\t');
        match (fields.next(), fields.next(), fields.next()) {
            (Some(b"f"), Some(size), Some(rel)) if !rel.is_empty() => {
                prep.step()?;
                let size = std::str::from_utf8(size)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                files.push((String::from_utf8_lossy(rel).into_owned(), size));
            }
            (Some(b"d"), Some(_), Some(rel)) if !rel.is_empty() => {
                dirs.push(String::from_utf8_lossy(rel).into_owned());
            }
            _ => {} // ignore malformed records
        }
    }
}
