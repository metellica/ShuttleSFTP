use tauri::State;

use crate::container::{ContainerInfo, PodInfo};
use crate::error::AppResult;
use crate::ssh::session::{
    ConnectParams, ContainerConnectSpec, PodConnectSpec, SessionManager,
};

#[tauri::command]
pub async fn connect(
    params: ConnectParams,
    session_manager: State<'_, SessionManager>,
) -> AppResult<String> {
    session_manager.connect(params).await
}

#[tauri::command]
pub async fn connect_container(
    spec: ContainerConnectSpec,
    session_manager: State<'_, SessionManager>,
) -> AppResult<String> {
    session_manager.connect_container(spec).await
}

#[tauri::command]
pub async fn connect_pod(
    spec: PodConnectSpec,
    session_manager: State<'_, SessionManager>,
) -> AppResult<String> {
    session_manager.connect_pod(spec).await
}

#[tauri::command]
pub async fn disconnect(
    session_id: String,
    session_manager: State<'_, SessionManager>,
) -> AppResult<()> {
    session_manager.disconnect(&session_id).await
}

/// List running containers on the local machine, an existing session's
/// host, or a new SSH host.
#[tauri::command]
pub async fn list_containers(
    via_session_id: Option<String>,
    via: Option<ConnectParams>,
    session_manager: State<'_, SessionManager>,
) -> AppResult<Vec<ContainerInfo>> {
    let (runner, _, _) = session_manager
        .host_runner(via_session_id.as_deref(), via.as_ref())
        .await?;
    crate::container::list_containers(runner.as_ref()).await
}

#[tauri::command]
pub async fn list_kube_contexts(
    via_session_id: Option<String>,
    via: Option<ConnectParams>,
    session_manager: State<'_, SessionManager>,
) -> AppResult<Vec<String>> {
    let (runner, _, _) = session_manager
        .host_runner(via_session_id.as_deref(), via.as_ref())
        .await?;
    crate::container::kube_contexts(runner.as_ref()).await
}

#[tauri::command]
pub async fn list_kube_namespaces(
    context: Option<String>,
    via_session_id: Option<String>,
    via: Option<ConnectParams>,
    session_manager: State<'_, SessionManager>,
) -> AppResult<Vec<String>> {
    let (runner, _, _) = session_manager
        .host_runner(via_session_id.as_deref(), via.as_ref())
        .await?;
    crate::container::kube_namespaces(runner.as_ref(), context.as_deref()).await
}

#[tauri::command]
pub async fn list_kube_pods(
    namespace: String,
    context: Option<String>,
    via_session_id: Option<String>,
    via: Option<ConnectParams>,
    session_manager: State<'_, SessionManager>,
) -> AppResult<Vec<PodInfo>> {
    let (runner, _, _) = session_manager
        .host_runner(via_session_id.as_deref(), via.as_ref())
        .await?;
    crate::container::kube_pods(runner.as_ref(), context.as_deref(), &namespace).await
}
