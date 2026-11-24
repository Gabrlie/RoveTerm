//! Temporary local copies of files selected through native document pickers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use rove_fs::{TransferId, TransferUpdate};
use rove_kernel::{Disposer, EventBus};
use serde::Serialize;
use tauri_plugin_rove_mobile::{MobileBridgeExt, MobileError, StagedFile};
use uuid::Uuid;

use crate::error::CommandError;

pub fn is_stage_uri(value: &str) -> bool {
    !value.contains('\0')
        && value.split_once("://").is_some_and(|(scheme, remainder)| {
            !remainder.is_empty()
                && (scheme.eq_ignore_ascii_case("content") || scheme.eq_ignore_ascii_case("file"))
        })
}

/// Owns an isolated staging directory until the caller finishes reading its file.
pub struct StagedTempFile {
    path: PathBuf,
    name: String,
    dir: PathBuf,
}

impl StagedTempFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn release_directory(&mut self) -> PathBuf {
        std::mem::take(&mut self.dir)
    }

    /// A share target may open the file after the native chooser has been presented.
    #[cfg(any(target_os = "android", test))]
    pub fn keep_until_sweep(mut self) {
        self.release_directory();
    }
}

impl Drop for StagedTempFile {
    fn drop(&mut self) {
        if !self.dir.as_os_str().is_empty() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StagedUpload {
    pub path: String,
    pub name: String,
}

#[derive(Default)]
struct Tracking {
    pending: HashMap<PathBuf, PathBuf>,
    active: HashMap<TransferId, PathBuf>,
    generation: u64,
}

impl Tracking {
    fn register(&mut self, mut staged: StagedTempFile) {
        self.pending
            .insert(std::mem::take(&mut staged.path), staged.release_directory());
    }
}

/// Owns pending and active upload copies. Blocking tasks share the same tracking state.
#[derive(Clone)]
pub struct MobileFiles {
    root: PathBuf,
    tracking: Arc<Mutex<Tracking>>,
}

impl MobileFiles {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            tracking: Arc::default(),
        }
    }

    pub fn sweep_all(&self) {
        let mut tracking = self.tracking.lock();
        tracking.pending.clear();
        tracking.active.clear();
        tracking.generation += 1;
        if let Err(error) = std::fs::remove_dir_all(&self.root)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, "removing mobile staging directory failed");
        }
    }

    pub fn stage_temp(
        &self,
        app: &tauri::AppHandle,
        uri: &str,
        max_bytes: Option<u64>,
    ) -> Result<StagedTempFile, CommandError> {
        self.stage_temp_with(|dir| app.rove_mobile().stage_file(uri, dir, max_bytes))
    }

    fn stage_temp_with(
        &self,
        stage: impl FnOnce(&str) -> Result<StagedFile, MobileError>,
    ) -> Result<StagedTempFile, CommandError> {
        let (mut staged, generation) = self.allocate_stage()?;
        let dir_str = path_str(&staged.dir)?;
        let file = stage(dir_str).map_err(|error| mobile_error(error, "fs.io"))?;
        check_generation(&self.tracking.lock(), generation)?;
        rove_fs::validate_name(&file.name)?;
        let path = staged.dir.join(&file.name);
        // Native APIs can return the canonical spelling of the app cache directory.
        if path != Path::new(&file.path)
            && staged
                .dir
                .canonicalize()
                .map_err(io_error)?
                .join(&file.name)
                != Path::new(&file.path)
        {
            return Err(CommandError::new("fs.io", "unexpected staged file path"));
        }
        staged.path = path;
        staged.name = file.name;
        Ok(staged)
    }

    fn allocate_stage(&self) -> Result<(StagedTempFile, u64), CommandError> {
        let dir = self.root.join(Uuid::now_v7().to_string());
        let tracking = self.tracking.lock();
        std::fs::create_dir_all(&self.root).map_err(io_error)?;
        std::fs::create_dir(&dir).map_err(io_error)?;
        let staged = StagedTempFile {
            path: PathBuf::new(),
            name: String::new(),
            dir,
        };
        Ok((staged, tracking.generation))
    }

    /// Places an Android app-data file under a root supported by the native file provider.
    #[cfg(any(target_os = "android", test))]
    pub fn stage_share_source(
        &self,
        path: &Path,
        source_dir: &Path,
    ) -> Result<StagedTempFile, CommandError> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| CommandError::new("input.invalid", "invalid share file name"))?;
        let source = path.canonicalize().map_err(io_error)?;
        if !source.starts_with(source_dir.canonicalize().map_err(io_error)?) || !source.is_file() {
            return Err(CommandError::new(
                "input.invalid",
                "share source must be a file inside the source directory",
            ));
        }
        let (mut staged, generation) = self.allocate_stage()?;
        staged.path = staged.dir.join(name);
        staged.name = name.to_owned();
        if std::fs::hard_link(&source, staged.path()).is_err() {
            std::fs::copy(&source, staged.path()).map_err(io_error)?;
        }
        check_generation(&self.tracking.lock(), generation)?;
        Ok(staged)
    }

    pub async fn stage_temp_async(
        &self,
        app: &tauri::AppHandle,
        uri: &str,
        max_bytes: Option<u64>,
    ) -> Result<StagedTempFile, CommandError> {
        let files = self.clone();
        let app = app.clone();
        let uri = uri.to_owned();
        tauri::async_runtime::spawn_blocking(move || files.stage_temp(&app, &uri, max_bytes))
            .await
            .map_err(|error| CommandError::new("app.task", error.to_string()))?
    }

    pub async fn prepare_upload(
        &self,
        app: &tauri::AppHandle,
        paths: &[String],
    ) -> Result<Vec<StagedUpload>, CommandError> {
        let files = self.clone();
        let app = app.clone();
        let paths = paths.to_vec();
        tauri::async_runtime::spawn_blocking(move || {
            files.prepare_upload_with(&paths, |uri| files.stage_temp(&app, uri, None))
        })
        .await
        .map_err(|error| CommandError::new("app.task", error.to_string()))?
    }

    fn prepare_upload_with(
        &self,
        paths: &[String],
        mut stage: impl FnMut(&str) -> Result<StagedTempFile, CommandError>,
    ) -> Result<Vec<StagedUpload>, CommandError> {
        let generation = self.tracking.lock().generation;
        let mut staged_files = Vec::new();
        let mut uploads = Vec::with_capacity(paths.len());
        for raw in paths {
            if is_stage_uri(raw) {
                let staged = stage(raw)?;
                uploads.push(StagedUpload {
                    path: path_str(staged.path())?.to_owned(),
                    name: staged.name().to_owned(),
                });
                staged_files.push(staged);
            } else {
                let name = absolute_local_path(raw)?
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| CommandError::new("input.invalid", "path has no file name"))?;
                uploads.push(StagedUpload {
                    path: raw.clone(),
                    name: name.to_owned(),
                });
            }
        }
        // Publish the batch only after every source has been prepared successfully.
        let mut tracking = self.tracking.lock();
        check_generation(&tracking, generation)?;
        for staged in staged_files {
            tracking.register(staged);
        }
        Ok(uploads)
    }

    pub fn discard_staged(&self, paths: &[String]) {
        let mut tracking = self.tracking.lock();
        for path in paths {
            if let Some(dir) = tracking.pending.remove(Path::new(path)) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    pub fn bind_transfer(&self, local_path: &str, id: TransferId) {
        let mut tracking = self.tracking.lock();
        if let Some(dir) = tracking.pending.remove(Path::new(local_path)) {
            tracking.active.insert(id, dir);
        }
    }

    pub fn finish_transfer(&self, id: TransferId) {
        let dir = self.tracking.lock().active.remove(&id);
        if let Some(dir) = dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

pub fn watch_transfers(events: &EventBus, mobile_files: Arc<MobileFiles>) -> Disposer {
    events.on::<TransferUpdate>(move |update| {
        if update.state.is_final() {
            mobile_files.finish_transfer(update.id);
        }
    })
}

pub(crate) fn absolute_local_path(raw: &str) -> Result<&Path, CommandError> {
    let path = Path::new(raw);
    if path.is_absolute() && !raw.contains('\0') {
        Ok(path)
    } else {
        Err(CommandError::new(
            "input.invalid",
            "file path must be absolute and contain no NUL bytes",
        ))
    }
}

pub(crate) fn mobile_error(error: MobileError, fallback_code: &'static str) -> CommandError {
    let code = match error {
        MobileError::InvalidInput(_) => "input.invalid",
        MobileError::Unsupported => "app.unsupported",
        _ => fallback_code,
    };
    CommandError::new(code, error.to_string())
}

fn check_generation(tracking: &Tracking, generation: u64) -> Result<(), CommandError> {
    if tracking.generation != generation {
        return Err(CommandError::new("fs.cancelled", "staging was interrupted"));
    }
    Ok(())
}

fn path_str(path: &Path) -> Result<&str, CommandError> {
    path.to_str()
        .ok_or_else(|| CommandError::new("input.invalid", "file path must be UTF-8"))
}

fn io_error(error: std::io::Error) -> CommandError {
    CommandError::new("fs.io", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_fs::TransferState;

    fn copy_fixture(files: &MobileFiles, name: &str) -> StagedTempFile {
        files
            .stage_temp_with(|dir| {
                let path = Path::new(dir).join(name);
                std::fs::write(&path, b"fixture").unwrap();
                Ok(StagedFile {
                    path: path.to_str().unwrap().to_owned(),
                    name: name.to_owned(),
                    size: 7,
                })
            })
            .unwrap()
    }

    fn stage_for_upload(files: &MobileFiles) -> (String, PathBuf) {
        let staged = copy_fixture(files, "upload.txt");
        let path = path_str(staged.path()).unwrap().to_owned();
        let dir = staged.dir.clone();
        files.tracking.lock().register(staged);
        (path, dir)
    }

    fn update(id: TransferId, state: TransferState) -> TransferUpdate {
        TransferUpdate {
            id,
            tag: "upload".into(),
            name: String::new(),
            src_label: String::new(),
            src_path: String::new(),
            dst_label: String::new(),
            dst_path: String::new(),
            state,
            bytes_done: 0,
            bytes_total: 0,
            files_done: 0,
            files_total: 0,
            skipped: false,
            error: None,
        }
    }

    #[test]
    fn detects_picker_uris() {
        for value in [
            "content://documents/42",
            "CONTENT://picker",
            "FiLe:///absolute",
        ] {
            assert!(is_stage_uri(value), "{value:?}");
        }
        for value in [
            "relative",
            "/absolute",
            "https://example.com/file",
            "content://",
            "file://",
            "file:///ab\0c",
            "content\0://picker",
        ] {
            assert!(!is_stage_uri(value), "{value:?}");
        }
    }

    #[test]
    fn temporary_copies_are_removed_on_drop_and_early_return() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let staged = copy_fixture(&files, "key");
        let dir = staged.dir.clone();
        assert!(staged.path().is_file());
        drop(staged);
        assert!(!dir.exists());

        let failed_read = || -> std::io::Result<()> {
            let staged = copy_fixture(&files, "config");
            std::fs::read(staged.path().with_file_name("missing"))?;
            Ok(())
        };
        assert!(failed_read().is_err());
        assert_eq!(std::fs::read_dir(&files.root).unwrap().count(), 0);
    }

    #[test]
    fn a_bridge_failure_removes_its_staging_directory() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let result = files.stage_temp_with(|dir| {
            std::fs::write(Path::new(dir).join("partial"), b"fixture").unwrap();
            Err(MobileError::InvalidInput("size limit".into()))
        });
        assert_eq!(result.err().unwrap().code, "input.invalid");
        assert_eq!(std::fs::read_dir(&files.root).unwrap().count(), 0);
    }

    #[test]
    fn share_source_survives_its_guard_until_sweep() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let source_dir = temp.path().join("logs");
        std::fs::create_dir(&source_dir).unwrap();
        let source = source_dir.join("roveterm.log");
        std::fs::write(&source, b"fixture").unwrap();
        let staged = files.stage_share_source(&source, &source_dir).unwrap();
        let path = staged.path().to_owned();
        staged.keep_until_sweep();
        assert_eq!(std::fs::read(&path).unwrap(), b"fixture");
        files.sweep_all();
        assert!(!path.exists());
        assert!(source.is_file());
    }

    #[test]
    fn discard_only_removes_matching_pending_copies() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let (staged, dir) = stage_for_upload(&files);
        let (other, other_dir) = stage_for_upload(&files);
        let unrelated = files.root.join("unrelated");
        std::fs::write(&unrelated, b"keep").unwrap();
        files.discard_staged(&[staged, unrelated.to_str().unwrap().to_owned()]);
        assert!(!dir.exists());
        assert!(other_dir.is_dir());
        assert!(unrelated.is_file());
        assert!(
            files
                .tracking
                .lock()
                .pending
                .contains_key(Path::new(&other))
        );
    }

    #[test]
    fn watcher_keeps_copies_for_live_transfers_and_removes_them_on_each_final_state() {
        let temp = tempfile::tempdir().unwrap();
        let files = Arc::new(MobileFiles::new(temp.path().join("rove-staged")));
        let events = EventBus::default();
        let _watcher = watch_transfers(&events, files.clone());
        for final_state in [
            TransferState::Done,
            TransferState::Failed,
            TransferState::Cancelled,
        ] {
            let (path, dir) = stage_for_upload(&files);
            let id = Uuid::now_v7().to_string().parse().unwrap();
            files.bind_transfer(&path, id);
            files.discard_staged(&[path]);
            for state in [TransferState::Queued, TransferState::Running] {
                events.emit(&update(id, state));
                assert!(dir.is_dir());
            }
            events.emit(&update(id, final_state));
            assert!(!dir.exists());
        }
        assert!(files.tracking.lock().active.is_empty());
    }

    #[test]
    fn sweep_clears_pending_active_and_crash_leftovers() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let (active, _) = stage_for_upload(&files);
        files.bind_transfer(&active, Uuid::now_v7().to_string().parse().unwrap());
        stage_for_upload(&files);
        std::fs::write(files.root.join("leftover"), b"fixture").unwrap();
        files.sweep_all();
        files.sweep_all();
        assert!(!files.root.exists());
        let tracking = files.tracking.lock();
        assert!(tracking.pending.is_empty());
        assert!(tracking.active.is_empty());
    }

    #[test]
    fn prepare_upload_passes_through_absolute_local_paths_with_their_basenames() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let path = temp.path().join("report draft.txt");
        std::fs::write(&path, b"fixture").unwrap();
        let local = path.to_str().unwrap().to_owned();
        let no_stage = |_: &str| panic!("local paths are passed through");
        let result = files
            .prepare_upload_with(std::slice::from_ref(&local), no_stage)
            .unwrap();
        assert_eq!(result[0].path, local);
        assert_eq!(result[0].name, "report draft.txt");
        assert!(!files.root.exists());

        let root = path
            .ancestors()
            .last()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        for invalid in ["relative/file".into(), format!("{local}\0"), root] {
            let error = files.prepare_upload_with(&[invalid], no_stage).unwrap_err();
            assert_eq!(error.code, "input.invalid");
        }
    }

    #[test]
    fn prepare_upload_rolls_back_only_the_failed_batch() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let (_, existing_dir) = stage_for_upload(&files);
        let error = files
            .prepare_upload_with(&["content://picker/1".into(), "relative".into()], |_| {
                Ok(copy_fixture(&files, "file.txt"))
            })
            .unwrap_err();
        assert_eq!(error.code, "input.invalid");
        assert!(existing_dir.is_dir());
        assert_eq!(std::fs::read_dir(&files.root).unwrap().count(), 1);
        assert_eq!(files.tracking.lock().pending.len(), 1);
    }

    #[test]
    fn prepare_upload_registers_each_successful_picker_copy() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let uploads = files
            .prepare_upload_with(
                &[
                    "content://picker/1".into(),
                    "file:///other/report.txt".into(),
                ],
                |_| Ok(copy_fixture(&files, "report.txt")),
            )
            .unwrap();
        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0].name, "report.txt");
        assert_eq!(uploads[1].name, "report.txt");
        assert_ne!(uploads[0].path, uploads[1].path);
        let paths: Vec<_> = uploads.into_iter().map(|upload| upload.path).collect();
        assert!(paths.iter().all(|path| Path::new(path).is_file()));
        files.discard_staged(&paths);
        assert!(paths.iter().all(|path| !Path::new(path).exists()));
    }

    #[test]
    fn sweep_invalidates_a_batch_that_started_earlier() {
        let temp = tempfile::tempdir().unwrap();
        let files = MobileFiles::new(temp.path().join("rove-staged"));
        let error = files
            .prepare_upload_with(&["content://picker/1".into()], |_| {
                files.sweep_all();
                Ok(copy_fixture(&files, "file.txt"))
            })
            .unwrap_err();
        assert_eq!(error.code, "fs.cancelled");
        assert_eq!(std::fs::read_dir(&files.root).unwrap().count(), 0);
        assert!(files.tracking.lock().pending.is_empty());
    }
}
