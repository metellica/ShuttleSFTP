//! Interactive terminals attached to a session: a PTY shell on the
//! local machine or over SSH, auto-attaching into containers/pods when
//! the browsed path lives inside one. Output is streamed to the
//! frontend as base64 `terminal:data` events; input/resize/close come
//! back through commands.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use tauri::Emitter;
use tokio::sync::{mpsc, Mutex};

use crate::error::{AppError, AppResult};
use crate::exec::shell_join;
use crate::fs::TerminalSpec;
use crate::ssh::session::RemoteSession;

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Control messages for a running terminal.
enum TermCmd {
    Input(Vec<u8>),
    Resize(u16, u16),
    Close,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DataEvent<'a> {
    id: &'a str,
    /// Raw output bytes, base64-encoded (may split UTF-8 sequences).
    data: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ExitEvent<'a> {
    id: &'a str,
}

type TermMap = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<TermCmd>>>>;

/// All live terminals, keyed by terminal id.
pub struct TerminalManager {
    terms: TermMap,
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self {
            terms: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl TerminalManager {
    /// Open a terminal for `path` in the given session. Returns the
    /// terminal id; output arrives as `terminal:data` events.
    pub async fn open(
        &self,
        app: tauri::AppHandle,
        session: Arc<Mutex<RemoteSession>>,
        path: &str,
        cols: u16,
        rows: u16,
    ) -> AppResult<String> {
        let (spec, ssh) = {
            let s = session.lock().await;
            (s.fs.terminal_spec(path).await?, s.ssh.clone())
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::unbounded_channel();

        if let Some(ssh) = ssh {
            open_ssh(&app, &id, ssh, spec, cols, rows, rx, self.terms.clone()).await?;
        } else {
            open_local(&app, &id, spec, cols, rows, rx, self.terms.clone())?;
        }
        self.terms.lock().await.insert(id.clone(), tx);
        Ok(id)
    }

    async fn send(&self, terminal_id: &str, cmd: TermCmd) -> AppResult<()> {
        let terms = self.terms.lock().await;
        let tx = terms
            .get(terminal_id)
            .ok_or_else(|| AppError::IoError("Terminal is closed".into()))?;
        tx.send(cmd)
            .map_err(|_| AppError::IoError("Terminal is closed".into()))
    }

    pub async fn input(&self, terminal_id: &str, data: Vec<u8>) -> AppResult<()> {
        self.send(terminal_id, TermCmd::Input(data)).await
    }

    pub async fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> AppResult<()> {
        self.send(terminal_id, TermCmd::Resize(cols, rows)).await
    }

    pub async fn close(&self, terminal_id: &str) -> AppResult<()> {
        // Ignore already-gone terminals: close must be idempotent.
        let _ = self.send(terminal_id, TermCmd::Close).await;
        self.terms.lock().await.remove(terminal_id);
        Ok(())
    }
}

/// Emit the exit event and drop the terminal from the registry.
fn finish(app: &tauri::AppHandle, terms: &TermMap, id: &str) {
    let _ = app.emit("terminal:exit", ExitEvent { id });
    if let Ok(mut map) = terms.try_lock() {
        map.remove(id);
    }
}

/// Shell command that starts an interactive shell over SSH for a spec.
fn ssh_command(spec: &TerminalSpec) -> String {
    match spec {
        TerminalSpec::HostDir(dir) if dir.is_empty() => r#"exec "${SHELL:-sh}""#.to_string(),
        TerminalSpec::HostDir(dir) => format!(
            r#"cd {} 2>/dev/null; exec "${{SHELL:-sh}}""#,
            crate::exec::shell_quote(dir)
        ),
        TerminalSpec::Command(argv) => shell_join(argv),
    }
}

/// PTY shell over an SSH channel (russh request_pty + exec).
async fn open_ssh(
    app: &tauri::AppHandle,
    id: &str,
    ssh: Arc<crate::ssh::client::SshHandle>,
    spec: TerminalSpec,
    cols: u16,
    rows: u16,
    mut rx: mpsc::UnboundedReceiver<TermCmd>,
    terms: TermMap,
) -> AppResult<()> {
    let channel = ssh
        .channel_open_session()
        .await
        .map_err(|e| AppError::IoError(format!("SSH channel error: {}", e)))?;
    channel
        .request_pty(
            false,
            "xterm-256color",
            cols as u32,
            rows as u32,
            0,
            0,
            &[],
        )
        .await
        .map_err(|e| AppError::IoError(format!("SSH pty error: {}", e)))?;
    let cmd = ssh_command(&spec);
    log::info!("Terminal {} over SSH: {}", id, cmd);
    channel
        .exec(true, cmd.as_bytes())
        .await
        .map_err(|e| AppError::IoError(format!("SSH exec error: {}", e)))?;

    let (mut read_half, write_half) = channel.split();

    // Output pump: channel messages -> frontend events.
    {
        let app = app.clone();
        let id = id.to_string();
        let terms = terms.clone();
        tokio::spawn(async move {
            while let Some(msg) = read_half.wait().await {
                match msg {
                    russh::ChannelMsg::Data { data } => {
                        let _ = app.emit(
                            "terminal:data",
                            DataEvent {
                                id: &id,
                                data: b64(&data),
                            },
                        );
                    }
                    russh::ChannelMsg::ExtendedData { data, ext: 1 } => {
                        let _ = app.emit(
                            "terminal:data",
                            DataEvent {
                                id: &id,
                                data: b64(&data),
                            },
                        );
                    }
                    _ => {}
                }
            }
            finish(&app, &terms, &id);
        });
    }

    // Input pump: frontend commands -> channel.
    tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                TermCmd::Input(data) => {
                    if write_half.data(&data[..]).await.is_err() {
                        break;
                    }
                }
                TermCmd::Resize(cols, rows) => {
                    let _ = write_half
                        .window_change(cols as u32, rows as u32, 0, 0)
                        .await;
                }
                TermCmd::Close => break,
            }
        }
        // Closing the channel ends the remote shell.
        let _ = write_half.close().await;
    });

    Ok(())
}

#[cfg(windows)]
fn default_shell() -> String {
    "powershell.exe".to_string()
}

#[cfg(not(windows))]
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// PTY shell on this machine (portable-pty: ConPTY / openpty).
fn open_local(
    app: &tauri::AppHandle,
    id: &str,
    spec: TerminalSpec,
    cols: u16,
    rows: u16,
    rx: mpsc::UnboundedReceiver<TermCmd>,
    terms: TermMap,
) -> AppResult<()> {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| AppError::IoError(format!("Cannot open pty: {}", e)))?;

    let cmd = match &spec {
        TerminalSpec::HostDir(dir) => {
            let mut c = CommandBuilder::new(default_shell());
            // "/" is the browser's start page, not a real cwd on Windows.
            if !dir.is_empty() && !(cfg!(windows) && dir == "/") {
                c.cwd(dir);
            }
            c
        }
        TerminalSpec::Command(argv) => {
            let (prog, args) = argv
                .split_first()
                .ok_or_else(|| AppError::IoError("Empty terminal command".into()))?;
            let mut c = CommandBuilder::new(prog);
            c.args(args);
            c
        }
    };
    log::info!("Terminal {} local: {:?}", id, spec);

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| AppError::IoError(format!("Cannot start shell: {}", e)))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::IoError(format!("Pty reader error: {}", e)))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::IoError(format!("Pty writer error: {}", e)))?;
    let mut killer = child.clone_killer();
    let master = pair.master;

    // Output pump thread (blocking reads).
    {
        let app = app.clone();
        let id = id.to_string();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = app.emit(
                            "terminal:data",
                            DataEvent {
                                id: &id,
                                data: b64(&buf[..n]),
                            },
                        );
                    }
                }
            }
        });
    }

    // Exit watcher: reports when the shell process ends.
    {
        let app = app.clone();
        let id = id.to_string();
        std::thread::spawn(move || {
            let _ = child.wait();
            finish(&app, &terms, &id);
        });
    }

    // Control thread: input/resize/close (blocking writes).
    {
        let mut rx = rx;
        std::thread::spawn(move || {
            while let Some(cmd) = rx.blocking_recv() {
                match cmd {
                    TermCmd::Input(data) => {
                        if std::io::Write::write_all(&mut writer, &data).is_err() {
                            break;
                        }
                        let _ = std::io::Write::flush(&mut writer);
                    }
                    TermCmd::Resize(cols, rows) => {
                        let _ = master.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                    TermCmd::Close => break,
                }
            }
            let _ = killer.kill();
        });
    }

    Ok(())
}
