//! Interactive terminals attached to a session: a PTY shell on the
//! local machine or over SSH, auto-attaching into containers/pods when
//! the browsed path lives inside one. Output is streamed to the
//! frontend as base64 `terminal:data` events; input/resize/close come
//! back through commands.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

/// Forward terminal output to the frontend.
fn emit_data(app: &tauri::AppHandle, id: &str, data: &[u8]) {
    let _ = app.emit(
        "terminal:data",
        DataEvent {
            id,
            data: b64(data),
        },
    );
}

/// Upper bound on each start-up step. Without it a connection that died
/// silently would leave the setup task (and its channel) around forever.
const SETUP_TIMEOUT: Duration = Duration::from_secs(60);

/// Bounds delivery of channel-close and connection-disconnect messages.
const CHANNEL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the output pump is given to notice a channel close before it
/// is dropped. It only forwards events, so nothing is lost by dropping it.
const READER_TIMEOUT: Duration = Duration::from_secs(5);

/// Upper bound on a whole teardown before its dedicated SSH connection is
/// invalidated and both pumps are aborted.
const TEARDOWN_TIMEOUT: Duration = Duration::from_secs(15);

type SshChannel = russh::Channel<russh::client::Msg>;

struct SshTerminal {
    channel: SshChannel,
    connection: crate::ssh::client::TerminalSshConnection,
}

fn timed_out(what: &str) -> AppError {
    AppError::IoError(format!("Timed out {}", what))
}

/// Terminals use a dedicated SSH connection. If orderly channel cleanup
/// stalls, dropping this connection is therefore safe and cannot disrupt
/// SFTP, transfers, or another terminal.
async fn disconnect_terminal_connection(connection: crate::ssh::client::TerminalSshConnection) {
    let result = tokio::time::timeout(
        DISCONNECT_TIMEOUT,
        connection
            .handle()
            .disconnect(russh::Disconnect::ByApplication, "terminal closed", "en"),
    )
    .await;
    if !matches!(result, Ok(Ok(()))) {
        log::warn!("Terminal SSH connection did not accept disconnect; aborting its transport");
    }
    connection.abort();
}

async fn shutdown_ssh_terminal(terminal: SshTerminal) {
    let close = tokio::time::timeout(CHANNEL_CLOSE_TIMEOUT, terminal.channel.close()).await;
    if !matches!(close, Ok(Ok(()))) {
        log::warn!("Terminal SSH channel did not accept close; invalidating its connection");
    }
    disconnect_terminal_connection(terminal.connection).await;
}

fn release_ssh_terminal(terminal: SshTerminal) {
    tokio::spawn(shutdown_ssh_terminal(terminal));
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

type TermMap = Arc<std::sync::Mutex<Registry>>;

/// Tears a terminal down without going through its input path, which may
/// be blocked (a wedged SSH connection, a pty buffer nobody drains).
type Force = Box<dyn FnOnce() + Send>;

struct CancelSetupOnDrop(Option<tokio::sync::watch::Sender<bool>>);

impl CancelSetupOnDrop {
    fn cancel(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(true);
        }
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for CancelSetupOnDrop {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// A registered terminal.
struct LiveTerm {
    /// Attempt-specific token returned by `reserve`. Stale commands and
    /// cleanup from an earlier terminal cannot affect a reused id.
    token: String,
    tx: mpsc::UnboundedSender<TermCmd>,
    rx: Option<mpsc::UnboundedReceiver<TermCmd>>,
    /// Installed once the shell is running. While a terminal is still
    /// starting there is nothing to force: setup is cancelled through
    /// `tx`, which nothing can block.
    force: Option<Force>,
}

#[derive(Default)]
struct Registry {
    live: HashMap<String, LiveTerm>,
}

impl Registry {
    /// Drop `id`, but only while `token` still owns it. Returns whether
    /// the entry was removed.
    fn remove_token(&mut self, id: &str, token: &str) -> bool {
        match self.live.get(id) {
            Some(t) if t.token == token => {
                self.live.remove(id);
                true
            }
            _ => false,
        }
    }

    fn remove_unclaimed_token(&mut self, id: &str, token: &str) {
        let unclaimed = self
            .live
            .get(id)
            .is_some_and(|term| term.token == token && term.rx.is_some());
        if unclaimed {
            self.live.remove(id);
        }
    }

    /// Attach the forced teardown to a terminal that just started. If the
    /// id is no longer ours the teardown is handed back, for the caller to
    /// run at once: the close that took the entry away could not.
    fn install_force(&mut self, id: &str, token: &str, force: Force) -> Option<Force> {
        match self.live.get_mut(id) {
            Some(t) if t.token == token => {
                t.force = Some(force);
                None
            }
            _ => Some(force),
        }
    }
}

/// Lock the registry, recovering a poisoned mutex: every operation on it
/// is a single insert, removal or field assignment, so no panic can leave
/// it half-updated.
fn lock_terms(terms: &TermMap) -> std::sync::MutexGuard<'_, Registry> {
    terms.lock().unwrap_or_else(|e| e.into_inner())
}

/// All live terminals, keyed by terminal id.
#[derive(Default)]
pub struct TerminalManager {
    terms: TermMap,
}

/// A terminal id claimed by `reserve`, holding its command receiver until
/// the terminal starts. Dropping it before that unregisters the id, so a
/// failed or abandoned open never leaves a stale entry behind.
pub struct TerminalSlot {
    id: String,
    token: String,
    rx: Option<mpsc::UnboundedReceiver<TermCmd>>,
    terms: TermMap,
    started: bool,
}

impl Drop for TerminalSlot {
    fn drop(&mut self) {
        if !self.started {
            lock_terms(&self.terms).remove_token(&self.id, &self.token);
        }
    }
}

/// Reports a terminal as gone exactly once, whichever shutdown path gets
/// there first (the shell exiting, or a forced teardown).
#[derive(Clone)]
struct ExitGuard {
    app: tauri::AppHandle,
    terms: TermMap,
    id: String,
    token: String,
    done: Arc<AtomicBool>,
}

impl ExitGuard {
    fn new(app: &tauri::AppHandle, terms: &TermMap, id: &str, token: &str) -> Self {
        Self {
            app: app.clone(),
            terms: terms.clone(),
            id: id.to_string(),
            token: token.to_string(),
            done: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Emit the exit event and drop the terminal from the registry.
    fn finish(&self) {
        if self.done.swap(true, Ordering::SeqCst) {
            return;
        }
        // A newer terminal may have taken this id over after ours was
        // closed; it must neither be evicted nor reported as exited.
        let superseded = {
            let mut reg = lock_terms(&self.terms);
            !reg.remove_token(&self.id, &self.token) && reg.live.contains_key(&self.id)
        };
        if !superseded {
            let _ = self.app.emit("terminal:exit", ExitEvent { id: &self.id });
        }
    }
}

impl TerminalManager {
    /// Reserve an id before opening. The returned token must accompany
    /// every later command, making stale commands harmless.
    pub fn reserve(&self, terminal_id: &str) -> AppResult<String> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut reg = lock_terms(&self.terms);
        if reg.live.contains_key(terminal_id) {
            return Err(AppError::IoError(format!(
                "Terminal {} already exists",
                terminal_id
            )));
        }
        let token = uuid::Uuid::new_v4().to_string();
        reg.live.insert(
            terminal_id.to_string(),
            LiveTerm {
                token: token.clone(),
                tx,
                rx: Some(rx),
                force: None,
            },
        );
        let terms = self.terms.clone();
        let id = terminal_id.to_string();
        let reservation_token = token.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(SETUP_TIMEOUT).await;
            lock_terms(&terms).remove_unclaimed_token(&id, &reservation_token);
        });
        Ok(token)
    }

    /// Take the reserved command receiver for one open attempt.
    pub fn claim(&self, terminal_id: &str, token: &str) -> AppResult<TerminalSlot> {
        let mut reg = lock_terms(&self.terms);
        let term = reg
            .live
            .get_mut(terminal_id)
            .filter(|term| term.token == token)
            .ok_or_else(|| AppError::IoError("Terminal reservation is closed".into()))?;
        let rx = term
            .rx
            .take()
            .ok_or_else(|| AppError::IoError("Terminal reservation was already used".into()))?;
        Ok(TerminalSlot {
            id: terminal_id.to_string(),
            token: token.to_string(),
            rx: Some(rx),
            terms: self.terms.clone(),
            started: false,
        })
    }

    /// Open a terminal for `path` in the given session, under the id
    /// claimed by `slot`. A close during setup cancels it; output arrives
    /// as `terminal:data` events.
    pub async fn open(
        &self,
        app: tauri::AppHandle,
        session: Arc<Mutex<RemoteSession>>,
        mut slot: TerminalSlot,
        path: &str,
        cols: u16,
        rows: u16,
    ) -> AppResult<()> {
        let terminal_id = slot.id.clone();
        let token = slot.token.clone();
        let mut rx = slot
            .rx
            .take()
            .ok_or_else(|| AppError::IoError("Terminal slot was already used".into()))?;
        while let Ok(cmd) = rx.try_recv() {
            if matches!(cmd, TermCmd::Close) {
                return Err(AppError::IoError(
                    "Terminal was closed before it started".into(),
                ));
            }
        }

        // Setup runs as its own task and this command only waits for it:
        // the task always finishes and closes whatever it built if nobody
        // is waiting any more, so a cancelled or timed-out open never
        // leaves an SSH channel behind.
        let (done_tx, mut done_rx) = tokio::sync::oneshot::channel();
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let setup_task = {
            let id = terminal_id.clone();
            let path = path.to_string();
            let app = app.clone();
            tokio::spawn(async move {
                // One deadline for the whole start-up: per-step timeouts
                // would let the steps add up to a multiple of it.
                let deadline = tokio::time::Instant::now() + SETUP_TIMEOUT;
                let started = setup(SetupRequest {
                    app,
                    id,
                    session,
                    path,
                    cols,
                    rows,
                    deadline,
                    cancel: cancel_rx,
                })
                .await;
                if let Err(Ok(Started::Ssh(terminal))) = done_tx.send(started) {
                    // Nobody is waiting any more: release what we opened.
                    release_ssh_terminal(terminal);
                }
            })
        };
        drop(setup_task);
        let mut cancel_setup = CancelSetupOnDrop(Some(cancel_tx));

        let outcome = tokio::select! {
            r = &mut done_rx => match r {
                Ok(started) => Outcome::Ready(started),
                Err(_) => Outcome::Ready(Err(AppError::IoError("Terminal setup was dropped".into()))),
            },
            _ = wait_close(&mut rx) => Outcome::Abandoned(AppError::IoError(
                "Terminal was closed before it started".into(),
            )),
            // Backstop only: setup is bounded by its own deadline, so this
            // fires solely if that task never gets scheduled.
            _ = tokio::time::sleep(SETUP_TIMEOUT + TEARDOWN_TIMEOUT) => {
                Outcome::Abandoned(timed_out("starting the terminal"))
            }
        };

        let started = match outcome {
            Outcome::Ready(started) => {
                cancel_setup.disarm();
                started?
            }
            Outcome::Abandoned(e) => {
                cancel_setup.cancel();
                discard_started(done_rx);
                // Dropping the slot unregisters the id for us.
                return Err(e);
            }
        };

        let force = match started {
            Started::Ssh(terminal) => {
                spawn_ssh_pumps(&app, &terminal_id, &token, terminal, rx, self.terms.clone())
            }
            Started::Local(spec) => open_local(
                &app,
                &terminal_id,
                &token,
                spec,
                cols,
                rows,
                rx,
                self.terms.clone(),
            )?,
        };
        // The terminal owns its registry entry from here on.
        slot.started = true;
        // A close that landed while the shell was starting already took the
        // entry away, and could not force a terminal that did not exist yet.
        let orphaned = lock_terms(&self.terms).install_force(&terminal_id, &token, force);
        if let Some(force) = orphaned {
            force();
        }
        Ok(())
    }

    async fn send(&self, terminal_id: &str, token: &str, cmd: TermCmd) -> AppResult<()> {
        let reg = lock_terms(&self.terms);
        let term = reg
            .live
            .get(terminal_id)
            .filter(|term| term.token == token)
            .ok_or_else(|| AppError::IoError("Terminal is closed".into()))?;
        term.tx
            .send(cmd)
            .map_err(|_| AppError::IoError("Terminal is closed".into()))
    }

    pub async fn input(&self, terminal_id: &str, token: &str, data: Vec<u8>) -> AppResult<()> {
        self.send(terminal_id, token, TermCmd::Input(data)).await
    }

    pub async fn resize(
        &self,
        terminal_id: &str,
        token: &str,
        cols: u16,
        rows: u16,
    ) -> AppResult<()> {
        self.send(terminal_id, token, TermCmd::Resize(cols, rows))
            .await
    }

    /// Close one reserved terminal attempt. Stale closes are idempotent.
    pub async fn close(&self, terminal_id: &str, token: &str) -> AppResult<()> {
        let term = {
            let mut reg = lock_terms(&self.terms);
            match reg.live.get(terminal_id) {
                Some(term) if term.token == token => reg.live.remove(terminal_id),
                _ => None,
            }
        };
        if let Some(term) = term {
            // Ask for a clean shutdown first, then make it happen within a
            // bounded time even if the terminal's input path is blocked.
            // Dropping the sender is itself a shutdown signal for a
            // terminal that is still starting.
            let _ = term.tx.send(TermCmd::Close);
            drop(term.tx);
            if let Some(force) = term.force {
                force();
            }
        }
        Ok(())
    }
}

/// What the start-up race in `open` produced.
enum Outcome {
    /// Setup finished (successfully or not) before anything cancelled it.
    Ready(AppResult<Started>),
    /// Nobody wants the terminal any more; whatever setup produces must
    /// be released rather than used.
    Abandoned(AppError),
}

/// A terminal whose (cancellable) setup phase finished.
enum Started {
    Ssh(SshTerminal),
    Local(TerminalSpec),
}

/// Work out what to run for `path` and, for a remote session, get a PTY
/// channel for it. Bounded as a whole by `deadline`.
struct SetupRequest {
    app: tauri::AppHandle,
    id: String,
    session: Arc<Mutex<RemoteSession>>,
    path: String,
    cols: u16,
    rows: u16,
    deadline: tokio::time::Instant,
    cancel: tokio::sync::watch::Receiver<bool>,
}

async fn setup(request: SetupRequest) -> AppResult<Started> {
    let SetupRequest {
        app,
        id,
        session,
        path,
        cols,
        rows,
        deadline,
        mut cancel,
    } = request;
    let (fs, ssh_params) = {
        let s = session.lock().await;
        (s.fs.clone(), s.ssh.as_ref().map(|_| s.params.clone()))
    };
    // Container discovery can use the shared session. Keep it in an owned
    // task so cancellation or timeout never drops a local child or SSH exec
    // channel mid-operation; a late result is simply discarded.
    let (spec_tx, mut spec_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = spec_tx.send(fs.terminal_spec(&path).await);
    });
    let spec = match tokio::time::timeout_at(deadline, &mut spec_rx).await {
        Ok(Ok(result)) => result?,
        Ok(Err(_)) => return Err(AppError::IoError("Terminal preparation was dropped".into())),
        Err(_) => {
            tokio::spawn(async move {
                let _ = spec_rx.await;
            });
            return Err(timed_out("preparing the terminal"));
        }
    };
    if *cancel.borrow() {
        return Err(AppError::IoError(
            "Terminal was closed before it started".into(),
        ));
    }
    match ssh_params {
        Some(params) => tokio::select! {
            result = async {
                let connection = tokio::time::timeout_at(
                deadline,
                crate::ssh::client::connect_terminal_ssh(&params),
            )
            .await
            .map_err(|_| timed_out("connecting the terminal"))??;
                Ok(Started::Ssh(
                ssh_setup(&app, &id, connection, &spec, cols, rows, deadline).await?,
                ))
            } => result,
            _ = cancel.changed() => Err(AppError::IoError(
                "Terminal was closed before it started".into(),
            )),
        },
        None => Ok(Started::Local(spec)),
    }
}

/// Resolves once the frontend closes a terminal that is still starting,
/// or once its sender is dropped. Commands that arrive before the shell
/// exists have nowhere to go and are discarded.
async fn wait_close(rx: &mut mpsc::UnboundedReceiver<TermCmd>) {
    while let Some(cmd) = rx.recv().await {
        if matches!(cmd, TermCmd::Close) {
            return;
        }
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

/// Setup of a PTY over SSH (russh request_pty + exec), bounded as a whole
/// by `deadline`. Once the channel exists it is always closed explicitly
/// on failure or timeout.
async fn ssh_setup(
    app: &tauri::AppHandle,
    id: &str,
    connection: crate::ssh::client::TerminalSshConnection,
    spec: &TerminalSpec,
    cols: u16,
    rows: u16,
    deadline: tokio::time::Instant,
) -> AppResult<SshTerminal> {
    let mut terminal = open_channel(connection, deadline).await?;

    let cmd = ssh_command(spec);
    log::info!("Terminal {} over SSH: {}", id, cmd);

    // From here the channel exists: never drop it without closing it.
    let handshake = async {
        terminal
            .channel
            .request_pty(true, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
            .await
            .map_err(|e| AppError::IoError(format!("SSH pty error: {}", e)))?;
        expect_reply(app, id, &mut terminal.channel, "PTY request").await?;
        terminal
            .channel
            .exec(true, cmd.as_bytes())
            .await
            .map_err(|e| AppError::IoError(format!("SSH exec error: {}", e)))?;
        expect_reply(app, id, &mut terminal.channel, "Shell start").await
    };
    let result = match tokio::time::timeout_at(deadline, handshake).await {
        Ok(r) => r,
        Err(_) => Err(timed_out("starting the remote shell")),
    };
    match result {
        Ok(()) => Ok(terminal),
        Err(e) => {
            shutdown_ssh_terminal(terminal).await;
            Err(e)
        }
    }
}

/// Open a channel on a terminal-only connection. On timeout the connection
/// is invalidated, which releases russh's otherwise uncancellable pending
/// channel reservation.
async fn open_channel(
    connection: crate::ssh::client::TerminalSshConnection,
    deadline: tokio::time::Instant,
) -> AppResult<SshTerminal> {
    match tokio::time::timeout_at(deadline, connection.handle().channel_open_session()).await {
        Ok(Ok(channel)) => Ok(SshTerminal {
            channel,
            connection,
        }),
        Ok(Err(e)) => Err(AppError::IoError(format!("SSH channel error: {}", e))),
        Err(_) => {
            disconnect_terminal_connection(connection).await;
            Err(timed_out("opening the SSH channel"))
        }
    }
}

/// Release a setup result that nobody is waiting for any more.
///
/// The receiver is moved here and awaited rather than polled and dropped:
/// polling cannot observe a send that is completing concurrently, and
/// dropping the receiver then lets that send succeed into a value nobody
/// ever reads, leaking the SSH channel inside it. Setup is bounded by its
/// own deadline, so this task ends with it.
fn discard_started(rx: tokio::sync::oneshot::Receiver<AppResult<Started>>) {
    tokio::spawn(async move {
        if let Ok(Ok(Started::Ssh(terminal))) = rx.await {
            shutdown_ssh_terminal(terminal).await;
        }
    });
}

/// Await the server's reply to a channel request. Output that arrives
/// before the reply is forwarded rather than dropped.
async fn expect_reply(
    app: &tauri::AppHandle,
    id: &str,
    channel: &mut SshChannel,
    what: &str,
) -> AppResult<()> {
    loop {
        match channel.wait().await {
            Some(russh::ChannelMsg::Success) => return Ok(()),
            Some(russh::ChannelMsg::Failure) => {
                return Err(AppError::IoError(format!(
                    "{} was refused by the server",
                    what
                )))
            }
            Some(russh::ChannelMsg::Data { data }) => emit_data(app, id, &data),
            Some(russh::ChannelMsg::ExtendedData { data, ext: 1 }) => emit_data(app, id, &data),
            Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => {
                return Err(AppError::IoError(format!(
                    "SSH channel closed during {}",
                    what.to_lowercase()
                )))
            }
            Some(_) => continue,
        }
    }
}

/// Start the output/input pumps for an established SSH terminal, and
/// return the teardown that forces them down.
fn spawn_ssh_pumps(
    app: &tauri::AppHandle,
    id: &str,
    token: &str,
    terminal: SshTerminal,
    mut rx: mpsc::UnboundedReceiver<TermCmd>,
    terms: TermMap,
) -> Force {
    let SshTerminal {
        channel,
        connection,
    } = terminal;
    let (mut read_half, write_half) = channel.split();
    let write_half = Arc::new(write_half);
    let exit = ExitGuard::new(app, &terms, id, token);
    // Reaches the input pump even while it is stuck writing to a stalled
    // connection, which a queued `TermCmd::Close` would not.
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();

    // Output pump: channel messages -> frontend events.
    let reader = {
        let app = app.clone();
        let id = id.to_string();
        let exit = exit.clone();
        tokio::spawn(async move {
            while let Some(msg) = read_half.wait().await {
                match msg {
                    russh::ChannelMsg::Data { data } => emit_data(&app, &id, &data),
                    russh::ChannelMsg::ExtendedData { data, ext: 1 } => emit_data(&app, &id, &data),
                    _ => {}
                }
            }
            exit.finish();
        })
    };
    let reader_abort = reader.abort_handle();

    // Input pump: frontend commands -> channel.
    let writer = {
        let exit = exit.clone();
        let reader_abort = reader_abort.clone();
        let writer_connection = connection.clone();
        tokio::spawn(async move {
            loop {
                let cmd = tokio::select! {
                    cmd = rx.recv() => match cmd {
                        Some(cmd) => cmd,
                        None => break,
                    },
                    _ = &mut cancel_rx => break,
                };
                match cmd {
                    // Race every write against the cancel: flow control on
                    // a stalled connection must not swallow the teardown.
                    TermCmd::Input(data) => tokio::select! {
                        r = write_half.data(&data[..]) => if r.is_err() { break },
                        _ = &mut cancel_rx => break,
                    },
                    TermCmd::Resize(cols, rows) => tokio::select! {
                        _ = write_half.window_change(cols as u32, rows as u32, 0, 0) => {}
                        _ = &mut cancel_rx => break,
                    },
                    TermCmd::Close => break,
                }
            }
            let close = tokio::time::timeout(CHANNEL_CLOSE_TIMEOUT, write_half.close()).await;
            drop(write_half);
            let close_enqueued = matches!(close, Ok(Ok(())));
            let close_confirmed = close_enqueued
                && matches!(
                    tokio::time::timeout(READER_TIMEOUT, reader).await,
                    Ok(Ok(()))
                );
            if !close_confirmed {
                reader_abort.abort();
            }
            // Disconnect is the escalation path for an unconfirmed close.
            // It is harmless on the dedicated connection after a clean one.
            disconnect_terminal_connection(writer_connection).await;
            exit.finish();
        })
    };
    let writer_abort = writer.abort_handle();

    Box::new(move || {
        let _ = cancel_tx.send(());
        tokio::spawn(async move {
            if tokio::time::timeout(TEARDOWN_TIMEOUT, writer)
                .await
                .is_err()
            {
                writer_abort.abort();
            }
            reader_abort.abort();
            disconnect_terminal_connection(connection).await;
            exit.finish();
        });
    })
}

#[cfg(windows)]
fn default_shell() -> String {
    "powershell.exe".to_string()
}

#[cfg(not(windows))]
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// PTY shell on this machine (portable-pty: ConPTY / openpty). Returns the
/// teardown that forces the shell down.
fn open_local(
    app: &tauri::AppHandle,
    id: &str,
    token: &str,
    spec: TerminalSpec,
    cols: u16,
    rows: u16,
    rx: mpsc::UnboundedReceiver<TermCmd>,
    terms: TermMap,
) -> AppResult<Force> {
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
            // Browse paths are posix-style ("/C:/Users/..." on Windows) and
            // need a native cwd. An empty path (virtual listing level) and
            // the Windows virtual root "/" (the drive list) are not real
            // directories: start in the user's home, like the SSH shells do.
            let virtual_root =
                dir.is_empty() || (cfg!(windows) && dir.trim_matches('/').is_empty());
            let cwd = if virtual_root {
                None
            } else {
                Some(crate::fs::local::native_path(dir)).filter(|p| p.is_dir())
            };
            match cwd {
                Some(p) => c.cwd(p),
                None => {
                    if let Some(home) = dirs::home_dir() {
                        c.cwd(home);
                    }
                }
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
    let mut force_killer = child.clone_killer();
    let master = pair.master;
    let exit = ExitGuard::new(app, &terms, id, token);

    // Output pump thread (blocking reads).
    {
        let app = app.clone();
        let id = id.to_string();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut reader, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => emit_data(&app, &id, &buf[..n]),
                }
            }
        });
    }

    // Exit watcher: reports when the shell process ends.
    {
        let exit = exit.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            exit.finish();
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

    Ok(Box::new(move || {
        // Kill from here rather than through the control thread: a write
        // into a pty buffer nobody drains blocks that thread, and killing
        // the shell is exactly what makes the write fail and unblock it.
        let _ = force_killer.kill();
    }))
}
