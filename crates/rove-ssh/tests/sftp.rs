mod common;

use std::sync::Arc;
use std::time::Duration;

use common::Harness;
use devsshd::DevConfig;
use rove_fs::{
    Conflict, EntryKind, FileSystem, FsError, LocalFs, TextEditError, TransferConfig,
    TransferQueue, TransferRequest, TransferState, TransferUpdate, read_text_file, remove_all,
    save_text_file, text_version,
};

fn rooted(root: &std::path::Path) -> DevConfig {
    DevConfig {
        sftp_root: Some(root.to_path_buf()),
        ..DevConfig::demo()
    }
}

#[tokio::test]
async fn browses_and_edits_the_remote_tree() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("hello.txt"), b"hi").unwrap();
    let harness = Harness::new(rooted(root.path())).await;
    let connection = harness.connect().await;
    let sftp = connection.open_sftp().await.unwrap();
    assert_eq!(
        connection.open_channels(),
        1,
        "the sftp channel holds a lease"
    );

    let home = sftp.home().await.unwrap();
    assert_eq!(home, root.path().canonicalize().unwrap().to_string_lossy());
    let dir = sftp.join(&home, "docs");
    sftp.mkdir(&dir).await.unwrap();
    let mut names: Vec<_> = sftp
        .list(&home)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| (entry.name, entry.metadata.kind))
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        names,
        vec![
            ("docs".to_owned(), EntryKind::Dir),
            ("hello.txt".to_owned(), EntryKind::File)
        ]
    );
    let hello = sftp.stat(&sftp.join(&home, "hello.txt")).await.unwrap();
    assert_eq!(hello.size, 2);

    let renamed = sftp.join(&dir, "moved.txt");
    sftp.rename(&sftp.join(&home, "hello.txt"), &renamed)
        .await
        .unwrap();
    assert!(root.path().join("docs/moved.txt").exists());
    assert!(matches!(
        sftp.stat(&sftp.join(&home, "hello.txt")).await,
        Err(FsError::NotFound(_))
    ));
    remove_all(&sftp, &dir).await.unwrap();
    assert!(!root.path().join("docs").exists());

    drop(sftp);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(connection.open_channels(), 0);
}

#[tokio::test]
async fn uploads_and_downloads_through_the_transfer_queue() {
    let remote = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let tree = local.path().join("project");
    std::fs::create_dir_all(tree.join("src")).unwrap();
    let payload: Vec<u8> = (0..1_500_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(tree.join("src/data.bin"), &payload).unwrap();
    std::fs::write(tree.join("README"), b"readme").unwrap();

    let harness = Harness::new(rooted(remote.path())).await;
    let connection = harness.connect().await;
    let sftp: Arc<dyn FileSystem> = Arc::new(connection.open_sftp().await.unwrap());
    let home = sftp.home().await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let _listener = harness.events.on::<TransferUpdate>(move |update| {
        let _ = tx.send(update.clone());
    });
    let queue = TransferQueue::new(TransferConfig::default(), harness.events.clone());
    let mut wait = async |id| loop {
        let update: TransferUpdate = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .unwrap()
            .unwrap();
        if update.id == id && update.state.is_final() {
            return update;
        }
    };

    let upload = queue.enqueue(TransferRequest {
        src: Arc::new(LocalFs),
        src_path: tree.to_string_lossy().into_owned(),
        dst: sftp.clone(),
        dst_dir: home.clone(),
        conflict: Conflict::Rename,
        tag: "upload".into(),
    });
    let uploaded = wait(upload).await;
    assert_eq!(uploaded.state, TransferState::Done, "{:?}", uploaded.error);
    assert_eq!(uploaded.files_done, 2);
    assert_eq!(
        std::fs::read(remote.path().join("project/src/data.bin")).unwrap(),
        payload
    );

    let back = local.path().join("back");
    std::fs::create_dir(&back).unwrap();
    let download = queue.enqueue(TransferRequest {
        src: sftp.clone(),
        src_path: sftp.join(&home, "project"),
        dst: Arc::new(LocalFs),
        dst_dir: back.to_string_lossy().into_owned(),
        conflict: Conflict::Rename,
        tag: "download".into(),
    });
    let downloaded = wait(download).await;
    assert_eq!(
        downloaded.state,
        TransferState::Done,
        "{:?}",
        downloaded.error
    );
    assert_eq!(
        std::fs::read(back.join("project/src/data.bin")).unwrap(),
        payload
    );
    assert_eq!(
        std::fs::read(back.join("project/README")).unwrap(),
        b"readme"
    );

    // Overwriting an existing remote file replaces it through the temporary file and keeps its
    // permissions.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            remote.path().join("project/README"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    std::fs::write(tree.join("README"), b"updated").unwrap();
    let again = queue.enqueue(TransferRequest {
        src: Arc::new(LocalFs),
        src_path: tree.join("README").to_string_lossy().into_owned(),
        dst: sftp.clone(),
        dst_dir: sftp.join(&home, "project"),
        conflict: Conflict::Overwrite,
        tag: "overwrite".into(),
    });
    let replaced = wait(again).await;
    assert_eq!(replaced.state, TransferState::Done, "{:?}", replaced.error);
    assert_eq!(
        std::fs::read(remote.path().join("project/README")).unwrap(),
        b"updated"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(remote.path().join("project/README"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let leftovers: Vec<_> = std::fs::read_dir(remote.path().join("project"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".part"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test]
async fn edits_utf8_text_with_version_checks_and_private_exclusive_staging() {
    use tokio::io::AsyncWriteExt;

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.conf");
    std::fs::write(&path, "\u{feff}old\r\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let harness = Harness::new(rooted(root.path())).await;
    let connection = harness.connect().await;
    let sftp = connection.open_sftp().await.unwrap();
    let remote = sftp.join(&sftp.home().await.unwrap(), "settings.conf");
    let file = read_text_file(&sftp, &remote).await.unwrap();
    assert_eq!(file.text, "\u{feff}old\r\n");
    let text = "\u{feff}中文\r\nnew\n";
    let version = save_text_file(&sftp, &remote, &file.version, text)
        .await
        .unwrap();
    assert_eq!(version, text_version(text.as_bytes()));
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    assert_eq!(
        save_text_file(&sftp, &remote, &file.version, "stale").await,
        Err(TextEditError::Conflict)
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);

    let staging = sftp.join(&sftp.home().await.unwrap(), "stage");
    let mut writer = sftp.create_new_private(&staging).await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.path().join("stage"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    writer.write_all(b"keep").await.unwrap();
    writer.shutdown().await.unwrap();
    assert!(sftp.create_new_private(&staging).await.is_err());
    assert_eq!(std::fs::read(root.path().join("stage")).unwrap(), b"keep");
}
