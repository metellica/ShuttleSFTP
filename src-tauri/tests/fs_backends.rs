// Integration tests of RemoteFs backends against the local machine.
use shuttle_sftp::fs::local::LocalFs;
use shuttle_sftp::fs::RemoteFs;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("shuttle-test-{}-{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn local_fs_roundtrip() {
    let dir = tmp_dir("localfs");
    let fs = LocalFs;
    let file = dir.join("a.txt");
    let file_s = file.to_string_lossy().to_string();
    let dir_s = dir.to_string_lossy().to_string();

    fs.write_file(&file_s, b"hello world").await.unwrap();
    let stat = fs.stat(&file_s).await.unwrap();
    assert_eq!(stat.size, 11);
    assert!(!stat.is_dir);

    let head = fs.read_head(&file_s, 5).await.unwrap();
    assert_eq!(&head, b"hello");

    let entries = fs.list_dir(&dir_s).await.unwrap();
    assert!(entries.iter().any(|e| e.name == "a.txt" && e.size == 11));

    // Streamed read at an offset
    let mut r = fs.open_read(&file_s, 6).await.unwrap();
    let mut buf = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut r.reader, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf, b"world");
    assert_eq!(r.total, 11);

    // Streamed write
    let out = dir.join("b.bin");
    let out_s = out.to_string_lossy().to_string();
    let mut w = fs.open_write(&out_s, 0).await.unwrap();
    tokio::io::AsyncWriteExt::write_all(&mut w, &vec![7u8; 100_000])
        .await
        .unwrap();
    w.finish().await.unwrap();
    assert_eq!(fs.stat(&out_s).await.unwrap().size, 100_000);

    // rename + remove
    let renamed = dir.join("c.bin").to_string_lossy().to_string();
    fs.rename(&out_s, &renamed).await.unwrap();
    fs.remove_file(&renamed).await.unwrap();
    assert!(fs.stat(&renamed).await.is_err());

    fs.remove_dir_all(&dir_s).await.unwrap();
}

#[tokio::test]
async fn walk_fs_dir_collects_tree() {
    let dir = tmp_dir("walk");
    std::fs::create_dir_all(dir.join("sub/inner")).unwrap();
    std::fs::write(dir.join("root.txt"), "r").unwrap();
    std::fs::write(dir.join("sub/x.txt"), "x").unwrap();
    std::fs::write(dir.join("sub/inner/y.txt"), "y").unwrap();

    let fs = LocalFs;
    let (dirs, files) = shuttle_sftp::fs::walk_fs_dir(&fs, &dir.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(dirs, vec!["sub", "sub/inner"]);
    assert_eq!(files, vec!["root.txt", "sub/inner/y.txt", "sub/x.txt"]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn prefix_fs_chroots_into_subtree() {
    use shuttle_sftp::fs::prefix::PrefixFs;
    use std::sync::Arc;

    let dir = tmp_dir("prefix");
    std::fs::create_dir_all(dir.join("rootfs/etc")).unwrap();
    std::fs::write(dir.join("rootfs/etc/hostname"), "container-1").unwrap();

    let prefix = dir.join("rootfs").to_string_lossy().replace('\\', "/");
    let view = PrefixFs::new(Arc::new(LocalFs), prefix);

    // "/" of the view is the rootfs dir
    let entries = view.list_dir("/").await.unwrap();
    assert!(entries.iter().any(|e| e.name == "etc" && e.is_dir));

    // entry paths are view-relative, not host paths
    let etc = view.list_dir("/etc").await.unwrap();
    let hostname = etc.iter().find(|e| e.name == "hostname").unwrap();
    assert!(
        hostname.path.starts_with('/') && !hostname.path.contains("rootfs"),
        "leaked host path: {}",
        hostname.path
    );

    assert_eq!(&view.read_head("/etc/hostname", 100).await.unwrap(), b"container-1");
    view.write_file("/etc/motd", b"hi").await.unwrap();
    assert!(dir.join("rootfs/etc/motd").exists());

    std::fs::remove_dir_all(&dir).unwrap();
}
