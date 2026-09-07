//! Self-update: find a newer installer on GitHub Releases, download and
//! verify it, hand off to Inno Setup, restart.
//!
//! Spec: `specs/self-update.md`. The flow, in order:
//!
//! 1. **Check** (`update.check` from Settings ▸ Updates, or once at startup a
//!    few seconds after launch) fetches `releases/latest` off-thread and
//!    compares the tag with `CARGO_PKG_VERSION`.
//! 2. **Offer.** A newer release opens [`ConfirmDialog::UpdateAvailable`] at
//!    startup once per version (`update_prompted_version` is persisted), and
//!    always shows an install button in Settings. Copies that were not set up
//!    by the installer (a `target\debug` build, an unpacked zip) get "open
//!    release page" instead: there is no registered install to replace.
//! 3. **Install** (`update.install`) downloads the `*-setup.exe` asset into
//!    the profile's data dir, verifies size and SHA-256 against the feed's
//!    digest, persists the layout with every tab intact, launches the
//!    installer silently, shuts the session daemon down (Inno refuses in-use
//!    files, and the daemon binary is replaced too) and exits. The installer
//!    waits for this pid to leave, installs, and relaunches the app, which
//!    restores the layout and spawns fresh shells for every pane.
//!
//! Threads never hold the state lock across network or disk I/O; they lock
//! only to apply a result and then ask the window for a rebuild. Every step
//! lands in `<config_dir>/update-events.jsonl` (see [`telemetry`]).

pub mod feed;
pub mod install;
pub mod telemetry;
pub mod transport;
pub mod version;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use unshit::app::{EventSink, ExternalEvent};

use crate::state::{
    mutate_with, push_error_toast, AppState, ConfirmDialog, SharedState, ToggleKey,
};
use telemetry::UpdateEventRecord;

pub use feed::{InstallerAsset, ReleaseInfo};
pub use install::InstallScope;

pub const DEFAULT_FEED_URL: &str =
    "https://api.github.com/repos/alangmartini/unshit-agentic-terminal-manager/releases/latest";
pub const RELEASES_PAGE_URL: &str =
    "https://github.com/alangmartini/unshit-agentic-terminal-manager/releases";
/// Replace the GitHub feed (tests, screenshots). `file://` URLs are allowed
/// only when this is set.
pub const ENV_FEED_URL: &str = "TM_UPDATE_FEED_URL";
/// Delay before the startup check, in milliseconds (default 8000).
pub const ENV_STARTUP_DELAY_MS: &str = "TM_UPDATE_STARTUP_DELAY_MS";
const DEFAULT_STARTUP_DELAY: Duration = Duration::from_secs(8);
/// Progress is applied to state at most this often, whichever comes first.
const PROGRESS_STEP_BYTES: u64 = 256 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckSource {
    /// The delayed check a few seconds after launch. Failures stay silent.
    Startup,
    /// The button in Settings ▸ Updates.
    Manual,
    /// `update.install` with no release known yet: check, then install.
    Install,
}

impl CheckSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckSource::Startup => "startup",
            CheckSource::Manual => "manual",
            CheckSource::Install => "install",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading {
        received: u64,
        total: u64,
    },
    Verifying,
    /// Installer launched; the process exits as soon as the daemon is down.
    Installing,
    Failed,
}

impl UpdatePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpdatePhase::Idle => "idle",
            UpdatePhase::Checking => "checking",
            UpdatePhase::UpToDate => "up_to_date",
            UpdatePhase::Available => "available",
            UpdatePhase::Downloading { .. } => "downloading",
            UpdatePhase::Verifying => "verifying",
            UpdatePhase::Installing => "installing",
            UpdatePhase::Failed => "failed",
        }
    }
}

/// Update machinery state, owned by `AppState` and cloned into `UiSnapshot`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateState {
    pub phase: UpdatePhase,
    /// Last successfully fetched release, newer than us or not.
    pub latest: Option<ReleaseInfo>,
    pub last_checked_unix_ms: Option<u64>,
    pub last_source: Option<CheckSource>,
    /// Version the startup prompt was last shown for (persisted), so each
    /// version is offered once at startup.
    pub prompted_version: Option<String>,
    /// Human-readable failure for the settings section / dialog.
    pub error: Option<String>,
    /// `None` when this copy was not set up by the installer.
    pub install_scope: Option<InstallScope>,
}

impl UpdateState {
    pub fn current_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// The fetched release when it is newer than the running version.
    pub fn newer_release(&self) -> Option<&ReleaseInfo> {
        let latest = self.latest.as_ref()?;
        version::is_newer(&latest.version, &version::current()).then_some(latest)
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Checking
                | UpdatePhase::Downloading { .. }
                | UpdatePhase::Verifying
                | UpdatePhase::Installing
        )
    }

    pub fn is_installed_copy(&self) -> bool {
        self.install_scope.is_some()
    }

    /// Release page for "what's new", falling back to the releases list.
    pub fn release_url(&self) -> String {
        self.latest
            .as_ref()
            .map(|release| release.html_url.clone())
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
            .unwrap_or_else(|| RELEASES_PAGE_URL.to_string())
    }

    /// One-line status for Settings ▸ Updates.
    pub fn status_line(&self) -> String {
        let current = Self::current_version();
        match &self.phase {
            UpdatePhase::Idle => {
                "Updates come from the project's GitHub Releases. Nothing has been checked yet this session."
                    .to_string()
            }
            UpdatePhase::Checking => "Checking for updates…".to_string(),
            UpdatePhase::UpToDate => format!("You're on the latest version (v{current})."),
            UpdatePhase::Available => match self.newer_release() {
                Some(release) => format!(
                    "Version {} is available. You're on v{current}.",
                    release.version
                ),
                None => "An update is available.".to_string(),
            },
            UpdatePhase::Downloading { received, total } => match self.newer_release() {
                Some(release) => format!(
                    "Downloading v{}… {}",
                    release.version,
                    progress_text(*received, *total)
                ),
                None => format!("Downloading… {}", progress_text(*received, *total)),
            },
            UpdatePhase::Verifying => "Verifying the downloaded installer…".to_string(),
            UpdatePhase::Installing => {
                "Installing… Terminal Manager closes now and reopens on the new version."
                    .to_string()
            }
            UpdatePhase::Failed => self
                .error
                .clone()
                .unwrap_or_else(|| "The last update attempt failed.".to_string()),
        }
    }
}

pub fn progress_percent(received: u64, total: u64) -> Option<u8> {
    (total > 0).then(|| (received.saturating_mul(100) / total).min(100) as u8)
}

pub fn progress_text(received: u64, total: u64) -> String {
    match progress_percent(received, total) {
        Some(percent) => format!(
            "{} of {} ({percent}%)",
            format_bytes(received),
            format_bytes(total)
        ),
        None => format_bytes(received),
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// What the worker threads need to hand results back: the shared state to
/// apply into and a rebuild trigger. Registered once at startup; absent in
/// unit tests, where the mutators run but no thread starts.
pub struct UpdateHooks {
    pub shared: SharedState,
    pub request_rebuild: Box<dyn Fn() + Send + Sync>,
}

static HOOKS: OnceLock<Arc<UpdateHooks>> = OnceLock::new();

pub fn register_hooks(hooks: UpdateHooks) {
    let _ = HOOKS.set(Arc::new(hooks));
}

fn hooks() -> Option<Arc<UpdateHooks>> {
    HOOKS.get().cloned()
}

pub struct FeedConfig {
    pub url: String,
    /// `true` when `TM_UPDATE_FEED_URL` is set: enables `file://` and lets
    /// the startup check run under dev/test profiles.
    pub overridden: bool,
}

pub fn feed_config() -> FeedConfig {
    match std::env::var(ENV_FEED_URL) {
        Ok(url) if !url.trim().is_empty() => FeedConfig {
            url: url.trim().to_string(),
            overridden: true,
        },
        _ => FeedConfig {
            url: DEFAULT_FEED_URL.to_string(),
            overridden: false,
        },
    }
}

/// Where installers are downloaded: the profile's cache dir
/// (`%LOCALAPPDATA%\com.godly.terminal[.<tag>]\updates` on Windows), never
/// the roaming config dir, so a 30 MB installer is not synced around.
pub fn updates_dir() -> Option<PathBuf> {
    crate::profile::cache_dir().map(|dir| dir.join("updates"))
}

fn new_record<'a>(
    event: &'static str,
    level: &'static str,
    state: &'a AppState,
) -> UpdateEventRecord<'a> {
    UpdateEventRecord::new(event, level, &state.restore_correlation_id)
}

fn startup_check_enabled(state: &AppState) -> bool {
    state
        .toggles
        .get(&ToggleKey::CheckUpdatesOnStartup)
        .copied()
        .unwrap_or(true)
}

/// Detect whether this exe is a registered install. Called once at startup.
pub fn init(state: &mut AppState) {
    state.update.install_scope = install::detect_install_scope();
    let mut record = new_record("update.init", "info", state);
    record.scope = state.update.install_scope.map(InstallScope::as_str);
    record.outcome = Some(if state.update.is_installed_copy() {
        "installed_copy"
    } else {
        "unmanaged_copy"
    });
    telemetry::record(&record);
}

// -- check ------------------------------------------------------------------

/// Start a check. Returns `false` when one (or a download) is already
/// running. Off the main thread the fetch runs in a worker; the result lands
/// through [`apply_check_result`].
pub fn begin_check(state: &mut AppState, source: CheckSource) -> bool {
    let config = feed_config();
    if !mark_checking(state, source, &config) {
        return false;
    }
    if let Some(hooks) = hooks() {
        spawn_check(hooks, source, config);
    }
    true
}

fn mark_checking(state: &mut AppState, source: CheckSource, config: &FeedConfig) -> bool {
    if state.update.busy() {
        return false;
    }
    state.update.phase = UpdatePhase::Checking;
    state.update.error = None;
    state.update.last_source = Some(source);
    let mut record = new_record("update.check_started", "info", state);
    record.source = Some(source.as_str());
    record.feed_overridden = Some(config.overridden);
    telemetry::record(&record);
    true
}

fn spawn_check(hooks: Arc<UpdateHooks>, source: CheckSource, config: FeedConfig) {
    let on_spawn_error = hooks.clone();
    let spawned = std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            let transport = transport::Transport::new(config.overridden);
            let result = feed::fetch_latest(&transport, &config.url);
            mutate_with(&hooks.shared, |st| {
                apply_check_result(st, source, result);
            });
            (hooks.request_rebuild)();
        });
    if spawned.is_err() {
        mutate_with(&on_spawn_error.shared, |st| {
            fail_worker_spawn(st, "update-check");
        });
    }
}

fn fail_worker_spawn(state: &mut AppState, worker: &'static str) {
    state.update.phase = UpdatePhase::Failed;
    state.update.error = Some("The update worker thread could not be started.".to_string());
    let mut record = new_record("update.worker_spawn_failed", "error", state);
    record.reason = Some(worker);
    telemetry::record(&record);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    Available,
    UpToDate,
    Failed,
}

/// Apply a finished check. Startup checks prompt once per version; manual
/// checks only update the settings section; install-driven checks chain
/// into [`begin_install`]. Failures never toast (an offline machine starts
/// this app every day); the settings section shows the message.
pub fn apply_check_result(
    state: &mut AppState,
    source: CheckSource,
    result: Result<ReleaseInfo, feed::FeedError>,
) -> CheckOutcome {
    state.update.last_checked_unix_ms = Some(telemetry::now_unix_ms());
    match result {
        Ok(release) => {
            let newer = version::is_newer(&release.version, &version::current());
            let mut record = new_record("update.check_completed", "info", state);
            record.source = Some(source.as_str());
            record.latest_version = Some(release.version.to_string());
            record.outcome = Some(if newer { "available" } else { "up_to_date" });
            telemetry::record(&record);

            state.update.error = None;
            state.update.latest = Some(release.clone());
            if !newer {
                state.update.phase = UpdatePhase::UpToDate;
                return CheckOutcome::UpToDate;
            }
            state.update.phase = UpdatePhase::Available;
            match source {
                CheckSource::Startup => maybe_prompt_on_startup(state, &release),
                CheckSource::Manual => {}
                CheckSource::Install => {
                    begin_install(state);
                }
            }
            CheckOutcome::Available
        }
        Err(error) => {
            state.update.phase = UpdatePhase::Failed;
            state.update.error = Some(error.to_string());
            let level = if source == CheckSource::Startup {
                "warn"
            } else {
                "error"
            };
            let mut record = new_record("update.check_failed", level, state);
            record.source = Some(source.as_str());
            record.error_kind = Some(error.kind());
            record.http_status = error.http_status();
            telemetry::record(&record);
            CheckOutcome::Failed
        }
    }
}

fn maybe_prompt_on_startup(state: &mut AppState, release: &ReleaseInfo) {
    let version = release.version.to_string();
    let skip_reason = if state.update.prompted_version.as_deref() == Some(version.as_str()) {
        Some("already_prompted")
    } else if state.confirm_dialog.is_some() {
        // Never replace a dialog the user is looking at; next launch asks.
        Some("dialog_open")
    } else {
        None
    };
    if let Some(reason) = skip_reason {
        let mut record = new_record("update.prompt_skipped", "info", state);
        record.latest_version = Some(version);
        record.reason = Some(reason);
        telemetry::record(&record);
        return;
    }
    state.confirm_dialog = Some(ConfirmDialog::UpdateAvailable);
    state.update.prompted_version = Some(version.clone());
    let saved = crate::persist::save_workspaces(state);
    let mut record = new_record("update.prompt_shown", "info", state);
    record.latest_version = Some(version);
    record.outcome = Some(if saved { "saved" } else { "persist_failed" });
    telemetry::record(&record);
}

/// `update.show_dialog`: open the offer for a known newer release without
/// touching the once-per-version bookkeeping (scripts, e2e).
pub fn show_prompt(state: &mut AppState) -> bool {
    if state.update.newer_release().is_none() || state.confirm_dialog.is_some() {
        return false;
    }
    state.confirm_dialog = Some(ConfirmDialog::UpdateAvailable);
    true
}

/// `update.later`: close the offer. A download in flight keeps going and
/// stays visible in Settings ▸ Updates.
pub fn dismiss_prompt(state: &mut AppState) -> bool {
    if !matches!(state.confirm_dialog, Some(ConfirmDialog::UpdateAvailable)) {
        return false;
    }
    state.confirm_dialog = None;
    let mut record = new_record("update.prompt_dismissed", "info", state);
    record.latest_version = state
        .update
        .newer_release()
        .map(|release| release.version.to_string());
    record.outcome = Some(if state.update.busy() {
        "hidden_while_busy"
    } else {
        "later"
    });
    telemetry::record(&record);
    true
}

/// `update.open_release_page`: the release notes in the default browser.
pub fn open_release_page(state: &mut AppState) -> bool {
    let url = state.update.release_url();
    match crate::browser::open_url(&url) {
        Ok(()) => {
            let record = new_record("update.release_page_opened", "info", state);
            telemetry::record(&record);
        }
        Err(error) => {
            let mut record = new_record("update.release_page_failed", "warn", state);
            record.error_kind = Some(io_error_kind(&error));
            telemetry::record(&record);
            push_error_toast(
                state,
                "The release page could not be opened in your browser. Visit the project's GitHub Releases page to download the installer.",
            );
        }
    }
    true
}

/// `update.startup_check.toggle`: flip and persist the startup check.
pub fn toggle_startup_check(state: &mut AppState) -> bool {
    let now_on = !startup_check_enabled(state);
    state
        .toggles
        .insert(ToggleKey::CheckUpdatesOnStartup, now_on);
    let saved = crate::persist::save_workspaces(state);
    let mut record = new_record("update.startup_check_toggled", "info", state);
    record.outcome = Some(if now_on { "on" } else { "off" });
    if !saved {
        record.error_kind = Some("workspace_write");
    }
    telemetry::record(&record);
    true
}

/// Why the delayed startup check should not run, if it should not.
pub fn startup_check_skip_reason(state: &AppState, feed_overridden: bool) -> Option<&'static str> {
    if !startup_check_enabled(state) {
        return Some("disabled");
    }
    // Dev builds and every scripted/e2e profile stay hermetic and off the
    // GitHub rate limit unless a feed was pointed at explicitly.
    if crate::profile::active_profile().is_some() && !feed_overridden {
        return Some("dev_profile");
    }
    if state.update.last_checked_unix_ms.is_some() || state.update.busy() {
        return Some("already_checked");
    }
    None
}

fn startup_delay() -> Duration {
    std::env::var(ENV_STARTUP_DELAY_MS)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_STARTUP_DELAY)
}

/// Start the delayed startup check on its own thread. Safe before the event
/// loop exists: an empty `sink` just means no rebuild is requested.
pub fn start_startup_check(shared: SharedState, sink: Arc<OnceLock<EventSink>>) {
    let on_spawn_error = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("update-startup-check".into())
        .spawn(move || {
            cleanup_stale_downloads(&shared);
            std::thread::sleep(startup_delay());
            let config = feed_config();
            let proceed = mutate_with(&shared, |st| {
                match startup_check_skip_reason(st, config.overridden) {
                    Some(reason) => {
                        let mut record = new_record("update.startup_check_skipped", "info", st);
                        record.reason = Some(reason);
                        telemetry::record(&record);
                        false
                    }
                    None => mark_checking(st, CheckSource::Startup, &config),
                }
            });
            if !proceed {
                return;
            }
            let transport = transport::Transport::new(config.overridden);
            let result = feed::fetch_latest(&transport, &config.url);
            mutate_with(&shared, |st| {
                apply_check_result(st, CheckSource::Startup, result);
            });
            if let Some(sink) = sink.get() {
                let _ = sink.send(ExternalEvent::RequestRebuild);
            }
        });
    if spawned.is_err() {
        mutate_with(&on_spawn_error, |st| {
            let mut record = new_record("update.worker_spawn_failed", "warn", st);
            record.reason = Some("update-startup-check");
            telemetry::record(&record);
        });
    }
}

/// Remove installers left over from earlier runs (the installer itself
/// cannot delete the file it is running from). Logs are kept for diagnosis.
fn cleanup_stale_downloads(shared: &SharedState) {
    let Some(dir) = updates_dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut removed = 0u64;
    let mut bytes = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("partial")
            });
        if !stale {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
            bytes += size;
        }
    }
    if removed == 0 {
        return;
    }
    mutate_with(shared, |st| {
        let mut record = new_record("update.stale_downloads_removed", "info", st);
        record.bytes = Some(bytes);
        record.total_bytes = Some(removed);
        telemetry::record(&record);
    });
}

// -- install ----------------------------------------------------------------

/// `update.install`. With a newer release known: download it and hand off.
/// With nothing known yet: check first (the `Install` source chains back
/// here). Unmanaged copies open the release page instead.
pub fn begin_install(state: &mut AppState) -> bool {
    if state.update.busy() {
        return false;
    }
    let Some(release) = state.update.newer_release().cloned() else {
        return begin_check(state, CheckSource::Install);
    };
    if !state.update.is_installed_copy() {
        let mut record = new_record("update.install_redirected", "info", state);
        record.latest_version = Some(release.version.to_string());
        record.reason = Some("unmanaged_copy");
        telemetry::record(&record);
        return open_release_page(state);
    }
    let Some(asset) = release.installer.clone() else {
        fail_install(
            state,
            "no_installer_asset",
            format!(
                "Release {} has no installer attached. Open the release page to update manually.",
                release.version
            ),
        );
        return true;
    };
    let Some(dir) = updates_dir() else {
        fail_install(
            state,
            "no_data_dir",
            "No local data directory is available to download the installer into.".to_string(),
        );
        return true;
    };
    state.update.phase = UpdatePhase::Downloading {
        received: 0,
        total: asset.size,
    };
    state.update.error = None;
    let mut record = new_record("update.download_started", "info", state);
    record.latest_version = Some(release.version.to_string());
    record.total_bytes = Some(asset.size);
    telemetry::record(&record);
    if let Some(hooks) = hooks() {
        spawn_download(hooks, asset, dir);
    }
    true
}

fn fail_install(state: &mut AppState, error_kind: &'static str, message: String) {
    state.update.phase = UpdatePhase::Failed;
    state.update.error = Some(message);
    let mut record = new_record("update.install_failed", "error", state);
    record.error_kind = Some(error_kind);
    record.latest_version = state
        .update
        .latest
        .as_ref()
        .map(|release| release.version.to_string());
    telemetry::record(&record);
}

fn spawn_download(hooks: Arc<UpdateHooks>, asset: InstallerAsset, dir: PathBuf) {
    let on_spawn_error = hooks.clone();
    let spawned = std::thread::Builder::new()
        .name("update-download".into())
        .spawn(move || {
            let transport = transport::Transport::new(feed_config().overridden);
            let mut last_report = Instant::now();
            let mut last_bytes = 0u64;
            let progress_hooks = hooks.clone();
            let mut progress = |received: u64, total: u64| {
                let due = received.saturating_sub(last_bytes) >= PROGRESS_STEP_BYTES
                    || last_report.elapsed() >= PROGRESS_INTERVAL
                    || (total > 0 && received >= total);
                if !due {
                    return;
                }
                last_bytes = received;
                last_report = Instant::now();
                mutate_with(&progress_hooks.shared, |st| {
                    apply_download_progress(st, received, total);
                });
                (progress_hooks.request_rebuild)();
            };
            let result = feed::download_installer(&transport, &asset, &dir, &mut progress);
            match result {
                Ok(downloaded) => {
                    let exit_now = mutate_with(&hooks.shared, |st| {
                        record_download_completed(st, &downloaded);
                        finish_install(st, &downloaded)
                    });
                    (hooks.request_rebuild)();
                    if exit_now {
                        crate::shutdown_now();
                    }
                }
                Err(error) => {
                    mutate_with(&hooks.shared, |st| apply_download_failed(st, &error));
                    (hooks.request_rebuild)();
                }
            }
        });
    if spawned.is_err() {
        mutate_with(&on_spawn_error.shared, |st| {
            fail_worker_spawn(st, "update-download");
        });
    }
}

pub fn apply_download_progress(state: &mut AppState, received: u64, total: u64) -> bool {
    match state.update.phase {
        UpdatePhase::Downloading { .. } => {
            state.update.phase = UpdatePhase::Downloading { received, total };
            true
        }
        _ => false,
    }
}

pub fn apply_download_failed(state: &mut AppState, error: &feed::DownloadError) {
    state.update.phase = UpdatePhase::Failed;
    state.update.error = Some(error.to_string());
    let mut record = new_record("update.download_failed", "error", state);
    record.error_kind = Some(error.kind());
    record.latest_version = state
        .update
        .latest
        .as_ref()
        .map(|release| release.version.to_string());
    telemetry::record(&record);
}

fn record_download_completed(state: &mut AppState, downloaded: &feed::Downloaded) {
    state.update.phase = UpdatePhase::Verifying;
    let mut record = new_record("update.download_completed", "info", state);
    record.bytes = Some(downloaded.bytes);
    record.elapsed_ms = Some(downloaded.elapsed_ms);
    record.outcome = Some(if downloaded.verified_digest {
        "digest_verified"
    } else {
        "size_only"
    });
    record.latest_version = state
        .update
        .latest
        .as_ref()
        .map(|release| release.version.to_string());
    telemetry::record(&record);
}

/// Production hand-off: real installer launch, real daemon shutdown.
fn finish_install(state: &mut AppState, downloaded: &feed::Downloaded) -> bool {
    finish_install_with(state, downloaded, install::launch_installer, |st| {
        st.pty_manager.shutdown_daemon_blocking(true)
    })
}

/// Persist the layout, launch the installer, stop the daemon. Returns `true`
/// when the process must exit now so the installer can replace the files.
///
/// Order matters: the layout is saved first with every tab intact (this is
/// what the relaunch restores); the installer is started before the daemon
/// goes down so a launch failure leaves the sessions untouched; and a daemon
/// that refuses to stop is only logged, since the installer then fails on
/// the in-use binary and relaunches this same version, which reattaches.
pub fn finish_install_with(
    state: &mut AppState,
    downloaded: &feed::Downloaded,
    launch: impl FnOnce(&Path, &[String]) -> io::Result<u32>,
    shutdown_daemon: impl FnOnce(&mut AppState) -> io::Result<()>,
) -> bool {
    let Some(scope) = state.update.install_scope else {
        fail_install(
            state,
            "unmanaged_copy",
            "This copy of Terminal Manager was not set up by the installer, so it cannot replace itself. Open the release page to update manually.".to_string(),
        );
        return false;
    };
    state.update.phase = UpdatePhase::Installing;

    if !crate::persist::save_workspaces(state) {
        fail_install(
            state,
            "workspace_write",
            "Your workspace layout could not be saved, so the update was not started. Check the config directory permissions and try again.".to_string(),
        );
        return false;
    }
    let record = new_record("update.layout_persisted", "info", state);
    telemetry::record(&record);

    let relaunch = std::env::current_exe().unwrap_or_default();
    let log_path = install::log_path_for(&downloaded.path);
    let args = install::installer_args(scope, std::process::id(), &relaunch, &log_path);
    let installer_pid = match launch(&downloaded.path, &args) {
        Ok(pid) => pid,
        Err(error) => {
            let kind = io_error_kind(&error);
            fail_install(
                state,
                kind,
                format!("The installer could not be started ({error}). Nothing was changed."),
            );
            return false;
        }
    };
    let mut record = new_record("update.install_launched", "info", state);
    record.pid = Some(installer_pid);
    record.scope = Some(scope.as_str());
    record.latest_version = state
        .update
        .latest
        .as_ref()
        .map(|release| release.version.to_string());
    telemetry::record(&record);

    match shutdown_daemon(state) {
        Ok(()) => {
            let mut record = new_record("update.daemon_shutdown", "info", state);
            record.outcome = Some("ok");
            telemetry::record(&record);
        }
        Err(error) => {
            let mut record = new_record("update.daemon_shutdown", "warn", state);
            record.outcome = Some("failed");
            record.error_kind = Some(io_error_kind(&error));
            telemetry::record(&record);
        }
    }

    let mut record = new_record("update.exiting", "info", state);
    record.pid = Some(std::process::id());
    telemetry::record(&record);
    true
}

fn io_error_kind(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        io::ErrorKind::NotConnected => "not_connected",
        io::ErrorKind::BrokenPipe => "broken_pipe",
        io::ErrorKind::TimedOut => "timed_out",
        io::ErrorKind::UnexpectedEof => "unexpected_eof",
        _ => "io_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{dispatch, seed_state, SettingsSection};
    use semver::Version;

    fn release(version: &str) -> ReleaseInfo {
        ReleaseInfo {
            version: Version::parse(version).unwrap(),
            tag: format!("v{version}"),
            title: version.to_string(),
            html_url: format!("https://example.invalid/releases/tag/v{version}"),
            installer: Some(InstallerAsset {
                name: format!("terminal-manager-{version}-setup.exe"),
                url: format!("https://example.invalid/{version}/setup.exe"),
                size: 4096,
                sha256: Some([0x42; 32]),
            }),
        }
    }

    fn newer() -> ReleaseInfo {
        release("99.0.0")
    }

    fn current_release() -> ReleaseInfo {
        release(UpdateState::current_version())
    }

    fn state_with(scope: Option<InstallScope>) -> AppState {
        let mut state = seed_state();
        state.update.install_scope = scope;
        state
    }

    #[test]
    fn default_state_is_idle_and_startup_check_is_on() {
        let state = seed_state();
        assert_eq!(state.update.phase, UpdatePhase::Idle);
        assert!(state.update.latest.is_none());
        assert!(startup_check_enabled(&state));
        assert!(!state.update.busy());
        assert!(state.update.status_line().contains("GitHub Releases"));
    }

    #[test]
    fn manual_check_marks_checking_and_refuses_reentry() {
        let mut state = seed_state();
        assert!(dispatch(&mut state, "update.check"));
        assert_eq!(state.update.phase, UpdatePhase::Checking);
        assert_eq!(state.update.last_source, Some(CheckSource::Manual));
        assert!(state.update.busy());
        assert!(!dispatch(&mut state, "update.check"), "already checking");
        assert!(!dispatch(&mut state, "update.install"), "busy");
    }

    #[test]
    fn newer_release_from_manual_check_is_available_without_a_dialog() {
        let mut state = seed_state();
        begin_check(&mut state, CheckSource::Manual);
        let outcome = apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
        assert_eq!(outcome, CheckOutcome::Available);
        assert_eq!(state.update.phase, UpdatePhase::Available);
        assert!(
            state.confirm_dialog.is_none(),
            "manual checks do not pop a dialog"
        );
        assert_eq!(state.update.prompted_version, None);
        assert!(state.update.last_checked_unix_ms.is_some());
        assert!(state.update.status_line().contains("99.0.0"));
        assert_eq!(
            state.update.newer_release().unwrap().version,
            Version::new(99, 0, 0)
        );
    }

    #[test]
    fn same_or_older_release_is_up_to_date() {
        let mut state = seed_state();
        apply_check_result(&mut state, CheckSource::Manual, Ok(current_release()));
        assert_eq!(state.update.phase, UpdatePhase::UpToDate);
        assert!(state.update.newer_release().is_none());
        assert!(state.update.status_line().contains("latest version"));

        apply_check_result(&mut state, CheckSource::Startup, Ok(release("0.0.1")));
        assert_eq!(state.update.phase, UpdatePhase::UpToDate);
        assert!(state.confirm_dialog.is_none());
    }

    #[test]
    fn startup_check_prompts_once_per_version_and_persists_it() {
        let mut state = seed_state();
        let outcome = apply_check_result(&mut state, CheckSource::Startup, Ok(newer()));
        assert_eq!(outcome, CheckOutcome::Available);
        assert_eq!(state.confirm_dialog, Some(ConfirmDialog::UpdateAvailable));
        assert_eq!(state.update.prompted_version.as_deref(), Some("99.0.0"));

        // Later closes it; the release stays available in settings.
        assert!(dispatch(&mut state, "update.later"));
        assert!(state.confirm_dialog.is_none());
        assert_eq!(state.update.phase, UpdatePhase::Available);
        assert!(!dispatch(&mut state, "update.later"), "nothing to dismiss");

        // The same version on the next startup check stays quiet.
        apply_check_result(&mut state, CheckSource::Startup, Ok(newer()));
        assert!(state.confirm_dialog.is_none());

        // A newer version prompts again.
        apply_check_result(&mut state, CheckSource::Startup, Ok(release("99.1.0")));
        assert_eq!(state.confirm_dialog, Some(ConfirmDialog::UpdateAvailable));
        assert_eq!(state.update.prompted_version.as_deref(), Some("99.1.0"));
    }

    #[test]
    fn startup_prompt_never_replaces_an_open_dialog() {
        let mut state = seed_state();
        state.confirm_dialog = Some(ConfirmDialog::KillAll { count: 1 });
        apply_check_result(&mut state, CheckSource::Startup, Ok(newer()));
        assert_eq!(
            state.confirm_dialog,
            Some(ConfirmDialog::KillAll { count: 1 })
        );
        assert_eq!(state.update.phase, UpdatePhase::Available);
        assert_eq!(
            state.update.prompted_version, None,
            "not counted as prompted"
        );
    }

    #[test]
    fn show_dialog_reopens_the_offer_without_touching_bookkeeping() {
        let mut state = seed_state();
        assert!(
            !dispatch(&mut state, "update.show_dialog"),
            "nothing known yet"
        );
        apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
        assert!(dispatch(&mut state, "update.show_dialog"));
        assert_eq!(state.confirm_dialog, Some(ConfirmDialog::UpdateAvailable));
        assert_eq!(state.update.prompted_version, None);
        assert!(!dispatch(&mut state, "update.show_dialog"), "already open");
    }

    #[test]
    fn generic_dialog_confirm_never_installs_and_escape_closes() {
        let mut state = seed_state();
        apply_check_result(&mut state, CheckSource::Startup, Ok(newer()));
        assert_eq!(state.confirm_dialog, Some(ConfirmDialog::UpdateAvailable));
        // A stray Enter routed to the generic confirm must not start an install.
        assert!(!dispatch(&mut state, "dialog.confirm"));
        assert_eq!(state.confirm_dialog, Some(ConfirmDialog::UpdateAvailable));
        assert_eq!(state.update.phase, UpdatePhase::Available);
        // Escape (modal.close) dismisses like Later.
        assert!(dispatch(&mut state, "modal.close"));
        assert!(state.confirm_dialog.is_none());
    }

    #[test]
    fn failed_checks_record_the_message_and_stay_silent() {
        let mut state = seed_state();
        let error = feed::FeedError::Transport(transport::TransportError::Http { status: 403 });
        let outcome = apply_check_result(&mut state, CheckSource::Startup, Err(error));
        assert_eq!(outcome, CheckOutcome::Failed);
        assert_eq!(state.update.phase, UpdatePhase::Failed);
        assert!(state.update.status_line().contains("rate limit"));
        assert!(state.confirm_dialog.is_none());
        assert!(state.toasts.is_empty(), "no toast for a failed check");
        // A later successful check clears the error.
        apply_check_result(&mut state, CheckSource::Manual, Ok(current_release()));
        assert_eq!(state.update.error, None);
    }

    #[test]
    fn startup_check_skip_reasons() {
        let mut state = seed_state();
        // Tests run under the dev profile: only an overridden feed lets it run.
        assert_eq!(
            startup_check_skip_reason(&state, false),
            Some("dev_profile")
        );
        assert_eq!(startup_check_skip_reason(&state, true), None);

        state
            .toggles
            .insert(ToggleKey::CheckUpdatesOnStartup, false);
        assert_eq!(startup_check_skip_reason(&state, true), Some("disabled"));
        state.toggles.insert(ToggleKey::CheckUpdatesOnStartup, true);

        state.update.last_checked_unix_ms = Some(1);
        assert_eq!(
            startup_check_skip_reason(&state, true),
            Some("already_checked")
        );
        state.update.last_checked_unix_ms = None;
        state.update.phase = UpdatePhase::Checking;
        assert_eq!(
            startup_check_skip_reason(&state, true),
            Some("already_checked")
        );
    }

    #[test]
    fn toggle_flips_and_is_reflected_in_persisted_state() {
        let mut state = seed_state();
        assert!(startup_check_enabled(&state));
        assert!(dispatch(&mut state, "update.startup_check.toggle"));
        assert!(!startup_check_enabled(&state));
        let persisted = crate::persist::PersistedState::from_state(&state);
        assert_eq!(persisted.check_updates_on_startup, Some(false));
        assert!(dispatch(&mut state, "update.startup_check.toggle"));
        assert!(startup_check_enabled(&state));
    }

    #[test]
    fn install_with_nothing_known_checks_first_with_the_install_source() {
        let mut state = state_with(Some(InstallScope::CurrentUser));
        assert!(dispatch(&mut state, "update.install"));
        assert_eq!(state.update.phase, UpdatePhase::Checking);
        assert_eq!(state.update.last_source, Some(CheckSource::Install));
        // The chained check finds a newer release and goes straight to download.
        apply_check_result(&mut state, CheckSource::Install, Ok(newer()));
        assert_eq!(
            state.update.phase,
            UpdatePhase::Downloading {
                received: 0,
                total: 4096
            }
        );
        assert!(state.update.busy());
    }

    #[test]
    fn install_on_installed_copy_starts_download_and_tracks_progress() {
        let mut state = state_with(Some(InstallScope::AllUsers));
        apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
        assert!(dispatch(&mut state, "update.install"));
        assert_eq!(
            state.update.phase,
            UpdatePhase::Downloading {
                received: 0,
                total: 4096
            }
        );
        assert!(apply_download_progress(&mut state, 2048, 4096));
        assert!(state.update.status_line().contains("50%"));
        assert!(
            !dispatch(&mut state, "update.install"),
            "busy while downloading"
        );
        // Hiding the dialog while busy is allowed; the download continues.
        state.confirm_dialog = Some(ConfirmDialog::UpdateAvailable);
        assert!(dispatch(&mut state, "update.later"));
        assert!(matches!(
            state.update.phase,
            UpdatePhase::Downloading { .. }
        ));

        let error = feed::DownloadError::DigestMismatch;
        apply_download_failed(&mut state, &error);
        assert_eq!(state.update.phase, UpdatePhase::Failed);
        assert!(state.update.status_line().contains("SHA-256"));
        assert!(
            !apply_download_progress(&mut state, 1, 1),
            "no progress after failure"
        );
        // Retry is possible from Failed.
        assert!(dispatch(&mut state, "update.install"));
        assert!(matches!(
            state.update.phase,
            UpdatePhase::Downloading { .. }
        ));
    }

    #[test]
    fn install_without_an_installer_asset_fails_with_a_message() {
        let mut state = state_with(Some(InstallScope::CurrentUser));
        let mut bare = newer();
        bare.installer = None;
        apply_check_result(&mut state, CheckSource::Manual, Ok(bare));
        assert!(dispatch(&mut state, "update.install"));
        assert_eq!(state.update.phase, UpdatePhase::Failed);
        assert!(state.update.status_line().contains("no installer"));
    }

    #[test]
    fn unmanaged_copy_is_never_installed_in_place() {
        let mut state = state_with(None);
        apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
        assert!(!state.update.is_installed_copy());
        // begin_install redirects to the release page; the browser call may
        // fail in CI, which only produces a toast, never a download.
        assert!(dispatch(&mut state, "update.install"));
        assert!(!matches!(
            state.update.phase,
            UpdatePhase::Downloading { .. }
        ));
        assert_eq!(state.update.phase, UpdatePhase::Available);
        assert!(state.update.release_url().contains("99.0.0"));
    }

    fn downloaded(dir: &Path) -> feed::Downloaded {
        feed::Downloaded {
            path: dir.join("terminal-manager-99.0.0-setup.exe"),
            bytes: 4096,
            elapsed_ms: 12,
            verified_digest: true,
        }
    }

    #[test]
    fn handoff_persists_launches_then_stops_daemon_and_requests_exit() {
        let mut state = state_with(Some(InstallScope::CurrentUser));
        apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
        let dir = std::env::temp_dir();
        let mut launched: Option<(PathBuf, Vec<String>)> = None;
        let mut daemon_stopped = false;
        let exit_now = finish_install_with(
            &mut state,
            &downloaded(&dir),
            |installer, args| {
                launched = Some((installer.to_path_buf(), args.to_vec()));
                Ok(777)
            },
            |_st| {
                daemon_stopped = true;
                Ok(())
            },
        );
        assert!(exit_now);
        assert!(daemon_stopped);
        assert_eq!(state.update.phase, UpdatePhase::Installing);
        let (installer, args) = launched.expect("installer launched");
        assert!(installer.ends_with("terminal-manager-99.0.0-setup.exe"));
        assert!(args.contains(&"/VERYSILENT".to_string()));
        assert!(args.contains(&"/CURRENTUSER".to_string()));
        assert!(args.contains(&"/SELFUPDATE=1".to_string()));
        assert!(args.contains(&format!("/PARENTPID={}", std::process::id())));
        assert!(args.iter().any(|a| a.starts_with("/RELAUNCH=")));
        assert!(args
            .iter()
            .any(|a| a.starts_with("/LOG=") && a.ends_with("terminal-manager-99.0.0-setup.log")));
    }

    #[test]
    fn handoff_still_exits_when_the_daemon_refuses_to_stop() {
        let mut state = state_with(Some(InstallScope::AllUsers));
        apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
        let exit_now = finish_install_with(
            &mut state,
            &downloaded(&std::env::temp_dir()),
            |_, args| {
                assert!(args.contains(&"/ALLUSERS".to_string()));
                Ok(1)
            },
            |_| Err(io::Error::new(io::ErrorKind::TimedOut, "no reply")),
        );
        assert!(
            exit_now,
            "the installer relaunches the old build on failure"
        );
    }

    #[test]
    fn handoff_aborts_without_exiting_when_the_installer_cannot_start() {
        let mut state = state_with(Some(InstallScope::CurrentUser));
        apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
        let mut daemon_touched = false;
        let exit_now = finish_install_with(
            &mut state,
            &downloaded(&std::env::temp_dir()),
            |_, _| Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked")),
            |_| {
                daemon_touched = true;
                Ok(())
            },
        );
        assert!(!exit_now);
        assert!(!daemon_touched, "sessions untouched when the launch fails");
        assert_eq!(state.update.phase, UpdatePhase::Failed);
        assert!(state.update.status_line().contains("could not be started"));
    }

    #[test]
    fn handoff_refuses_unmanaged_copies() {
        let mut state = state_with(None);
        apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
        let exit_now = finish_install_with(
            &mut state,
            &downloaded(&std::env::temp_dir()),
            |_, _| panic!("must not launch"),
            |_| panic!("must not stop the daemon"),
        );
        assert!(!exit_now);
        assert_eq!(state.update.phase, UpdatePhase::Failed);
    }

    #[test]
    fn progress_formatting() {
        assert_eq!(progress_percent(0, 0), None);
        assert_eq!(progress_percent(5, 10), Some(50));
        assert_eq!(progress_percent(20, 10), Some(100));
        assert_eq!(format_bytes(512), "1 KB");
        assert_eq!(format_bytes(8 * 1024 * 1024 + 512 * 1024), "8.5 MB");
        assert_eq!(
            progress_text(1024 * 1024, 4 * 1024 * 1024),
            "1.0 MB of 4.0 MB (25%)"
        );
        assert_eq!(progress_text(2048, 0), "2 KB");
    }

    #[test]
    fn updates_settings_section_is_addressable_by_label() {
        assert_eq!(
            SettingsSection::from_label("updates"),
            Some(SettingsSection::Updates)
        );
        assert_eq!(SettingsSection::Updates.label(), "updates");
        assert!(SettingsSection::all().contains(&SettingsSection::Updates));
        let mut state = seed_state();
        assert!(dispatch(&mut state, "settings.section:updates"));
        assert!(state.settings_open);
        assert_eq!(state.settings_section, SettingsSection::Updates);
    }

    #[test]
    fn feed_config_defaults_to_github() {
        // The env var is process-global; only assert the default shape here.
        let config = FeedConfig {
            url: DEFAULT_FEED_URL.to_string(),
            overridden: false,
        };
        assert!(config.url.starts_with("https://api.github.com/repos/"));
        assert!(config.url.ends_with("/releases/latest"));
        assert!(!config.overridden);
    }
}
