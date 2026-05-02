//! PTY process manager for a terminal emulator.
//!
//! Manages pseudo-terminal sessions mapped to UI pane IDs. Each pane gets its
//! own shell process with independent stdin/stdout and size tracking. Built on
//! top of `portable_pty` (0.8) for cross-platform support (Windows, macOS,
//! Linux).

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// A single PTY session: the child process, a writer for stdin, the master PTY
/// handle (needed for resize), and the current terminal size.
pub struct PtyPair {
    child: Box<dyn Child + Send>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    size: PtySize,
    spawn_cwd: Option<PathBuf>,
}

/// Manages PTY sessions keyed by pane ID.
pub struct PtyManager {
    pairs: HashMap<u32, PtyPair>,
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyManager {
    /// Create an empty manager with no active sessions.
    pub fn new() -> Self {
        Self {
            pairs: HashMap::new(),
        }
    }

    /// Spawn a new shell process for the given pane.
    ///
    /// Returns the reader (stdout) half so the caller can consume output in a
    /// background thread. The writer half is stored internally and accessible
    /// via [`write`](Self::write).
    ///
    /// The shell is chosen from the `SHELL` environment variable. If that is
    /// unset, it falls back to `bash` on Unix or `powershell.exe` on Windows.
    /// The working directory is set to `cwd` if provided, otherwise the user's
    /// home directory.
    pub fn spawn(
        &mut self,
        pane_id: u32,
        cols: u16,
        rows: u16,
    ) -> std::io::Result<Box<dyn Read + Send>> {
        self.spawn_in(pane_id, cols, rows, None)
    }

    /// Like [`spawn`](Self::spawn) but with an explicit working directory.
    pub fn spawn_in(
        &mut self,
        pane_id: u32,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> std::io::Result<Box<dyn Read + Send>> {
        let pty_system = native_pty_system();

        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pty_pair = pty_system.openpty(size).map_err(std::io::Error::other)?;

        let shell = default_shell();

        let mut cmd = CommandBuilder::new(&shell);
        if let Some(dir) = cwd {
            cmd.cwd(dir);
            // PowerShell profiles commonly end with `Set-Location <some-dir>`,
            // which overrides the OS-level cwd we just set. Pass the same dir
            // via `-NoExit -Command "Set-Location ..."` so it runs AFTER the
            // profile and wins.
            if is_powershell_shell(&shell) {
                for arg in build_powershell_cwd_args(dir) {
                    cmd.arg(arg);
                }
            }
        } else if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }

        let child = pty_pair
            .slave
            .spawn_command(cmd)
            .map_err(std::io::Error::other)?;

        let reader = pty_pair
            .master
            .try_clone_reader()
            .map_err(std::io::Error::other)?;

        let writer = pty_pair
            .master
            .take_writer()
            .map_err(std::io::Error::other)?;

        self.pairs.insert(
            pane_id,
            PtyPair {
                child,
                writer,
                master: pty_pair.master,
                size,
                spawn_cwd: cwd.map(Path::to_path_buf),
            },
        );

        Ok(reader)
    }

    /// Return the directory the pane's shell was spawned in, if one was
    /// provided to [`spawn_in`](Self::spawn_in). Intended for tests and
    /// diagnostics; mirrors the `cwd` argument used at spawn time.
    pub fn spawn_cwd(&self, pane_id: u32) -> Option<&Path> {
        self.pairs
            .get(&pane_id)
            .and_then(|p| p.spawn_cwd.as_deref())
    }

    /// Write raw bytes to the PTY stdin for the given pane.
    ///
    /// Returns an error if the pane does not exist or the write fails.
    pub fn write(&mut self, pane_id: u32, data: &[u8]) -> std::io::Result<()> {
        let pair = self.pairs.get_mut(&pane_id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no PTY for pane {pane_id}"),
            )
        })?;
        pair.writer.write_all(data)?;
        pair.writer.flush()
    }

    /// Resize the PTY for the given pane to new column/row dimensions.
    ///
    /// This is a best-effort operation: if the pane does not exist or the
    /// resize call fails, it is silently ignored. This keeps the caller's
    /// resize logic simple.
    pub fn resize(&mut self, pane_id: u32, cols: u16, rows: u16) {
        if let Some(pair) = self.pairs.get_mut(&pane_id) {
            let new_size = PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            };
            if pair.master.resize(new_size).is_ok() {
                pair.size = new_size;
            }
        }
    }

    /// Kill the child process and remove the PTY entry for the given pane.
    ///
    /// Silently ignored if the pane does not exist.
    pub fn destroy(&mut self, pane_id: u32) {
        if let Some(mut pair) = self.pairs.remove(&pane_id) {
            let _ = pair.child.kill();
        }
    }

    /// Kill all child processes and remove every PTY entry.
    pub fn destroy_all(&mut self) {
        let ids: Vec<u32> = self.pairs.keys().copied().collect();
        for id in ids {
            self.destroy(id);
        }
    }

    /// Check whether a PTY session exists for the given pane.
    pub fn has(&self, pane_id: u32) -> bool {
        self.pairs.contains_key(&pane_id)
    }
}

impl Drop for PtyManager {
    fn drop(&mut self) {
        self.destroy_all();
    }
}

impl Drop for PtyPair {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Detect the default shell for the current platform.
///
/// Reads the `SHELL` environment variable first. On Windows, only honors
/// `SHELL` when it points at a native Windows shell (powershell, pwsh,
/// cmd); anything else (including Git Bash's
/// `C:\Program Files\Git\usr\bin\bash.exe` and WSL's `/usr/bin/bash`)
/// falls back to `powershell.exe` because non-native shells in Windows
/// ConPTY are a degraded experience.
pub fn default_shell() -> String {
    pick_default_shell(std::env::var("SHELL").ok().as_deref(), cfg!(windows))
}

fn pick_default_shell(shell_env: Option<&str>, is_windows: bool) -> String {
    if !is_windows {
        return shell_env
            .map(str::to_string)
            .unwrap_or_else(|| "bash".to_string());
    }

    if let Some(shell) = shell_env {
        if is_windows_native_shell(shell) {
            return shell.to_string();
        }
    }

    "powershell.exe".to_string()
}

/// Returns true when `path` points at a shell that runs natively on
/// Windows (powershell, pwsh, cmd). Used by the Windows branch of
/// `pick_default_shell` to whitelist `SHELL` overrides; non-native
/// values like `bash.exe` fall back to PowerShell.
fn is_windows_native_shell(path: &str) -> bool {
    shell_file_stem(path)
        .map(|stem| {
            let lower = stem.to_ascii_lowercase();
            matches!(lower.as_str(), "powershell" | "pwsh" | "cmd")
        })
        .unwrap_or(false)
}

/// Returns true when `shell` points at `powershell` or `pwsh` (with or
/// without a `.exe` suffix, any path prefix, case-insensitive stem).
pub fn is_powershell_shell(shell: &str) -> bool {
    shell_file_stem(shell)
        .map(|stem| stem.eq_ignore_ascii_case("powershell") || stem.eq_ignore_ascii_case("pwsh"))
        .unwrap_or(false)
}

fn shell_file_stem(shell: &str) -> Option<&str> {
    let file_name = shell
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())?;
    let stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name);
    (!stem.is_empty()).then_some(stem)
}

/// Build the `-NoExit -Command "Set-Location ..."` args that force PowerShell
/// into `dir` *after* the user profile has run. Single quotes in the path are
/// doubled to keep the PowerShell single-quoted string well-formed.
pub fn build_powershell_cwd_args(dir: &Path) -> Vec<String> {
    let escaped = dir.to_string_lossy().replace('\'', "''");
    vec![
        "-NoExit".to_string(),
        "-Command".to_string(),
        format!("Set-Location -LiteralPath '{escaped}'"),
    ]
}

/// Compose the full args list for a spawn. User supplied args come
/// first; the PowerShell cwd workaround (if applicable) is appended
/// after so it runs once the user's profile + args have settled. For
/// non PowerShell shells, only the user args are returned.
pub fn build_spawn_args(shell: &str, user_args: &[String], cwd: Option<&Path>) -> Vec<String> {
    let mut args: Vec<String> = user_args.to_vec();
    if let Some(dir) = cwd {
        if is_powershell_shell(shell) {
            args.extend(build_powershell_cwd_args(dir));
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_empty_manager() {
        let mgr = PtyManager::new();
        assert!(!mgr.has(0));
        assert!(!mgr.has(1));
        assert!(!mgr.has(999));
    }

    #[test]
    fn has_returns_false_for_nonexistent_pane() {
        let mgr = PtyManager::new();
        assert!(!mgr.has(42));
    }

    #[test]
    fn write_to_nonexistent_pane_returns_error() {
        let mut mgr = PtyManager::new();
        let result = mgr.write(42, b"hello");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn resize_nonexistent_pane_does_not_panic() {
        let mut mgr = PtyManager::new();
        mgr.resize(42, 120, 40); // should be a silent no-op
    }

    #[test]
    fn destroy_nonexistent_pane_does_not_panic() {
        let mut mgr = PtyManager::new();
        mgr.destroy(42); // should be a silent no-op
    }

    #[test]
    fn spawn_creates_pty_session() {
        let mut mgr = PtyManager::new();
        let pane_id = 10;

        let result = mgr.spawn(pane_id, 80, 24);
        assert!(result.is_ok(), "spawn failed: {:?}", result.err());

        assert!(mgr.has(pane_id));
    }

    #[test]
    fn write_to_spawned_pane_succeeds() {
        let mut mgr = PtyManager::new();
        let pane_id = 20;
        let _reader = mgr.spawn(pane_id, 80, 24).expect("spawn failed");

        let result = mgr.write(pane_id, b"echo hello\n");
        assert!(result.is_ok(), "write failed: {:?}", result.err());

        // Cleanup
        mgr.destroy(pane_id);
    }

    #[test]
    fn resize_spawned_pane_does_not_panic() {
        let mut mgr = PtyManager::new();
        let pane_id = 30;
        let _reader = mgr.spawn(pane_id, 80, 24).expect("spawn failed");

        mgr.resize(pane_id, 120, 40);
        // Verify the session still exists after resize
        assert!(mgr.has(pane_id));

        mgr.destroy(pane_id);
    }

    #[test]
    fn destroy_removes_pty_session() {
        let mut mgr = PtyManager::new();
        let pane_id = 40;
        let _reader = mgr.spawn(pane_id, 80, 24).expect("spawn failed");
        assert!(mgr.has(pane_id));

        mgr.destroy(pane_id);
        assert!(!mgr.has(pane_id));
    }

    #[test]
    fn spawn_multiple_panes() {
        let mut mgr = PtyManager::new();

        let _r1 = mgr.spawn(1, 80, 24).expect("spawn 1 failed");
        let _r2 = mgr.spawn(2, 100, 30).expect("spawn 2 failed");

        assert!(mgr.has(1));
        assert!(mgr.has(2));
        assert!(!mgr.has(3));

        mgr.destroy(1);
        assert!(!mgr.has(1));
        assert!(mgr.has(2));

        mgr.destroy(2);
        assert!(!mgr.has(2));
    }

    #[test]
    fn resize_spawned_pane_updates_size() {
        let mut mgr = PtyManager::new();
        let pane_id = 50;
        let _reader = mgr.spawn(pane_id, 80, 24).expect("spawn failed");

        // Resize to new dimensions
        mgr.resize(pane_id, 120, 40);

        // Verify the pair's size was updated
        let pair = mgr.pairs.get(&pane_id).unwrap();
        assert_eq!(pair.size.cols, 120);
        assert_eq!(pair.size.rows, 40);

        mgr.destroy(pane_id);
    }

    #[test]
    fn default_shell_returns_nonempty_string() {
        let shell = default_shell();
        assert!(!shell.is_empty());
    }

    #[test]
    fn pick_default_shell_honors_shell_env_on_unix() {
        assert_eq!(pick_default_shell(Some("/bin/zsh"), false), "/bin/zsh");
    }

    #[test]
    fn pick_default_shell_falls_back_to_bash_on_unix_when_unset() {
        assert_eq!(pick_default_shell(None, false), "bash");
    }

    #[test]
    fn pick_default_shell_falls_back_to_powershell_on_windows_when_unset() {
        assert_eq!(pick_default_shell(None, true), "powershell.exe");
    }

    #[test]
    fn pick_default_shell_honors_native_windows_shells() {
        assert_eq!(pick_default_shell(Some("cmd.exe"), true), "cmd.exe");
        assert_eq!(
            pick_default_shell(Some("C:\\Windows\\System32\\cmd.exe"), true),
            "C:\\Windows\\System32\\cmd.exe"
        );
        assert_eq!(pick_default_shell(Some("pwsh.exe"), true), "pwsh.exe");
        assert_eq!(
            pick_default_shell(Some("powershell.exe"), true),
            "powershell.exe"
        );
        // Case-insensitive match on the file stem.
        assert_eq!(
            pick_default_shell(Some("PowerShell.EXE"), true),
            "PowerShell.EXE"
        );
    }

    #[test]
    fn pick_default_shell_falls_back_for_non_native_shells_on_windows() {
        // Git Bash sets SHELL to a Windows-style path that ends in bash.exe;
        // WSL sets it to a Unix-style /usr/bin/bash. Both run a non-native
        // shell under ConPTY which is a degraded experience. Fall back to
        // PowerShell so the user gets the integrated Windows shell by
        // default. They can opt back in by setting SHELL=cmd.exe or
        // SHELL=pwsh.exe explicitly.
        assert_eq!(
            pick_default_shell(Some("/usr/bin/bash"), true),
            "powershell.exe"
        );
        assert_eq!(pick_default_shell(Some("/bin/sh"), true), "powershell.exe");
        assert_eq!(
            pick_default_shell(Some("C:\\Program Files\\Git\\usr\\bin\\bash.exe"), true),
            "powershell.exe"
        );
        assert_eq!(pick_default_shell(Some("zsh"), true), "powershell.exe");
        assert_eq!(pick_default_shell(Some("fish.exe"), true), "powershell.exe");
    }

    #[test]
    fn destroy_then_write_returns_error() {
        let mut mgr = PtyManager::new();
        let pane_id = 60;
        let _reader = mgr.spawn(pane_id, 80, 24).expect("spawn failed");
        mgr.destroy(pane_id);
        let result = mgr.write(pane_id, b"test");
        assert!(result.is_err());
    }

    #[test]
    fn destroy_then_resize_is_noop() {
        let mut mgr = PtyManager::new();
        let pane_id = 70;
        let _reader = mgr.spawn(pane_id, 80, 24).expect("spawn failed");
        mgr.destroy(pane_id);
        // Should not panic
        mgr.resize(pane_id, 120, 40);
        assert!(!mgr.has(pane_id));
    }

    #[test]
    fn write_error_message_contains_pane_id() {
        let mut mgr = PtyManager::new();
        let result = mgr.write(999, b"test");
        let err = result.unwrap_err();
        assert!(err.to_string().contains("999"));
    }

    #[test]
    fn multiple_writes_to_same_pane() {
        let mut mgr = PtyManager::new();
        let pane_id = 80;
        let _reader = mgr.spawn(pane_id, 80, 24).expect("spawn failed");
        // Multiple writes should all succeed
        assert!(mgr.write(pane_id, b"echo 1\n").is_ok());
        assert!(mgr.write(pane_id, b"echo 2\n").is_ok());
        assert!(mgr.write(pane_id, b"echo 3\n").is_ok());
        mgr.destroy(pane_id);
    }

    #[test]
    fn is_powershell_shell_detects_known_variants() {
        assert!(is_powershell_shell("powershell.exe"));
        assert!(is_powershell_shell("powershell"));
        assert!(is_powershell_shell("pwsh.exe"));
        assert!(is_powershell_shell("pwsh"));
        assert!(is_powershell_shell("PowerShell.exe"));
        assert!(is_powershell_shell(
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        ));
        assert!(is_powershell_shell("/usr/bin/pwsh"));
    }

    #[test]
    fn is_powershell_shell_rejects_non_powershell_shells() {
        assert!(!is_powershell_shell("bash"));
        assert!(!is_powershell_shell("/bin/bash"));
        assert!(!is_powershell_shell("zsh"));
        assert!(!is_powershell_shell("cmd.exe"));
        assert!(!is_powershell_shell("fish"));
        assert!(!is_powershell_shell(""));
    }

    #[test]
    fn build_powershell_cwd_args_emits_noexit_command_set_location() {
        let args = build_powershell_cwd_args(Path::new(r"C:\Users\alanm\project"));
        assert_eq!(
            args,
            vec![
                "-NoExit".to_string(),
                "-Command".to_string(),
                r"Set-Location -LiteralPath 'C:\Users\alanm\project'".to_string(),
            ]
        );
    }

    #[test]
    fn build_powershell_cwd_args_doubles_single_quotes_in_path() {
        // Regression: a path containing a single quote must be escaped by
        // doubling, otherwise the PowerShell single-quoted string breaks
        // and we either fail to cd or (worse) execute injected code.
        let args = build_powershell_cwd_args(Path::new(r"C:\Users\a'b\proj"));
        assert_eq!(args[2], r"Set-Location -LiteralPath 'C:\Users\a''b\proj'");
    }

    // refs #140: build_spawn_args is the central place that combines a
    // user supplied args list with the daemon side PowerShell cwd
    // workaround. Order matters: user args must come first, the cwd
    // workaround must be appended after so it runs once the user's
    // profile / args have settled.

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn build_spawn_args_passes_user_args_through_for_non_powershell() {
        let args = build_spawn_args("bash", &s(&["--login", "-i"]), None);
        assert_eq!(args, s(&["--login", "-i"]));
    }

    #[test]
    fn build_spawn_args_omits_powershell_cwd_args_for_non_powershell_shell() {
        let args = build_spawn_args("bash", &s(&[]), Some(Path::new("/tmp")));
        assert!(
            args.is_empty(),
            "non PowerShell shells must not get cwd workaround args, got {args:?}"
        );
    }

    #[test]
    fn build_spawn_args_skips_powershell_cwd_args_when_no_cwd_given() {
        let args = build_spawn_args("pwsh.exe", &s(&["-NoLogo"]), None);
        assert_eq!(
            args,
            s(&["-NoLogo"]),
            "no cwd means no Set-Location workaround; user args pass through unchanged"
        );
    }

    #[test]
    fn build_spawn_args_appends_powershell_cwd_args_after_user_args() {
        let args = build_spawn_args(
            "pwsh.exe",
            &s(&["-NoLogo"]),
            Some(Path::new(r"C:\Users\alanm\project")),
        );
        assert_eq!(
            args,
            s(&[
                "-NoLogo",
                "-NoExit",
                "-Command",
                r"Set-Location -LiteralPath 'C:\Users\alanm\project'",
            ]),
            "PowerShell cwd workaround must come AFTER user args so the user's \
             profile + args run first and the workaround takes effect last"
        );
    }
}
