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
fn install_refuses_a_release_without_a_digest() {
    let mut state = state_with(Some(InstallScope::CurrentUser));
    let mut unsigned = newer();
    unsigned.installer.as_mut().unwrap().sha256 = None;
    apply_check_result(&mut state, CheckSource::Manual, Ok(unsigned));
    assert!(dispatch(&mut state, "update.install"));
    assert_eq!(state.update.phase, UpdatePhase::Failed);
    assert!(
        state
            .update
            .error
            .as_deref()
            .is_some_and(|e| e.contains("SHA-256 checksum")),
        "{:?}",
        state.update.error
    );
    assert!(!state.update.busy(), "nothing was downloaded");
    assert!(
        state.update.newer_release().is_some(),
        "the release stays known so the release page can be opened"
    );
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
fn handoff_persists_launches_and_preserves_daemon_before_exit() {
    let mut state = state_with(Some(InstallScope::CurrentUser));
    apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
    let dir = std::env::temp_dir();
    let mut launched: Option<(PathBuf, Vec<String>)> = None;
    let exit_now = finish_install_with(&mut state, &downloaded(&dir), |installer, args| {
        launched = Some((installer.to_path_buf(), args.to_vec()));
        Ok(777)
    });
    assert!(exit_now);
    assert_eq!(state.update.phase, UpdatePhase::Installing);
    let (installer, args) = launched.expect("installer launched");
    assert!(installer.ends_with("terminal-manager-99.0.0-setup.exe"));
    assert!(args.contains(&"/VERYSILENT".to_string()));
    assert!(args.contains(&"/CURRENTUSER".to_string()));
    assert!(args.contains(&"/SELFUPDATE=1".to_string()));
    assert!(args.contains(&format!("/PARENTPID={}", std::process::id())));
    assert!(args.iter().any(|a| a.starts_with("/RELAUNCH=")));
    assert!(args.contains(&format!(
        "/DAEMONSOCKET={}",
        crate::ptyd_socket_path().display()
    )));
    assert!(args
        .iter()
        .any(|a| a.starts_with("/LOG=") && a.ends_with("terminal-manager-99.0.0-setup.log")));
}

#[test]
fn handoff_preserves_machine_install_scope() {
    let mut state = state_with(Some(InstallScope::AllUsers));
    apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
    let exit_now =
        finish_install_with(&mut state, &downloaded(&std::env::temp_dir()), |_, args| {
            assert!(args.contains(&"/ALLUSERS".to_string()));
            Ok(1)
        });
    assert!(
        exit_now,
        "the installer relaunches the old build on failure"
    );
}

#[test]
fn handoff_aborts_without_exiting_when_the_installer_cannot_start() {
    let mut state = state_with(Some(InstallScope::CurrentUser));
    apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
    let exit_now = finish_install_with(&mut state, &downloaded(&std::env::temp_dir()), |_, _| {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "blocked"))
    });
    assert!(!exit_now);
    assert_eq!(state.update.phase, UpdatePhase::Failed);
    assert!(state.update.status_line().contains("could not be started"));
}

#[test]
fn handoff_refuses_unmanaged_copies() {
    let mut state = state_with(None);
    apply_check_result(&mut state, CheckSource::Manual, Ok(newer()));
    let exit_now = finish_install_with(&mut state, &downloaded(&std::env::temp_dir()), |_, _| {
        panic!("must not launch")
    });
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
