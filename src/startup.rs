//! Per-user Windows login startup registration.
//!
//! The Windows registry is the durable source of truth. Workspace state only
//! mirrors the current status for rendering, so toggling this preference can
//! never silently diverge from the operating system.

#[cfg(windows)]
use std::path::Path;

#[cfg(windows)]
const MAX_RUN_COMMAND_UTF16_UNITS: usize = 260;
#[cfg(any(windows, test))]
const DEFAULT_VALUE_NAME: &str = "Unshit Terminal Manager";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistrationStatus {
    Disabled,
    Enabled,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupErrorKind {
    CurrentExecutable,
    InvalidExecutable,
    CommandTooLong,
    UnsupportedProfile,
    RegistryRead,
    RegistryWrite,
    RegistryDelete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupError {
    kind: StartupErrorKind,
    os_code: Option<u32>,
}

impl StartupError {
    pub(crate) fn new(kind: StartupErrorKind) -> Self {
        Self {
            kind,
            os_code: None,
        }
    }

    #[cfg(windows)]
    fn from_windows(kind: StartupErrorKind, os_code: u32) -> Self {
        Self {
            kind,
            os_code: Some(os_code),
        }
    }

    pub fn kind(self) -> StartupErrorKind {
        self.kind
    }

    pub fn os_code(self) -> Option<u32> {
        self.os_code
    }

    /// Allowlisted telemetry classification. Raw executable paths, registry
    /// data, and localized OS messages never leave this module.
    pub fn telemetry_kind(self) -> &'static str {
        let denied = self.os_code == Some(5);
        match (self.kind, denied) {
            (StartupErrorKind::CurrentExecutable, _) => "current_executable",
            (StartupErrorKind::InvalidExecutable, _) => "invalid_executable",
            (StartupErrorKind::CommandTooLong, _) => "command_too_long",
            (StartupErrorKind::UnsupportedProfile, _) => "unsupported_profile",
            (StartupErrorKind::RegistryRead, true) => "registry_read_access_denied",
            (StartupErrorKind::RegistryRead, false) => "registry_read_failed",
            (StartupErrorKind::RegistryWrite, true) => "registry_write_access_denied",
            (StartupErrorKind::RegistryWrite, false) => "registry_write_failed",
            (StartupErrorKind::RegistryDelete, true) => "registry_delete_access_denied",
            (StartupErrorKind::RegistryDelete, false) => "registry_delete_failed",
        }
    }
}

#[cfg(any(windows, test))]
fn value_name(profile: Option<&str>) -> String {
    match profile {
        Some(tag) => format!("{DEFAULT_VALUE_NAME} [{tag}]"),
        None => DEFAULT_VALUE_NAME.to_string(),
    }
}

fn profile_is_supported(profile: Option<&str>, env_override_present: bool) -> bool {
    !env_override_present && (profile.is_none() || profile == Some("dev"))
}

fn profile_override_present() -> bool {
    std::env::var_os(crate::profile::ENV_PROFILE).is_some()
}

/// Login startup is a Windows-only preference. The default installed profile
/// and the auto-detected dev profile are supported. Processes with any
/// `TM_PROFILE` override are excluded because a process-local environment
/// override is not guaranteed to exist at the next Windows sign-in.
pub fn is_supported() -> bool {
    cfg!(windows)
        && profile_is_supported(crate::profile::active_profile(), profile_override_present())
}

#[cfg(any(windows, test))]
fn classify_stored_command(expected: &[u16], stored: Option<&[u16]>) -> RegistrationStatus {
    let Some(stored) = stored else {
        return RegistrationStatus::Disabled;
    };
    let expected = expected.strip_suffix(&[0]).unwrap_or(expected);
    let stored = stored.strip_suffix(&[0]).unwrap_or(stored);
    if stored == expected {
        RegistrationStatus::Enabled
    } else {
        RegistrationStatus::Stale
    }
}

#[cfg(windows)]
fn command_for_executable(path: &Path) -> Result<Vec<u16>, StartupError> {
    use std::os::windows::ffi::OsStrExt;

    if !path.is_absolute() {
        return Err(StartupError::new(StartupErrorKind::InvalidExecutable));
    }
    let executable: Vec<u16> = path.as_os_str().encode_wide().collect();
    if executable.is_empty() || executable.iter().any(|unit| matches!(unit, 0 | 34)) {
        return Err(StartupError::new(StartupErrorKind::InvalidExecutable));
    }

    let mut command = Vec::with_capacity(executable.len().saturating_add(3));
    command.push(b'"' as u16);
    command.extend(executable);
    command.push(b'"' as u16);
    if command.len() > MAX_RUN_COMMAND_UTF16_UNITS {
        return Err(StartupError::new(StartupErrorKind::CommandTooLong));
    }
    command.push(0);
    Ok(command)
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn read_registration(
    value_name: &str,
    expected_command: &[u16],
) -> Result<RegistrationStatus, StartupError> {
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
        ERROR_UNSUPPORTED_TYPE,
    };
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RRF_ZEROONFAILURE,
    };

    const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let subkey = wide_null(RUN_SUBKEY);
    let value_name = wide_null(value_name);
    // The documented Run command limit is 260 UTF-16 units. The extra slot is
    // exclusively for REG_SZ's terminating NUL; registry-controlled sizes are
    // never used for allocation.
    let mut stored = [0u16; MAX_RUN_COMMAND_UTF16_UNITS + 1];
    let mut stored_type = 0;
    let mut byte_len =
        u32::try_from(std::mem::size_of_val(&stored)).expect("fixed startup buffer fits in u32");
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ | RRF_ZEROONFAILURE,
            &mut stored_type,
            stored.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    match status {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(RegistrationStatus::Disabled),
        ERROR_MORE_DATA | ERROR_UNSUPPORTED_TYPE => Ok(RegistrationStatus::Stale),
        ERROR_SUCCESS => {
            let byte_len = byte_len as usize;
            if stored_type != REG_SZ
                || byte_len == 0
                || byte_len % std::mem::size_of::<u16>() != 0
                || byte_len > std::mem::size_of_val(&stored)
            {
                return Ok(RegistrationStatus::Stale);
            }
            let units = byte_len / std::mem::size_of::<u16>();
            Ok(classify_stored_command(
                expected_command,
                Some(&stored[..units]),
            ))
        }
        code => Err(StartupError::from_windows(
            StartupErrorKind::RegistryRead,
            code,
        )),
    }
}

#[cfg(windows)]
fn write_registration(value_name: &str, command: &[u16]) -> Result<(), StartupError> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};

    const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let subkey = wide_null(RUN_SUBKEY);
    let value_name = wide_null(value_name);
    let byte_len = command
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|len| u32::try_from(len).ok())
        .ok_or_else(|| StartupError::new(StartupErrorKind::CommandTooLong))?;
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            REG_SZ,
            command.as_ptr().cast(),
            byte_len,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(StartupError::from_windows(
            StartupErrorKind::RegistryWrite,
            status,
        ))
    }
}

#[cfg(windows)]
fn delete_registration(value_name: &str) -> Result<(), StartupError> {
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, HKEY_CURRENT_USER};

    const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let subkey = wide_null(RUN_SUBKEY);
    let value_name = wide_null(value_name);
    let status =
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), value_name.as_ptr()) };
    match status {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(()),
        code => Err(StartupError::from_windows(
            StartupErrorKind::RegistryDelete,
            code,
        )),
    }
}

/// Read the current process profile's owned login startup entry.
pub fn status() -> Result<RegistrationStatus, StartupError> {
    let profile = crate::profile::active_profile();
    if !profile_is_supported(profile, profile_override_present()) {
        return Err(StartupError::new(StartupErrorKind::UnsupportedProfile));
    }

    #[cfg(windows)]
    {
        let executable = std::env::current_exe()
            .map_err(|_| StartupError::new(StartupErrorKind::CurrentExecutable))?;
        let command = command_for_executable(&executable)?;
        read_registration(&value_name(profile), &command)
    }
    #[cfg(not(windows))]
    {
        Err(StartupError::new(StartupErrorKind::UnsupportedProfile))
    }
}

/// Atomically update the current process profile's owned login startup entry.
/// Disabling is idempotent and does not need to resolve the executable path.
pub fn set_enabled(enabled: bool) -> Result<(), StartupError> {
    let profile = crate::profile::active_profile();
    if !profile_is_supported(profile, profile_override_present()) {
        return Err(StartupError::new(StartupErrorKind::UnsupportedProfile));
    }

    #[cfg(windows)]
    {
        let name = value_name(profile);
        if enabled {
            let executable = std::env::current_exe()
                .map_err(|_| StartupError::new(StartupErrorKind::CurrentExecutable))?;
            let command = command_for_executable(&executable)?;
            write_registration(&name, &command)
        } else {
            delete_registration(&name)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err(StartupError::new(StartupErrorKind::UnsupportedProfile))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_dev_registrations_have_separate_owned_value_names() {
        assert_eq!(value_name(None), "Unshit Terminal Manager");
        assert_eq!(value_name(Some("dev")), "Unshit Terminal Manager [dev]");
    }

    #[test]
    fn only_auto_resolved_default_and_dev_profiles_are_registered_at_login() {
        assert!(profile_is_supported(None, false));
        assert!(profile_is_supported(Some("dev"), false));
        assert!(!profile_is_supported(Some("test-run"), false));
        assert!(!profile_is_supported(None, true));
        assert!(!profile_is_supported(Some("dev"), true));
    }

    #[cfg(windows)]
    #[test]
    fn startup_command_quotes_the_absolute_executable_and_terminates_reg_sz() {
        use std::os::windows::ffi::OsStringExt;

        let command = command_for_executable(Path::new(
            r"C:\Program Files\Terminal Manager\terminal-manager.exe",
        ))
        .expect("valid command");
        let decoded = std::ffi::OsString::from_wide(&command[..command.len() - 1]);

        assert_eq!(
            decoded,
            r#""C:\Program Files\Terminal Manager\terminal-manager.exe""#
        );
        assert_eq!(command.last(), Some(&0));
    }

    #[cfg(windows)]
    #[test]
    fn startup_command_rejects_relative_quoted_and_overlong_paths() {
        assert_eq!(
            command_for_executable(Path::new("terminal-manager.exe"))
                .expect_err("relative path")
                .kind(),
            StartupErrorKind::InvalidExecutable
        );
        assert_eq!(
            command_for_executable(Path::new(r#"C:\bad"name\terminal-manager.exe"#))
                .expect_err("embedded quote")
                .kind(),
            StartupErrorKind::InvalidExecutable
        );

        let oversized = format!(r"C:\{}\terminal-manager.exe", "a".repeat(260));
        assert_eq!(
            command_for_executable(Path::new(&oversized))
                .expect_err("overlong command")
                .kind(),
            StartupErrorKind::CommandTooLong
        );
    }

    #[cfg(windows)]
    #[test]
    fn startup_command_limit_excludes_the_reg_sz_terminator() {
        let at_limit = format!(r"C:\{}", "a".repeat(255));
        let command = command_for_executable(Path::new(&at_limit)).expect("260-unit command");
        assert_eq!(command.len(), MAX_RUN_COMMAND_UTF16_UNITS + 1);
        assert_eq!(command.last(), Some(&0));

        let over_limit = format!(r"C:\{}", "a".repeat(256));
        assert_eq!(
            command_for_executable(Path::new(&over_limit))
                .expect_err("261-unit command")
                .kind(),
            StartupErrorKind::CommandTooLong
        );
    }

    #[test]
    fn command_status_distinguishes_missing_exact_and_stale_values() {
        let expected = [b'"' as u16, b'C' as u16, b'"' as u16, 0];
        let other = [b'"' as u16, b'D' as u16, b'"' as u16, 0];

        assert_eq!(
            classify_stored_command(&expected, None),
            RegistrationStatus::Disabled
        );
        assert_eq!(
            classify_stored_command(&expected, Some(&expected)),
            RegistrationStatus::Enabled
        );
        assert_eq!(
            classify_stored_command(&expected, Some(&other)),
            RegistrationStatus::Stale
        );
    }

    #[test]
    fn installers_remove_exactly_the_owned_default_run_value() {
        const DEFINITION: &str = "#define MyStartupValueName \"Unshit Terminal Manager\"";
        const EXPECTED: &str = "Root: HKCU; Subkey: \"Software\\Microsoft\\Windows\\CurrentVersion\\Run\"; ValueType: none; ValueName: \"{#MyStartupValueName}\"; Flags: dontcreatekey uninsdeletevalue";
        let standard = include_str!("../packaging/terminal-manager.iss");
        let non_gpu = include_str!("../packaging/terminal-manager-non-gpu.iss");

        assert_eq!(DEFAULT_VALUE_NAME, "Unshit Terminal Manager");
        assert!(standard.contains(DEFINITION));
        assert!(standard.contains(EXPECTED));
        assert!(non_gpu.contains(DEFINITION));
        assert!(non_gpu.contains(EXPECTED));
    }
}
