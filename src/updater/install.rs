//! Installed-copy detection and the silent Inno Setup hand-off.
//!
//! The installer (`packaging/terminal-manager.iss`) registers an uninstall
//! key named after its fixed AppId, in HKCU for a per-user install and in
//! HKLM for a per-machine one. This copy is "installed" when its own
//! directory equals that key's `InstallLocation`; a copy running from
//! `target\debug` or an unpacked zip is not, and the UI offers the release
//! page instead of an in-place update.
//!
//! The hand-off runs the downloaded installer with Inno's silent switches
//! plus three custom parameters the script reads: `/SELFUPDATE=1` marks a
//! self-update run, `/PARENTPID=<pid>` tells `PrepareToInstall` which
//! process to wait for before the in-use check, and `/RELAUNCH=<exe>` is
//! what `DeinitializeSetup` starts once Setup ends, whether it succeeded
//! (new binary) or aborted (old binary, which reattaches to any surviving
//! sessions).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallScope {
    CurrentUser,
    AllUsers,
}

impl InstallScope {
    pub fn as_str(self) -> &'static str {
        match self {
            InstallScope::CurrentUser => "current_user",
            InstallScope::AllUsers => "all_users",
        }
    }

    /// The Inno command-line switch that reproduces this scope silently.
    pub fn inno_switch(self) -> &'static str {
        match self {
            InstallScope::CurrentUser => "/CURRENTUSER",
            InstallScope::AllUsers => "/ALLUSERS",
        }
    }
}

/// Dev/e2e override for the registry lookup: `user`, `machine` or `none`.
pub const ENV_INSTALL_SCOPE: &str = "TM_UPDATE_INSTALL_SCOPE";
/// Inno's uninstall key for our AppId (`{{...}}` in the .iss is one brace).
pub const UNINSTALL_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{B3E1B6B2-7C44-4E2E-9C1A-0A1D2E3F4A5B}_is1";

/// Where this executable was installed by the installer, if it was.
pub fn detect_install_scope() -> Option<InstallScope> {
    if let Some(forced) = std::env::var_os(ENV_INSTALL_SCOPE) {
        return scope_override(&forced.to_string_lossy());
    }
    let exe = std::env::current_exe().ok()?;
    scope_for(
        &exe,
        read_install_location(RegistryRoot::CurrentUser).as_deref(),
        read_install_location(RegistryRoot::LocalMachine).as_deref(),
    )
}

pub fn scope_override(value: &str) -> Option<InstallScope> {
    match value.trim().to_ascii_lowercase().as_str() {
        "user" | "current_user" | "currentuser" => Some(InstallScope::CurrentUser),
        "machine" | "all_users" | "allusers" => Some(InstallScope::AllUsers),
        _ => None,
    }
}

/// Pure half of [`detect_install_scope`]: the executable's directory must
/// equal a registered `InstallLocation` (case-insensitive, separators and
/// trailing slashes normalised).
pub fn scope_for(
    exe: &Path,
    user_location: Option<&str>,
    machine_location: Option<&str>,
) -> Option<InstallScope> {
    let exe_dir = normalize_dir(&exe.parent()?.to_string_lossy());
    if exe_dir.is_empty() {
        return None;
    }
    if user_location.map(normalize_dir).as_deref() == Some(exe_dir.as_str()) {
        return Some(InstallScope::CurrentUser);
    }
    if machine_location.map(normalize_dir).as_deref() == Some(exe_dir.as_str()) {
        return Some(InstallScope::AllUsers);
    }
    None
}

fn normalize_dir(raw: &str) -> String {
    raw.trim()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug)]
enum RegistryRoot {
    CurrentUser,
    LocalMachine,
}

#[cfg(windows)]
fn read_install_location(root: RegistryRoot) -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
    };

    let hkey = match root {
        RegistryRoot::CurrentUser => HKEY_CURRENT_USER,
        RegistryRoot::LocalMachine => HKEY_LOCAL_MACHINE,
    };
    let subkey = wide_null(UNINSTALL_SUBKEY);
    let value_name = wide_null("InstallLocation");
    // Fixed buffer: install paths are far below 2048 UTF-16 units and the
    // registry-reported size is never used for allocation.
    let mut stored = [0u16; 2048];
    let mut stored_type = 0u32;
    let mut byte_len =
        u32::try_from(std::mem::size_of_val(&stored)).expect("fixed registry buffer fits in u32");
    let status = unsafe {
        RegGetValueW(
            hkey,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            &mut stored_type,
            stored.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let units = (byte_len as usize / std::mem::size_of::<u16>()).min(stored.len());
    let text = String::from_utf16_lossy(&stored[..units]);
    let text = text.trim_end_matches('\0').trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(not(windows))]
fn read_install_location(_root: RegistryRoot) -> Option<String> {
    None
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Custom Inno parameter marking a self-update run (read by the .iss).
pub const SELFUPDATE_ARG: &str = "/SELFUPDATE=1";

/// Command line for the downloaded installer.
///
/// Standard switches: fully silent, no message boxes (an in-use failure
/// aborts instead of prompting), no reboot, no cancel, and no Restart
/// Manager relaunch (the script relaunches the app itself). The scope switch
/// reproduces the existing install's scope so a per-user copy never turns
/// into a per-machine one. `/LOG` lands next to the installer so a silent
/// failure is diagnosable afterwards.
pub fn installer_args(
    scope: InstallScope,
    parent_pid: u32,
    relaunch_exe: &Path,
    log_path: &Path,
) -> Vec<String> {
    vec![
        "/VERYSILENT".to_string(),
        "/SUPPRESSMSGBOXES".to_string(),
        "/NORESTART".to_string(),
        "/NOCANCEL".to_string(),
        "/NORESTARTAPPLICATIONS".to_string(),
        scope.inno_switch().to_string(),
        SELFUPDATE_ARG.to_string(),
        format!("/PARENTPID={parent_pid}"),
        format!("/RELAUNCH={}", relaunch_exe.display()),
        format!("/LOG={}", log_path.display()),
    ]
}

/// Start the installer detached from this process and return its pid. The
/// child must outlive us: no console, no inherited stdio, own process group.
pub fn launch_installer(installer: &Path, args: &[String]) -> io::Result<u32> {
    let mut command = Command::new(installer);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = installer.parent() {
        command.current_dir(dir);
    }
    apply_detached_flags(&mut command);
    let child = command.spawn()?;
    Ok(child.id())
}

#[cfg(windows)]
fn apply_detached_flags(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(windows))]
fn apply_detached_flags(_command: &mut Command) {}

/// Log file the installer writes for a given downloaded installer.
pub fn log_path_for(installer: &Path) -> PathBuf {
    installer.with_extension("log")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real uninstall key of the app installed on this machine.
    /// Ignored because it depends on the machine; run it once per installer
    /// change with `--ignored --nocapture` to prove the registry read works
    /// on real data (buffer, terminator, path shape).
    #[test]
    #[ignore]
    fn real_registry_install_location_is_readable() {
        let user = read_install_location(RegistryRoot::CurrentUser);
        let machine = read_install_location(RegistryRoot::LocalMachine);
        eprintln!("HKCU InstallLocation = {user:?}");
        eprintln!("HKLM InstallLocation = {machine:?}");
        let found = user
            .or(machine)
            .expect("Terminal Manager is registered in HKCU or HKLM on this machine");
        assert!(
            Path::new(&found).join("terminal-manager.exe").is_file(),
            "InstallLocation {found} does not contain terminal-manager.exe"
        );
    }

    #[test]
    fn scope_matches_exe_directory_against_install_location() {
        let exe = Path::new(
            r"C:\Users\Someone\AppData\Local\Programs\Terminal Manager\terminal-manager.exe",
        );
        // Inno writes the location with a trailing backslash.
        assert_eq!(
            scope_for(
                exe,
                Some(r"C:\Users\Someone\AppData\Local\Programs\Terminal Manager\"),
                None
            ),
            Some(InstallScope::CurrentUser)
        );
        // Case and separators do not matter.
        assert_eq!(
            scope_for(
                exe,
                None,
                Some("c:/users/someone/appdata/local/programs/terminal manager")
            ),
            Some(InstallScope::AllUsers)
        );
        // A dev build elsewhere is not an installed copy even with a key present.
        assert_eq!(
            scope_for(
                Path::new(r"C:\dev\repo\target\debug\terminal-manager.exe"),
                Some(r"C:\Users\Someone\AppData\Local\Programs\Terminal Manager\"),
                Some(r"C:\Program Files\Terminal Manager\")
            ),
            None
        );
        // A subdirectory of the install dir is not the install dir.
        assert_eq!(
            scope_for(
                Path::new(r"C:\Program Files\Terminal Manager\tools\x.exe"),
                None,
                Some(r"C:\Program Files\Terminal Manager\")
            ),
            None
        );
        assert_eq!(scope_for(exe, None, None), None);
    }

    #[test]
    fn scope_override_accepts_documented_values() {
        assert_eq!(scope_override("user"), Some(InstallScope::CurrentUser));
        assert_eq!(scope_override(" MACHINE "), Some(InstallScope::AllUsers));
        assert_eq!(scope_override("none"), None);
        assert_eq!(scope_override(""), None);
    }

    #[test]
    fn installer_args_are_silent_scoped_and_carry_the_handoff_params() {
        let args = installer_args(
            InstallScope::CurrentUser,
            4242,
            Path::new(r"C:\Apps\Terminal Manager\terminal-manager.exe"),
            Path::new(r"C:\Data\updates\terminal-manager-0.5.0-setup.log"),
        );
        assert_eq!(args[0], "/VERYSILENT");
        assert!(args.contains(&"/SUPPRESSMSGBOXES".to_string()));
        assert!(args.contains(&"/NORESTART".to_string()));
        assert!(args.contains(&"/NOCANCEL".to_string()));
        assert!(args.contains(&"/NORESTARTAPPLICATIONS".to_string()));
        assert!(args.contains(&"/CURRENTUSER".to_string()));
        assert!(!args.contains(&"/ALLUSERS".to_string()));
        assert!(args.contains(&"/SELFUPDATE=1".to_string()));
        assert!(args.contains(&"/PARENTPID=4242".to_string()));
        assert!(
            args.contains(&r"/RELAUNCH=C:\Apps\Terminal Manager\terminal-manager.exe".to_string())
        );
        assert!(
            args.contains(&r"/LOG=C:\Data\updates\terminal-manager-0.5.0-setup.log".to_string())
        );
        // Interactive-only switches never appear.
        assert!(!args.iter().any(|a| a == "/SILENT" || a == "/SP-"));

        let machine = installer_args(InstallScope::AllUsers, 1, Path::new("x"), Path::new("y"));
        assert!(machine.contains(&"/ALLUSERS".to_string()));
        assert!(!machine.contains(&"/CURRENTUSER".to_string()));
    }

    #[test]
    fn log_sits_next_to_the_installer() {
        let log = log_path_for(Path::new(r"C:\d\updates\terminal-manager-0.5.0-setup.exe"));
        assert_eq!(
            log.to_string_lossy(),
            r"C:\d\updates\terminal-manager-0.5.0-setup.log"
        );
    }

    #[test]
    fn launching_a_missing_installer_fails_cleanly() {
        let missing = std::env::temp_dir().join("tm-update-missing-installer-does-not-exist.exe");
        let error = launch_installer(&missing, &["/VERYSILENT".to_string()]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
