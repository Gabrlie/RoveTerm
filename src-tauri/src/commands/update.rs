//! Application update checking and signed in-app installation.
//!
//! Desktop builds prefer `tauri-plugin-updater` (minisign-verified `latest.json` artifacts) and
//! fall back to the GitHub Releases API when signed metadata is unavailable. Stable builds read
//! `/releases/latest/download/latest.json`; the beta channel discovers the newest SemVer release
//! (pre-release or stable) from GitHub Releases and points the signed updater at that tag's
//! `latest.json`. Android reads `latest-android.json` for the selected channel and falls back to
//! the GitHub Releases API; iOS queries the GitHub Releases API directly. External download URLs
//! are strictly allowlisted to this repository's GitHub Releases namespace.

#[cfg(any(target_os = "android", test))]
use std::collections::BTreeMap;
use std::time::Duration;

use rove_http::reqwest::{StatusCode, Url};
use rove_vault::UpdateChannel;
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, ipc::Channel};

use crate::error::CommandError;
use crate::state::AppState;

const RELEASE_REPO: &str = "Gabrlie/RoveTerm";
const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/Gabrlie/RoveTerm/releases/latest";
const GITHUB_RELEASES_LIST_URL: &str =
    "https://api.github.com/repos/Gabrlie/RoveTerm/releases?per_page=15";
#[cfg(target_os = "android")]
const ANDROID_MANIFEST_URL: &str =
    "https://github.com/Gabrlie/RoveTerm/releases/latest/download/latest-android.json";
const RELEASE_PAGE_URL: &str = "https://github.com/Gabrlie/RoveTerm/releases";
const RELEASE_PATH_PREFIX: &str = "/Gabrlie/RoveTerm/releases";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RELEASE_RESPONSE_BYTES: usize = 1024 * 1024;

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
    /// Effective update channel evaluated for this check.
    pub channel: UpdateChannel,
    /// `true` when a strictly newer release was found.
    pub available: bool,
    /// Version string of the newer release (without a leading `v`), when `available` is `true`.
    pub version: Option<String>,
    /// `true` when the offered version is a SemVer pre-release (e.g. `-beta.1`).
    pub prerelease: bool,
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

fn resolve_channel(stored: Option<UpdateChannel>, current: &Version) -> UpdateChannel {
    stored.unwrap_or_else(|| {
        if current.pre.is_empty() {
            UpdateChannel::Stable
        } else {
            UpdateChannel::Beta
        }
    })
}

fn is_valid_release_tag(tag: &str) -> bool {
    let trimmed = tag.trim();
    !trimmed.is_empty()
        && !trimmed.contains("..")
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
}

fn tag_release_url(version: &Version) -> String {
    format!("https://github.com/{RELEASE_REPO}/releases/tag/v{version}")
}

fn tag_download_url(tag: &str, filename: &str) -> Result<Url, CommandError> {
    let trimmed = tag.trim();
    if !is_valid_release_tag(trimmed) {
        return Err(CommandError::new(
            "update.invalid_response",
            format!("invalid release tag {trimmed:?}"),
        ));
    }
    let raw = format!("https://github.com/{RELEASE_REPO}/releases/download/{trimmed}/{filename}");
    if !is_allowed_release_url(&raw) {
        return Err(CommandError::new(
            "update.invalid_response",
            "release asset URL is outside the allowed repository",
        ));
    }
    Url::parse(&raw).map_err(|error| {
        CommandError::new(
            "update.invalid_response",
            format!("invalid release asset URL: {error}"),
        )
    })
}

fn up_to_date(current_version: String, channel: UpdateChannel) -> UpdateInfo {
    UpdateInfo {
        current_version,
        channel,
        available: false,
        version: None,
        prerelease: false,
        notes: String::new(),
        published_at: None,
        release_url: RELEASE_PAGE_URL.to_owned(),
        install_mode: UpdateInstallMode::External,
        asset_sha256: None,
        asset_size: None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseCandidate {
    tag_name: String,
    version: Version,
    prerelease: bool,
    notes: String,
    published_at: Option<String>,
    release_url: String,
}

impl ReleaseCandidate {
    fn into_external_info(self, current_version: String, channel: UpdateChannel) -> UpdateInfo {
        UpdateInfo {
            current_version,
            channel,
            available: true,
            version: Some(self.version.to_string()),
            prerelease: self.prerelease,
            notes: self.notes,
            published_at: self.published_at,
            release_url: self.release_url,
            install_mode: UpdateInstallMode::External,
            asset_sha256: None,
            asset_size: None,
        }
    }
}

fn candidate_from_release(
    release: &GithubRelease,
    channel: UpdateChannel,
) -> Result<Option<ReleaseCandidate>, CommandError> {
    if release.draft {
        return Ok(None);
    }
    let version = parse_semver(&release.tag_name)?;
    let prerelease = release.prerelease || !version.pre.is_empty();
    if channel == UpdateChannel::Stable && prerelease {
        return Ok(None);
    }
    let release_url = if release.html_url.trim().is_empty() {
        tag_release_url(&version)
    } else if is_allowed_release_url(&release.html_url) {
        release.html_url.trim().to_owned()
    } else {
        return Err(CommandError::new(
            "update.invalid_response",
            "release URL is outside the allowed repository",
        ));
    };
    Ok(Some(ReleaseCandidate {
        tag_name: release.tag_name.trim().to_owned(),
        version,
        prerelease,
        notes: release.body.clone().unwrap_or_default(),
        published_at: release.published_at.clone(),
        release_url,
    }))
}

/// Compares a GitHub release payload against `current_raw` and validates its URL.
fn compare_release(
    current_raw: &str,
    release: &GithubRelease,
    channel: UpdateChannel,
) -> Result<UpdateInfo, CommandError> {
    let current = parse_semver(current_raw)?;
    let Some(candidate) = candidate_from_release(release, channel)? else {
        return Ok(up_to_date(current.to_string(), channel));
    };
    if candidate.version <= current {
        return Ok(up_to_date(current.to_string(), channel));
    }
    Ok(candidate.into_external_info(current.to_string(), channel))
}

/// Selects the strictly newest valid release matching `channel` from a GitHub releases list.
fn select_latest_release(
    current: &Version,
    releases: &[GithubRelease],
    channel: UpdateChannel,
) -> Result<Option<ReleaseCandidate>, CommandError> {
    let mut best: Option<ReleaseCandidate> = None;
    for release in releases {
        if release.draft {
            continue;
        }
        let Ok(version) = parse_semver(&release.tag_name) else {
            continue;
        };
        let prerelease = release.prerelease || !version.pre.is_empty();
        if channel == UpdateChannel::Stable && prerelease {
            continue;
        }
        if version <= *current {
            continue;
        }
        let release_url = if release.html_url.trim().is_empty() {
            tag_release_url(&version)
        } else if is_allowed_release_url(&release.html_url) {
            release.html_url.trim().to_owned()
        } else {
            return Err(CommandError::new(
                "update.invalid_response",
                "release URL is outside the allowed repository",
            ));
        };
        let candidate = ReleaseCandidate {
            tag_name: release.tag_name.trim().to_owned(),
            version,
            prerelease,
            notes: release.body.clone().unwrap_or_default(),
            published_at: release.published_at.clone(),
            release_url,
        };
        if best
            .as_ref()
            .is_none_or(|existing| candidate.version > existing.version)
        {
            best = Some(candidate);
        }
    }
    Ok(best)
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
    channel: UpdateChannel,
) -> Result<UpdateInfo, CommandError> {
    let current = parse_semver(current_raw)?;
    let manifest: AndroidManifest = serde_json::from_str(payload).map_err(|error| {
        CommandError::new(
            "update.invalid_response",
            format!("invalid Android update manifest: {error}"),
        )
    })?;
    let latest = parse_semver(&manifest.version)?;
    let prerelease = !latest.pre.is_empty();
    if channel == UpdateChannel::Stable && prerelease {
        return Ok(up_to_date(current.to_string(), channel));
    }
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
        return Ok(up_to_date(current.to_string(), channel));
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
        channel,
        available: true,
        version: Some(latest.to_string()),
        prerelease,
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

async fn check_github_stable_release(current_version: &str) -> Result<UpdateInfo, CommandError> {
    let Some(body) =
        fetch_limited_text(GITHUB_LATEST_RELEASE_URL, "application/vnd.github+json").await?
    else {
        return Ok(up_to_date(
            current_version.to_owned(),
            UpdateChannel::Stable,
        ));
    };
    let release: GithubRelease = serde_json::from_str(&body).map_err(|error| {
        CommandError::new(
            "update.invalid_response",
            format!("could not decode GitHub release: {error}"),
        )
    })?;
    compare_release(current_version, &release, UpdateChannel::Stable)
}

async fn fetch_beta_candidate(current: &Version) -> Result<Option<ReleaseCandidate>, CommandError> {
    let Some(body) =
        fetch_limited_text(GITHUB_RELEASES_LIST_URL, "application/vnd.github+json").await?
    else {
        return Ok(None);
    };
    let releases: Vec<GithubRelease> = serde_json::from_str(&body).map_err(|error| {
        CommandError::new(
            "update.invalid_response",
            format!("could not decode GitHub releases list: {error}"),
        )
    })?;
    select_latest_release(current, &releases, UpdateChannel::Beta)
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

#[cfg(desktop)]
async fn check_desktop_updater(
    updater: tauri_plugin_updater::Updater,
    state: &AppState,
    current: &Version,
    current_version: &str,
    channel: UpdateChannel,
) -> Result<Option<UpdateInfo>, CommandError> {
    match updater.check().await {
        Ok(Some(update)) => {
            let version_parsed = parse_semver(&update.version)?;
            let prerelease = !version_parsed.pre.is_empty();
            if channel == UpdateChannel::Stable && prerelease {
                *state.updates.pending.lock().await = None;
                tracing::debug!(
                    current = %current_version,
                    offered = %version_parsed,
                    "ignoring prerelease on stable channel"
                );
                return Ok(Some(up_to_date(current_version.to_owned(), channel)));
            }
            if version_parsed <= *current {
                *state.updates.pending.lock().await = None;
                return Ok(Some(up_to_date(current_version.to_owned(), channel)));
            }
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
                ?channel,
                "signed update available"
            );
            Ok(Some(UpdateInfo {
                current_version: current_version.to_owned(),
                channel,
                available: true,
                version: Some(normalized),
                prerelease,
                notes,
                published_at,
                release_url,
                install_mode: UpdateInstallMode::InApp,
                asset_sha256: None,
                asset_size: None,
            }))
        }
        Ok(None) => {
            *state.updates.pending.lock().await = None;
            tracing::debug!(current = %current_version, ?channel, "signed updater reports up to date");
            Ok(Some(up_to_date(current_version.to_owned(), channel)))
        }
        Err(error) => {
            *state.updates.pending.lock().await = None;
            tracing::info!(
                %error,
                ?channel,
                "signed updater check unavailable; falling back to GitHub Releases"
            );
            Ok(None)
        }
    }
}

/// Checks for a newer RoveTerm release on the effective channel (`stable` or `beta`).
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
    let current_semver = parse_semver(&current_version)?;
    let stored_channel = state
        .vault
        .device_preferences()
        .ok()
        .and_then(|prefs| prefs.updates.channel);
    let channel = resolve_channel(stored_channel, &current_semver);

    match channel {
        UpdateChannel::Stable => {
            #[cfg(desktop)]
            {
                use tauri_plugin_updater::UpdaterExt;

                match app.updater() {
                    Ok(updater) => {
                        if let Some(info) = check_desktop_updater(
                            updater,
                            &state,
                            &current_semver,
                            &current_version,
                            channel,
                        )
                        .await?
                        {
                            return Ok(info);
                        }
                    }
                    Err(error) => {
                        tracing::info!(
                            %error,
                            "signed updater unavailable; falling back to GitHub Releases"
                        );
                    }
                }
                return check_github_stable_release(&current_version).await;
            }

            #[cfg(target_os = "android")]
            {
                match fetch_limited_text(ANDROID_MANIFEST_URL, "application/json").await {
                    Ok(Some(body)) => match parse_android_manifest(
                        &current_version,
                        &body,
                        current_android_abi(),
                        channel,
                    ) {
                        Ok(info) => return Ok(info),
                        Err(error) => {
                            tracing::info!(
                                error = %error.message,
                                "Android manifest invalid; falling back to GitHub Releases"
                            );
                        }
                    },
                    Ok(None) => {}
                    Err(error) => {
                        tracing::info!(
                            error = %error.message,
                            "Android manifest fetch failed; falling back to GitHub Releases"
                        );
                    }
                }
                return check_github_stable_release(&current_version).await;
            }

            #[cfg(target_os = "ios")]
            {
                check_github_stable_release(&current_version).await
            }
        }
        UpdateChannel::Beta => {
            let Some(candidate) = fetch_beta_candidate(&current_semver).await? else {
                #[cfg(desktop)]
                {
                    *state.updates.pending.lock().await = None;
                }
                return Ok(up_to_date(current_version, channel));
            };

            #[cfg(desktop)]
            {
                use tauri_plugin_updater::UpdaterExt;

                match tag_download_url(&candidate.tag_name, "latest.json")
                    .and_then(|manifest_url| {
                        url::Url::parse(manifest_url.as_str()).map_err(|error| {
                            CommandError::new("update.invalid_response", error.to_string())
                        })
                    }) {
                    Ok(manifest_url) => {
                        match app
                            .updater_builder()
                            .endpoints(vec![manifest_url])
                            .and_then(|builder| builder.build())
                        {
                            Ok(updater) => {
                                if let Some(info) = check_desktop_updater(
                                    updater,
                                    &state,
                                    &current_semver,
                                    &current_version,
                                    channel,
                                )
                                .await?
                                    && info.available
                                {
                                    return Ok(info);
                                }
                            }
                            Err(error) => {
                                tracing::info!(
                                    %error,
                                    tag = %candidate.tag_name,
                                    "beta signed updater unavailable; falling back to GitHub Release"
                                );
                            }
                        }
                    }
                    Err(error) => {
                        tracing::info!(
                            error = %error.message,
                            tag = %candidate.tag_name,
                            "beta manifest URL invalid; falling back to GitHub Release"
                        );
                    }
                }
                *state.updates.pending.lock().await = None;
                Ok(candidate.into_external_info(current_version, channel))
            }

            #[cfg(target_os = "android")]
            {
                if let Ok(manifest_url) =
                    tag_download_url(&candidate.tag_name, "latest-android.json")
                {
                    match fetch_limited_text(manifest_url.as_str(), "application/json").await {
                        Ok(Some(body)) => match parse_android_manifest(
                            &current_version,
                            &body,
                            current_android_abi(),
                            channel,
                        ) {
                            Ok(info) if info.available => return Ok(info),
                            Ok(_) => {}
                            Err(error) => {
                                tracing::info!(
                                    error = %error.message,
                                    tag = %candidate.tag_name,
                                    "beta Android manifest invalid; falling back to GitHub Release"
                                );
                            }
                        },
                        Ok(None) => {}
                        Err(error) => {
                            tracing::info!(
                                error = %error.message,
                                tag = %candidate.tag_name,
                                "beta Android manifest fetch failed; falling back to GitHub Release"
                            );
                        }
                    }
                }
                Ok(candidate.into_external_info(current_version, channel))
            }

            #[cfg(target_os = "ios")]
            {
                Ok(candidate.into_external_info(current_version, channel))
            }
        }
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
    fn resolves_update_channel_from_preference_or_running_version() {
        let stable = Version::parse("0.2.1").unwrap();
        let beta = Version::parse("0.3.0-beta.1").unwrap();
        assert_eq!(resolve_channel(None, &stable), UpdateChannel::Stable);
        assert_eq!(resolve_channel(None, &beta), UpdateChannel::Beta);
        assert_eq!(
            resolve_channel(Some(UpdateChannel::Beta), &stable),
            UpdateChannel::Beta
        );
        assert_eq!(
            resolve_channel(Some(UpdateChannel::Stable), &beta),
            UpdateChannel::Stable
        );
    }

    #[test]
    fn validates_tag_download_urls() {
        let url = tag_download_url("v0.3.0-beta.1", "latest.json").unwrap();
        assert_eq!(
            url.as_str(),
            "https://github.com/Gabrlie/RoveTerm/releases/download/v0.3.0-beta.1/latest.json"
        );
        for bad in ["", "../v0.3.0", "v0.3.0/evil", "v0.3.0?x=1", "v0.3.0 beta"] {
            assert!(tag_download_url(bad, "latest.json").is_err(), "{bad}");
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
            UpdateChannel::Stable,
        )
        .unwrap();
        assert!(newer.available);
        assert_eq!(newer.channel, UpdateChannel::Stable);
        assert!(!newer.prerelease);
        assert_eq!(newer.version.as_deref(), Some("0.2.0"));
        assert_eq!(newer.install_mode, UpdateInstallMode::External);
        assert!(newer.notes.contains("Faster startup"));

        let same = compare_release(
            "0.2.0",
            &sample_release(
                "v0.2.0",
                "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0",
            ),
            UpdateChannel::Stable,
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
            UpdateChannel::Stable,
        )
        .unwrap();
        assert!(!older.available);

        let mut prerelease = sample_release(
            "v0.3.0-beta.1",
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.3.0-beta.1",
        );
        prerelease.prerelease = true;
        assert!(
            !compare_release("0.2.0", &prerelease, UpdateChannel::Stable)
                .unwrap()
                .available
        );
        let beta = compare_release("0.2.0", &prerelease, UpdateChannel::Beta).unwrap();
        assert!(beta.available);
        assert_eq!(beta.channel, UpdateChannel::Beta);
        assert!(beta.prerelease);
        assert_eq!(beta.version.as_deref(), Some("0.3.0-beta.1"));

        let bad_url = compare_release(
            "0.1.0",
            &sample_release("v0.2.0", "https://evil.example/download"),
            UpdateChannel::Stable,
        );
        assert!(bad_url.is_err());
    }

    #[test]
    fn selects_highest_release_for_stable_and_beta_channels() {
        let mut draft = sample_release(
            "v0.4.0-beta.1",
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.4.0-beta.1",
        );
        draft.draft = true;
        draft.prerelease = true;

        let mut beta1 = sample_release(
            "v0.3.0-beta.1",
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.3.0-beta.1",
        );
        beta1.prerelease = true;

        let mut beta2 = sample_release(
            "v0.3.0-beta.2",
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.3.0-beta.2",
        );
        beta2.prerelease = true;

        let stable = sample_release(
            "v0.2.1",
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.1",
        );
        let non_semver = sample_release("nightly", "https://github.com/Gabrlie/RoveTerm/releases");
        let list = vec![draft, beta1, beta2, stable, non_semver];

        let cur = Version::parse("0.2.1").unwrap();
        assert!(
            select_latest_release(&cur, &list, UpdateChannel::Stable)
                .unwrap()
                .is_none()
        );
        let picked = select_latest_release(&cur, &list, UpdateChannel::Beta)
            .unwrap()
            .unwrap();
        assert_eq!(picked.tag_name, "v0.3.0-beta.2");
        assert!(picked.prerelease);

        // Beta users graduate seamlessly when 0.3.0 stable arrives.
        let mut with_final = list;
        with_final.push(sample_release(
            "v0.3.0",
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.3.0",
        ));
        let cur_beta = Version::parse("0.3.0-beta.2").unwrap();
        let graduated = select_latest_release(&cur_beta, &with_final, UpdateChannel::Beta)
            .unwrap()
            .unwrap();
        assert_eq!(graduated.tag_name, "v0.3.0");
        assert!(!graduated.prerelease);
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

        let update =
            parse_android_manifest("0.1.0", &manifest, "arm64-v8a", UpdateChannel::Stable).unwrap();
        assert!(update.available);
        assert_eq!(update.channel, UpdateChannel::Stable);
        assert!(!update.prerelease);
        assert_eq!(update.version.as_deref(), Some("0.2.0"));
        assert_eq!(update.asset_sha256.as_deref(), Some(sha.as_str()));
        assert_eq!(update.asset_size, Some(18_500_000));
        assert!(update.release_url.ends_with("arm64-v8a.apk"));

        // Falls back to release page when ABI is not present yet.
        let fallback =
            parse_android_manifest("0.1.0", &manifest, "x86_64", UpdateChannel::Stable).unwrap();
        assert!(fallback.available);
        assert_eq!(
            fallback.release_url,
            "https://github.com/Gabrlie/RoveTerm/releases/tag/v0.2.0"
        );
        assert_eq!(fallback.asset_sha256, None);

        // Pre-release Android manifests are ignored on Stable and accepted on Beta.
        let beta_manifest = manifest.replace("0.2.0", "0.3.0-beta.1");
        assert!(
            !parse_android_manifest("0.2.1", &beta_manifest, "arm64-v8a", UpdateChannel::Stable)
                .unwrap()
                .available
        );
        let beta_update =
            parse_android_manifest("0.2.1", &beta_manifest, "arm64-v8a", UpdateChannel::Beta)
                .unwrap();
        assert!(beta_update.available);
        assert!(beta_update.prerelease);
        assert_eq!(beta_update.version.as_deref(), Some("0.3.0-beta.1"));

        // Rejects tampered host or malformed SHA-256.
        let bad_sha = manifest.replace(&sha, "not-hex");
        assert!(
            parse_android_manifest("0.1.0", &bad_sha, "arm64-v8a", UpdateChannel::Stable).is_err()
        );
        let bad_host = manifest.replace("https://github.com/", "https://evil.test/");
        assert!(
            parse_android_manifest("0.1.0", &bad_host, "arm64-v8a", UpdateChannel::Stable).is_err()
        );
    }
}
