use tauri::State;

use crate::error::AppResult;
use crate::ssh::session::{ConnectParams, SessionManager};

#[tauri::command]
pub async fn connect(
    params: ConnectParams,
    session_manager: State<'_, SessionManager>,
) -> AppResult<String> {
    session_manager.connect(params).await
}

#[tauri::command]
pub async fn disconnect(
    session_id: String,
    session_manager: State<'_, SessionManager>,
) -> AppResult<()> {
    session_manager.disconnect(&session_id).await
}
