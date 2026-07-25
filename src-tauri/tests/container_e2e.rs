// End-to-end test of the container backend against a real local Docker
// daemon. Skipped (pass trivially) when Docker isn't available.
use std::sync::Arc;

use shuttle_sftp::container::{list_containers, ExecFs, ExecTarget, RuntimeKind};
use shuttle_sftp::exec::{argv, CommandRunner, LocalRunner};
use shuttle_sftp::fs::RemoteFs;

async fn docker_available() -> bool {
    matches!(
        LocalRunner.run(&argv(&["docker", "info"]), None).await,
        Ok(out) if out.success()
    )
}

/// Start a throwaway busybox container; returns its id.
async fn start_test_container() -> String {
    let out = LocalRunner
        .run(
            &argv(&[
                "docker", "run", "-d", "--rm", "--name", "shuttle-e2e-test",
                "busybox", "sleep", "300",
            ]),
            None,
        )
        .await
        .unwrap();
    assert!(out.success(), "docker run failed: {}", out.stderr);
    out.stdout_string().trim().to_string()
}

async fn stop_test_container() {
    let _ = LocalRunner
        .run(&argv(&["docker", "rm", "-f", "shuttle-e2e-test"]), None)
        .await;
}

#[tokio::test]
async fn docker_exec_fs_end_to_end() {
    if !docker_available().await {
        eprintln!("SKIP: docker daemon not available");
        return;
    }
    stop_test_container().await; // clean any leftover
    let id = start_test_container().await;

    let fs = ExecFs::new(
        Arc::new(LocalRunner),
        ExecTarget::container(RuntimeKind::Docker, id.clone()),
    );

    // probe: busybox has sh/stat/cat
    fs.probe().await.expect("probe should pass on busybox");

    // list_containers finds it
    let list = list_containers(&LocalRunner).await.unwrap();
    assert!(
        list.iter().any(|c| id.starts_with(&c.id) || c.id.starts_with(&id)),
        "test container not in listing: {:?}",
        list
    );

    // mkdir + write + stat + read_head
    fs.mkdir("/tmp/shuttle/sub").await.unwrap();
    fs.write_file("/tmp/shuttle/hello.txt", b"hello container")
        .await
        .unwrap();
    let st = fs.stat("/tmp/shuttle/hello.txt").await.unwrap();
    assert_eq!(st.size, 15);
    assert!(!st.is_dir);
    let head = fs.read_head("/tmp/shuttle/hello.txt", 5).await.unwrap();
    assert_eq!(&head, b"hello");

    // list_dir sees both entries with sane metadata
    let entries = fs.list_dir("/tmp/shuttle").await.unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"hello.txt") && names.contains(&"sub"));
    let sub = entries.iter().find(|e| e.name == "sub").unwrap();
    assert!(sub.is_dir);

    // streaming read (offset) and streaming write
    let mut r = fs.open_read("/tmp/shuttle/hello.txt", 6).await.unwrap();
    let mut buf = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut r.reader, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf, b"container");
    assert_eq!(r.total, 15);

    let payload = vec![b'x'; 200_000];
    let mut w = fs.open_write("/tmp/shuttle/big.bin", 0).await.unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut w, &payload).await.unwrap();
    w.finish().await.unwrap();
    assert_eq!(fs.stat("/tmp/shuttle/big.bin").await.unwrap().size, 200_000);

    // rename + delete
    fs.rename("/tmp/shuttle/big.bin", "/tmp/shuttle/big2.bin")
        .await
        .unwrap();
    fs.remove_file("/tmp/shuttle/big2.bin").await.unwrap();
    assert!(fs.stat("/tmp/shuttle/big2.bin").await.is_err());
    fs.remove_dir_all("/tmp/shuttle").await.unwrap();
    assert!(fs.stat("/tmp/shuttle").await.is_err());

    // paths with spaces and quotes are handled safely
    fs.write_file("/tmp/it's a file.txt", b"quoted").await.unwrap();
    assert_eq!(fs.stat("/tmp/it's a file.txt").await.unwrap().size, 6);
    fs.remove_file("/tmp/it's a file.txt").await.unwrap();

    stop_test_container().await;
}

#[tokio::test]
async fn distroless_probe_fails_helpfully() {
    if !docker_available().await {
        eprintln!("SKIP: docker daemon not available");
        return;
    }
    // hello-world's image has no shell at all
    let _ = LocalRunner
        .run(&argv(&["docker", "rm", "-f", "shuttle-e2e-distroless"]), None)
        .await;
    let out = LocalRunner
        .run(
            &argv(&[
                "docker", "create", "--name", "shuttle-e2e-distroless", "hello-world",
            ]),
            None,
        )
        .await
        .unwrap();
    if !out.success() {
        eprintln!("SKIP: cannot create hello-world container: {}", out.stderr);
        return;
    }
    let id = out.stdout_string().trim().to_string();
    let fs = ExecFs::new(
        Arc::new(LocalRunner),
        ExecTarget::container(RuntimeKind::Docker, id),
    );
    let err = fs.probe().await.expect_err("probe must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("distroless") || msg.contains("shell tools"),
        "unexpected error message: {}",
        msg
    );
    let _ = LocalRunner
        .run(&argv(&["docker", "rm", "-f", "shuttle-e2e-distroless"]), None)
        .await;
}
