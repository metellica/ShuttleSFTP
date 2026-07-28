use base64::Engine;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::ssh::SessionManager;
use crate::terminal::TerminalManager;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOpenRequest {
    terminal_id: String,
    terminal_token: String,
    session_id: String,
    path: String,
    cols: u16,
    rows: u16,
}

/// Reserve a terminal id and return its attempt-specific command token.
#[tauri::command]
pub fn terminal_reserve(
    terminal_id: String,
    terminals: State<'_, TerminalManager>,
) -> AppResult<String> {
    terminals.reserve(&terminal_id)
}

/// Open an interactive terminal at `path` using an acknowledged reservation.
/// Container and pod paths auto-attach (docker/nerdctl/crictl/kubectl exec).
#[tauri::command]
pub async fn terminal_open(
    request: TerminalOpenRequest,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    terminals: State<'_, TerminalManager>,
) -> AppResult<()> {
    let slot = terminals.claim(&request.terminal_id, &request.terminal_token)?;
    let session = session_manager.get_session(&request.session_id).await?;
    terminals
        .open(
            app,
            session,
            slot,
            &request.path,
            request.cols,
            request.rows,
        )
        .await
}

/// Feed keyboard input (base64-encoded bytes) to a terminal.
#[tauri::command]
pub async fn terminal_input(
    terminal_id: String,
    terminal_token: String,
    data: String,
    terminals: State<'_, TerminalManager>,
) -> AppResult<()> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| AppError::IoError(format!("Invalid terminal input: {}", e)))?;
    terminals.input(&terminal_id, &terminal_token, bytes).await
}

#[tauri::command]
pub async fn terminal_resize(
    terminal_id: String,
    terminal_token: String,
    cols: u16,
    rows: u16,
    terminals: State<'_, TerminalManager>,
) -> AppResult<()> {
    terminals
        .resize(&terminal_id, &terminal_token, cols, rows)
        .await
}

#[tauri::command]
pub async fn terminal_close(
    terminal_id: String,
    terminal_token: String,
    terminals: State<'_, TerminalManager>,
) -> AppResult<()> {
    terminals.close(&terminal_id, &terminal_token).await
}
