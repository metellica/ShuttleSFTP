use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use russh::keys::key::PrivateKeyWithHashAlg;
use russh::keys::*;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::error::{AppError, AppResult};
use crate::ssh::auth::AuthMethod;
use crate::ssh::session::ConnectParams;

/// Minimal russh client handler.
pub struct ClientHandler;

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

pub type SshHandle = russh::client::Handle<ClientHandler>;

#[derive(Clone)]
struct TransportAbort {
    inner: Arc<TransportAbortInner>,
}

struct TransportAbortInner {
    aborted: AtomicBool,
    read_waker: std::sync::Mutex<Option<Waker>>,
    write_waker: std::sync::Mutex<Option<Waker>>,
}

impl TransportAbort {
    fn new() -> Self {
        Self {
            inner: Arc::new(TransportAbortInner {
                aborted: AtomicBool::new(false),
                read_waker: std::sync::Mutex::new(None),
                write_waker: std::sync::Mutex::new(None),
            }),
        }
    }

    fn abort(&self) {
        if self.inner.aborted.swap(true, Ordering::SeqCst) {
            return;
        }
        for slot in [&self.inner.read_waker, &self.inner.write_waker] {
            if let Some(waker) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
                waker.wake();
            }
        }
    }

    fn check(&self) -> io::Result<()> {
        if self.inner.aborted.load(Ordering::SeqCst) {
            Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "SSH terminal transport aborted",
            ))
        } else {
            Ok(())
        }
    }

    fn register(&self, slot: &std::sync::Mutex<Option<Waker>>, cx: &Context<'_>) -> io::Result<()> {
        self.check()?;
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
        self.check()
    }
}

struct KillableTcpStream {
    stream: tokio::net::TcpStream,
    abort: TransportAbort,
}

impl AsyncRead for KillableTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(e) = self.abort.register(&self.abort.inner.read_waker, cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for KillableTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(e) = self.abort.register(&self.abort.inner.write_waker, cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(e) = self.abort.register(&self.abort.inner.write_waker, cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(e) = self.abort.register(&self.abort.inner.write_waker, cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

struct AbortOnDrop(Option<TransportAbort>);

impl AbortOnDrop {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(abort) = self.0.take() {
            abort.abort();
        }
    }
}

#[derive(Clone)]
pub struct TerminalSshConnection {
    inner: Arc<TerminalSshConnectionInner>,
}

struct TerminalSshConnectionInner {
    handle: Arc<SshHandle>,
    abort: TransportAbort,
}

impl Drop for TerminalSshConnectionInner {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

impl TerminalSshConnection {
    pub fn handle(&self) -> &SshHandle {
        &self.inner.handle
    }

    pub fn abort(&self) {
        self.inner.abort.abort();
    }
}

/// Detect a peer that stopped answering. Without this russh's session loop
/// waits forever on a wedged connection, and every future that depends on
/// it — channel opens, writes, closes, reads — waits with it, so cleanup
/// could never be bounded.
const KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
const KEEPALIVE_MAX: usize = 3;

/// Establish an authenticated SSH connection.
pub async fn connect_ssh(params: &ConnectParams) -> AppResult<Arc<SshHandle>> {
    let config = ssh_config();
    let session = russh::client::connect(config, (&*params.host, params.port), ClientHandler)
        .await
        .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;
    authenticate(session, params).await
}

/// Establish an SSH connection whose transport can be forcibly aborted.
/// Interactive terminals use this so timeout cleanup does not depend on
/// russh's bounded command queue or the operating system's TCP timeout.
pub async fn connect_terminal_ssh(params: &ConnectParams) -> AppResult<TerminalSshConnection> {
    let config = ssh_config();
    let socket = tokio::net::TcpStream::connect((&*params.host, params.port))
        .await
        .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;
    if config.nodelay {
        socket
            .set_nodelay(true)
            .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;
    }
    let abort = TransportAbort::new();
    let abort_on_drop = AbortOnDrop(Some(abort.clone()));
    let stream = KillableTcpStream {
        stream: socket,
        abort: abort.clone(),
    };
    let session = russh::client::connect_stream(config, stream, ClientHandler)
        .await
        .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;
    let handle = authenticate(session, params).await?;
    abort_on_drop.disarm();
    Ok(TerminalSshConnection {
        inner: Arc::new(TerminalSshConnectionInner { handle, abort }),
    })
}

fn ssh_config() -> Arc<russh::client::Config> {
    Arc::new(russh::client::Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: KEEPALIVE_MAX,
        ..Default::default()
    })
}

async fn authenticate(mut session: SshHandle, params: &ConnectParams) -> AppResult<Arc<SshHandle>> {
    let auth_result = match &params.auth {
        AuthMethod::Password { password } => session
            .authenticate_password(&params.username, password)
            .await
            .map_err(|e| AppError::AuthFailed(e.to_string()))?,
        AuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => {
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
            return Err(AppError::AuthFailed(
                "Agent auth not yet implemented".into(),
            ));
        }
    };

    if !auth_result.success() {
        return Err(AppError::AuthFailed(
            "Authentication rejected by server".into(),
        ));
    }

    Ok(Arc::new(session))
}
