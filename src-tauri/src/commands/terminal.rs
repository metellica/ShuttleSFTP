use base64::Engine;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::ssh::SessionManager;
use crate::terminal::TerminalManager;

/// Open an interactive terminal at `path` in a session. Container and
/// pod paths auto-attach (docker/nerdctl/crictl/kubectl exec).
#[tauri::command]
pub async fn terminal_open(
    session_id: String,
    path: String,
    cols: u16,
    rows: u16,
    app: tauri::AppHandle,
    session_manager: State<'_, SessionManager>,
    terminals: State<'_, TerminalManager>,
) -> AppResult<String> {
    let session = session_manager.get_session(&session_id).await?;
    terminals.open(app, session, &path, cols, rows).await
}

/// Feed keyboard input (base64-encoded bytes) to a terminal.
#[tauri::command]
pub async fn terminal_input(
    terminal_id: String,
    data: String,
    terminals: State<'_, TerminalManager>,
) -> AppResult<()> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| AppError::IoError(format!("Invalid terminal input: {}", e)))?;
    terminals.input(&terminal_id, bytes).await
}

#[tauri::command]
pub async fn terminal_resize(
    terminal_id: String,
    cols: u16,
    rows: u16,
    terminals: State<'_, TerminalManager>,
) -> AppResult<()> {
    terminals.resize(&terminal_id, cols, rows).await
}

#[tauri::command]
pub async fn terminal_close(
    terminal_id: String,
    terminals: State<'_, TerminalManager>,
) -> AppResult<()> {
    terminals.close(&terminal_id).await
}
