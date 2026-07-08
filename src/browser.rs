//! Opening links from the terminal in the user's default browser.
//!
//! Terminal output is untrusted (it can come from a remote SSH session or any
//! program), so a Ctrl+clicked "URL" is attacker-influenced data. Two rules
//! keep that safe:
//!
//! 1. Only `http://` and `https://` are ever opened, so a click can never
//!    launch an arbitrary protocol handler (`file:`, `vscode:`, a custom
//!    scheme) with hostile arguments.
//! 2. The URL is handed to the OS via the shell **association** API
//!    (`ShellExecuteW` on Windows), never through a command interpreter, so URL
//!    metacharacters such as `&` cannot be reinterpreted as shell syntax.

/// Reject anything we are not willing to launch. Enforced independently of the
/// terminal-side detector so this stays safe even if a caller passes an
/// unvetted string.
fn validate(url: &str) -> std::io::Result<()> {
    let ok = url.len() <= 4096
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && (url.starts_with("http://") || url.starts_with("https://"));
    if ok {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to open non-http(s) or malformed URL",
        ))
    }
}

/// Open `url` in the default browser. Returns an error for a rejected URL or if
/// the OS fails to resolve a handler.
pub fn open_url(url: &str) -> std::io::Result<()> {
    validate(url)?;
    open_validated(url)
}

#[cfg(windows)]
fn open_validated(url: &str) -> std::io::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::shellapi::ShellExecuteW;

    // `SW_SHOWNORMAL`; hardcoded to avoid pulling in the `winuser` feature.
    const SW_SHOWNORMAL: i32 = 1;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let verb = wide("open");
    let file = wide(url);
    // SAFETY: `verb`/`file` are NUL-terminated UTF-16 buffers kept alive across
    // the call; every other pointer argument is intentionally null. The "open"
    // verb performs a file-association lookup and never tokenizes `file`, so the
    // URL is passed to the browser as a single opaque argument.
    let hinst = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns a value <= 32 on failure (it is a legacy HINSTANCE
    // that doubles as an error code).
    if hinst as isize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "ShellExecuteW failed (code {})",
            hinst as isize
        )))
    }
}

#[cfg(not(windows))]
fn open_validated(url: &str) -> std::io::Result<()> {
    // Non-Windows is not a shipped target; best-effort via `xdg-open` so the
    // feature is still exercisable on a Linux dev box. `xdg-open` receives the
    // URL as a single argv entry, so there is no shell to interpret it.
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn accepts_http_and_https() {
        assert!(validate("http://example.com").is_ok());
        assert!(validate("https://example.com/a?b=c&d=e#f").is_ok());
    }

    #[test]
    fn rejects_other_schemes() {
        assert!(validate("file:///etc/passwd").is_err());
        assert!(validate("javascript:alert(1)").is_err());
        assert!(validate("vscode://foo").is_err());
        assert!(validate("ftp://host/x").is_err());
    }

    #[test]
    fn rejects_whitespace_and_control() {
        assert!(validate("http://ex.com/ a").is_err());
        assert!(validate("http://ex.com/\n").is_err());
        assert!(validate("http://ex.com/\t").is_err());
    }

    #[test]
    fn rejects_absurdly_long() {
        let long = format!("http://{}", "a".repeat(5000));
        assert!(validate(&long).is_err());
    }
}
