use std::sync::Arc;

use russh::keys::key::PrivateKeyWithHashAlg;
use russh::keys::*;

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

/// Establish an authenticated SSH connection.
pub async fn connect_ssh(params: &ConnectParams) -> AppResult<Arc<SshHandle>> {
    let config = Arc::new(russh::client::Config::default());
    let handler = ClientHandler;

    let mut session = russh::client::connect(config, (&*params.host, params.port), handler)
        .await
        .map_err(|e| AppError::ConnectionFailed(e.to_string()))?;

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

    Ok(Arc::new(session))
}
