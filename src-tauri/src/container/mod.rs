use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::error::{AppError, AppResult};
use crate::exec::{CommandRunner, ExecStream};
use crate::fs::{FileEntry, FileStat, FsReader, FsWriter, RemoteFs};

/// Container runtime CLI used to reach a container.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeKind {
    Docker,
    Nerdctl,
    Crictl,
    Kubectl,
}

impl RuntimeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeKind::Docker => "docker",
            RuntimeKind::Nerdctl => "nerdctl",
            RuntimeKind::Crictl => "crictl",
            RuntimeKind::Kubectl => "kubectl",
        }
    }
}

/// A running container as shown in the connect dialog picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub runtime: RuntimeKind,
    /// K8s pod this container belongs to (crictl listings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pod: Option<String>,
}

/// A pod as shown in the K8s picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub node: Option<String>,
    pub phase: String,
    pub containers: Vec<String>,
}

// ---------------------------------------------------------------------------
// Container listing
// ---------------------------------------------------------------------------

fn parse_ps_lines(out: &str, runtime: RuntimeKind) -> Vec<ContainerInfo> {
    out.lines()
        .filter_map(|line| {
            let mut it = line.split('\t');
            let id = it.next()?.trim().to_string();
            if id.is_empty() {
                return None;
            }
            Some(ContainerInfo {
                id,
                name: it.next().unwrap_or("").to_string(),
                image: it.next().unwrap_or("").to_string(),
                state: it.next().unwrap_or("").to_string(),
                runtime,
                pod: None,
            })
        })
        .collect()
}

fn parse_crictl_json(out: &str) -> Vec<ContainerInfo> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(out) else {
        return Vec::new();
    };
    let Some(items) = v.get("containers").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|c| {
            let id = c.get("id")?.as_str()?.to_string();
            let name = c
                .pointer("/metadata/name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let image = c
                .pointer("/image/image")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let state = c
                .get("state")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .replace("CONTAINER_", "")
                .to_lowercase();
            let pod = c
                .pointer("/labels/io.kubernetes.pod.name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());
            Some(ContainerInfo {
                id,
                name,
                image,
                state,
                runtime: RuntimeKind::Crictl,
                pod,
            })
        })
        .collect()
}

/// List running containers on the runner's machine, probing every known
/// runtime CLI (docker, nerdctl, crictl) and merging the results.
pub async fn list_containers(runner: &dyn CommandRunner) -> AppResult<Vec<ContainerInfo>> {
    let mut all: Vec<ContainerInfo> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    let fmt = "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}";
    for (rt, bin) in [(RuntimeKind::Docker, "docker"), (RuntimeKind::Nerdctl, "nerdctl")] {
        match runner
            .run(&[bin.to_string(), "ps".into(), "--format".into(), fmt.into()], None)
            .await
        {
            Ok(out) if out.success() => all.extend(parse_ps_lines(&out.stdout_string(), rt)),
            Ok(out) => errors.push(format!("{}: {}", bin, out.stderr.trim())),
            Err(e) => errors.push(format!("{}: {}", bin, e)),
        }
    }
    match runner
        .run(
            &["crictl".to_string(), "ps".into(), "-o".into(), "json".into()],
            None,
        )
        .await
    {
        Ok(out) if out.success() => {
            let found = parse_crictl_json(&out.stdout_string());
            // Skip containers already reported by docker/nerdctl (id prefix match)
            for c in found {
                if !all.iter().any(|e| c.id.starts_with(&e.id) || e.id.starts_with(&c.id)) {
                    all.push(c);
                }
            }
        }
        Ok(out) => errors.push(format!("crictl: {}", out.stderr.trim())),
        Err(e) => errors.push(format!("crictl: {}", e)),
    }

    if all.is_empty() && !errors.is_empty() {
        return Err(AppError::IoError(format!(
            "No container runtime reachable on {}: {}",
            runner.location(),
            errors.join("; ")
        )));
    }
    Ok(all)
}

// ---------------------------------------------------------------------------
// K8s addressing (kubectl-based)
// ---------------------------------------------------------------------------

fn kubectl_base(context: Option<&str>) -> Vec<String> {
    let mut v = vec!["kubectl".to_string()];
    if let Some(c) = context {
        if !c.is_empty() {
            v.push("--context".into());
            v.push(c.to_string());
        }
    }
    v
}

pub async fn kube_contexts(runner: &dyn CommandRunner) -> AppResult<Vec<String>> {
    let out = runner
        .run(
            &crate::exec::argv(&["kubectl", "config", "get-contexts", "-o", "name"]),
            None,
        )
        .await?
        .check("kubectl config get-contexts")?;
    Ok(out
        .stdout_string()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

pub async fn kube_namespaces(
    runner: &dyn CommandRunner,
    context: Option<&str>,
) -> AppResult<Vec<String>> {
    let mut argv = kubectl_base(context);
    argv.extend(crate::exec::argv(&["get", "ns", "-o", "name"]));
    let out = runner.run(&argv, None).await?.check("kubectl get ns")?;
    Ok(out
        .stdout_string()
        .lines()
        .map(|l| l.trim().trim_start_matches("namespace/").to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

pub async fn kube_pods(
    runner: &dyn CommandRunner,
    context: Option<&str>,
    namespace: &str,
) -> AppResult<Vec<PodInfo>> {
    let mut argv = kubectl_base(context);
    argv.extend(crate::exec::argv(&["get", "pods", "-n", namespace, "-o", "json"]));
    let out = runner.run(&argv, None).await?.check("kubectl get pods")?;
    let v: serde_json::Value = serde_json::from_str(&out.stdout_string())
        .map_err(|e| AppError::IoError(format!("Cannot parse kubectl output: {}", e)))?;
    let Some(items) = v.get("items").and_then(|i| i.as_array()) else {
        return Ok(Vec::new());
    };
    Ok(items
        .iter()
        .filter_map(|p| {
            let name = p.pointer("/metadata/name")?.as_str()?.to_string();
            let containers = p
                .pointer("/spec/containers")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            Some(PodInfo {
                name,
                namespace: namespace.to_string(),
                node: p
                    .pointer("/spec/nodeName")
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string()),
                phase: p
                    .pointer("/status/phase")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string(),
                containers,
            })
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Exec target: how to run a command inside the container
// ---------------------------------------------------------------------------

/// Builds the argv prefix that runs a command inside a container.
#[derive(Debug, Clone)]
pub struct ExecTarget {
    pub runtime: RuntimeKind,
    /// Container id (docker/nerdctl/crictl) — empty for kubectl.
    pub container_id: String,
    /// kubectl addressing.
    pub context: Option<String>,
    pub namespace: Option<String>,
    pub pod: Option<String>,
    pub container_name: Option<String>,
}

impl ExecTarget {
    pub fn container(runtime: RuntimeKind, id: impl Into<String>) -> Self {
        Self {
            runtime,
            container_id: id.into(),
            context: None,
            namespace: None,
            pod: None,
            container_name: None,
        }
    }

    pub fn pod(
        context: Option<String>,
        namespace: String,
        pod: String,
        container: Option<String>,
    ) -> Self {
        Self {
            runtime: RuntimeKind::Kubectl,
            container_id: String::new(),
            context,
            namespace: Some(namespace),
            pod: Some(pod),
            container_name: container,
        }
    }

    /// argv prefix in front of the in-container command.
    pub fn exec_prefix(&self) -> Vec<String> {
        match self.runtime {
            RuntimeKind::Kubectl => {
                let mut v = kubectl_base(self.context.as_deref());
                v.push("exec".into());
                v.push("-i".into());
                if let Some(ns) = &self.namespace {
                    v.push("-n".into());
                    v.push(ns.clone());
                }
                if let Some(pod) = &self.pod {
                    v.push(pod.clone());
                }
                if let Some(c) = &self.container_name {
                    v.push("-c".into());
                    v.push(c.clone());
                }
                v.push("--".into());
                v
            }
            rt => vec![
                rt.as_str().to_string(),
                "exec".into(),
                "-i".into(),
                self.container_id.clone(),
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// ExecFs: RemoteFs over `<runtime> exec` + POSIX shell tools
// ---------------------------------------------------------------------------

/// File system access through exec'ing shell commands inside a container.
/// Works with any runtime CLI that supports `exec -i` (docker, nerdctl,
/// crictl, kubectl). Requires sh + coreutils (stat/cat/...) in the image.
pub struct ExecFs {
    runner: Arc<dyn CommandRunner>,
    target: ExecTarget,
}

impl ExecFs {
    pub fn new(runner: Arc<dyn CommandRunner>, target: ExecTarget) -> Self {
        Self { runner, target }
    }

    fn sh(&self, script: &str, args: &[&str]) -> Vec<String> {
        let mut argv = self.target.exec_prefix();
        argv.push("sh".into());
        argv.push("-c".into());
        argv.push(script.to_string());
        argv.push("sh".into()); // $0
        argv.extend(args.iter().map(|s| s.to_string()));
        argv
    }

    /// Verify the container has the shell tools this backend needs.
    /// Distroless / scratch images fail here with a helpful message.
    pub async fn probe(&self) -> AppResult<()> {
        let argv = self.sh(
            "command -v stat >/dev/null 2>&1 && command -v cat >/dev/null 2>&1 && echo __SHUTTLE_OK__",
            &[],
        );
        let out = self.runner.run(&argv, None).await?;
        if out.stdout_string().contains("__SHUTTLE_OK__") {
            Ok(())
        } else {
            Err(AppError::ConnectionFailed(format!(
                "Container has no usable shell tools (sh/stat/cat) — likely a distroless or scratch image. \
                 Options: browse it via the host's containerd rootfs (SSH to the host as root), or for K8s \
                 attach a debug container (kubectl debug). Detail: {}",
                if out.stderr.trim().is_empty() {
                    "exec produced no output".to_string()
                } else {
                    out.stderr.trim().to_string()
                }
            )))
        }
    }

    fn parse_stat_line(line: &str) -> Option<(u32, u64, u64, String)> {
        let mut parts = line.splitn(4, '|');
        let mode = u32::from_str_radix(parts.next()?.trim(), 16).ok()?;
        let size = parts.next()?.trim().parse().ok()?;
        let mtime = parts.next()?.trim().parse().unwrap_or(0);
        let name = parts.next()?.to_string();
        Some((mode, size, mtime, name))
    }
}

const S_IFMT: u32 = 0xF000;
const S_IFDIR: u32 = 0x4000;

/// Reader that owns the whole exec stream (keeps the channel alive).
struct ExecReader {
    stream: ExecStream,
}

impl AsyncRead for ExecReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stream.stdout).poll_read(cx, buf)
    }
}

/// Writer streaming into `cat > file` inside the container.
struct ExecWriter {
    stream: ExecStream,
    what: String,
}

impl AsyncWrite for ExecWriter {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        std::pin::Pin::new(&mut self.stream.stdin).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.stream.stdin).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.stream.stdin).poll_shutdown(cx)
    }
}

#[async_trait]
impl FsWriter for ExecWriter {
    async fn finish(mut self: Box<Self>) -> AppResult<()> {
        let ExecStream {
            mut stdin, done, ..
        } = self.stream;
        stdin
            .shutdown()
            .await
            .map_err(|e| AppError::IoError(format!("Container write close error: {}", e)))?;
        // Dropping stdin closes the pipe: that's what actually delivers EOF
        // to the child (shutdown alone doesn't on all platforms).
        drop(stdin);
        // Wait for the exec command to exit and verify success.
        match done.await {
            Ok(done) => {
                if done.exit.unwrap_or(0) != 0 {
                    return Err(AppError::TransferError(format!(
                        "{} failed (exit {}): {}",
                        self.what,
                        done.exit.unwrap_or(0),
                        done.stderr.trim()
                    )));
                }
                Ok(())
            }
            Err(_) => Ok(()), // status channel dropped: treat as done
        }
    }
}

#[async_trait]
impl RemoteFs for ExecFs {
    fn kind(&self) -> &'static str {
        "exec"
    }

    /// Writes go through `cat >`, which cannot append at an offset.
    fn supports_resume(&self) -> bool {
        false
    }

    async fn stat(&self, path: &str) -> AppResult<FileStat> {
        let argv = self.sh(r#"stat -c '%f|%s|%Y|%n' -- "$1""#, &[path]);
        let out = self.runner.run(&argv, None).await?.check("stat")?;
        let line = out.stdout_string();
        let (mode, size, _, _) = ExecFs::parse_stat_line(line.trim())
            .ok_or_else(|| AppError::IoError(format!("Cannot parse stat output: {}", line)))?;
        Ok(FileStat {
            size,
            is_dir: mode & S_IFMT == S_IFDIR,
        })
    }

    async fn list_dir(&self, path: &str) -> AppResult<Vec<FileEntry>> {
        let script = r#"cd -- "$1" || exit 1
for f in * .[!.]* ..?*; do
  if [ -e "$f" ] || [ -L "$f" ]; then stat -c '%f|%s|%Y|%n' -- "$f" 2>/dev/null; fi
done
exit 0"#;
        let argv = self.sh(script, &[path]);
        let out = self.runner.run(&argv, None).await?;
        if !out.success() {
            return Err(AppError::IoError(format!(
                "Cannot list {}: {}",
                path,
                out.stderr.trim()
            )));
        }
        let mut result = Vec::new();
        for line in out.stdout_string().lines() {
            if let Some((mode, size, mtime, name)) = ExecFs::parse_stat_line(line) {
                if name == "." || name == ".." {
                    continue;
                }
                result.push(FileEntry {
                    path: crate::fs::join_path(path, &name),
                    is_dir: mode & S_IFMT == S_IFDIR,
                    size,
                    modified: mtime,
                    permissions: Some(crate::fs::mode_to_string(mode)),
                    name,
                });
            }
        }
        Ok(result)
    }

    async fn mkdir(&self, path: &str) -> AppResult<()> {
        let argv = self.sh(r#"mkdir -p -- "$1""#, &[path]);
        self.runner.run(&argv, None).await?.check("mkdir")?;
        Ok(())
    }

    async fn remove_file(&self, path: &str) -> AppResult<()> {
        let argv = self.sh(r#"rm -f -- "$1""#, &[path]);
        self.runner.run(&argv, None).await?.check("rm")?;
        Ok(())
    }

    async fn remove_dir_all(&self, path: &str) -> AppResult<()> {
        let argv = self.sh(r#"rm -rf -- "$1""#, &[path]);
        self.runner.run(&argv, None).await?.check("rm -rf")?;
        Ok(())
    }

    async fn rename(&self, old_path: &str, new_path: &str) -> AppResult<()> {
        let argv = self.sh(r#"mv -- "$1" "$2""#, &[old_path, new_path]);
        self.runner.run(&argv, None).await?.check("mv")?;
        Ok(())
    }

    async fn read_head(&self, path: &str, max_bytes: usize) -> AppResult<Vec<u8>> {
        let script = format!(r#"head -c {} -- "$1""#, max_bytes);
        let argv = self.sh(&script, &[path]);
        let out = self.runner.run(&argv, None).await?.check("read")?;
        Ok(out.stdout)
    }

    async fn write_file(&self, path: &str, data: &[u8]) -> AppResult<()> {
        let argv = self.sh(r#"cat > "$1""#, &[path]);
        self.runner
            .run(&argv, Some(data.to_vec()))
            .await?
            .check("write")?;
        Ok(())
    }

    async fn open_read(&self, path: &str, offset: u64) -> AppResult<FsReader> {
        let total = self.stat(path).await?.size;
        let script = if offset > 0 {
            format!(r#"tail -c +{} -- "$1""#, offset + 1)
        } else {
            r#"cat -- "$1""#.to_string()
        };
        let argv = self.sh(&script, &[path]);
        let stream = self.runner.spawn(&argv).await?;
        Ok(FsReader {
            total,
            reader: Box::new(ExecReader { stream }),
        })
    }

    async fn open_write(&self, path: &str, offset: u64) -> AppResult<Box<dyn FsWriter>> {
        if offset > 0 {
            return Err(AppError::TransferError(
                "Container exec transfers cannot resume mid-file".into(),
            ));
        }
        let argv = self.sh(r#"cat > "$1""#, &[path]);
        let stream = self.runner.spawn(&argv).await?;
        Ok(Box::new(ExecWriter {
            stream,
            what: format!("write to {}", path),
        }))
    }
}

// ---------------------------------------------------------------------------
// containerd rootfs resolution (direct host-side access, distroless-proof)
// ---------------------------------------------------------------------------

/// Find the host path of a container's root file system, if resolvable.
/// Checks docker's overlay MergedDir and containerd's runtime v2 task dirs.
pub async fn resolve_rootfs(
    runner: &dyn CommandRunner,
    runtime: RuntimeKind,
    container_id: &str,
) -> Option<String> {
    // docker: ask the engine for the merged overlay dir
    if runtime == RuntimeKind::Docker {
        if let Ok(out) = runner
            .run(
                &crate::exec::argv(&[
                    "docker",
                    "inspect",
                    "-f",
                    "{{.GraphDriver.Data.MergedDir}}",
                    container_id,
                ]),
                None,
            )
            .await
        {
            if out.success() {
                let p = out.stdout_string().trim().to_string();
                if p.starts_with('/') && dir_exists(runner, &p).await {
                    return Some(p);
                }
            }
        }
    }

    // containerd runtime v2 task dirs, one per namespace
    for ns in ["k8s.io", "moby", "default"] {
        let p = format!(
            "/run/containerd/io.containerd.runtime.v2.task/{}/{}/rootfs",
            ns, container_id
        );
        if dir_exists(runner, &p).await {
            return Some(p);
        }
    }
    None
}

async fn dir_exists(runner: &dyn CommandRunner, path: &str) -> bool {
    let argv = vec![
        "sh".to_string(),
        "-c".into(),
        r#"[ -d "$1" ] && echo yes"#.into(),
        "sh".into(),
        path.to_string(),
    ];
    matches!(runner.run(&argv, None).await, Ok(out) if out.stdout_string().contains("yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_docker_ps_lines() {
        let out = "abc123\tredis\tredis:7\tUp 3 hours\nxyz\t\timg\t\n";
        let list = parse_ps_lines(out, RuntimeKind::Docker);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "abc123");
        assert_eq!(list[0].name, "redis");
        assert_eq!(list[0].image, "redis:7");
        assert_eq!(list[0].state, "Up 3 hours");
    }

    #[test]
    fn parses_crictl_json() {
        let out = r#"{"containers":[{"id":"c1","metadata":{"name":"app"},
            "image":{"image":"nginx:1.27"},"state":"CONTAINER_RUNNING",
            "labels":{"io.kubernetes.pod.name":"web-1"}}]}"#;
        let list = parse_crictl_json(out);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "app");
        assert_eq!(list[0].state, "running");
        assert_eq!(list[0].pod.as_deref(), Some("web-1"));
    }

    #[test]
    fn exec_prefix_docker() {
        let t = ExecTarget::container(RuntimeKind::Docker, "abc");
        assert_eq!(t.exec_prefix(), vec!["docker", "exec", "-i", "abc"]);
    }

    #[test]
    fn exec_prefix_kubectl_full() {
        let t = ExecTarget::pod(
            Some("prod".into()),
            "ns1".into(),
            "web-1".into(),
            Some("app".into()),
        );
        assert_eq!(
            t.exec_prefix(),
            vec![
                "kubectl", "--context", "prod", "exec", "-i", "-n", "ns1", "web-1", "-c",
                "app", "--"
            ]
        );
    }

    #[test]
    fn parses_stat_line() {
        // 0x81a4 = regular file 0644
        let (mode, size, mtime, name) = ExecFs::parse_stat_line("81a4|1234|1700000000|hello world.txt").unwrap();
        assert_eq!(mode & S_IFMT, 0x8000);
        assert_eq!(size, 1234);
        assert_eq!(mtime, 1700000000);
        assert_eq!(name, "hello world.txt");
        // 0x41ed = directory 0755
        let (mode, ..) = ExecFs::parse_stat_line("41ed|4096|0|somedir").unwrap();
        assert_eq!(mode & S_IFMT, S_IFDIR);
    }
}
