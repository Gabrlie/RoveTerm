//! Working copies and version-checked uploads for the desktop external editor.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rove_fs::{
    FileSystem, TEXT_FILE_LIMIT, TextEditError, TextFile, read_text_file, save_text_file,
    text_version, validate_text,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const POLL: Duration = Duration::from_millis(500);
const STABLE_FOR: Duration = Duration::from_secs(1);
const MANIFEST_LIMIT: usize = 64 * 1024;
const PATH_LIMIT: usize = 4096;
const SESSION_LIMIT: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EditId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EditState {
    Watching,
    Uploading,
    Saved,
    Conflict,
    Failed,
    Recovered,
    Closed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalEditError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalEditUpdate {
    pub id: EditId,
    pub label: String,
    pub path: String,
    pub local_path: String,
    pub state: EditState,
    pub error: Option<ExternalEditError>,
    pub revision: u64,
}

pub type UpdateSink = Arc<dyn Fn(ExternalEditUpdate) + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("invalid external edit path")]
    InvalidPath,
    #[error("external edit session was not found")]
    Missing,
    #[error("external edit session is busy")]
    Busy,
    #[error("too many external edit sessions are open")]
    Limit,
    #[error("no external editor is configured")]
    EditorMissing,
    #[error("could not start external editor: {0}")]
    EditorLaunch(String),
    #[error("reconnect to the same host before retrying this file")]
    Reconnect,
    #[error("the local copy is not stable")]
    LocalChanged,
    #[error("could not access the external editor working copy: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Text(#[from] TextEditError),
}

impl EditError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPath => "input.invalid",
            Self::Missing => "fs.edit_missing",
            Self::Busy => "fs.edit_busy",
            Self::Limit => "fs.edit_limit",
            Self::EditorMissing => "fs.edit_editor_missing",
            Self::EditorLaunch(_) => "fs.edit_editor_launch",
            Self::Reconnect => "fs.edit_reconnect",
            Self::LocalChanged => "fs.edit_local_changed",
            Self::Io(_) => "fs.edit_local_io",
            Self::Text(error) => text_error_code(error),
        }
    }
}

pub fn text_error_code(error: &TextEditError) -> &'static str {
    match error {
        TextEditError::Conflict => "fs.edit_conflict",
        TextEditError::TooLarge => "fs.edit_too_large",
        TextEditError::NotText => "fs.edit_not_text",
        TextEditError::NotRegularFile => "fs.edit_not_regular",
        TextEditError::InvalidVersion => "fs.edit_invalid_version",
        TextEditError::UnsupportedCreate => "fs.edit_unsupported",
        TextEditError::Fs(_) => "fs.edit_error",
    }
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    id: EditId,
    label: String,
    path: String,
    version: String,
}

struct Status {
    fs: Option<Arc<dyn FileSystem>>,
    version: String,
    state: EditState,
    error: Option<ExternalEditError>,
    revision: u64,
}

struct Session {
    id: EditId,
    label: String,
    path: String,
    dir: PathBuf,
    document: PathBuf,
    operation: AsyncMutex<()>,
    status: Mutex<Status>,
    stop: CancellationToken,
}

impl Session {
    fn new(root: &Path, manifest: Manifest, fs: Option<Arc<dyn FileSystem>>) -> Self {
        let dir = root.join(manifest.id.0.to_string());
        let document = dir.join("copy").join(working_name(&manifest.path));
        let state = if fs.is_some() {
            EditState::Watching
        } else {
            EditState::Recovered
        };
        Self {
            id: manifest.id,
            label: manifest.label,
            path: manifest.path,
            dir,
            document,
            operation: AsyncMutex::new(()),
            status: Mutex::new(Status {
                fs,
                version: manifest.version,
                state,
                error: None,
                revision: 0,
            }),
            stop: CancellationToken::new(),
        }
    }

    fn manifest(&self, version: String) -> Manifest {
        Manifest {
            id: self.id,
            label: self.label.clone(),
            path: self.path.clone(),
            version,
        }
    }

    fn update(&self) -> ExternalEditUpdate {
        let status = self.status.lock();
        ExternalEditUpdate {
            id: self.id,
            label: self.label.clone(),
            path: self.path.clone(),
            local_path: self.document.to_string_lossy().into_owned(),
            state: status.state,
            error: status.error.clone(),
            revision: status.revision,
        }
    }

    fn set_state(&self, state: EditState, error: Option<&EditError>, emit: &UpdateSink) {
        {
            let mut status = self.status.lock();
            status.state = state;
            status.error = error.map(|error| ExternalEditError {
                code: error.code(),
                message: error.to_string(),
            });
            status.revision += 1;
        }
        emit(self.update());
    }

    fn failed(&self, error: &EditError, emit: &UpdateSink) {
        let state = if matches!(error, EditError::Text(TextEditError::Conflict)) {
            EditState::Conflict
        } else {
            EditState::Failed
        };
        self.set_state(state, Some(error), emit);
    }

    fn filesystem(
        &self,
        reopened: Option<Arc<dyn FileSystem>>,
    ) -> Result<Arc<dyn FileSystem>, EditError> {
        // The provider label comes from the authenticated SSH target and includes the route.
        match reopened.or_else(|| self.status.lock().fs.clone()) {
            Some(fs) if fs.label() == self.label => Ok(fs),
            _ => Err(EditError::Reconnect),
        }
    }

    fn live(&self) -> Result<(), EditError> {
        if self.stop.is_cancelled() {
            Err(EditError::Missing)
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Sessions {
    by_id: HashMap<EditId, Arc<Session>>,
    watchers: HashMap<EditId, JoinHandle<()>>,
    closing: bool,
}

pub struct ExternalEditManager {
    root: PathBuf,
    sessions: Mutex<Sessions>,
    shutdown: AsyncMutex<()>,
}

impl ExternalEditManager {
    /// Reads recovery manifests without touching the remote host. Unavailable local storage does
    /// not prevent the rest of the application from starting.
    pub fn new(data_dir: &Path) -> Self {
        let root = data_dir.join("external-edits");
        let manager = Self {
            root,
            sessions: Mutex::new(Sessions::default()),
            shutdown: AsyncMutex::new(()),
        };
        if cfg!(desktop) {
            manager.restore();
        }
        manager
    }

    /// Creates a private working copy and launches the configured editor without a shell. Failed
    /// launches leave the copy visible so the user can open it again after fixing their settings.
    pub async fn start(
        &self,
        emit: UpdateSink,
        fs: Arc<dyn FileSystem>,
        path: &str,
        editor: rove_vault::ExternalEditorPrefs,
    ) -> Result<ExternalEditUpdate, EditError> {
        if !valid_remote_path(path) {
            return Err(EditError::InvalidPath);
        }
        if editor.program.is_empty() {
            return Err(EditError::EditorMissing);
        }
        let label = fs.label();
        {
            let sessions = self.sessions.lock();
            check_slot(&sessions, &label, path)?;
        }
        let TextFile { text, version } = read_text_file(fs.as_ref(), path).await?;
        let manifest = Manifest {
            id: EditId(Uuid::now_v7()),
            label,
            path: path.into(),
            version,
        };
        let session = Arc::new(Session::new(&self.root, manifest, Some(fs)));
        let _op = session.operation.lock().await;
        {
            let mut sessions = self.sessions.lock();
            check_slot(&sessions, &session.label, &session.path)?;
            sessions.by_id.insert(session.id, session.clone());
        }
        let root = self.root.clone();
        let copy = session.clone();
        let created = blocking(move || initialize(&root, &copy, text.as_bytes())).await;
        if let Err(error) = created {
            self.sessions.lock().by_id.remove(&session.id);
            return Err(error);
        }
        if session.live().is_ok() {
            match launch(&editor, &session.document).await {
                Ok(()) => {
                    session.set_state(EditState::Watching, None, &emit);
                    self.ensure_watcher(&session, emit);
                }
                Err(error) => session.failed(&error, &emit),
            }
        }
        Ok(session.update())
    }

    /// Relauches an existing local copy without changing its upload or conflict state, except
    /// that a previous launch failure can start watching again after a successful launch.
    pub async fn reopen(
        &self,
        id: EditId,
        editor: rove_vault::ExternalEditorPrefs,
        emit: UpdateSink,
    ) -> Result<(), EditError> {
        let session = self.get(id)?;
        let _op = session.operation.lock().await;
        session.live()?;
        if editor.program.is_empty() {
            return Err(EditError::EditorMissing);
        }
        let path = session.document.clone();
        blocking(move || regular_copy(&path).map(|_| ())).await?;
        launch(&editor, &session.document).await?;
        let restart = {
            let status = session.status.lock();
            status.fs.is_some()
                && status
                    .error
                    .as_ref()
                    .is_some_and(|error| error.code == "fs.edit_editor_launch")
        };
        if restart {
            session.set_state(EditState::Watching, None, &emit);
            self.ensure_watcher(&session, emit);
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<ExternalEditUpdate> {
        self.sessions
            .lock()
            .by_id
            .values()
            .map(|session| session.update())
            .collect()
    }

    /// Returns the version a user can explicitly approve replacing; never returns file content.
    pub async fn remote_version(
        &self,
        id: EditId,
        reopened: Option<Arc<dyn FileSystem>>,
    ) -> Result<String, EditError> {
        let session = self.get(id)?;
        let _op = session.operation.lock().await;
        session.live()?;
        let fs = session.filesystem(reopened)?;
        Ok(read_text_file(fs.as_ref(), &session.path).await?.version)
    }

    /// Reattaches to the same SSH target and tries a stable local save. An explicit replacement
    /// is conditional on the version shown before the user confirmed it.
    pub async fn retry(
        &self,
        id: EditId,
        reopened: Option<Arc<dyn FileSystem>>,
        approved: Option<String>,
        emit: UpdateSink,
    ) -> Result<(), EditError> {
        let session = self.get(id)?;
        let _op = session.operation.lock().await;
        session.live()?;
        let fs = session.filesystem(reopened)?;
        session.status.lock().fs = Some(fs);
        session.set_state(EditState::Uploading, None, &emit);
        let result = async {
            let bytes = stable_copy(&session.document).await?;
            session.live()?;
            commit(&session, &bytes, approved.as_deref()).await
        }
        .await;
        match result {
            Ok(()) => {
                session.set_state(EditState::Saved, None, &emit);
                self.ensure_watcher(&session, emit);
                Ok(())
            }
            Err(error) => {
                session.failed(&error, &emit);
                Err(error)
            }
        }
    }

    /// After the user has closed their document, uploads any last stable changes and removes the
    /// working copy only when it still matches the uploaded version.
    pub async fn finish(
        &self,
        id: EditId,
        reopened: Option<Arc<dyn FileSystem>>,
        emit: UpdateSink,
    ) -> Result<(), EditError> {
        let session = self.get(id)?;
        let _op = session.operation.lock().await;
        session.live()?;
        if let Some(fs) = reopened {
            let fs = session.filesystem(Some(fs))?;
            session.status.lock().fs = Some(fs);
        }
        session.set_state(EditState::Uploading, None, &emit);
        let result = async {
            let bytes = stable_copy(&session.document).await?;
            session.live()?;
            commit(&session, &bytes, None).await?;
            let current = read_local(session.document.clone()).await?;
            if current != bytes {
                return Err(EditError::LocalChanged);
            }
            self.close(&session, &emit).await
        }
        .await;
        if let Err(error) = result {
            session.failed(&error, &emit);
            return Err(error);
        }
        Ok(())
    }

    /// Removes an explicitly discarded copy. Never races an upload for this session.
    pub async fn discard(&self, id: EditId, emit: UpdateSink) -> Result<(), EditError> {
        let session = self.get(id)?;
        let _op = session.operation.lock().await;
        session.live()?;
        self.close(&session, &emit).await
    }

    pub fn reveal(&self, id: EditId) -> Result<(), EditError> {
        let session = self.get(id)?;
        match regular_copy(&session.document) {
            Ok(_) => crate::reveal::reveal(&session.document)?,
            Err(EditError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                crate::reveal::reveal(&session.dir)?
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    pub fn needs_exit_confirmation(&self) -> bool {
        let sessions = self.sessions.lock();
        !sessions.closing && !sessions.by_id.is_empty()
    }

    /// Stops automatic uploads and waits for any in-flight replacement. Leaves working copies
    /// intact and does not quit an editor that may also contain unrelated user documents.
    pub async fn shutdown(&self) {
        let _shutdown = self.shutdown.lock().await;
        let (sessions, watchers) = {
            let mut state = self.sessions.lock();
            state.closing = true;
            let sessions: Vec<_> = state.by_id.values().cloned().collect();
            for session in &sessions {
                session.stop.cancel();
            }
            (sessions, std::mem::take(&mut state.watchers))
        };
        for (_, watcher) in watchers {
            if let Err(error) = watcher.await {
                tracing::warn!(%error, "external edit watcher stopped unexpectedly");
            }
        }
        for session in sessions {
            let _idle = session.operation.lock().await;
        }
    }

    fn get(&self, id: EditId) -> Result<Arc<Session>, EditError> {
        let sessions = self.sessions.lock();
        if sessions.closing {
            return Err(EditError::Busy);
        }
        sessions.by_id.get(&id).cloned().ok_or(EditError::Missing)
    }

    fn ensure_watcher(&self, session: &Arc<Session>, emit: UpdateSink) {
        let mut state = self.sessions.lock();
        if state.closing || session.stop.is_cancelled() {
            return;
        }
        state
            .watchers
            .entry(session.id)
            .or_insert_with(|| tokio::spawn(watch(session.clone(), emit)));
    }

    async fn close(&self, session: &Arc<Session>, emit: &UpdateSink) -> Result<(), EditError> {
        if let Err(error) = tokio::fs::remove_dir_all(&session.dir).await
            && error.kind() != io::ErrorKind::NotFound
        {
            return Err(error.into());
        }
        session.stop.cancel();
        {
            let mut sessions = self.sessions.lock();
            sessions.by_id.remove(&session.id);
            // The watcher cannot hold the gate here. The stop token ends it after this operation.
            sessions.watchers.remove(&session.id);
        }
        session.set_state(EditState::Closed, None, emit);
        Ok(())
    }

    fn restore(&self) {
        let Ok(meta) = std::fs::symlink_metadata(&self.root) else {
            return;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() || kind.is_symlink() {
                continue;
            }
            let Ok(bytes) = read_bounded(&entry.path().join("manifest.json"), MANIFEST_LIMIT)
            else {
                continue;
            };
            let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
                continue;
            };
            if entry.file_name() != manifest.id.0.to_string().as_str()
                || !valid_remote_path(&manifest.path)
                || !valid_version(&manifest.version)
            {
                continue;
            }
            let session = Arc::new(Session::new(&self.root, manifest, None));
            // Even an edited copy that is now too large or not text remains available for recovery.
            if regular_copy(&session.document).is_ok() {
                self.sessions.lock().by_id.insert(session.id, session);
            }
        }
    }
}

impl Drop for ExternalEditManager {
    fn drop(&mut self) {
        for session in self.sessions.get_mut().by_id.values() {
            session.stop.cancel();
        }
    }
}

fn check_slot(sessions: &Sessions, label: &str, path: &str) -> Result<(), EditError> {
    if sessions.closing
        || sessions
            .by_id
            .values()
            .any(|copy| copy.label == label && copy.path == path)
    {
        return Err(EditError::Busy);
    }
    if sessions.by_id.len() >= SESSION_LIMIT {
        return Err(EditError::Limit);
    }
    Ok(())
}

/// Waits for the same content or error to appear across the entire stability window. A missing
/// file between the two renames of an editor's safe-save sequence does not pause uploads.
#[derive(Default)]
struct Stability<T>(Option<(T, Instant)>);

impl<T: PartialEq> Stability<T> {
    fn ready(&mut self, sample: T, now: Instant) -> bool {
        if let Some((previous, first)) = &self.0
            && previous == &sample
        {
            now.duration_since(*first) >= STABLE_FOR
        } else {
            self.0 = Some((sample, now));
            false
        }
    }
}

async fn watch(session: Arc<Session>, emit: UpdateSink) {
    let mut interval = tokio::time::interval(POLL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut stable = Stability::<Result<String, &'static str>>(None);
    loop {
        tokio::select! { biased; _ = session.stop.cancelled() => return, _ = interval.tick() => {} }
        let Ok(_op) = session.operation.try_lock() else {
            stable.0 = None;
            continue;
        };
        if !matches!(
            session.status.lock().state,
            EditState::Watching | EditState::Saved
        ) {
            stable.0 = None;
            continue;
        }
        let sample = read_local(session.document.clone()).await;
        if session.stop.is_cancelled() {
            return;
        }
        let version = sample
            .as_ref()
            .map(|bytes| text_version(bytes))
            .map_err(|error| error.code());
        if version
            .as_ref()
            .is_ok_and(|version| *version == session.status.lock().version)
        {
            stable.0 = None;
            continue;
        }
        if !stable.ready(version, Instant::now()) {
            continue;
        }
        stable.0 = None;
        let bytes = match sample {
            Ok(bytes) => bytes,
            Err(error) => {
                session.failed(&error, &emit);
                continue;
            }
        };
        session.set_state(EditState::Uploading, None, &emit);
        match commit(&session, &bytes, None).await {
            Ok(()) => session.set_state(EditState::Saved, None, &emit),
            Err(error) => session.failed(&error, &emit),
        }
    }
}

async fn stable_copy(path: &Path) -> Result<Vec<u8>, EditError> {
    let first = read_local(path.to_owned()).await?;
    tokio::time::sleep(STABLE_FOR).await;
    let next = read_local(path.to_owned()).await?;
    if first == next {
        Ok(next)
    } else {
        Err(EditError::LocalChanged)
    }
}

/// Must be called with the operation gate held. A later local save is left for the next poll,
/// rather than misreported as failure of a snapshot that already reached the server.
async fn commit(session: &Session, bytes: &[u8], approved: Option<&str>) -> Result<(), EditError> {
    let text = validate_text(bytes)?;
    let local_version = text_version(bytes);
    let baseline = session.status.lock().version.clone();
    let version = if local_version == baseline && approved.is_none() {
        baseline
    } else {
        let fs = session.filesystem(None)?;
        let expected = approved.unwrap_or(&baseline);
        match save_text_file(fs.as_ref(), &session.path, expected, text).await {
            Ok(version) => version,
            Err(TextEditError::Conflict) if approved.is_none() => {
                // A lost response to the preceding upload can leave the bytes on the server.
                if read_text_file(fs.as_ref(), &session.path)
                    .await
                    .is_ok_and(|file| file.version == local_version)
                {
                    local_version
                } else {
                    return Err(TextEditError::Conflict.into());
                }
            }
            Err(error) => return Err(error.into()),
        }
    };
    session.status.lock().version = version.clone();
    let manifest = session.manifest(version);
    let path = session.dir.join("manifest.json");
    blocking(move || write_manifest(&path, &manifest)).await
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, EditError> + Send + 'static,
) -> Result<T, EditError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| io::Error::other(error))?
}

async fn read_local(path: PathBuf) -> Result<Vec<u8>, EditError> {
    blocking(move || {
        let bytes = read_bounded(&path, TEXT_FILE_LIMIT)?;
        validate_text(&bytes)?;
        Ok(bytes)
    })
    .await
}

fn regular_copy(path: &Path) -> Result<std::fs::Metadata, EditError> {
    let parent = path.parent().ok_or(EditError::InvalidPath)?;
    let dir = std::fs::symlink_metadata(parent)?;
    let file = std::fs::symlink_metadata(path)?;
    if !dir.is_dir()
        || dir.file_type().is_symlink()
        || !file.is_file()
        || file.file_type().is_symlink()
    {
        return Err(EditError::LocalChanged);
    }
    Ok(file)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, EditError> {
    let before = regular_copy(path)?;
    if before.len() > limit as u64 {
        return Err(TextEditError::TooLarge.into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(EditError::LocalChanged);
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(TextEditError::TooLarge.into());
    }
    let after = regular_copy(path)?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != after.len()
    {
        return Err(EditError::LocalChanged);
    }
    Ok(bytes)
}

fn valid_remote_path(path: &str) -> bool {
    path.len() <= PATH_LIMIT
        && path.starts_with('/')
        && !path.ends_with('/')
        && !path.contains('\0')
}

fn valid_version(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Keeps ordinary basenames (such as Dockerfile and .env) without using them as local paths.
fn working_name(remote: &str) -> &str {
    let name = remote.rsplit('/').next().unwrap_or_default();
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if name.is_empty()
        || name.len() > 200
        || name.ends_with(['.', ' '])
        || reserved
        || name.contains(['\\', ':', '"', '<', '>', '|', '?', '*'])
        || name.chars().any(char::is_control)
    {
        "document.txt"
    } else {
        name
    }
}

fn private_dir(path: &Path, existing: bool) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if existing && error.kind() == io::ErrorKind::AlreadyExists => {
            let meta = std::fs::symlink_metadata(path)?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                return Err(io::Error::other(
                    "external edit directory is not a real directory",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn private_file(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn initialize(root: &Path, session: &Session, bytes: &[u8]) -> Result<(), EditError> {
    private_dir(root, true)?;
    private_dir(&session.dir, false)?;
    let result = (|| {
        private_dir(&session.dir.join("copy"), false)?;
        let mut file = private_file(&session.document)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        write_manifest(
            &session.dir.join("manifest.json"),
            &session.manifest(session.status.lock().version.clone()),
        )
    })();
    if result.is_err()
        && let Err(error) = std::fs::remove_dir_all(&session.dir)
    {
        tracing::debug!(kind = ?error.kind(), "could not clean up an incomplete external edit");
    }
    result
}

fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), EditError> {
    let bytes = serde_json::to_vec(manifest).map_err(io::Error::other)?;
    if bytes.len() > MANIFEST_LIMIT {
        return Err(EditError::InvalidPath);
    }
    let temp = path.with_extension(format!("{}.part", Uuid::now_v7()));
    let mut file = private_file(&temp)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err()
        && let Err(error) = std::fs::remove_file(&temp)
    {
        tracing::debug!(kind = ?error.kind(), "could not clean up a staged external edit manifest");
    }
    Ok(result?)
}

fn editor_args(args: &[String], file: &Path) -> Vec<OsString> {
    let mut has_file = false;
    let mut resolved = Vec::with_capacity(args.len() + 1);
    for arg in args {
        let mut parts = arg.split("{file}");
        let mut value = OsString::from(parts.next().unwrap_or_default());
        for part in parts {
            has_file = true;
            value.push(file);
            value.push(part);
        }
        resolved.push(value);
    }
    if !has_file {
        resolved.push(file.into());
    }
    resolved
}

async fn launch(editor: &rove_vault::ExternalEditorPrefs, file: &Path) -> Result<(), EditError> {
    let program = Path::new(&editor.program);
    let mut command = if cfg!(target_os = "macos")
        && program
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    {
        let mut command = tokio::process::Command::new("/usr/bin/open");
        command.arg("-a").arg(program).arg(file);
        command
    } else {
        let mut command = tokio::process::Command::new(program);
        command.args(editor_args(&editor.args, file));
        command
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| EditError::EditorLaunch(error.to_string()))?;
    // Collect errors from short-lived launchers; a GUI editor need not stay attached to us.
    match tokio::time::timeout(POLL, child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(EditError::EditorLaunch(status.to_string())),
        Ok(Err(error)) => Err(EditError::EditorLaunch(error.to_string())),
        Err(_) => {
            tokio::spawn(async move {
                if let Err(error) = child.wait().await {
                    tracing::debug!(kind = ?error.kind(), "could not reap external editor");
                }
            });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn sink() -> (UpdateSink, mpsc::UnboundedReceiver<ExternalEditUpdate>) {
        let (send, recv) = mpsc::unbounded_channel();
        (
            Arc::new(move |update| {
                let _ = send.send(update);
            }),
            recv,
        )
    }

    #[test]
    fn file_argument_and_basename_are_not_paths_or_shell_fragments() {
        let path = Path::new("/tmp/work copy #1.conf");
        let args = editor_args(
            &["--wait".into(), "--file={file}".into(), "two words".into()],
            path,
        );
        assert_eq!(
            args,
            ["--wait", "--file=/tmp/work copy #1.conf", "two words"]
        );
        assert_eq!(
            editor_args(&["--wait".into()], path),
            ["--wait", "/tmp/work copy #1.conf"]
        );
        assert_eq!(working_name("/etc/nginx.conf"), "nginx.conf");
        assert_eq!(working_name("/project/Dockerfile"), "Dockerfile");
        assert_eq!(working_name("/project/.env"), ".env");
        assert_eq!(working_name("/project/../CON.txt"), "document.txt");
        assert_eq!(working_name("/project/C:\\elsewhere.txt"), "document.txt");
    }

    #[test]
    fn only_a_stable_sample_becomes_ready() {
        let mut stable = Stability(None);
        let start = Instant::now();
        assert!(!stable.ready("first", start));
        assert!(!stable.ready("second", start + POLL));
        assert!(!stable.ready("second", start + STABLE_FOR));
        assert!(stable.ready("second", start + STABLE_FOR + POLL));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_editor_launch_keeps_a_reopenable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("example.conf");
        std::fs::write(&remote, b"original\n").unwrap();
        let fs: Arc<dyn FileSystem> = Arc::new(rove_fs::LocalFs);
        let manager = ExternalEditManager::new(dir.path());
        let (emit, _updates) = sink();
        let editor = rove_vault::ExternalEditorPrefs {
            program: dir.path().join("no-editor").to_str().unwrap().into(),
            args: Vec::new(),
        };
        let row = manager
            .start(emit.clone(), fs, remote.to_str().unwrap(), editor)
            .await
            .unwrap();
        assert_eq!(row.state, EditState::Failed);
        assert_eq!(row.error.unwrap().code, "fs.edit_editor_launch");
        let local = PathBuf::from(row.local_path);
        assert_eq!(std::fs::read(&local).unwrap(), b"original\n");
        let editor = rove_vault::ExternalEditorPrefs {
            program: "/usr/bin/true".into(),
            args: Vec::new(),
        };
        manager.reopen(row.id, editor, emit.clone()).await.unwrap();
        assert_eq!(manager.list()[0].state, EditState::Watching);
        // An explicit discard must not upload even if the copy now contains other bytes.
        std::fs::write(&local, b"discard me\n").unwrap();
        manager.discard(row.id, emit).await.unwrap();
        assert_eq!(std::fs::read(&remote).unwrap(), b"original\n");
        assert!(!local.exists());
        assert!(manager.list().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uploads_pauses_on_conflict_and_recovers_with_the_approved_version() {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote.conf");
        std::fs::write(&remote, b"before\r\n").unwrap();
        let config = devsshd::DevConfig {
            sftp_root: Some(dir.path().to_owned()),
            ..devsshd::DevConfig::demo()
        };
        let key = config.host_key.public_key();
        let saved_key = rove_vault::HostKey {
            algorithm: key.algorithm().as_str().into(),
            public_key: key.to_openssh().unwrap(),
            fingerprint: key.fingerprint(Default::default()).to_string(),
        };
        let server = devsshd::DevServer::start("127.0.0.1:0", config)
            .await
            .unwrap();
        let target = crate::connect::quick_target(&format!("demo@{}", server.addr())).unwrap();
        let vault = Arc::new(
            rove_vault::Vault::open_in_memory(Arc::new(rove_vault::MemoryKeyCache::default()))
                .unwrap(),
        );
        vault
            .trust_host_key(&target.hostname, target.port, &target.route, &saved_key)
            .unwrap();
        let prompts = Arc::new(rove_ssh::HostKeyPrompts::new(
            rove_kernel::EventBus::default(),
            Duration::from_secs(1),
        ));
        let ssh = rove_ssh::SshConnector::new(
            &rove_ssh::SshConfig::default(),
            Arc::new(rove_ssh::VaultVerifier::new(vault, prompts)),
        );
        let auth = rove_ssh::AuthSpec::Password(zeroize::Zeroizing::new("demo".into()));
        let connection = ssh.connect(&target, &auth).await.unwrap();
        let fs = connection.open_sftp().await.unwrap();
        let path = fs.join(&fs.home().await.unwrap(), "remote.conf");
        let fs: Arc<dyn FileSystem> = Arc::new(fs);
        let manager = ExternalEditManager::new(dir.path());
        let (emit, mut events) = sink();
        let editor = rove_vault::ExternalEditorPrefs {
            program: "/usr/bin/true".into(),
            args: Vec::new(),
        };
        let row = manager
            .start(emit.clone(), fs, &path, editor)
            .await
            .unwrap();
        let local = PathBuf::from(row.local_path);
        assert!(local.ends_with("remote.conf"));
        tokio::fs::write(&local, b"local save\r\n").await.unwrap();
        wait_for(&mut events, EditState::Saved).await;
        assert_eq!(std::fs::read(&remote).unwrap(), b"local save\r\n");

        connection.disconnect().await;
        tokio::fs::write(&local, b"saved while offline")
            .await
            .unwrap();
        wait_for(&mut events, EditState::Failed).await;
        assert_eq!(std::fs::read(&remote).unwrap(), b"local save\r\n");
        assert_eq!(std::fs::read(&local).unwrap(), b"saved while offline");
        let connection = ssh.connect(&target, &auth).await.unwrap();
        let fs: Arc<dyn FileSystem> = Arc::new(connection.open_sftp().await.unwrap());
        manager
            .retry(row.id, Some(fs.clone()), None, emit.clone())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&remote).unwrap(), b"saved while offline");

        tokio::fs::write(&remote, b"server edit").await.unwrap();
        tokio::fs::write(&local, b"pending edit").await.unwrap();
        wait_for(&mut events, EditState::Conflict).await;
        assert_eq!(std::fs::read(&remote).unwrap(), b"server edit");
        manager.shutdown().await;
        assert_eq!(std::fs::read(&local).unwrap(), b"pending edit");

        let manager = ExternalEditManager::new(dir.path());
        assert_eq!(manager.list()[0].state, EditState::Recovered);
        assert!(matches!(
            manager.retry(row.id, None, None, emit.clone()).await,
            Err(EditError::Reconnect)
        ));
        let stale = manager
            .remote_version(row.id, Some(fs.clone()))
            .await
            .unwrap();
        tokio::fs::write(&remote, b"new server edit").await.unwrap();
        assert!(matches!(
            manager
                .retry(row.id, Some(fs.clone()), Some(stale), emit.clone())
                .await,
            Err(EditError::Text(TextEditError::Conflict))
        ));
        assert_eq!(std::fs::read(&remote).unwrap(), b"new server edit");
        assert_eq!(std::fs::read(&local).unwrap(), b"pending edit");
        let approved = manager
            .remote_version(row.id, Some(fs.clone()))
            .await
            .unwrap();
        manager
            .retry(row.id, Some(fs), Some(approved), emit.clone())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&remote).unwrap(), b"pending edit");
        manager.finish(row.id, None, emit).await.unwrap();
        assert!(!local.exists());
        assert!(manager.list().is_empty());
    }

    #[cfg(unix)]
    async fn wait_for(
        events: &mut mpsc::UnboundedReceiver<ExternalEditUpdate>,
        expected: EditState,
    ) {
        tokio::time::timeout(Duration::from_secs(6), async {
            while let Some(event) = events.recv().await {
                if event.state == expected {
                    return;
                }
                assert_ne!(event.state, EditState::Failed, "{:?}", event.error);
            }
            panic!("external edit event channel closed");
        })
        .await
        .expect("external edit did not reach the expected state");
    }

    #[cfg(unix)]
    #[test]
    fn private_working_copy_does_not_follow_a_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("private");
        private_dir(&folder, false).unwrap();
        let file = folder.join("text.conf");
        private_file(&file).unwrap().write_all(b"text").unwrap();
        assert_eq!(
            std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let link = folder.join("link.conf");
        symlink(file, &link).unwrap();
        assert!(matches!(
            read_bounded(&link, 20),
            Err(EditError::LocalChanged)
        ));
    }
}
