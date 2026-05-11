//! `cargo xtask` entry point.
//!
//! Exposes the `profile` subcommand which replaces the six
//! `scripts/profile-{cpu,memory,all}.{sh,ps1}` helpers with a single
//! cross-platform Rust binary.
//!
//! Usage:
//! ```text
//! cargo xtask profile cpu    [--out-dir target/profile] [--rate 1000]
//! cargo xtask profile memory [--out-dir target/profile]
//! cargo xtask profile all    [--out-dir target/profile] [--rate 1000]
//! ```

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod args;
mod desktop_regression;
mod profile;

fn main() -> ExitCode {
    let raw: Vec<OsString> = env::args_os().skip(1).collect();
    match args::Cli::parse(&raw) {
        Ok(args::Cli::DesktopRegression(opts)) => match desktop_regression::run(&opts) {
            Ok(desktop_regression::RunOutcome::Success) => ExitCode::SUCCESS,
            Ok(desktop_regression::RunOutcome::Failed) => ExitCode::from(1),
            Err(e) => {
                eprintln!("xtask: {e}");
                ExitCode::from(1)
            }
        },
        Ok(args::Cli::Profile(opts)) => match profile::run(&opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("xtask: {e}");
                ExitCode::from(1)
            }
        },
        Ok(args::Cli::Help) => {
            print_usage();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("xtask: {e}");
            eprintln!();
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn print_usage() {
    println!("cargo xtask <subcommand>");
    println!();
    println!("Subcommands:");
    println!("  desktop-regression [--list] [--suite ID] [--observe off|basic|full]");
    println!("  profile cpu    [--out-dir DIR] [--rate HZ]  Record CPU profile via samply");
    println!("  profile memory [--out-dir DIR]              Record heap profile via dhat");
    println!("  profile all    [--out-dir DIR] [--rate HZ]  Record both, then open dashboard");
    println!();
    println!("Defaults:");
    println!("  --out-dir target/profile");
    println!("  --rate    1000");
    println!();
    println!(
        "Also exposed as cargo aliases: cargo profile-cpu, cargo profile-memory, cargo profile-all"
    );
}

/// Compute the path to the built app binary for the host platform.
pub fn binary_path(repo_root: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "terminal-manager.exe"
    } else {
        "terminal-manager"
    };
    repo_root.join("target").join("release").join(name)
}

/// Open `path` in the host's default browser / file association.
pub fn open_in_browser(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // `cmd /c start "" <path>` handles spaces and URL-like paths correctly.
        Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .status()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).status().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).status().map(|_| ())
    }
}

/// Ensure `dir` exists, creating all missing parents.
pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

/// Remove `file` if it exists. Silent no-op if absent.
pub fn remove_if_exists(file: &Path) -> std::io::Result<()> {
    match fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Check whether an executable is on PATH.
pub fn which(tool: &str) -> bool {
    let path = match env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };
    let exts: Vec<String> = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.BAT;.CMD".into())
            .split(';')
            .map(|s| s.to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{tool}{ext}"));
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_path_has_expected_layout() {
        let root = Path::new("C:/repo");
        let bin = binary_path(root);
        assert!(
            bin.ends_with("target/release/terminal-manager")
                || bin.ends_with("target/release/terminal-manager.exe")
        );
        assert!(bin.starts_with(root));
    }

    #[test]
    fn binary_path_platform_suffix() {
        let root = Path::new("/repo");
        let bin = binary_path(root);
        let name = bin.file_name().unwrap().to_string_lossy().into_owned();
        if cfg!(windows) {
            assert_eq!(name, "terminal-manager.exe");
        } else {
            assert_eq!(name, "terminal-manager");
        }
    }

    #[test]
    fn remove_if_exists_is_idempotent_when_absent() {
        let tmp = env::temp_dir().join("xtask-nonexistent-abc123");
        assert!(!tmp.exists());
        remove_if_exists(&tmp).expect("no error when file is absent");
    }

    #[test]
    fn remove_if_exists_removes_present_file() {
        let tmp = env::temp_dir().join("xtask-remove-test.tmp");
        fs::write(&tmp, b"x").unwrap();
        assert!(tmp.exists());
        remove_if_exists(&tmp).unwrap();
        assert!(!tmp.exists());
    }

    #[test]
    fn ensure_dir_creates_nested_paths() {
        let tmp = env::temp_dir().join("xtask-ensure-dir/nested/deep");
        let _ = fs::remove_dir_all(env::temp_dir().join("xtask-ensure-dir"));
        ensure_dir(&tmp).unwrap();
        assert!(tmp.is_dir());
        let _ = fs::remove_dir_all(env::temp_dir().join("xtask-ensure-dir"));
    }

    #[test]
    fn which_finds_cargo() {
        // Cargo must exist for us to even run this test.
        assert!(which("cargo"));
    }

    #[test]
    fn which_rejects_bogus_tool() {
        assert!(!which("definitely-not-a-real-tool-xyz-987"));
    }
}
