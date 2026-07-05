use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::desktop_regression::diagnostics::DiagnosticLaunchConfig;
use crate::desktop_regression::win32::{self, WindowHandle};

pub struct AppLogFiles {
    pub stdout_name: String,
    pub stderr_name: String,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl AppLogFiles {
    pub fn create(run_dir: &Path, suite_id: &str) -> Result<Self, String> {
        let stdout_name = crate::desktop_regression::artifacts::suite_artifact_name(
            suite_id,
            "app.stdout",
            "log",
        );
        let stderr_name = crate::desktop_regression::artifacts::suite_artifact_name(
            suite_id,
            "app.stderr",
            "log",
        );
        let stdout_path = run_dir.join(&stdout_name);
        let stderr_path = run_dir.join(&stderr_name);
        File::create(&stdout_path).map_err(|e| {
            format!(
                "failed to create app stdout log {}: {e}",
                stdout_path.display()
            )
        })?;
        File::create(&stderr_path).map_err(|e| {
            format!(
                "failed to create app stderr log {}: {e}",
                stderr_path.display()
            )
        })?;

        Ok(Self {
            stdout_name,
            stderr_name,
            stdout_path,
            stderr_path,
        })
    }

    pub fn artifact_names(&self) -> [String; 2] {
        [self.stdout_name.clone(), self.stderr_name.clone()]
    }
}

pub struct AppSession {
    child: Child,
    window: WindowHandle,
    support_processes_before: Vec<u32>,
    /// Directory the spawned daemon binary lives in (sibling of the app
    /// exe). Cleanup only ever kills daemons running from here so an
    /// installed app's daemon — or a dev instance launched mid-run —
    /// is never collateral damage.
    daemon_dir: Option<PathBuf>,
    /// Ephemeral per-session config dir (TM_CONFIG_DIR), removed on close.
    config_dir: Option<PathBuf>,
}

impl AppSession {
    pub fn launch_with_logs(
        exe_path: &Path,
        workspace_root: &Path,
        logs: Option<&AppLogFiles>,
        diagnostics: Option<&DiagnosticLaunchConfig>,
    ) -> Result<Self, String> {
        Self::launch_with_logs_and_env(
            exe_path,
            workspace_root,
            logs,
            diagnostics,
            &BTreeMap::new(),
        )
    }

    pub fn launch_with_logs_and_env(
        exe_path: &Path,
        workspace_root: &Path,
        logs: Option<&AppLogFiles>,
        diagnostics: Option<&DiagnosticLaunchConfig>,
        extra_env: &BTreeMap<&str, String>,
    ) -> Result<Self, String> {
        if !exe_path.is_file() {
            return Err(format!("missing built binary: {}", exe_path.display()));
        }

        let support_processes_before = process_ids_by_image("unshit-ptyd.exe");
        let mut command = Command::new(exe_path);
        command.current_dir(workspace_root);
        // Isolate every test launch in its own instance profile: unique
        // daemon pipe, unique notify pipe, throwaway config dir. A test
        // run must never attach to (or persist into) the user's real
        // session. Callers can still override via extra_env below.
        let isolation = SessionIsolation::fresh();
        command.env("TM_PROFILE", &isolation.profile);
        command.env("TM_CONFIG_DIR", &isolation.config_dir);
        apply_diagnostics_env(&mut command, diagnostics);
        for (key, value) in extra_env {
            command.env(key, value);
        }
        if let Some(logs) = logs {
            let stdout = File::create(&logs.stdout_path).map_err(|e| {
                format!(
                    "failed to open app stdout log {}: {e}",
                    logs.stdout_path.display()
                )
            })?;
            let stderr = File::create(&logs.stderr_path).map_err(|e| {
                format!(
                    "failed to open app stderr log {}: {e}",
                    logs.stderr_path.display()
                )
            })?;
            command
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr));
        }
        let child = command
            .spawn()
            .map_err(|e| format!("failed to launch {}: {e}", exe_path.display()))?;
        let pid = child.id();
        let window = match win32::find_window_for_process(
            pid,
            Duration::from_secs(10),
            &["terminal manager", "terminal.mgr"],
            &["terminal"],
        ) {
            Ok(window) => window,
            Err(err) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
        };

        thread::sleep(Duration::from_millis(500));
        Ok(Self {
            child,
            window,
            support_processes_before,
            daemon_dir: exe_path.parent().map(Path::to_path_buf),
            config_dir: Some(isolation.config_dir),
        })
    }

    pub fn window(&self) -> WindowHandle {
        self.window
    }

    pub fn process_id(&self) -> u32 {
        self.child.id()
    }

    pub fn close_now(&mut self) -> Result<(), String> {
        if self
            .child
            .try_wait()
            .map_err(|e| format!("failed to query app process {}: {e}", self.child.id()))?
            .is_some()
        {
            self.cleanup_session_leftovers();
            return Ok(());
        }

        let _ = win32::close_window(self.window);
        let deadline = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < deadline {
            if self
                .child
                .try_wait()
                .map_err(|e| format!("failed to query app process {}: {e}", self.child.id()))?
                .is_some()
            {
                self.cleanup_session_leftovers();
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }

        self.child
            .kill()
            .map_err(|e| format!("failed to kill app process {}: {e}", self.child.id()))?;
        self.child
            .wait()
            .map_err(|e| format!("failed to wait for app process {}: {e}", self.child.id()))?;
        self.cleanup_session_leftovers();
        Ok(())
    }

    /// Kill the daemon this session spawned (and nothing else), then
    /// drop its throwaway config dir.
    fn cleanup_session_leftovers(&self) {
        kill_new_session_daemons(
            &self.support_processes_before,
            self.daemon_dir.as_deref(),
        );
        if let Some(dir) = &self.config_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Per-session isolation values handed to the app via env.
struct SessionIsolation {
    profile: String,
    config_dir: PathBuf,
}

impl SessionIsolation {
    fn fresh() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let profile = format!("reg{}x{n}", std::process::id());
        let config_dir = std::env::temp_dir()
            .join("tm-desktop-regression")
            .join(&profile);
        Self {
            profile,
            config_dir,
        }
    }
}

pub fn apply_diagnostics_env(command: &mut Command, diagnostics: Option<&DiagnosticLaunchConfig>) {
    if let Some(diagnostics) = diagnostics {
        for (key, value) in diagnostics.env_vars() {
            command.env(key, value);
        }
    }
}

impl Drop for AppSession {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            self.cleanup_session_leftovers();
            return;
        }

        let _ = self.close_now();
    }
}

pub fn prepare_app_binary(
    workspace_root: &Path,
    skip_build: bool,
    exe_path: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(exe_path) = exe_path {
        return Ok(workspace_root.join(exe_path));
    }

    if !skip_build {
        build_app(workspace_root)?;
    }

    Ok(workspace_root
        .join("target")
        .join("debug")
        .join(platform_binary_name()))
}

fn build_app(workspace_root: &Path) -> Result<(), String> {
    stop_processes_that_lock_debug_binaries(workspace_root)?;

    let cargo = cargo_program();
    run_cargo_build(
        &cargo,
        workspace_root,
        &[
            "build",
            "-p",
            "terminal-manager",
            "--bin",
            "terminal-manager",
        ],
        "terminal-manager",
    )?;
    run_cargo_build(
        &cargo,
        workspace_root,
        &["build", "-p", "unshit-ptyd", "--bin", "unshit-ptyd"],
        "unshit-ptyd",
    )?;
    Ok(())
}

/// The build below writes `target\debug`, so only processes running
/// *from that directory* can lock its output. Never touch the installed
/// app or a dev instance running from `target\release`.
fn stop_processes_that_lock_debug_binaries(workspace_root: &Path) -> Result<(), String> {
    let debug_dir = workspace_root.join("target").join("debug");
    let mut stopped = 0usize;
    for image_name in build_locking_image_names() {
        let pids: Vec<u32> = processes_by_image(image_name)
            .into_iter()
            .filter(|entry| {
                entry
                    .exe
                    .as_deref()
                    .is_some_and(|exe| path_is_under(exe, &debug_dir))
            })
            .map(|entry| entry.pid)
            .collect();
        if pids.is_empty() {
            continue;
        }

        println!(
            "desktop-regression: stopping {} {} process(es) running from {} before build",
            pids.len(),
            image_name,
            debug_dir.display()
        );
        for pid in pids {
            if kill_process_by_id(pid, image_name)? {
                stopped += 1;
            }
        }
    }

    if stopped > 0 {
        thread::sleep(Duration::from_millis(500));
    }

    Ok(())
}

fn build_locking_image_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["terminal-manager.exe", "unshit-ptyd.exe"]
    } else {
        &[]
    }
}

fn run_cargo_build(
    cargo: &Path,
    workspace_root: &Path,
    args: &[&str],
    label: &str,
) -> Result<(), String> {
    let status = Command::new(cargo)
        .args(args)
        .current_dir(workspace_root)
        .status()
        .map_err(|e| {
            format!(
                "failed to start {label} build with {}: {e}",
                cargo.display()
            )
        })?;
    if !status.success() {
        return Err(format!("{label} build failed with status {status}"));
    }

    Ok(())
}

fn cargo_program() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO").map(PathBuf::from) {
        if path.is_file() {
            return path;
        }
    }

    if cfg!(windows) {
        if let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            let stable = profile
                .join(".rustup")
                .join("toolchains")
                .join("stable-x86_64-pc-windows-msvc")
                .join("bin")
                .join("cargo.exe");
            if stable.is_file() {
                return stable;
            }
        }
    }

    PathBuf::from("cargo")
}

fn platform_binary_name() -> &'static str {
    if cfg!(windows) {
        "terminal-manager.exe"
    } else {
        "terminal-manager"
    }
}

fn process_ids_by_image(image_name: &str) -> Vec<u32> {
    processes_by_image(image_name)
        .into_iter()
        .map(|p| p.pid)
        .collect()
}

struct ProcessEntry {
    pid: u32,
    exe: Option<PathBuf>,
}

/// List processes by image name with their executable paths so kill
/// logic can filter by *where a process runs from* instead of only its
/// name. An installed Terminal Manager shares the image name with repo
/// builds; the path is the only reliable discriminator.
fn processes_by_image(image_name: &str) -> Vec<ProcessEntry> {
    if !cfg!(windows) {
        return Vec::new();
    }

    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name='{image_name}'\" | \
         ForEach-Object {{ \"$($_.ProcessId)`t$($_.ExecutablePath)\" }}"
    );
    let Ok(output) = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_pid_exe_lines(&String::from_utf8_lossy(&output.stdout))
}

fn parse_pid_exe_lines(output: &str) -> Vec<ProcessEntry> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (pid, exe) = match line.split_once('\t') {
                Some((pid, exe)) => (pid, exe.trim()),
                None => (line, ""),
            };
            let pid = pid.trim().parse::<u32>().ok()?;
            Some(ProcessEntry {
                pid,
                exe: (!exe.is_empty()).then(|| PathBuf::from(exe)),
            })
        })
        .collect()
}

/// Windows paths compare case-insensitively; both sides come in as
/// absolute paths.
fn path_is_under(path: &Path, root: &Path) -> bool {
    let normalize =
        |p: &Path| p.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    let p = normalize(path);
    let r = normalize(root);
    let r = r.trim_end_matches('\\');
    p == r || p.starts_with(&format!("{r}\\"))
}

/// Kill daemons that appeared during a test session, but only those
/// running from the session's own binary dir. A daemon with an unknown
/// or foreign path (the installed app, a dev instance the user opened
/// mid-run) is always spared.
fn kill_new_session_daemons(existing: &[u32], daemon_dir: Option<&Path>) {
    for entry in processes_by_image("unshit-ptyd.exe") {
        if existing.contains(&entry.pid) {
            continue;
        }
        let from_session_dir = match (&entry.exe, daemon_dir) {
            (Some(exe), Some(dir)) => exe.parent().is_some_and(|p| path_is_under(p, dir)),
            _ => false,
        };
        if !from_session_dir {
            continue;
        }
        let _ = Command::new("taskkill")
            .args(["/PID", &entry.pid.to_string(), "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn kill_process_by_id(pid: u32, image_name: &str) -> Result<bool, String> {
    let pid_string = pid.to_string();
    let output = Command::new("taskkill")
        .args(["/PID", &pid_string, "/F"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to start taskkill for {image_name} pid={pid}: {e}"))?;

    if output.status.success() {
        return Ok(true);
    }

    if !process_ids_by_image(image_name).contains(&pid) {
        return Ok(false);
    }

    Err(format!(
        "failed to stop existing {image_name} pid={pid} before build ({}); close it and retry",
        command_failure_details(&output.stdout, &output.stderr, output.status)
    ))
}

fn command_failure_details(
    stdout: &[u8],
    stderr: &[u8],
    status: std::process::ExitStatus,
) -> String {
    command_failure_details_text(stdout, stderr, &status.to_string())
}

fn command_failure_details_text(stdout: &[u8], stderr: &[u8], status: &str) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_owned();
    if !stderr.is_empty() {
        return format!("status {status}: {stderr}");
    }

    let stdout = String::from_utf8_lossy(stdout).trim().to_owned();
    if !stdout.is_empty() {
        return format!("status {status}: {stdout}");
    }

    format!("status {status}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_regression::diagnostics::{
        diagnostic_launch_for_mode, ENV_DIAGNOSTICS_ENABLE, ENV_DIAGNOSTICS_PIPE_NAME,
        ENV_DIAGNOSTICS_TOKEN,
    };
    use terminal_manager_diagnostics::ObserveMode;

    #[test]
    fn prepare_app_binary_uses_explicit_path_when_skip_building() {
        let root = Path::new("C:/repo");
        let path = prepare_app_binary(
            root,
            true,
            Some(Path::new("target/debug/terminal-manager.exe")),
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from("C:/repo").join("target/debug/terminal-manager.exe")
        );
    }

    #[test]
    fn parses_pid_exe_lines_with_and_without_paths() {
        let output =
            "26372\tC:\\repo\\target\\debug\\unshit-ptyd.exe\r\n999\t\r\n1234\r\n\r\n";

        let entries = parse_pid_exe_lines(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].pid, 26372);
        assert_eq!(
            entries[0].exe.as_deref(),
            Some(Path::new(r"C:\repo\target\debug\unshit-ptyd.exe"))
        );
        assert_eq!(entries[1].pid, 999);
        assert!(entries[1].exe.is_none(), "empty path must map to None");
        assert_eq!(entries[2].pid, 1234);
        assert!(entries[2].exe.is_none());
    }

    #[test]
    fn path_is_under_is_case_insensitive_and_boundary_safe() {
        assert!(path_is_under(
            Path::new(r"C:\Repo\Target\Debug\unshit-ptyd.exe"),
            Path::new(r"c:\repo\target\debug")
        ));
        assert!(!path_is_under(
            Path::new(r"C:\repo\target\debug-other\unshit-ptyd.exe"),
            Path::new(r"C:\repo\target\debug")
        ));
        assert!(!path_is_under(
            Path::new(r"C:\Users\a\AppData\Local\Programs\Terminal Manager\unshit-ptyd.exe"),
            Path::new(r"C:\repo\target\debug")
        ));
    }

    #[test]
    fn build_cleanup_targets_windows_app_and_daemon_binaries() {
        if cfg!(windows) {
            assert_eq!(
                build_locking_image_names(),
                &["terminal-manager.exe", "unshit-ptyd.exe"]
            );
        } else {
            assert!(build_locking_image_names().is_empty());
        }
    }

    #[test]
    fn command_failure_details_prefers_stderr() {
        let details = command_failure_details_text(b"stdout text", b"stderr text", "exit 7");

        assert!(details.contains("status"));
        assert!(details.contains("stderr text"));
        assert!(!details.contains("stdout text"));
    }

    #[test]
    fn app_log_files_use_suite_artifact_names() {
        let dir = std::env::temp_dir().join(format!("xtask-dr-app-logs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let logs = AppLogFiles::create(&dir, "edge-resize-stability").unwrap();

        assert_eq!(
            logs.artifact_names(),
            [
                "edge-resize-stability-app.stdout.log".to_owned(),
                "edge-resize-stability-app.stderr.log".to_owned()
            ]
        );
        assert!(dir.join(&logs.stdout_name).is_file());
        assert!(dir.join(&logs.stderr_name).is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn diagnostic_env_is_absent_for_off_launches() {
        let mut command = Command::new("terminal-manager.exe");

        apply_diagnostics_env(&mut command, None);

        let envs = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.map(|raw| raw.to_string_lossy().to_string()),
                )
            })
            .collect::<Vec<_>>();
        assert!(!envs
            .iter()
            .any(|(key, _)| key.starts_with("TM_DIAGNOSTICS_")));
    }

    #[test]
    fn diagnostic_env_is_set_for_observed_launches() {
        let launch = diagnostic_launch_for_mode(ObserveMode::Basic, "run-1", "edge").unwrap();
        let mut command = Command::new("terminal-manager.exe");

        apply_diagnostics_env(&mut command, Some(&launch));

        let envs = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_string(),
                    value.map(|raw| raw.to_string_lossy().to_string()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            envs.get(ENV_DIAGNOSTICS_ENABLE),
            Some(&Some("1".to_owned()))
        );
        assert_eq!(
            envs.get(ENV_DIAGNOSTICS_PIPE_NAME),
            Some(&Some(launch.pipe_name))
        );
        assert_eq!(envs.get(ENV_DIAGNOSTICS_TOKEN), Some(&Some(launch.token)));
    }
}
