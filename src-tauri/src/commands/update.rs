//! Application update checking and signed in-app installation.
//!
//! Desktop builds prefer `tauri-plugin-updater` (minisign-verified `latest.json` artifacts) and
//! fall back to the GitHub Releases API when signed metadata is unavailable. Android reads the
//! dedicated `latest-android.json` manifest first and falls back to the GitHub Releases API; iOS
//! queries the GitHub Releases API directly. External download URLs are strictly allowlisted to
//! this repository's GitHub Releases namespace.

#[cfg(any(target_os = "android", test))]
use std::collections::BTreeMap;
use std::time::Duration;

use rove_http::reqwest::{StatusCode, Url};
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, ipc::Channel};

use crate::error::CommandError;
use crate::state::AppState;

const RELEASE_REPO: &str = "Gabrlie/RoveTerm";
const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/Gabrlie/RoveTerm/releases/latest";
#[cfg(target_os = "android")]
const ANDROID_MANIFEST_URL: &str =
    "https://github.com/Gabrlie/RoveTerm/releases/latest/download/latest-android.json";
const RELEASE_PAGE_URL: &str = "https://github.com/Gabrlie/RoveTerm/releases";
const RELEASE_PATH_PREFIX: &str = "/Gabrlie/RoveTerm/releases";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RELEASE_RESPONSE_BYTES: usize = 512 * 1024;

/// How an available update can be installed on the current platform and bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateInstallMode {
    /// Downloaded, signature-verified and applied inside the running application.
    InApp,
    /// Opened on the allowlisted GitHub Releases page for manual download.
    External,
}

/// Result returned to the UI after checking for updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInfo {
    /// Version of the running binary.
    pub current_version: String,
    /// `true` when a strictly newer release was found.
    pub available: bool,
    /// Version string of the newer release (without a leading `v`), when `available` is `true`.
    pub version: Option<String>,
    /// Plain Markdown release notes safe to render with the UI's Markdown viewer.
    pub notes: String,
    /// ISO-8601 / RFC-3339 publication timestamp when supplied by the release feed.
    pub published_at: Option<String>,
    /// Allowlisted GitHub Releases URL for viewing or downloading this release.
    pub release_url: String,
    /// Whether this build can apply the update in place or should open the release page.
    pub install_mode: UpdateInstallMode,
    /// Optional SHA-256 digest (lowercase hex) of the platform asset (used by Android manifests).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_sha256: Option<String>,
    /// Optional byte size of the platform asset (used by Android manifests).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_size: Option<u64>,
}

/// Progress events streamed over a Tauri IPC channel during `update_install`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum UpdateProgress {
    Started { total: Option<u64> },
    Downloading { downloaded: u64, total: Option<u64> },
    Installing,
    Finished,
}

#[cfg(desktop)]
pub struct PendingUpdate {
    pub version: String,
    pub update: tauri_plugin_updater::Update,
}

/// Shared update state held in [`AppState`].
#[derive(Default)]
pub struct UpdateState {
    /// Serializes concurrent update checks.
    pub check_lock: tokio::sync::Mutex<()>,
    /// The verified updater handle cached from the most recent desktop check.
    #[cfg(desktop)]
    pub pending: tokio::sync::Mutex<Option<PendingUpdate>>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
struct AndroidManifest {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    pub_date: Option<String>,
    #[serde(default)]
    release_url: Option<String>,
    #[serde(default)]
    platforms: BTreeMap<String, AndroidAbiAsset>,
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, Deserialize)]
struct AndroidAbiAsset {
    url: String,
    sha256: String,
    #[serde(default)]
    size: Option<u64>,
}

/// Parses a SemVer version string, accepting an optional leading `v` or `V`.
fn parse_semver(raw: &str) -> Result<Version, CommandError> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    Version::parse(stripped).map_err(|error| {
        CommandError::new(
            "update.invalid_response",
            format!("invalid semantic version {trimmed:?}: {error}"),
        )
    })
}

/// Returns `true` only for HTTPS URLs inside `https://github.com/Gabrlie/RoveTerm/releases`.
fn is_allowed_release_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw.trim()) else {
        return false;
    };
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    if !host.eq_ignore_ascii_case("github.com") {
        return false;
    }
    let path = url.path();
    path == RELEASE_PATH_PREFIX || path.starts_with(&format!("{RELEASE_PATH_PREFIX}/"))
}

fn tag_release_url(version: &Version) -> String {
    format!("https://github.com/{RELEASE_REPO}/releases/tag/v{version}")
}

fn up_to_date(current_version: String) -> UpdateInfo {
    UpdateInfo {
        current_version,
        available: false,
        version: None,
        notes: String::new(),
        published_at: None,
        release_url: RELEASE_PAGE_URL.to_owned(),
        install_mode: UpdateInstallMode::External,
        asset_sha256: None,
        asset_size: None,
    }
}

/// Compares a GitHub release payload against `current_raw` and validates its URL.
fn compare_release(current_raw: &str, release: &GithubRelease) -> Result<UpdateInfo, CommandError> {
    let current = parse_semver(current_raw)?;
    if release.draft || release.prerelease {
        return Ok(up_to_date(current.to_string()));
    }
    let latest = parse_semver(&release.tag_name)?;
    let release_url = if release.html_url.trim().is_empty() {
        tag_release_url(&latest)
    } else if is_allowed_release_url(&release.html_url) {
        release.html_url.trim().to_owned()
    } else {
        return Err(CommandError::new(
            "update.invalid_response",
            "release URL is outside the allowed repository",
        ));
    };
    if latest <= current {
        return Ok(up_to_date(current.to_string()));
    }
    Ok(UpdateInfo {
        current_version: current.to_string(),
        available: true,
        version: Some(latest.to_string()),
        notes: release.body.clone().unwrap_or_default(),
        published_at: release.published_at.clone(),
        release_url,
        install_mode: UpdateInstallMode::External,
        asset_sha256: None,
        asset_size: None,
    })
}

#[cfg(any(target_os = "android", test))]
fn is_valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Parses `latest-android.json`, validates its URLs and SHA-256 digest, and compares SemVer.
#[cfg(any(target_os = "android", test))]
fn parse_android_manifest(
    current_raw: &str,
    payload: &str,
    abi: &str,
) -> Result<UpdateInfo, CommandError> {
    let current = parse_semver(current_raw)?;
    let manifest: AndroidManifest = serde_json::from_str(payload).map_err(|error| {
        CommandError::new(
            "update.invalid_response",
            format!("invalid Android update manifest: {error}"),
        )
    })?;
    let latest = parse_semver(&manifest.version)?;
    let release_url = match manifest.release_url.as_deref().map(str::trim) {
        Some(url) if !url.is_empty() => {
            if !is_allowed_release_url(url) {
                return Err(CommandError::new(
                    "update.invalid_response",
                    "Android manifest release URL is outside the allowed repository",
                ));
            }
            url.to_owned()
        }
        _ => tag_release_url(&latest),
    };
    if latest <= current {
        return Ok(up_to_date(current.to_string()));
    }
    let asset = manifest
        .platforms
        .get(abi)
        .or_else(|| manifest.platforms.get("universal"));
    let (download_url, sha256, size) = match asset {
        Some(asset) => {
            if !is_allowed_release_url(&asset.url) {
                return Err(CommandError::new(
                    "update.invalid_response",
                    "Android APK URL is outside the allowed repository",
                ));
            }
            let digest = asset.sha256.trim().to_ascii_lowercase();
            if !is_valid_sha256(&digest) {
                return Err(CommandError::new(
                    "update.invalid_response",
                    "Android APK SHA-256 digest is not valid",
                ));
            }
            (asset.url.trim().to_owned(), Some(digest), asset.size)
        }
        None => (release_url, None, None),
    };
    Ok(UpdateInfo {
        current_version: current.to_string(),
        available: true,
        version: Some(latest.to_string()),
        notes: manifest.notes,
        published_at: manifest.pub_date,
        release_url: download_url,
        install_mode: UpdateInstallMode::External,
        asset_sha256: sha256,
        asset_size: size,
    })
}

async fn fetch_limited_text(url: &str, accept: &str) -> Result<Option<String>, CommandError> {
    let client = rove_http::client(Some(REQUEST_TIMEOUT))
        .map_err(|error| CommandError::new("update.check_failed", error.to_string()))?;
    let response = client
        .get(url)
        .header("Accept", accept)
        .send()
        .await
        .map_err(|error| {
            CommandError::new("update.check_failed", error.without_url().to_string())
        })?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(CommandError::new(
            "update.check_failed",
            format!("HTTP {}", response.status().as_u16()),
        ));
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_RELEASE_RESPONSE_BYTES as u64)
    {
        return Err(CommandError::new(
            "update.invalid_response",
            "update response exceeds size limit",
        ));
    }
    let bytes = response.bytes().await.map_err(|error| {
        CommandError::new("update.check_failed", error.without_url().to_string())
    })?;
    if bytes.len() > MAX_RELEASE_RESPONSE_BYTES {
        return Err(CommandError::new(
            "update.invalid_response",
            "update response exceeds size limit",
        ));
    }
    let text = String::from_utf8(bytes.to_vec()).map_err(|error| {
        CommandError::new("update.invalid_response", format!("invalid UTF-8: {error}"))
    })?;
    Ok(Some(text))
}

async fn check_github_release(current_version: &str) -> Result<UpdateInfo, CommandError> {
    let Some(body) =
        fetch_limited_text(GITHUB_LATEST_RELEASE_URL, "application/vnd.github+json").await?
    else {
        return Ok(up_to_date(current_version.to_owned()));
    };
    let release: GithubRelease = serde_json::from_str(&body).map_err(|error| {
        CommandError::new(
            "update.invalid_response",
            format!("could not decode GitHub release: {error}"),
        )
    })?;
    compare_release(current_version, &release)
}

#[cfg(target_os = "android")]
fn current_android_abi() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "arm64-v8a"
    }
    #[cfg(target_arch = "arm")]
    {
        "armeabi-v7a"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "x86_64")))]
    {
        "universal"
    }
}

/// Checks for a newer RoveTerm release.
///
/// Desktop builds try the signed `tauri-plugin-updater` feed first so eligible updates can install
/// in-app, falling back to the GitHub Releases API when signed metadata is not yet published.
#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateInfo, CommandError> {
    let _guard = state.updates.check_lock.lock().await;
    let current_version = app.package_info().version.to_string();

    #[cfg(desktop)]
    {
        use tauri_plugin_updater::UpdaterExt;

        let updater = app.updater();
        match updater {
            Ok(updater) => match updater.check().await {
                Ok(Some(update)) => {
                    let version_parsed = parse_semver(&update.version)?;
                    let normalized = version_parsed.to_string();
                    let notes = update.body.clone().unwrap_or_default();
                    let published_at = update.date.map(|date| date.to_string());
                    let release_url = tag_release_url(&version_parsed);
                    *state.updates.pending.lock().await = Some(PendingUpdate {
                        version: normalized.clone(),
                        update,
                    });
                    tracing::info!(
                        current = %current_version,
                        latest = %normalized,
                        "signed update available"
                    );
                    return Ok(UpdateInfo {
                        current_version,
                        available: true,
                        version: Some(normalized),
                        notes,
                        published_at,
                        release_url,
                        install_mode: UpdateInstallMode::InApp,
                        asset_sha256: None,
                        asset_size: None,
                    });
                }
                Ok(None) => {
                    *state.updates.pending.lock().await = None;
                    tracing::debug!(current = %current_version, "signed updater reports up to date");
                    return Ok(up_to_date(current_version));
                }
                Err(error) => {
                    *state.updates.pending.lock().await = None;
                    tracing::info!(
                        %error,
                        "signed updater check unavailable; falling back to GitHub Releases"
                    );
                }
            },
            Err(error) => {
                tracing::info!(
                    %error,
                    "signed updater unavailable; falling back to GitHub Releases"
                );
            }
        }
        return check_github_release(&current_version).await;
    }

    #[cfg(target_os = "android")]
    {
        match fetch_limited_text(ANDROID_MANIFEST_URL, "application/json").await {
            Ok(Some(body)) => {
                match parse_android_manifest(&current_version, &body, current_android_abi()) {
                    Ok(info) => return Ok(info),
                    Err(error) => {
                        tracing::info!(
                            error = %error.message,
                            "Android manifest invalid; falling back to GitHub Releases"
                        );
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::info!(
                    error = %error.message,
                    "Android manifest fetch failed; falling back to GitHub Releases"
                );
            }
        }
        return check_github_release(&current_version).await;
    }

    #[cfg(target_os = "ios")]
    {
        check_github_release(&current_version).await
    }
}

/// Downloads, verifies and installs the pending update matching `version`, streaming progress over
/// `on_progress`.
#[tauri::command]
pub async fn update_install(
    state: State<'_, AppState>,
    version: String,
    on_progress: Channel<UpdateProgress>,
) -> Result<(), CommandError> {
    #[cfg(desktop)]
    {
        let requested = parse_semver(&version)?.to_string();
        let pending = {
            let mut slot = state.updates.pending.lock().await;
            match slot.take() {
                Some(pending) if pending.version == requested => pending,
                other => {
                    *slot = other;
                    return Err(CommandError::new(
                        "update.not_ready",
                        "no verified update matches the requested version; check again first",
                    ));
                }
            }
        };

        let mut downloaded: u64 = 0;
        let mut started = false;
        let progress_channel = on_progress.clone();
        let finish_channel = on_progress.clone();
        let result = pending
            .update
            .download_and_install(
                move |chunk_len, content_length| {
                    if !started {
                        started = true;
                        let _ = progress_channel.send(UpdateProgress::Started {
                            total: content_length,
                        });
                    }
                    downloaded = downloaded.saturating_add(chunk_len as u64);
                    let _ = progress_channel.send(UpdateProgress::Downloading {
                        downloaded,
                        total: content_length,
                    });
                },
                move || {
                    let _ = finish_channel.send(UpdateProgress::Installing);
                },
            )
            .await;

        match result {
            Ok(()) => {
                let _ = on_progress.send(UpdateProgress::Finished);
                tracing::info!(version = %requested, "update installed; waiting for restart");
                Ok(())
            }
            Err(error) => {
                tracing::warn!(version = %requested, %error, "update installation failed");
                *state.updates.pending.lock().await = Some(pending);
                Err(CommandError::new(
                    "update.install_failed",
                    error.to_string(),
                ))
            }
        }
    }

    #[cfg(mobile)]
    {
        let _ = (state, version, on_progress);
        Err(CommandError::new(
            "update.unsupported",
            "in-app update installation is not supported on this platform yet",
        ))
    }
}

/// Cleanly shuts down active sessions and services, then restarts RoveTerm into the installed
/// update.
#[tauri::command]
pub async fn update_restart(app: AppHandle) -> Result<(), CommandError> {
    tracing::info!("restarting application to apply update");
    crate::shutdown_async(&app).await;
    app.restart();
}

/// Opens an allowlisted RoveTerm GitHub Releases URL in the system browser.
#[tauri::command]
pub async fn update_open_release(app: AppHandle, url: String) -> Result<(), CommandError> {
    if !is_allowed_release_url(&url) {
        return Err(CommandError::new(
            "input.invalid",
            "release URL must point to RoveTerm's GitHub Releases",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::reveal::open_url(&app, url.trim())
            .map_err(|error| CommandError::new("app.io", error))
    })
    .await
    .map_err(|error| CommandError::new("app.task", error.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_release(tag: &str, url: &str) -> GithubRelease {
        GithubRelease {
            tag_name: tag.into(),
            body: Some("## What's New\n- Faster startup".into()),
            html_url: url.into(),
            published_at: Some("2026-11-22T08:00:00Z".into()),
            draft: false,
            prerelease: false,
        }
    }

    #[test]
    fn parses_semver_with_optional_v_prefix_and_prerelease() {
        assert_eq!(parse_semver("0.2.0").unwrap(), Version::new(0, 2, 0));
        assert_eq!(parse_semver("v0.2.0").unwrap(), Version::new(0, 2, 0));
        assert_eq!(
            parse_semver("  V0.3.0-beta.1  ").unwrap(),
            Version::parse("0.3.0-beta.1").unwrap()
        );
        assert!(parse_semver("not-a-version").is_err());
        assert!(parse_semver("").is_err());
    }

    #[test]
    fn allowlists_only_repository_github_releases_urls() {
        assert!(is_allowed_release_url(
            "https://github.com/Gabrlie/RoveTerm/releases"
        ));
        assert!(is_allowed_release_url(
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0"
        ));
        assert!(is_allowed_release_url(
            "https://github.com/Gabrlie/RoveTerm/releases/download/v0.2.0/RoveTerm.apk"
        ));
        for rejected in [
            "http://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0",
            "https://evil.example/Gabrlie/RoveTerm/releases/tag/v0.2.0",
            "https://github.com/OtherUser/RoveTerm/releases/tag/v0.2.0",
            "https://github.com/Gabrlie/RoveTerm/releases-evil/v0.2.0",
            "https://user:pass@github.com/Gabrlie/RoveTerm/releases",
            "javascript:alert(1)",
        ] {
            assert!(!is_allowed_release_url(rejected), "{rejected}");
        }
    }

    #[test]
    fn compares_github_releases_for_newer_same_older_and_invalid_urls() {
        let newer = compare_release(
            "0.1.0",
            &sample_release(
                "v0.2.0",
                "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0",
            ),
        )
        .unwrap();
        assert!(newer.available);
        assert_eq!(newer.version.as_deref(), Some("0.2.0"));
        assert_eq!(newer.install_mode, UpdateInstallMode::External);
        assert!(newer.notes.contains("Faster startup"));

        let same = compare_release(
            "0.2.0",
            &sample_release(
                "v0.2.0",
                "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0",
            ),
        )
        .unwrap();
        assert!(!same.available);
        assert_eq!(same.version, None);

        let older = compare_release(
            "0.3.0",
            &sample_release(
                "v0.2.0",
                "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0",
            ),
        )
        .unwrap();
        assert!(!older.available);

        let mut prerelease = sample_release(
            "v0.3.0-beta.1",
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.3.0-beta.1",
        );
        prerelease.prerelease = true;
        assert!(!compare_release("0.2.0", &prerelease).unwrap().available);

        let bad_url = compare_release(
            "0.1.0",
            &sample_release("v0.2.0", "https://evil.example/download"),
        );
        assert!(bad_url.is_err());
    }

    #[test]
    fn parses_and_validates_android_manifest() {
        let sha = "a".repeat(64);
        let manifest = serde_json::json!({
            "version": "0.2.0",
            "notes": "Android release notes",
            "pub_date": "2026-11-22T09:00:00Z",
            "release_url": "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0",
            "platforms": {
                "arm64-v8a": {
                    "url": "https://github.com/Gabrlie/RoveTerm/releases/download/v0.2.0/RoveTerm_0.2.0_arm64-v8a.apk",
                    "sha256": sha,
                    "size": 18_500_000u64
                }
            }
        })
        .to_string();

        let update = parse_android_manifest("0.1.0", &manifest, "arm64-v8a").unwrap();
        assert!(update.available);
        assert_eq!(update.version.as_deref(), Some("0.2.0"));
        assert_eq!(update.asset_sha256.as_deref(), Some(sha.as_str()));
        assert_eq!(update.asset_size, Some(18_500_000));
        assert!(update.release_url.ends_with("arm64-v8a.apk"));

        // Falls back to release page when ABI is not present yet.
        let fallback = parse_android_manifest("0.1.0", &manifest, "x86_64").unwrap();
        assert!(fallback.available);
        assert_eq!(
            fallback.release_url,
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0"
        );
        assert_eq!(fallback.asset_sha256, None);

        // Rejects tampered host or malformed SHA-256.
        let bad_sha = manifest.replace(&sha, "not-hex");
        assert!(parse_android_manifest("0.1.0", &bad_sha, "arm64-v8a").is_err());
        let bad_host = manifest.replace("https://github.com/", "https://evil.test/");
        assert!(parse_android_manifest("0.1.0", &bad_host, "arm64-v8a").is_err());
    }
}
