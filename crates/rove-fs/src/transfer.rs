//! Copies files and directory trees between two [`FileSystem`]s with bounded concurrency, progress
//! events and cancellation.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rove_kernel::{Event, EventBus};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{EntryKind, FileSystem, FsError};

/// `[plugin.fs]` configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TransferConfig {
    /// Transfers running at the same time (1–16); the rest wait in the queue.
    pub max_concurrent: usize,
    /// Copy buffer per transfer, KiB (16–4096).
    pub buffer_kib: usize,
    /// Minimum interval between progress events of one transfer, milliseconds (50–5000).
    pub progress_interval_ms: u64,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 3,
            buffer_kib: 256,
            progress_interval_ms: 200,
        }
    }
}

impl TransferConfig {
    /// Checks every field against its documented range.
    pub fn validate(&self) -> Result<(), String> {
        let check = |name: &str, value: u64, min: u64, max: u64| {
            if (min..=max).contains(&value) {
                Ok(())
            } else {
                Err(format!("{name} must be {min}..={max}, got {value}"))
            }
        };
        check("max_concurrent", self.max_concurrent as u64, 1, 16)?;
        check("buffer_kib", self.buffer_kib as u64, 16, 4096)?;
        check("progress_interval_ms", self.progress_interval_ms, 50, 5000)
    }
}

/// Identifier of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransferId(Uuid);

impl fmt::Display for TransferId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for TransferId {
    type Err = FsError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(text)
            .map(Self)
            .map_err(|_| FsError::InvalidPath(format!("invalid transfer id {text:?}")))
    }
}

/// What to do when the destination name already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Conflict {
    /// Replace the existing file; directories are merged.
    Overwrite,
    /// Pick a free name: `name (1).ext`, `name (2).ext`, ….
    Rename,
    /// Leave the destination alone and finish without copying.
    Skip,
}

/// One item to copy: `src_path` (a file or directory) into the directory `dst_dir`.
pub struct TransferRequest {
    /// Source file system.
    pub src: Arc<dyn FileSystem>,
    /// Source file or directory.
    pub src_path: String,
    /// Destination file system.
    pub dst: Arc<dyn FileSystem>,
    /// Existing destination directory.
    pub dst_dir: String,
    /// Name collision policy for the top-level item.
    pub conflict: Conflict,
    /// Caller-defined label carried in every update, such as `upload`.
    pub tag: String,
}

/// Lifecycle of a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    /// Waiting for a free slot.
    Queued,
    /// Copying.
    Running,
    /// Finished; `skipped` tells whether anything was copied.
    Done,
    /// Stopped by an error.
    Failed,
    /// Stopped by [`TransferQueue::cancel`].
    Cancelled,
}

impl TransferState {
    /// Returns `true` for states that no longer change.
    pub fn is_final(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// Live event: the state or progress of a transfer changed. Also the row type of
/// [`TransferQueue::list`].
#[derive(Debug, Clone, Serialize)]
pub struct TransferUpdate {
    /// Transfer identifier.
    pub id: TransferId,
    /// The request's tag.
    pub tag: String,
    /// Top-level item name.
    pub name: String,
    /// Source provider label.
    pub src_label: String,
    /// Source path.
    pub src_path: String,
    /// Destination provider label.
    pub dst_label: String,
    /// Destination path of the top-level item; empty until resolved.
    pub dst_path: String,
    /// Current state.
    pub state: TransferState,
    /// Bytes copied so far.
    pub bytes_done: u64,
    /// Bytes to copy; known once the source tree is scanned.
    pub bytes_total: u64,
    /// Files copied so far.
    pub files_done: u64,
    /// Files to copy.
    pub files_total: u64,
    /// The destination existed and [`Conflict::Skip`] applied.
    pub skipped: bool,
    /// Failure reason when `state` is `failed`.
    pub error: Option<FsError>,
}
impl Event for TransferUpdate {}

struct Job {
    update: TransferUpdate,
    cancel: CancellationToken,
}

type Jobs = Arc<Mutex<HashMap<TransferId, Job>>>;

/// Runs transfers. At most `max_concurrent` copy at a time; the others wait as `queued`.
pub struct TransferQueue {
    config: TransferConfig,
    events: EventBus,
    permits: Arc<Semaphore>,
    jobs: Jobs,
    reserved: Reserved,
}

/// Destination paths claimed by running transfers, as (file system label, path), the path
/// lowercased on file systems that fold case. Two transfers never pick the same top-level
/// destination, so "rename" cannot make them overwrite each other.
type Reserved = Arc<Mutex<std::collections::HashSet<(String, String)>>>;

/// Releases a reserved destination when the transfer ends.
struct Reservation {
    reserved: Reserved,
    key: (String, String),
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.reserved.lock().remove(&self.key);
    }
}

impl TransferQueue {
    /// Creates a queue that reports progress on `events`.
    pub fn new(config: TransferConfig, events: EventBus) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(config.max_concurrent)),
            config,
            events,
            jobs: Arc::default(),
            reserved: Arc::default(),
        }
    }

    /// Queues `request` and returns its identifier. Must be called inside a Tokio runtime.
    pub fn enqueue(&self, request: TransferRequest) -> TransferId {
        let id = TransferId(Uuid::now_v7());
        let update = TransferUpdate {
            id,
            tag: request.tag.clone(),
            name: basename(&request.src_path),
            src_label: request.src.label(),
            src_path: request.src_path.clone(),
            dst_label: request.dst.label(),
            dst_path: String::new(),
            state: TransferState::Queued,
            bytes_done: 0,
            bytes_total: 0,
            files_done: 0,
            files_total: 0,
            skipped: false,
            error: None,
        };
        let cancel = CancellationToken::new();
        self.jobs.lock().insert(
            id,
            Job {
                update: update.clone(),
                cancel: cancel.clone(),
            },
        );
        self.events.emit(&update);
        let mut runner = Runner {
            update,
            jobs: self.jobs.clone(),
            events: self.events.clone(),
            interval: Duration::from_millis(self.config.progress_interval_ms),
            last_emit: Instant::now(),
            buffer: self.config.buffer_kib * 1024,
            cancel,
            reserved: self.reserved.clone(),
        };
        let permits = self.permits.clone();
        tokio::spawn(async move {
            let permit = tokio::select! {
                () = runner.cancel.cancelled() => None,
                permit = permits.acquire_owned() => permit.ok(),
            };
            let result = match permit {
                Some(_permit) => {
                    runner.set_state(TransferState::Running);
                    runner.run(&request).await
                }
                None => Err(FsError::Cancelled),
            };
            runner.finish(result);
        });
        id
    }

    /// Cancels a queued or running transfer. Returns `false` when it is unknown or finished.
    pub fn cancel(&self, id: TransferId) -> bool {
        match self.jobs.lock().get(&id) {
            Some(job) if !job.update.state.is_final() => {
                job.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    /// Cancels every unfinished transfer.
    pub fn cancel_all(&self) {
        for job in self.jobs.lock().values() {
            job.cancel.cancel();
        }
    }

    /// Returns every known transfer, oldest first.
    pub fn list(&self) -> Vec<TransferUpdate> {
        let mut rows: Vec<_> = self
            .jobs
            .lock()
            .values()
            .map(|job| job.update.clone())
            .collect();
        rows.sort_by_key(|row| row.id.0);
        rows
    }

    /// Forgets finished transfers.
    pub fn clear_finished(&self) {
        self.jobs
            .lock()
            .retain(|_, job| !job.update.state.is_final());
    }
}

/// Last path component, accepting both `/` and `\` separators.
fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed)
        .to_owned()
}

/// Fails when overwriting would put a directory in place of a file or a file in place of a
/// directory: replacing a directory would move its whole content away.
fn check_same_kind(path: &str, wanted: EntryKind, existing: EntryKind) -> Result<(), FsError> {
    let is_dir = |kind| kind == EntryKind::Dir;
    if is_dir(wanted) != is_dir(existing) {
        return Err(FsError::AlreadyExists(format!(
            "{path} exists as a {}",
            if is_dir(existing) {
                "directory"
            } else {
                "file"
            }
        )));
    }
    Ok(())
}

/// Joins `name` to `dir` on `fs`, refusing names that are not a single component (`..`,
/// separators) or that the platform would not place directly inside `dir` (a Windows drive prefix
/// such as `C:x`).
fn child_path(fs: &dyn FileSystem, dir: &str, name: &str) -> Result<String, FsError> {
    crate::validate_name(name)?;
    let path = fs.join(dir, name);
    let parent = fs.parent(&path);
    let normalize = |path: &str| path.trim_end_matches(['/', '\\']).to_owned();
    if parent.as_deref().map(normalize) != Some(normalize(dir)) || basename(&path) != name {
        return Err(FsError::InvalidPath(name.to_owned()));
    }
    Ok(path)
}

/// `report.txt` + 2 → `report (2).txt`; names without an extension or starting with a dot get the
/// suffix at the end.
fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({n}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({n})"),
    }
}

enum Step {
    Dir(String),
    File { src: String, dst: String, size: u64 },
}

struct Runner {
    update: TransferUpdate,
    jobs: Jobs,
    events: EventBus,
    interval: Duration,
    last_emit: Instant,
    buffer: usize,
    cancel: CancellationToken,
    reserved: Reserved,
}

impl Runner {
    fn publish(&mut self) {
        self.last_emit = Instant::now();
        if let Some(job) = self.jobs.lock().get_mut(&self.update.id) {
            job.update = self.update.clone();
        }
        self.events.emit(&self.update);
    }

    fn set_state(&mut self, state: TransferState) {
        self.update.state = state;
        self.publish();
    }

    fn progress(&mut self) {
        if self.last_emit.elapsed() >= self.interval {
            self.publish();
        }
    }

    fn finish(&mut self, result: Result<(), FsError>) {
        self.update.state = match result {
            Ok(()) => TransferState::Done,
            Err(FsError::Cancelled) => TransferState::Cancelled,
            Err(error) => {
                self.update.error = Some(error);
                TransferState::Failed
            }
        };
        self.publish();
    }

    fn check(&self) -> Result<(), FsError> {
        if self.cancel.is_cancelled() {
            Err(FsError::Cancelled)
        } else {
            Ok(())
        }
    }

    async fn exists(fs: &dyn FileSystem, path: &str) -> Result<bool, FsError> {
        match fs.stat(path).await {
            Ok(_) => Ok(true),
            Err(FsError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Claims `path` on the destination for this transfer; `None` when another running transfer
    /// holds it.
    fn reserve(&self, request: &TransferRequest, path: &str) -> Option<Reservation> {
        let path = if request.dst.folds_case() {
            path.to_lowercase()
        } else {
            path.to_owned()
        };
        let key = (request.dst.label(), path);
        self.reserved
            .lock()
            .insert(key.clone())
            .then(|| Reservation {
                reserved: self.reserved.clone(),
                key,
            })
    }

    /// Picks and reserves the destination path of the top-level item, or `None` to skip. A path
    /// another running transfer reserved counts as taken. Overwriting refuses to replace a file
    /// with a directory or a directory with a file.
    async fn destination(
        &self,
        request: &TransferRequest,
        src_kind: EntryKind,
    ) -> Result<Option<(String, Reservation)>, FsError> {
        let dst = request.dst.as_ref();
        let name = &self.update.name;
        let path = child_path(dst, &request.dst_dir, name)?;
        let existing = match dst.stat(&path).await {
            Ok(metadata) => Some(metadata.kind),
            Err(FsError::NotFound(_)) => None,
            Err(error) => return Err(error),
        };
        match (existing, request.conflict) {
            (None, _) | (Some(_), Conflict::Overwrite) => {
                if let Some(kind) = existing {
                    check_same_kind(&path, src_kind, kind)?;
                }
                match self.reserve(request, &path) {
                    Some(reservation) => Ok(Some((path, reservation))),
                    // Another transfer is writing this name right now.
                    None if request.conflict == Conflict::Rename => {
                        self.renamed(request, name).await
                    }
                    None if request.conflict == Conflict::Skip => Ok(None),
                    None => Err(FsError::AlreadyExists(path)),
                }
            }
            (Some(_), Conflict::Skip) => Ok(None),
            (Some(_), Conflict::Rename) => self.renamed(request, name).await,
        }
    }

    /// The first free `name (n).ext` in the destination directory, reserved.
    async fn renamed(
        &self,
        request: &TransferRequest,
        name: &str,
    ) -> Result<Option<(String, Reservation)>, FsError> {
        let dst = request.dst.as_ref();
        for n in 1..=999 {
            let candidate = child_path(dst, &request.dst_dir, &numbered(name, n))?;
            if Self::exists(dst, &candidate).await? {
                continue;
            }
            if let Some(reservation) = self.reserve(request, &candidate) {
                return Ok(Some((candidate, reservation)));
            }
        }
        Err(FsError::AlreadyExists(dst.join(&request.dst_dir, name)))
    }

    /// Expands the source into directory and file steps, parents before children.
    async fn plan(
        &mut self,
        request: &TransferRequest,
        dst_path: &str,
    ) -> Result<Vec<Step>, FsError> {
        let src = request.src.as_ref();
        let dst = request.dst.as_ref();
        let top = src.stat(&request.src_path).await?;
        if top.kind != EntryKind::Dir {
            return Ok(vec![Step::File {
                src: request.src_path.clone(),
                dst: dst_path.to_owned(),
                size: top.size,
            }]);
        }
        let mut steps = vec![Step::Dir(dst_path.to_owned())];
        let mut pending = vec![(request.src_path.clone(), dst_path.to_owned())];
        while let Some((src_dir, dst_dir)) = pending.pop() {
            self.check()?;
            for entry in src.list(&src_dir).await? {
                // Names come from the source (possibly a remote server) and must stay one
                // component below the destination directory.
                let dst_child = child_path(dst, &dst_dir, &entry.name)?;
                let src_child = src.join(&src_dir, &entry.name);
                let metadata = match entry.metadata.kind {
                    // Follow links to files; linked directories are skipped to avoid cycles.
                    EntryKind::Symlink => match src.stat(&src_child).await {
                        Ok(target) if target.kind == EntryKind::File => target,
                        _ => continue,
                    },
                    EntryKind::Other => continue,
                    _ => entry.metadata,
                };
                if metadata.kind == EntryKind::Dir {
                    steps.push(Step::Dir(dst_child.clone()));
                    pending.push((src_child, dst_child));
                } else {
                    steps.push(Step::File {
                        src: src_child,
                        dst: dst_child,
                        size: metadata.size,
                    });
                }
            }
        }
        Ok(steps)
    }

    async fn run(&mut self, request: &TransferRequest) -> Result<(), FsError> {
        let src_kind = request.src.stat(&request.src_path).await?.kind;
        let Some((dst_path, _reservation)) = self.destination(request, src_kind).await? else {
            self.update.skipped = true;
            return Ok(());
        };
        self.update.dst_path = dst_path.clone();
        let steps = self.plan(request, &dst_path).await?;
        for step in &steps {
            if let Step::File { size, .. } = step {
                self.update.files_total += 1;
                self.update.bytes_total += size;
            }
        }
        self.publish();
        for step in &steps {
            self.check()?;
            match step {
                Step::Dir(path) => {
                    if let Err(error) = request.dst.mkdir(path).await {
                        // An existing directory is merged into (some SFTP servers report it as
                        // a generic failure); anything else in its place is an error.
                        match request.dst.stat(path).await {
                            Ok(existing) => check_same_kind(path, EntryKind::Dir, existing.kind)?,
                            Err(_) => return Err(error),
                        }
                    }
                }
                Step::File { src, dst, .. } => {
                    self.copy_file(request, src, dst).await?;
                    self.update.files_done += 1;
                    self.progress();
                }
            }
        }
        Ok(())
    }

    /// Copies `src` to a temporary file beside `dst`, then moves it over `dst`. The temporary file
    /// gets an existing `dst`'s permissions before any data is written, so the copy is never more
    /// readable than the file it replaces. A failure at any point leaves an existing `dst`
    /// untouched and removes only the temporary file.
    async fn copy_file(
        &mut self,
        request: &TransferRequest,
        src: &str,
        dst: &str,
    ) -> Result<(), FsError> {
        let fs = request.dst.as_ref();
        let dir = fs
            .parent(dst)
            .ok_or_else(|| FsError::InvalidPath(dst.to_owned()))?;
        let name: String = basename(dst).chars().take(100).collect();
        let temp = fs.join(&dir, &format!(".{name}.{}.part", self.update.id));
        let keep_mode = match fs.stat(dst).await {
            Ok(existing) => {
                check_same_kind(dst, EntryKind::File, existing.kind)?;
                // Only "overwrite" replaces files; with "rename" or "skip" a file that appeared
                // meanwhile is left alone.
                if request.conflict != Conflict::Overwrite {
                    return Err(FsError::AlreadyExists(dst.to_owned()));
                }
                existing.permissions
            }
            Err(FsError::NotFound(_)) => None,
            Err(error) => return Err(error),
        };
        let result = match self.copy(request, src, &temp, keep_mode).await {
            Ok(()) => Self::replace(fs, &temp, dst, &format!("{temp}.old")).await,
            Err(error) => Err(error),
        };
        if result.is_err()
            && let Err(cleanup) = fs.remove_file(&temp).await
            && !matches!(cleanup, FsError::NotFound(_))
        {
            tracing::debug!(%cleanup, temp, "removing temporary file failed");
        }
        result
    }

    /// Moves `temp` to `dst` without ever deleting `dst` while it is the only copy. When rename
    /// refuses an existing target (SFTP version 3), `dst` is first moved aside to `backup`, then
    /// `temp` takes its place and the backup is removed; if that fails the backup is moved back.
    async fn replace(
        fs: &dyn FileSystem,
        temp: &str,
        dst: &str,
        backup: &str,
    ) -> Result<(), FsError> {
        let first = match fs.rename(temp, dst).await {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if !Self::exists(fs, dst).await? {
            return Err(first);
        }
        fs.rename(dst, backup).await?;
        match fs.rename(temp, dst).await {
            Ok(()) => {
                if let Err(error) = fs.remove_file(backup).await {
                    tracing::warn!(%error, backup, "removing the replaced file failed");
                }
                Ok(())
            }
            Err(error) => {
                if let Err(restore) = fs.rename(backup, dst).await {
                    tracing::error!(%restore, backup, dst, "restoring the original file failed");
                }
                Err(error)
            }
        }
    }

    /// Copies `src` to `dst`. With `mode`, `dst` gets those permissions right after it is created,
    /// before data is written.
    async fn copy(
        &mut self,
        request: &TransferRequest,
        src: &str,
        dst: &str,
        mode: Option<u32>,
    ) -> Result<(), FsError> {
        let mut reader = request.src.open_read(src).await?;
        let mut writer = request.dst.open_write(dst).await?;
        if let Some(mode) = mode {
            request.dst.set_permissions(dst, mode).await?;
        }
        let mut buffer = vec![0; self.buffer];
        let cancel = self.cancel.clone();
        loop {
            let read = tokio::select! {
                () = cancel.cancelled() => return Err(FsError::Cancelled),
                read = reader.read(&mut buffer) => read.map_err(|error| FsError::from_io(&error, src))?,
            };
            if read == 0 {
                break;
            }
            tokio::select! {
                () = cancel.cancelled() => return Err(FsError::Cancelled),
                written = writer.write_all(&buffer[..read]) => {
                    written.map_err(|error| FsError::from_io(&error, dst))?;
                }
            }
            self.update.bytes_done += read as u64;
            self.progress();
        }
        writer
            .shutdown()
            .await
            .map_err(|error| FsError::from_io(&error, dst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_and_numbering() {
        assert_eq!(basename("/a/b.txt"), "b.txt");
        assert_eq!(basename("/a/dir/"), "dir");
        assert_eq!(basename(r"C:\x\y.log"), "y.log");
        assert_eq!(numbered("b.txt", 1), "b (1).txt");
        assert_eq!(numbered("archive.tar.gz", 2), "archive.tar (2).gz");
        assert_eq!(numbered(".bashrc", 1), ".bashrc (1)");
        assert_eq!(numbered("dir", 3), "dir (3)");
    }

    #[test]
    fn config_ranges() {
        assert!(TransferConfig::default().validate().is_ok());
        let config = TransferConfig {
            max_concurrent: 0,
            ..TransferConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
