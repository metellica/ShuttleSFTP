// End-to-end test of the SSH leg (SftpClient as RemoteFs + SshRunner)
// against a local sshd (WSL). Controlled by env vars so CI without sshd
// skips cleanly:
//   SHUTTLE_TEST_SSH_HOST / _PORT / _USER / _PASS
use std::sync::Arc;

use shuttle_sftp::exec::{CommandRunner, SshRunner};
use shuttle_sftp::fs::RemoteFs;
use shuttle_sftp::ssh::auth::AuthMethod;
use shuttle_sftp::ssh::session::ConnectParams;
use shuttle_sftp::ssh::sftp::SftpClient;

fn ssh_env() -> Option<ConnectParams> {
    let host = std::env::var("SHUTTLE_TEST_SSH_HOST").ok()?;
    let user = std::env::var("SHUTTLE_TEST_SSH_USER").ok()?;
    let pass = std::env::var("SHUTTLE_TEST_SSH_PASS").ok()?;
    let port = std::env::var("SHUTTLE_TEST_SSH_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(22);
    Some(ConnectParams {
        host,
        port,
        username: user,
        auth: AuthMethod::Password { password: pass },
    })
}

#[tokio::test]
async fn ssh_sftp_and_exec_end_to_end() {
    let Some(params) = ssh_env() else {
        eprintln!("SKIP: SHUTTLE_TEST_SSH_* not set");
        return;
    };

    let sftp = SftpClient::connect(&params).await.expect("ssh connect");
    let ssh = sftp.ssh_handle();

    // --- SftpClient as RemoteFs -------------------------------------------
    let base = format!("/tmp/shuttle-ssh-test-{}", std::process::id());
    sftp.mkdir(&base).await.unwrap();
    let f = format!("{}/hello.txt", base);
    sftp.write_file(&f, b"hello over sftp").await.unwrap();
    assert_eq!(sftp.stat(&f).await.unwrap().size, 15);
    assert_eq!(&sftp.read_head(&f, 5).await.unwrap(), b"hello");

    let entries = sftp.list_dir(&base).await.unwrap();
    assert!(entries.iter().any(|e| e.name == "hello.txt"));

    // Streamed read at offset
    let mut r = sftp.open_read(&f, 6).await.unwrap();
    let mut buf = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut r.reader, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf, b"over sftp");

    // Streamed write
    let f2 = format!("{}/big.bin", base);
    let mut w = sftp.open_write(&f2, 0).await.unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut w, &vec![9u8; 300_000])
        .await
        .unwrap();
    w.finish().await.unwrap();
    assert_eq!(sftp.stat(&f2).await.unwrap().size, 300_000);

    // --- SshRunner: capture + streaming ------------------------------------
    let runner = SshRunner::new(ssh, "test@wsl");
    let out = runner
        .run(
            &shuttle_sftp::exec::argv(&["cat", "--", &f]),
            None,
        )
        .await
        .unwrap();
    assert!(out.success());
    assert_eq!(out.stdout_string(), "hello over sftp");

    // stdin round-trip through a remote command
    let out = runner
        .run(
            &shuttle_sftp::exec::argv(&["wc", "-c"]),
            Some(vec![b'z'; 12345]),
        )
        .await
        .unwrap();
    assert!(out.success());
    assert_eq!(out.stdout_string().trim(), "12345");

    // streaming spawn: remote `cat` echoes what we write
    let mut stream = runner
        .spawn(&shuttle_sftp::exec::argv(&["cat"]))
        .await
        .unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.stdin.write_all(b"stream me").await.unwrap();
    stream.stdin.shutdown().await.unwrap();
    let mut echoed = String::new();
    stream.stdout.read_to_string(&mut echoed).await.unwrap();
    assert_eq!(echoed, "stream me");
    let done = stream.done.await.unwrap();
    assert_eq!(done.exit, Some(0));

    // paths with quotes/spaces via shell_join
    let weird = format!("{}/it's a file.txt", base);
    sftp.write_file(&weird, b"quoted").await.unwrap();
    let out = runner
        .run(&shuttle_sftp::exec::argv(&["cat", "--", &weird]), None)
        .await
        .unwrap();
    assert_eq!(out.stdout_string(), "quoted");

    // cleanup
    sftp.remove_dir_all(&base).await.unwrap();
    assert!(sftp.stat(&base).await.is_err());
}

#[tokio::test]
async fn ssh_relay_copy_between_endpoints() {
    let Some(params) = ssh_env() else {
        eprintln!("SKIP: SHUTTLE_TEST_SSH_* not set");
        return;
    };

    // local -> remote -> local relay through the RemoteFs streams,
    // mirroring what TransferEngine::run_copy does.
    let sftp = Arc::new(SftpClient::connect(&params).await.unwrap());
    let local = shuttle_sftp::fs::local::LocalFs;

    let tmp = std::env::temp_dir().join(format!("shuttle-relay-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let src = tmp.join("src.bin");
    let payload: Vec<u8> = (0..500_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&src, &payload).unwrap();

    let remote_path = format!("/tmp/shuttle-relay-{}.bin", std::process::id());

    // upload: LocalFs reader -> SftpClient writer
    let mut r = local.open_read(&src.to_string_lossy(), 0).await.unwrap();
    let mut w = sftp.open_write(&remote_path, 0).await.unwrap();
    tokio::io::copy(&mut r.reader, &mut w).await.unwrap();
    w.finish().await.unwrap();
    assert_eq!(sftp.stat(&remote_path).await.unwrap().size, 500_000);

    // download back and compare bytes
    let back = tmp.join("back.bin");
    let mut r = sftp.open_read(&remote_path, 0).await.unwrap();
    let mut w = local.open_write(&back.to_string_lossy(), 0).await.unwrap();
    tokio::io::copy(&mut r.reader, &mut w).await.unwrap();
    w.finish().await.unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), payload);

    sftp.remove_file(&remote_path).await.unwrap();
    std::fs::remove_dir_all(&tmp).unwrap();
}
