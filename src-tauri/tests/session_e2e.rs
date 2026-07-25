// End-to-end test of SessionManager with a real local Docker container:
// the exact code path the `connect_container` Tauri command runs.
use shuttle_sftp::exec::{argv, CommandRunner, LocalRunner};
use shuttle_sftp::ssh::session::{ContainerConnectSpec, SessionManager};
use shuttle_sftp::container::RuntimeKind;

async fn docker_available() -> bool {
    matches!(
        LocalRunner.run(&argv(&["docker", "info"]), None).await,
        Ok(out) if out.success()
    )
}

#[tokio::test]
async fn session_manager_container_lifecycle() {
    if !docker_available().await {
        eprintln!("SKIP: docker daemon not available");
        return;
    }
    let _ = LocalRunner
        .run(&argv(&["docker", "rm", "-f", "shuttle-sm-test"]), None)
        .await;
    let out = LocalRunner
        .run(
            &argv(&[
                "docker", "run", "-d", "--rm", "--name", "shuttle-sm-test",
                "busybox", "sleep", "300",
            ]),
            None,
        )
        .await
        .unwrap();
    assert!(out.success(), "docker run failed: {}", out.stderr);
    let cid = out.stdout_string().trim().to_string();

    let mgr = SessionManager::new();
    let sid = mgr
        .connect_container(ContainerConnectSpec {
            runtime: RuntimeKind::Docker,
            container_id: cid.clone(),
            name: Some("shuttle-sm-test".into()),
            via_session_id: None,
            via: None,
            prefer_rootfs: true, // no SSH leg locally -> falls back to exec
        })
        .await
        .expect("connect_container");

    // Browse and edit through the session's RemoteFs, like the
    // filesystem commands do.
    let session = mgr.get_session(&sid).await.unwrap();
    {
        let s = session.lock().await;
        assert_eq!(s.fs.kind(), "exec");
        let entries = s.fs.list_dir("/").await.unwrap();
        assert!(entries.iter().any(|e| e.name == "bin"));

        s.fs.write_file("/tmp/from-session.txt", b"session write")
            .await
            .unwrap();
        let head = s.fs.read_head("/tmp/from-session.txt", 100).await.unwrap();
        assert_eq!(&head, b"session write");
    }

    mgr.disconnect(&sid).await.unwrap();
    assert!(mgr.get_session(&sid).await.is_err());

    let _ = LocalRunner
        .run(&argv(&["docker", "rm", "-f", "shuttle-sm-test"]), None)
        .await;
}
