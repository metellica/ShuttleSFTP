use std::sync::Arc;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;

use crate::error::{AppError, AppResult};
use crate::ssh::client::ClientHandler;

/// Captured result of a finished command.
#[derive(Debug)]
pub struct ExecOutput {
    pub exit: Option<u32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl ExecOutput {
    pub fn success(&self) -> bool {
        self.exit.unwrap_or(0) == 0
    }

    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    /// Error out unless the command exited 0.
    pub fn check(self, what: &str) -> AppResult<Self> {
        if self.success() {
            Ok(self)
        } else {
            let detail = if self.stderr.trim().is_empty() {
                self.stdout_string()
            } else {
                self.stderr.clone()
            };
            Err(AppError::IoError(format!(
                "{} failed (exit {}): {}",
                what,
                self.exit.unwrap_or(0),
                detail.trim()
            )))
        }
    }
}

/// Completion info of a streaming command.
#[derive(Debug)]
pub struct ExecDone {
    pub exit: Option<u32>,
    pub stderr: String,
}

/// A spawned command with streaming stdin/stdout.
pub struct ExecStream {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    /// Resolves when the command finishes.
    pub done: oneshot::Receiver<ExecDone>,
}

/// Runs argv-style commands on a target machine: the local machine or a
/// remote host over an existing SSH connection.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Run to completion, capturing stdout/stderr.
    async fn run(&self, argv: &[String], stdin: Option<Vec<u8>>) -> AppResult<ExecOutput>;
    /// Spawn with streaming stdin/stdout.
    async fn spawn(&self, argv: &[String]) -> AppResult<ExecStream>;
    /// Short label of where commands run ("local" or "user@host").
    fn location(&self) -> String;
}

pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Quote a string for POSIX shell.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

pub fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_shell_args() {
        assert_eq!(shell_quote("simple-file.txt"), "simple-file.txt");
        assert_eq!(shell_quote("/var/log/app.log"), "/var/log/app.log");
        assert_eq!(shell_quote("has space"), "'has space'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn joins_argv() {
        let argv = vec!["docker".to_string(), "exec".into(), "a b".into()];
        assert_eq!(shell_join(&argv), "docker exec 'a b'");
    }

    #[tokio::test]
    async fn local_runner_runs_and_streams() {
        let runner = LocalRunner;
        // `run` with stdin capture (cmd on Windows, sh elsewhere)
        #[cfg(windows)]
        let argv = argv(&["cmd", "/c", "findstr", "x"]);
        #[cfg(not(windows))]
        let argv = argv(&["grep", "x"]);
        let out = runner
            .run(&argv, Some(b"axc\nbyd\n".to_vec()))
            .await
            .unwrap();
        assert!(out.success());
        assert!(out.stdout_string().contains("axc"));

        // `spawn` echoes matching stdin lines to stdout
        #[cfg(windows)]
        let argv2 = super::argv(&["cmd", "/c", "findstr", "hello"]);
        #[cfg(not(windows))]
        let argv2 = super::argv(&["cat"]);
        let mut stream = runner.spawn(&argv2).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream.stdin.write_all(b"hello\r\n").await.unwrap();
        stream.stdin.shutdown().await.unwrap();
        // Dropping stdin is what delivers EOF to the child on Windows
        let closed: Box<dyn tokio::io::AsyncWrite + Send + Unpin> =
            Box::new(tokio::io::sink());
        drop(std::mem::replace(&mut stream.stdin, closed));
        let mut buf = String::new();
        stream.stdout.read_to_string(&mut buf).await.unwrap();
        assert!(buf.contains("hello"));
        let done = stream.done.await.unwrap();
        assert_eq!(done.exit, Some(0));
    }
}

// ---------------------------------------------------------------------------
// Local machine runner
// ---------------------------------------------------------------------------

pub struct LocalRunner;

fn local_command(argv: &[String]) -> AppResult<tokio::process::Command> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| AppError::IoError("Empty command".into()))?;
    let mut cmd = tokio::process::Command::new(prog);
    cmd.args(args);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(cmd)
}

#[async_trait]
impl CommandRunner for LocalRunner {
    async fn run(&self, argv: &[String], stdin: Option<Vec<u8>>) -> AppResult<ExecOutput> {
        let mut cmd = local_command(argv)?;
        cmd.stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        });
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::IoError(format!("Cannot run {}: {}", argv[0], e)))?;
        if let Some(data) = stdin {
            if let Some(mut sin) = child.stdin.take() {
                let _ = sin.write_all(&data).await;
                let _ = sin.shutdown().await;
            }
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| AppError::IoError(format!("Command error: {}", e)))?;
        Ok(ExecOutput {
            exit: out.status.code().map(|c| c as u32),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        })
    }

    async fn spawn(&self, argv: &[String]) -> AppResult<ExecStream> {
        let mut cmd = local_command(argv)?;
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::IoError(format!("Cannot run {}: {}", argv[0], e)))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::IoError("No stdin pipe".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::IoError("No stdout pipe".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::IoError("No stderr pipe".into()))?;
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut err_buf = String::new();
            let _ = stderr.read_to_string(&mut err_buf).await;
            let exit = child
                .wait()
                .await
                .ok()
                .and_then(|s| s.code())
                .map(|c| c as u32);
            let _ = tx.send(ExecDone {
                exit,
                stderr: err_buf,
            });
        });
        Ok(ExecStream {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            done: rx,
        })
    }

    fn location(&self) -> String {
        "local".to_string()
    }
}

// ---------------------------------------------------------------------------
// SSH runner (exec channel over an existing russh connection)
// ---------------------------------------------------------------------------

pub struct SshRunner {
    handle: Arc<russh::client::Handle<ClientHandler>>,
    label: String,
}

impl SshRunner {
    pub fn new(handle: Arc<russh::client::Handle<ClientHandler>>, label: impl Into<String>) -> Self {
        Self {
            handle,
            label: label.into(),
        }
    }
}

#[async_trait]
impl CommandRunner for SshRunner {
    async fn run(&self, argv: &[String], stdin: Option<Vec<u8>>) -> AppResult<ExecOutput> {
        let cmd = shell_join(argv);
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::IoError(format!("SSH channel error: {}", e)))?;
        channel
            .exec(true, cmd.as_bytes())
            .await
            .map_err(|e| AppError::IoError(format!("SSH exec error: {}", e)))?;
        if let Some(data) = stdin {
            channel
                .data(&data[..])
                .await
                .map_err(|e| AppError::IoError(format!("SSH stdin error: {}", e)))?;
        }
        channel
            .eof()
            .await
            .map_err(|e| AppError::IoError(format!("SSH eof error: {}", e)))?;

        let mut channel = channel;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                russh::ChannelMsg::ExtendedData { data, ext: 1 } => {
                    stderr.extend_from_slice(&data)
                }
                russh::ChannelMsg::ExitStatus { exit_status } => exit = Some(exit_status),
                russh::ChannelMsg::Failure => {
                    return Err(AppError::IoError("SSH exec request rejected".into()))
                }
                _ => {}
            }
        }
        Ok(ExecOutput {
            exit,
            stdout,
            stderr: String::from_utf8_lossy(&stderr).to_string(),
        })
    }

    async fn spawn(&self, argv: &[String]) -> AppResult<ExecStream> {
        let cmd = shell_join(argv);
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::IoError(format!("SSH channel error: {}", e)))?;
        channel
            .exec(true, cmd.as_bytes())
            .await
            .map_err(|e| AppError::IoError(format!("SSH exec error: {}", e)))?;

        let (mut read_half, write_half) = channel.split();
        let stdin = write_half.make_writer();
        // Pump channel messages into a pipe so the consumer gets a plain
        // AsyncRead, while stderr and the exit status are collected aside.
        let (pipe_writer, pipe_reader) = tokio::io::duplex(256 * 1024);
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut pipe_writer = pipe_writer;
            let mut stderr = Vec::new();
            let mut exit = None;
            while let Some(msg) = read_half.wait().await {
                match msg {
                    russh::ChannelMsg::Data { data } => {
                        if pipe_writer.write_all(&data).await.is_err() {
                            break; // consumer dropped
                        }
                    }
                    russh::ChannelMsg::ExtendedData { data, ext: 1 } => {
                        stderr.extend_from_slice(&data)
                    }
                    russh::ChannelMsg::ExitStatus { exit_status } => exit = Some(exit_status),
                    _ => {}
                }
            }
            drop(pipe_writer);
            let _ = tx.send(ExecDone {
                exit,
                stderr: String::from_utf8_lossy(&stderr).to_string(),
            });
        });
        // Keep the write half alive inside the stdin box: dropping it closes
        // the channel. Bundle both via a wrapper.
        struct SshStdin<W: AsyncWrite + Unpin + Send> {
            writer: W,
            _half: russh::ChannelWriteHalf<russh::client::Msg>,
        }
        impl<W: AsyncWrite + Unpin + Send> AsyncWrite for SshStdin<W> {
            fn poll_write(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &[u8],
            ) -> std::task::Poll<Result<usize, std::io::Error>> {
                std::pin::Pin::new(&mut self.writer).poll_write(cx, buf)
            }
            fn poll_flush(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), std::io::Error>> {
                std::pin::Pin::new(&mut self.writer).poll_flush(cx)
            }
            fn poll_shutdown(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), std::io::Error>> {
                std::pin::Pin::new(&mut self.writer).poll_shutdown(cx)
            }
        }
        Ok(ExecStream {
            stdin: Box::new(SshStdin {
                writer: stdin,
                _half: write_half,
            }),
            stdout: Box::new(pipe_reader),
            done: rx,
        })
    }

    fn location(&self) -> String {
        self.label.clone()
    }
}
