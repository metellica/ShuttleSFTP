//! HostFs: presents a host's real file system plus virtual directories
//! exposing the containers and K8s pods reachable from that host:
//!
//! ```text
//! /                    real fs (SFTP or local)
//! /@containers         running containers (docker / nerdctl / crictl)
//! /@containers/<name>/...      files inside that container
//! /@pods               kubectl namespaces
//! /@pods/<ns>/<pod>/<container>/...   files inside that pod container
//! ```
//!
//! Container access prefers direct rootfs-over-host (works for distroless
//! images, needs an SSH leg) and falls back to exec + shell tools.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::container::{
    resolve_rootfs, ContainerInfo, ExecFs, ExecTarget, RuntimeKind,
};
use crate::error::{AppError, AppResult};
use crate::exec::CommandRunner;
use crate::fs::prefix::PrefixFs;
use crate::fs::{FileEntry, FileStat, FsReader, FsWriter, RemoteFs};
use crate::ssh::client::SshHandle;

pub const CONTAINERS_DIR: &str = "@containers";
pub const PODS_DIR: &str = "@pods";

/// Where a path routes to.
enum Route {
    /// The host's real file system.
    Base(String),
    /// The /@containers listing itself.
    ContainersRoot,
    /// A path inside a container: (container name, path inside it).
    Container(String, String),
    /// The /@pods namespace listing.
    PodsRoot,
    /// Namespace level: lists pods.
    PodsNamespace(String),
    /// Pod level: lists its containers.
    Pod(String, String),
    /// A path inside a pod container: (ns, pod, container, inner path).
    PodContainer(String, String, String, String),
}

fn virtual_dir_entry(parent: &str, name: &str) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        path: crate::fs::join_path(parent, name),
        is_dir: true,
        size: 0,
        modified: 0,
        permissions: None,
    }
}

fn dir_stat() -> FileStat {
    FileStat { size: 0, is_dir: true }
}

fn readonly_err(what: &str) -> AppError {
    AppError::IoError(format!(
        "{} is a virtual directory; operations are only possible on files inside it",
        what
    ))
}

/// A container endpoint resolved on demand and cached for the session.
struct ContainerEntry {
    info: ContainerInfo,
    fs: Option<Arc<dyn RemoteFs>>,
}

pub struct HostFs {
    base: Arc<dyn RemoteFs>,
    runner: Arc<dyn CommandRunner>,
    ssh: Option<Arc<SshHandle>>,
    /// Display name -> container entry, filled by listing /@containers.
    containers: Mutex<HashMap<String, ContainerEntry>>,
    /// "ns/pod/container" -> fs, filled on first access.
    pod_fs: Mutex<HashMap<String, Arc<dyn RemoteFs>>>,
}

impl HostFs {
    pub fn new(
        base: Arc<dyn RemoteFs>,
        runner: Arc<dyn CommandRunner>,
        ssh: Option<Arc<SshHandle>>,
    ) -> Self {
        Self {
            base,
            runner,
            ssh,
            containers: Mutex::new(HashMap::new()),
            pod_fs: Mutex::new(HashMap::new()),
        }
    }

    fn route(&self, path: &str) -> Route {
        let trimmed = path.trim_start_matches('/');
        let mut parts = trimmed.split('/').filter(|s| !s.is_empty());
        match parts.next() {
            Some(CONTAINERS_DIR) => match parts.next() {
                None => Route::ContainersRoot,
                Some(name) => {
                    let rest: Vec<&str> = parts.collect();
                    let inner = if rest.is_empty() {
                        "/".to_string()
                    } else {
                        format!("/{}", rest.join("/"))
                    };
                    Route::Container(name.to_string(), inner)
                }
            },
            Some(PODS_DIR) => {
                let ns = parts.next();
                let pod = parts.next();
                let container = parts.next();
                let rest: Vec<&str> = parts.collect();
                match (ns, pod, container) {
                    (None, ..) => Route::PodsRoot,
                    (Some(ns), None, _) => Route::PodsNamespace(ns.to_string()),
                    (Some(ns), Some(pod), None) => Route::Pod(ns.to_string(), pod.to_string()),
                    (Some(ns), Some(pod), Some(c)) => {
                        let inner = if rest.is_empty() {
                            "/".to_string()
                        } else {
                            format!("/{}", rest.join("/"))
                        };
                        Route::PodContainer(ns.to_string(), pod.to_string(), c.to_string(), inner)
                    }
                }
            }
            _ => Route::Base(path.to_string()),
        }
    }

    /// Refresh the container index from the runtimes on the host.
    async fn refresh_containers(&self) -> AppResult<Vec<ContainerInfo>> {
        let list = crate::container::list_containers(self.runner.as_ref()).await?;
        let mut map = self.containers.lock().unwrap();
        // Keep already-resolved fs instances for containers still present.
        let mut fresh: HashMap<String, ContainerEntry> = HashMap::new();
        for info in &list {
            let mut name = if info.name.is_empty() {
                info.id.chars().take(12).collect()
            } else {
                info.name.clone()
            };
            if fresh.contains_key(&name) {
                name = format!("{}-{}", name, info.id.chars().take(12).collect::<String>());
            }
            let fs = map
                .remove(&name)
                .filter(|e| e.info.id == info.id)
                .and_then(|e| e.fs);
            fresh.insert(
                name,
                ContainerEntry {
                    info: info.clone(),
                    fs,
                },
            );
        }
        *map = fresh;
        Ok(list)
    }

    /// The RemoteFs for a named container, resolving and caching it.
    async fn container_fs(&self, name: &str) -> AppResult<Arc<dyn RemoteFs>> {
        // Fast path: cached
        let info = {
            let map = self.containers.lock().unwrap();
            match map.get(name) {
                Some(e) => {
                    if let Some(fs) = &e.fs {
                        return Ok(fs.clone());
                    }
                    Some(e.info.clone())
                }
                None => None,
            }
        };
        // Unknown name (e.g. bookmark straight into a container): refresh
        let info = match info {
            Some(i) => i,
            None => {
                self.refresh_containers().await?;
                let map = self.containers.lock().unwrap();
                map.get(name)
                    .map(|e| e.info.clone())
                    .ok_or_else(|| {
                        AppError::IoError(format!("No running container named '{}'", name))
                    })?
            }
        };

        let fs = self.build_container_fs(&info).await?;
        let mut map = self.containers.lock().unwrap();
        if let Some(e) = map.get_mut(name) {
            e.fs = Some(fs.clone());
        }
        Ok(fs)
    }

    /// Rootfs-over-host when possible (distroless-proof), exec fallback.
    async fn build_container_fs(&self, info: &ContainerInfo) -> AppResult<Arc<dyn RemoteFs>> {
        if self.ssh.is_some() || self.runner.location() == "local" {
            if let Some(prefix) =
                resolve_rootfs(self.runner.as_ref(), info.runtime, &info.id).await
            {
                // Reach the rootfs through the same channel as the base fs
                // (SFTP for SSH hosts, direct fs for local Linux hosts).
                let view: Arc<dyn RemoteFs> =
                    Arc::new(PrefixFs::new(self.base.clone(), prefix.clone()));
                if view.list_dir("/").await.is_ok() {
                    log::info!("Container {} via rootfs {}", info.id, prefix);
                    return Ok(view);
                }
            }
        }
        let target = ExecTarget::container(info.runtime, info.id.clone());
        let fs = ExecFs::new(self.runner.clone(), target);
        fs.probe().await?;
        log::info!(
            "Container {} via {} exec on {}",
            info.id,
            info.runtime.as_str(),
            self.runner.location()
        );
        Ok(Arc::new(fs))
    }

    /// The RemoteFs for a pod container, resolving and caching it.
    async fn pod_container_fs(
        &self,
        ns: &str,
        pod: &str,
        container: &str,
    ) -> AppResult<Arc<dyn RemoteFs>> {
        let key = format!("{}/{}/{}", ns, pod, container);
        if let Some(fs) = self.pod_fs.lock().unwrap().get(&key) {
            return Ok(fs.clone());
        }
        let target = ExecTarget::pod(
            None,
            ns.to_string(),
            pod.to_string(),
            Some(container.to_string()),
        );
        let fs = ExecFs::new(self.runner.clone(), target);
        fs.probe().await?;
        let fs: Arc<dyn RemoteFs> = Arc::new(fs);
        self.pod_fs.lock().unwrap().insert(key, fs.clone());
        Ok(fs)
    }

    /// Resolve any routed path to (fs, inner path). Virtual listing levels
    /// return an error naming the operation.
    async fn resolve(&self, path: &str, op: &str) -> AppResult<(Arc<dyn RemoteFs>, String)> {
        match self.route(path) {
            Route::Base(p) => Ok((self.base.clone(), p)),
            Route::Container(name, inner) => {
                Ok((self.container_fs(&name).await?, inner))
            }
            Route::PodContainer(ns, pod, c, inner) => {
                Ok((self.pod_container_fs(&ns, &pod, &c).await?, inner))
            }
            _ => Err(readonly_err(op)),
        }
    }
}

#[async_trait]
impl RemoteFs for HostFs {
    fn kind(&self) -> &'static str {
        "host"
    }

    fn supports_resume_at(&self, path: &str) -> bool {
        match self.route(path) {
            Route::Base(p) => self.base.supports_resume_at(&p),
            Route::Container(name, inner) => {
                let map = self.containers.lock().unwrap();
                map.get(&name)
                    .and_then(|e| e.fs.as_ref())
                    .map(|fs| fs.supports_resume_at(&inner))
                    .unwrap_or(false)
            }
            Route::PodContainer(ns, pod, c, inner) => {
                let key = format!("{}/{}/{}", ns, pod, c);
                let map = self.pod_fs.lock().unwrap();
                map.get(&key)
                    .map(|fs| fs.supports_resume_at(&inner))
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    fn server_read_cmd(&self, path: &str) -> Option<String> {
        match self.route(path) {
            Route::Base(p) => self.base.server_read_cmd(&p),
            Route::Container(name, inner) => {
                let map = self.containers.lock().unwrap();
                let e = map.get(&name)?;
                match &e.fs {
                    Some(fs) => fs.server_read_cmd(&inner),
                    None => None,
                }
            }
            Route::PodContainer(ns, pod, c, inner) => {
                let key = format!("{}/{}/{}", ns, pod, c);
                let map = self.pod_fs.lock().unwrap();
                map.get(&key)?.server_read_cmd(&inner)
            }
            _ => None,
        }
    }

    fn server_write_cmd(&self, path: &str) -> Option<String> {
        match self.route(path) {
            Route::Base(p) => self.base.server_write_cmd(&p),
            Route::Container(name, inner) => {
                let map = self.containers.lock().unwrap();
                let e = map.get(&name)?;
                match &e.fs {
                    Some(fs) => fs.server_write_cmd(&inner),
                    None => None,
                }
            }
            Route::PodContainer(ns, pod, c, inner) => {
                let key = format!("{}/{}/{}", ns, pod, c);
                let map = self.pod_fs.lock().unwrap();
                map.get(&key)?.server_write_cmd(&inner)
            }
            _ => None,
        }
    }

    fn server_scan_cmd(&self, dir: &str) -> Option<String> {
        match self.route(dir) {
            Route::Base(p) => self.base.server_scan_cmd(&p),
            Route::Container(name, inner) => {
                let map = self.containers.lock().unwrap();
                let e = map.get(&name)?;
                match &e.fs {
                    Some(fs) => fs.server_scan_cmd(&inner),
                    None => None,
                }
            }
            Route::PodContainer(ns, pod, c, inner) => {
                let key = format!("{}/{}/{}", ns, pod, c);
                let map = self.pod_fs.lock().unwrap();
                map.get(&key)?.server_scan_cmd(&inner)
            }
            _ => None,
        }
    }

    async fn stat(&self, path: &str) -> AppResult<FileStat> {
        match self.route(path) {
            Route::Base(p) => self.base.stat(&p).await,
            Route::Container(name, inner) => {
                self.container_fs(&name).await?.stat(&inner).await
            }
            Route::PodContainer(ns, pod, c, inner) => {
                self.pod_container_fs(&ns, &pod, &c)
                    .await?
                    .stat(&inner)
                    .await
            }
            _ => Ok(dir_stat()),
        }
    }

    async fn list_dir(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        match self.route(path) {
            Route::Base(p) => {
                let mut entries = self.base.list_dir(&p).await?;
                if p == "/" {
                    entries.push(virtual_dir_entry("/", CONTAINERS_DIR));
                    entries.push(virtual_dir_entry("/", PODS_DIR));
                }
                Ok(entries)
            }
            Route::ContainersRoot => {
                self.refresh_containers().await?;
                let map = self.containers.lock().unwrap();
                let parent = format!("/{}", CONTAINERS_DIR);
                let mut entries: Vec<FileEntry> = map
                    .iter()
                    .map(|(name, e)| {
                        let mut entry = virtual_dir_entry(&parent, name);
                        // Show image name where permissions usually go
                        entry.permissions = Some(format!(
                            "{} · {}",
                            e.info.runtime.as_str(),
                            e.info.image
                        ));
                        entry
                    })
                    .collect();
                entries.sort_by(|a, b| a.name.cmp(&b.name));
                Ok(entries)
            }
            Route::Container(name, inner) => {
                let fs = self.container_fs(&name).await?;
                let mut entries = fs.list_dir(&inner).await?;
                let prefix = format!("/{}/{}", CONTAINERS_DIR, name);
                for e in &mut entries {
                    e.path = format!("{}{}", prefix, e.path);
                }
                Ok(entries)
            }
            Route::PodsRoot => {
                let namespaces =
                    crate::container::kube_namespaces(self.runner.as_ref(), None).await?;
                let parent = format!("/{}", PODS_DIR);
                Ok(namespaces
                    .iter()
                    .map(|ns| virtual_dir_entry(&parent, ns))
                    .collect())
            }
            Route::PodsNamespace(ns) => {
                let pods = crate::container::kube_pods(self.runner.as_ref(), None, &ns).await?;
                let parent = format!("/{}/{}", PODS_DIR, ns);
                Ok(pods
                    .iter()
                    .map(|p| {
                        let mut e = virtual_dir_entry(&parent, &p.name);
                        e.permissions = Some(p.phase.clone());
                        e
                    })
                    .collect())
            }
            Route::Pod(ns, pod) => {
                let pods = crate::container::kube_pods(self.runner.as_ref(), None, &ns).await?;
                let info = pods
                    .into_iter()
                    .find(|p| p.name == pod)
                    .ok_or_else(|| AppError::IoError(format!("No pod {}/{}", ns, pod)))?;
                let parent = format!("/{}/{}/{}", PODS_DIR, ns, pod);
                Ok(info
                    .containers
                    .iter()
                    .map(|c| virtual_dir_entry(&parent, c))
                    .collect())
            }
            Route::PodContainer(ns, pod, c, inner) => {
                let fs = self.pod_container_fs(&ns, &pod, &c).await?;
                let mut entries = fs.list_dir(&inner).await?;
                let prefix = format!("/{}/{}/{}/{}", PODS_DIR, ns, pod, c);
                for e in &mut entries {
                    e.path = format!("{}{}", prefix, e.path);
                }
                Ok(entries)
            }
        }
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        let (fs, inner) = self.resolve(path, "mkdir").await?;
        fs.mkdir(&inner).await
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        let (fs, inner) = self.resolve(path, "delete").await?;
        fs.remove_file(&inner).await
    }

    async fn remove_dir_all(&self, path: &str) -> AppResult<()> {
        let (fs, inner) = self.resolve(path, "delete").await?;
        fs.remove_dir_all(&inner).await
    }

    fn fast_remove_dir(&self, path: &str) -> bool {
        match self.route(path) {
            Route::Base(p) => self.base.fast_remove_dir(&p),
            // Containers/pods delete via exec `rm -rf`: fast
            _ => true,
        }
    }

    async fn rename(&self, old_path: &str, new_path: &str) -> AppResult<()> {
        let (fs, old_inner) = self.resolve(old_path, "rename").await?;
        let (fs2, new_inner) = self.resolve(new_path, "rename").await?;
        if !Arc::ptr_eq(&fs, &fs2) {
            return Err(AppError::IoError(
                "Cannot rename across endpoints; use copy instead".into(),
            ));
        }
        fs.rename(&old_inner, &new_inner).await
    }

    async fn read_head(&self, path: &str, max_bytes: usize) -> AppResult<Vec<u8>> {
        let (fs, inner) = self.resolve(path, "read").await?;
        fs.read_head(&inner, max_bytes).await
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> AppResult<()> {
        let (fs, inner) = self.resolve(path, "write").await?;
        fs.write_file(&inner, data).await
    }

    async fn open_read(&self, path: &str, offset: u64) -> AppResult<FsReader> {
        let (fs, inner) = self.resolve(path, "read").await?;
        fs.open_read(&inner, offset).await
    }

    async fn open_write(&self, path: &str, offset: u64) -> AppResult<Box<dyn FsWriter>> {
        let (fs, inner) = self.resolve(path, "write").await?;
        fs.open_write(&inner, offset).await
    }
}

/// Runtime kind re-export used by the session layer.
pub type ContainerRuntime = RuntimeKind;
