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
