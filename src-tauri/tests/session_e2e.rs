// End-to-end test of a local session's virtual /@containers directory
// against a real local Docker daemon — the exact path the UI browses.
use shuttle_sftp::exec::{argv, CommandRunner, LocalRunner};
use shuttle_sftp::ssh::session::SessionManager;

async fn docker_available() -> bool {
    matches!(
        LocalRunner.run(&argv(&["docker", "info"]), None).await,
        Ok(out) if out.success()
    )
}

#[tokio::test]
async fn local_session_virtual_container_dirs() {
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

    let mgr = SessionManager::new();
    let sid = mgr.connect_local().await.expect("connect_local");
    let session = mgr.get_session(&sid).await.unwrap();
    let fs = { session.lock().await.fs.clone() };

    // Root listing includes the virtual dirs
    let root = fs.list_dir("/").await.unwrap();
    assert!(root.iter().any(|e| e.name == "@containers" && e.is_dir));
    assert!(root.iter().any(|e| e.name == "@pods" && e.is_dir));

    // /@containers lists the running container with runtime/image info
    let containers = fs.list_dir("/@containers").await.unwrap();
    let c = containers
        .iter()
        .find(|e| e.name == "shuttle-sm-test")
        .expect("test container listed");
    assert!(c.is_dir);
    assert!(c.path.starts_with("/@containers/"));
    assert!(c.permissions.as_deref().unwrap_or("").contains("busybox"));

    // Browse inside the container through the virtual path
    let inside = fs.list_dir("/@containers/shuttle-sm-test").await.unwrap();
    assert!(inside.iter().any(|e| e.name == "bin"));
    // entry paths keep the virtual prefix so navigation works
    assert!(inside
        .iter()
        .all(|e| e.path.starts_with("/@containers/shuttle-sm-test/")));

    // Write + read a file inside the container via virtual path
    fs.write_file("/@containers/shuttle-sm-test/tmp/vdir.txt", b"virtual dirs")
        .await
        .unwrap();
    let head = fs
        .read_head("/@containers/shuttle-sm-test/tmp/vdir.txt", 100)
        .await
        .unwrap();
    assert_eq!(&head, b"virtual dirs");
    let st = fs
        .stat("/@containers/shuttle-sm-test/tmp/vdir.txt")
        .await
        .unwrap();
    assert_eq!(st.size, 12);

    // stat of virtual levels reports a directory
    assert!(fs.stat("/@containers").await.unwrap().is_dir);

    // deleting a virtual level is rejected
    assert!(fs.remove_dir_all("/@containers").await.is_err());

    // server-side copy commands are exposed for container paths
    let read_cmd = fs.server_read_cmd("/@containers/shuttle-sm-test/tmp/vdir.txt");
    assert!(
        read_cmd.as_deref().unwrap_or("").contains("docker exec"),
        "read cmd: {:?}",
        read_cmd
    );

    mgr.disconnect(&sid).await.unwrap();
    let _ = LocalRunner
        .run(&argv(&["docker", "rm", "-f", "shuttle-sm-test"]), None)
        .await;
}
