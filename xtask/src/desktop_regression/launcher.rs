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
            kill_new_processes_by_image("unshit-ptyd.exe", &self.support_processes_before);
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
                kill_new_processes_by_image("unshit-ptyd.exe", &self.support_processes_before);
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
        kill_new_processes_by_image("unshit-ptyd.exe", &self.support_processes_before);
        Ok(())
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
            kill_new_processes_by_image("unshit-ptyd.exe", &self.support_processes_before);
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
    stop_processes_that_lock_debug_binaries()?;

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

fn stop_processes_that_lock_debug_binaries() -> Result<(), String> {
    let mut stopped = 0usize;
    for image_name in build_locking_image_names() {
        let pids = process_ids_by_image(image_name);
        if pids.is_empty() {
            continue;
        }

        println!(
            "desktop-regression: stopping {} existing {} process(es) before build",
            pids.len(),
            image_name
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
    if !cfg!(windows) {
        return Vec::new();
    }

    let filter = format!("IMAGENAME eq {image_name}");
    let Ok(output) = Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_tasklist_csv(&String::from_utf8_lossy(&output.stdout))
}

fn kill_new_processes_by_image(image_name: &str, existing: &[u32]) {
    for pid in process_ids_by_image(image_name) {
        if existing.contains(&pid) {
            continue;
        }
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
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

fn parse_tasklist_csv(output: &str) -> Vec<u32> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.contains("No tasks are running") {
                return None;
            }
            let fields = trimmed.trim_matches('"').split("\",\"").collect::<Vec<_>>();
            fields.get(1).and_then(|pid| pid.parse::<u32>().ok())
        })
        .collect()
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
    fn parses_tasklist_csv_process_ids() {
        let output = "\"unshit-ptyd.exe\",\"26372\",\"Console\",\"1\",\"12,340 K\"\r\n";

        assert_eq!(parse_tasklist_csv(output), vec![26372]);
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
