use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rove_fs::{
    Conflict, FileSystem, LocalFs, TransferConfig, TransferId, TransferQueue, TransferRequest,
    TransferState, TransferUpdate, remove_all,
};
use rove_kernel::EventBus;
use tokio::sync::mpsc;

fn queue(config: TransferConfig) -> (TransferQueue, mpsc::UnboundedReceiver<TransferUpdate>) {
    let events = EventBus::default();
    let (tx, rx) = mpsc::unbounded_channel();
    std::mem::forget(events.on::<TransferUpdate>(move |update| {
        let _ = tx.send(update.clone());
    }));
    (TransferQueue::new(config, events), rx)
}

async fn wait_final(
    rx: &mut mpsc::UnboundedReceiver<TransferUpdate>,
    id: TransferId,
) -> TransferUpdate {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let update = rx.recv().await.expect("event stream");
            if update.id == id && update.state.is_final() {
                return update;
            }
        }
    })
    .await
    .expect("transfer finished in time")
}

fn request(src: &Path, dst_dir: &Path, conflict: Conflict) -> TransferRequest {
    TransferRequest {
        src: Arc::new(LocalFs),
        src_path: src.to_string_lossy().into_owned(),
        dst: Arc::new(LocalFs),
        dst_dir: dst_dir.to_string_lossy().into_owned(),
        conflict,
        tag: "test".into(),
    }
}

#[tokio::test]
async fn copies_a_directory_tree_and_reports_totals() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let tree = src.path().join("tree");
    std::fs::create_dir_all(tree.join("sub/deeper")).unwrap();
    std::fs::write(tree.join("a.txt"), b"hello").unwrap();
    std::fs::write(tree.join("sub/b.bin"), vec![7u8; 300_000]).unwrap();
    std::fs::write(tree.join("sub/deeper/c"), b"").unwrap();

    let (queue, mut rx) = queue(TransferConfig::default());
    let id = queue.enqueue(request(&tree, dst.path(), Conflict::Rename));
    let done = wait_final(&mut rx, id).await;

    assert_eq!(done.state, TransferState::Done, "{:?}", done.error);
    assert_eq!(done.files_total, 3);
    assert_eq!(done.files_done, 3);
    assert_eq!(done.bytes_total, 300_005);
    assert_eq!(done.bytes_done, 300_005);
    let copied = dst.path().join("tree");
    assert_eq!(std::fs::read(copied.join("a.txt")).unwrap(), b"hello");
    assert_eq!(
        std::fs::read(copied.join("sub/b.bin")).unwrap().len(),
        300_000
    );
    assert!(copied.join("sub/deeper/c").exists());
    assert_eq!(queue.list().len(), 1);
}

#[tokio::test]
async fn conflict_policies() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let file = src.path().join("note.txt");
    std::fs::write(&file, b"new").unwrap();
    std::fs::write(dst.path().join("note.txt"), b"old").unwrap();
    let (queue, mut rx) = queue(TransferConfig::default());

    let skipped = wait_final(
        &mut rx,
        queue.enqueue(request(&file, dst.path(), Conflict::Skip)),
    )
    .await;
    assert!(skipped.skipped);
    assert_eq!(std::fs::read(dst.path().join("note.txt")).unwrap(), b"old");

    let renamed = wait_final(
        &mut rx,
        queue.enqueue(request(&file, dst.path(), Conflict::Rename)),
    )
    .await;
    assert!(
        renamed.dst_path.ends_with("note (1).txt"),
        "{}",
        renamed.dst_path
    );
    assert_eq!(
        std::fs::read(dst.path().join("note (1).txt")).unwrap(),
        b"new"
    );

    let replaced = wait_final(
        &mut rx,
        queue.enqueue(request(&file, dst.path(), Conflict::Overwrite)),
    )
    .await;
    assert_eq!(replaced.state, TransferState::Done);
    assert_eq!(std::fs::read(dst.path().join("note.txt")).unwrap(), b"new");

    queue.clear_finished();
    assert!(queue.list().is_empty());
}

#[tokio::test]
async fn queued_transfers_can_be_cancelled() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let big = src.path().join("big");
    std::fs::write(&big, vec![0u8; 8 * 1024 * 1024]).unwrap();
    let small = src.path().join("small");
    std::fs::write(&small, b"x").unwrap();
    let config = TransferConfig {
        max_concurrent: 1,
        buffer_kib: 16,
        ..TransferConfig::default()
    };
    let (queue, mut rx) = queue(config);
    let first = queue.enqueue(request(&big, dst.path(), Conflict::Overwrite));
    let second = queue.enqueue(request(&small, dst.path(), Conflict::Overwrite));
    assert!(queue.cancel(second));
    let cancelled = wait_final(&mut rx, second).await;
    assert_eq!(cancelled.state, TransferState::Cancelled);
    assert!(!dst.path().join("small").exists());
    let finished = wait_final(&mut rx, first).await;
    assert_eq!(finished.state, TransferState::Done);
    assert!(
        !queue.cancel(first),
        "finished transfers cannot be cancelled"
    );
}

#[tokio::test]
async fn missing_source_fails_with_not_found() {
    let dst = tempfile::tempdir().unwrap();
    let (queue, mut rx) = queue(TransferConfig::default());
    let missing = dst.path().join("nope");
    let failed = wait_final(
        &mut rx,
        queue.enqueue(request(&missing, dst.path(), Conflict::Rename)),
    )
    .await;
    assert_eq!(failed.state, TransferState::Failed);
    assert!(matches!(failed.error, Some(rove_fs::FsError::NotFound(_))));
}

#[tokio::test]
async fn remove_all_deletes_a_tree() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().join("t");
    std::fs::create_dir_all(tree.join("x/y")).unwrap();
    std::fs::write(tree.join("x/y/z"), b"1").unwrap();
    std::fs::write(tree.join("f"), b"2").unwrap();
    remove_all(&LocalFs, &tree.to_string_lossy()).await.unwrap();
    assert!(!tree.exists());
    let fs = LocalFs;
    assert!(
        fs.list(&dir.path().to_string_lossy())
            .await
            .unwrap()
            .is_empty()
    );
}

/// A local file system whose directory listings and reads can be tampered with, standing in for
/// a misbehaving server.
struct Hostile {
    /// Extra entry name injected into every listing.
    injected: Option<&'static str>,
    /// `open_read` fails for every path.
    unreadable: bool,
    /// `rename` fails when moving a temporary transfer file.
    rename_fails: bool,
}

#[async_trait::async_trait]
impl FileSystem for Hostile {
    fn label(&self) -> String {
        "hostile".into()
    }
    async fn home(&self) -> Result<String, rove_fs::FsError> {
        LocalFs.home().await
    }
    async fn list(&self, path: &str) -> Result<Vec<rove_fs::DirEntry>, rove_fs::FsError> {
        let mut entries = LocalFs.list(path).await?;
        if let Some(name) = self.injected
            && let Some(first) = entries.first().cloned()
        {
            entries.push(rove_fs::DirEntry {
                name: name.into(),
                ..first
            });
        }
        Ok(entries)
    }
    async fn stat(&self, path: &str) -> Result<rove_fs::Metadata, rove_fs::FsError> {
        LocalFs.stat(path).await
    }
    async fn lstat(&self, path: &str) -> Result<rove_fs::Metadata, rove_fs::FsError> {
        LocalFs.lstat(path).await
    }
    async fn mkdir(&self, path: &str) -> Result<(), rove_fs::FsError> {
        LocalFs.mkdir(path).await
    }
    async fn remove_file(&self, path: &str) -> Result<(), rove_fs::FsError> {
        LocalFs.remove_file(path).await
    }
    async fn remove_dir(&self, path: &str) -> Result<(), rove_fs::FsError> {
        LocalFs.remove_dir(path).await
    }
    async fn rename(&self, from: &str, to: &str) -> Result<(), rove_fs::FsError> {
        if self.rename_fails && from.ends_with(".part") {
            return Err(rove_fs::FsError::Io(format!(
                "injected rename failure: {from}"
            )));
        }
        LocalFs.rename(from, to).await
    }
    async fn set_permissions(&self, path: &str, mode: u32) -> Result<(), rove_fs::FsError> {
        LocalFs.set_permissions(path, mode).await
    }
    async fn open_read(&self, path: &str) -> Result<rove_fs::FsReader, rove_fs::FsError> {
        if self.unreadable {
            return Err(rove_fs::FsError::PermissionDenied(path.into()));
        }
        LocalFs.open_read(path).await
    }
    async fn open_write(&self, path: &str) -> Result<rove_fs::FsWriter, rove_fs::FsError> {
        LocalFs.open_write(path).await
    }
    fn join(&self, dir: &str, name: &str) -> String {
        LocalFs.join(dir, name)
    }
    fn parent(&self, path: &str) -> Option<String> {
        LocalFs.parent(path)
    }
}

#[tokio::test]
async fn source_names_cannot_escape_the_destination() {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("remote/tree");
    let dst = root.path().join("downloads");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(src.join("ok.txt"), b"fine").unwrap();
    std::fs::write(root.path().join("victim.txt"), b"precious").unwrap();

    let (queue, mut rx) = queue(TransferConfig::default());
    let mut evil = request(&src, &dst, Conflict::Rename);
    evil.src = Arc::new(Hostile {
        injected: Some("../../victim.txt"),
        unreadable: false,
        rename_fails: false,
    });
    let done = wait_final(&mut rx, queue.enqueue(evil)).await;
    assert_eq!(done.state, TransferState::Failed, "{done:?}");
    assert_eq!(
        std::fs::read(root.path().join("victim.txt")).unwrap(),
        b"precious"
    );
}

#[tokio::test]
async fn a_failed_overwrite_keeps_the_existing_file() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let file = src.path().join("data.bin");
    std::fs::write(&file, b"new").unwrap();
    std::fs::write(dst.path().join("data.bin"), b"old").unwrap();

    let (queue, mut rx) = queue(TransferConfig::default());
    let mut failing = request(&file, dst.path(), Conflict::Overwrite);
    failing.src = Arc::new(Hostile {
        injected: None,
        unreadable: true,
        rename_fails: false,
    });
    let done = wait_final(&mut rx, queue.enqueue(failing)).await;
    assert_eq!(done.state, TransferState::Failed);
    assert_eq!(std::fs::read(dst.path().join("data.bin")).unwrap(), b"old");
    // No temporary file is left behind.
    let names: Vec<_> = std::fs::read_dir(dst.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["data.bin"]);
}

#[cfg(unix)]
#[tokio::test]
async fn remove_all_removes_a_directory_link_but_not_its_target() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("real");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"keep").unwrap();
    let tree = root.path().join("tree");
    std::fs::create_dir(&tree).unwrap();
    std::os::unix::fs::symlink(&target, tree.join("inner-link")).unwrap();
    let top_link = root.path().join("top-link");
    std::os::unix::fs::symlink(&target, &top_link).unwrap();

    remove_all(&LocalFs, &top_link.to_string_lossy())
        .await
        .unwrap();
    assert!(!top_link.exists() && std::fs::symlink_metadata(&top_link).is_err());
    remove_all(&LocalFs, &tree.to_string_lossy()).await.unwrap();
    assert!(!tree.exists());
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"keep");
}

#[tokio::test]
async fn a_failed_rename_restores_the_existing_file() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let file = src.path().join("data.bin");
    std::fs::write(&file, b"new").unwrap();
    std::fs::write(dst.path().join("data.bin"), b"old").unwrap();

    let (queue, mut rx) = queue(TransferConfig::default());
    let mut failing = request(&file, dst.path(), Conflict::Overwrite);
    failing.dst = Arc::new(Hostile {
        injected: None,
        unreadable: false,
        rename_fails: true,
    });
    let done = wait_final(&mut rx, queue.enqueue(failing)).await;
    assert_eq!(done.state, TransferState::Failed);
    assert_eq!(std::fs::read(dst.path().join("data.bin")).unwrap(), b"old");
    let names: Vec<_> = std::fs::read_dir(dst.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["data.bin"]);
}

#[cfg(unix)]
#[tokio::test]
async fn overwriting_keeps_the_existing_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let (queue, mut rx) = queue(TransferConfig::default());
    for (name, mode) in [("secret.txt", 0o600), ("tool.sh", 0o755)] {
        let file = src.path().join(name);
        std::fs::write(&file, b"new").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let target = dst.path().join(name);
        std::fs::write(&target, b"old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode)).unwrap();

        let done = wait_final(
            &mut rx,
            queue.enqueue(request(&file, dst.path(), Conflict::Overwrite)),
        )
        .await;
        assert_eq!(done.state, TransferState::Done, "{:?}", done.error);
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        let kept = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(kept, mode, "{name}");
    }
}

#[tokio::test]
async fn concurrent_renames_pick_different_names() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let (queue, mut rx) = queue(TransferConfig::default());
    let mut ids = Vec::new();
    for (folder, content) in [
        ("a", vec![1u8; 2_000_000]),
        ("b", vec![2u8; 2_000_000]),
        ("c", vec![3u8; 2_000_000]),
    ] {
        let dir = src.path().join(folder);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("note.bin"), content).unwrap();
        ids.push(queue.enqueue(request(&dir.join("note.bin"), dst.path(), Conflict::Rename)));
    }
    // Collect every final update in one pass: waiting per id would drop the others' events.
    let mut finished = std::collections::HashMap::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        while finished.len() < ids.len() {
            let update = rx.recv().await.unwrap();
            if ids.contains(&update.id) && update.state.is_final() {
                finished.insert(update.id, update);
            }
        }
    })
    .await
    .expect("transfers finished in time");
    for done in finished.values() {
        assert_eq!(done.state, TransferState::Done, "{:?}", done.error);
    }
    let mut firsts: Vec<u8> = ["note.bin", "note (1).bin", "note (2).bin"]
        .iter()
        .map(|name| std::fs::read(dst.path().join(name)).unwrap()[0])
        .collect();
    firsts.sort();
    assert_eq!(firsts, [1, 2, 3]);
}

#[tokio::test]
async fn overwrite_never_replaces_a_directory_with_a_file_or_back() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let (queue, mut rx) = queue(TransferConfig::default());

    // A file named like an existing directory.
    std::fs::write(src.path().join("data"), b"file").unwrap();
    std::fs::create_dir(dst.path().join("data")).unwrap();
    std::fs::write(dst.path().join("data/inside.txt"), b"keep").unwrap();
    let done = wait_final(
        &mut rx,
        queue.enqueue(request(
            &src.path().join("data"),
            dst.path(),
            Conflict::Overwrite,
        )),
    )
    .await;
    assert_eq!(done.state, TransferState::Failed);
    assert_eq!(
        std::fs::read(dst.path().join("data/inside.txt")).unwrap(),
        b"keep"
    );

    // A directory named like an existing file.
    std::fs::create_dir(src.path().join("logs")).unwrap();
    std::fs::write(src.path().join("logs/a.log"), b"new").unwrap();
    std::fs::write(dst.path().join("logs"), b"old file").unwrap();
    let done = wait_final(
        &mut rx,
        queue.enqueue(request(
            &src.path().join("logs"),
            dst.path(),
            Conflict::Overwrite,
        )),
    )
    .await;
    assert_eq!(done.state, TransferState::Failed);
    assert_eq!(std::fs::read(dst.path().join("logs")).unwrap(), b"old file");

    let mut names: Vec<_> = std::fs::read_dir(dst.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["data", "logs"]);
}

/// A local directory that behaves like a case-insensitive volume: every path below `root` is
/// stored lowercased.
struct CaseFolding {
    root: String,
}

impl CaseFolding {
    fn fold(&self, path: &str) -> String {
        match path.strip_prefix(&self.root) {
            Some(rest) => format!("{}{}", self.root, rest.to_lowercase()),
            None => path.to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl FileSystem for CaseFolding {
    fn label(&self) -> String {
        "folding".into()
    }
    fn folds_case(&self) -> bool {
        true
    }
    async fn home(&self) -> Result<String, rove_fs::FsError> {
        Ok(self.root.clone())
    }
    async fn list(&self, path: &str) -> Result<Vec<rove_fs::DirEntry>, rove_fs::FsError> {
        LocalFs.list(&self.fold(path)).await
    }
    async fn stat(&self, path: &str) -> Result<rove_fs::Metadata, rove_fs::FsError> {
        LocalFs.stat(&self.fold(path)).await
    }
    async fn lstat(&self, path: &str) -> Result<rove_fs::Metadata, rove_fs::FsError> {
        LocalFs.lstat(&self.fold(path)).await
    }
    async fn mkdir(&self, path: &str) -> Result<(), rove_fs::FsError> {
        LocalFs.mkdir(&self.fold(path)).await
    }
    async fn remove_file(&self, path: &str) -> Result<(), rove_fs::FsError> {
        LocalFs.remove_file(&self.fold(path)).await
    }
    async fn remove_dir(&self, path: &str) -> Result<(), rove_fs::FsError> {
        LocalFs.remove_dir(&self.fold(path)).await
    }
    async fn rename(&self, from: &str, to: &str) -> Result<(), rove_fs::FsError> {
        LocalFs.rename(&self.fold(from), &self.fold(to)).await
    }
    async fn set_permissions(&self, path: &str, mode: u32) -> Result<(), rove_fs::FsError> {
        LocalFs.set_permissions(&self.fold(path), mode).await
    }
    async fn open_read(&self, path: &str) -> Result<rove_fs::FsReader, rove_fs::FsError> {
        LocalFs.open_read(&self.fold(path)).await
    }
    async fn open_write(&self, path: &str) -> Result<rove_fs::FsWriter, rove_fs::FsError> {
        LocalFs.open_write(&self.fold(path)).await
    }
    fn join(&self, dir: &str, name: &str) -> String {
        LocalFs.join(dir, name)
    }
    fn parent(&self, path: &str) -> Option<String> {
        LocalFs.parent(path)
    }
}

#[tokio::test]
async fn concurrent_names_differing_in_case_do_not_collide_on_a_folding_volume() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let folding: Arc<dyn FileSystem> = Arc::new(CaseFolding {
        root: dst.path().to_string_lossy().into_owned(),
    });
    let (queue, mut rx) = queue(TransferConfig::default());
    let mut ids = Vec::new();
    for (name, content) in [("Report.bin", 1u8), ("report.bin", 2u8)] {
        let file = src.path().join(name);
        std::fs::write(&file, vec![content; 2_000_000]).unwrap();
        let mut request = request(&file, dst.path(), Conflict::Rename);
        request.dst = folding.clone();
        ids.push(queue.enqueue(request));
    }
    let mut finished = std::collections::HashMap::new();
    tokio::time::timeout(Duration::from_secs(20), async {
        while finished.len() < ids.len() {
            let update = rx.recv().await.unwrap();
            if ids.contains(&update.id) && update.state.is_final() {
                finished.insert(update.id, update);
            }
        }
    })
    .await
    .expect("transfers finished in time");
    for done in finished.values() {
        assert_eq!(done.state, TransferState::Done, "{:?}", done.error);
    }
    let mut firsts: Vec<u8> = std::fs::read_dir(dst.path())
        .unwrap()
        .map(|entry| std::fs::read(entry.unwrap().path()).unwrap()[0])
        .collect();
    firsts.sort();
    assert_eq!(firsts, [1, 2]);
}

/// Runs three large copies under `limit` and returns the most that were running at once.
async fn peak_running(initial: usize, limit: usize) -> usize {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let (queue, mut rx) = queue(TransferConfig {
        max_concurrent: initial,
        ..TransferConfig::default()
    });
    queue.set_max_concurrent(limit);
    let mut ids = Vec::new();
    for name in ["a.bin", "b.bin", "c.bin"] {
        std::fs::write(src.path().join(name), vec![7u8; 3_000_000]).unwrap();
        ids.push(queue.enqueue(request(&src.path().join(name), dst.path(), Conflict::Rename)));
    }
    let mut running = std::collections::HashSet::new();
    let mut finished = 0;
    let mut peak = 0;
    tokio::time::timeout(Duration::from_secs(20), async {
        while finished < ids.len() {
            let update = rx.recv().await.unwrap();
            if update.state.is_final() {
                running.remove(&update.id);
                finished += 1;
            } else if update.state == TransferState::Running {
                running.insert(update.id);
            }
            peak = peak.max(running.len());
        }
    })
    .await
    .expect("transfers finished in time");
    peak
}

#[tokio::test]
async fn concurrency_limit_changes_at_runtime() {
    assert_eq!(peak_running(1, 3).await, 3);
    assert_eq!(peak_running(3, 1).await, 1);
}
