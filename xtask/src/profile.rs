//! Runs the CPU / memory profile passes. Ports the logic from
//! `scripts/profile-{cpu,memory,all}.{sh,ps1}` into a cross-platform binary.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::args::{ProfileKind, ProfileOpts};
use crate::{binary_path, ensure_dir, open_in_browser, remove_if_exists, which};

/// Entry point invoked from `main.rs` after argument parsing.
pub fn run(opts: &ProfileOpts) -> Result<(), String> {
    let repo_root = workspace_root()?;
    let out_dir = repo_root.join(&opts.out_dir);
    ensure_dir(&out_dir).map_err(|e| format!("failed to create {}: {e}", out_dir.display()))?;

    match opts.kind {
        ProfileKind::Cpu => run_cpu(&repo_root, &out_dir, opts.rate),
        ProfileKind::Memory => run_memory(&repo_root, &out_dir),
        ProfileKind::All => {
            run_cpu(&repo_root, &out_dir, opts.rate)?;
            run_memory(&repo_root, &out_dir)?;
            open_dashboard(&repo_root)
        }
    }
}

fn run_cpu(repo_root: &Path, out_dir: &Path, rate: u32) -> Result<(), String> {
    if !which("samply") {
        println!("==> samply not found. Installing via cargo...");
        run_status(
            Command::new("cargo")
                .args(["install", "samply"])
                .current_dir(repo_root),
        )
        .map_err(|e| format!("cargo install samply failed: {e}"))?;
    }

    println!("==> cargo build --release");
    run_status(
        Command::new("cargo")
            .args(["build", "--release"])
            .current_dir(repo_root),
    )
    .map_err(|e| format!("cargo build failed: {e}"))?;

    let bin = binary_path(repo_root);
    if !bin.is_file() {
        return Err(format!("binary not found at {}", bin.display()));
    }

    let cpu_file = out_dir.join("cpu.json.gz");
    remove_if_exists(&cpu_file)
        .map_err(|e| format!("failed to remove {}: {e}", cpu_file.display()))?;

    let rate_s = rate.to_string();
    let cpu_file_str = cpu_file.to_string_lossy().into_owned();
    println!();
    println!(
        "==> samply record --save-only --output {cpu_file_str} --rate {rate_s} -- {}",
        bin.display()
    );
    println!("    Exercise the UI, then close the window to stop recording.");
    println!();

    run_status(
        Command::new("samply")
            .args([
                "record",
                "--save-only",
                "--output",
                &cpu_file_str,
                "--rate",
                &rate_s,
                "--",
            ])
            .arg(&bin)
            .current_dir(repo_root),
    )
    .map_err(|e| format!("samply record failed: {e}"))?;

    println!();
    if cpu_file.is_file() {
        let size = std::fs::metadata(&cpu_file).map(|m| m.len()).unwrap_or(0);
        println!(
            "==> CPU profile written: {} ({size} bytes)",
            cpu_file.display()
        );
        println!("    Open scripts/profile.html (CPU card) and drop the file on the viewer.");
    } else {
        eprintln!(
            "WARNING: expected {} but the file was not produced.",
            cpu_file.display()
        );
    }
    Ok(())
}

fn run_memory(repo_root: &Path, out_dir: &Path) -> Result<(), String> {
    println!("==> cargo build --release --features profiling");
    run_status(
        Command::new("cargo")
            .args(["build", "--release", "--features", "profiling"])
            .current_dir(repo_root),
    )
    .map_err(|e| format!("cargo build failed: {e}"))?;

    let bin = binary_path(repo_root);
    if !bin.is_file() {
        return Err(format!("binary not found at {}", bin.display()));
    }

    let heap_file = out_dir.join("dhat-heap.json");
    remove_if_exists(&heap_file)
        .map_err(|e| format!("failed to remove {}: {e}", heap_file.display()))?;

    println!();
    println!("==> Launching app with dhat heap profiling.");
    println!("    Exercise the UI, then close the window to flush the profile.");
    println!();

    run_status(Command::new(&bin).current_dir(repo_root))
        .map_err(|e| format!("app launch failed: {e}"))?;

    println!();
    if heap_file.is_file() {
        let size = std::fs::metadata(&heap_file).map(|m| m.len()).unwrap_or(0);
        println!(
            "==> Heap profile written: {} ({size} bytes)",
            heap_file.display()
        );
        println!("    Open scripts/profile.html (Memory card) and drop the file on the viewer.");
    } else {
        eprintln!(
            "WARNING: expected {} but the file was not produced.",
            heap_file.display()
        );
        eprintln!(
            "         Make sure the app closed via the window close button or Ctrl+C (not kill)."
        );
    }
    Ok(())
}

fn open_dashboard(repo_root: &Path) -> Result<(), String> {
    let dash = repo_root.join("scripts").join("profile.html");
    println!();
    println!("==> Opening {}", dash.display());
    open_in_browser(&dash).map_err(|e| format!("failed to open {}: {e}", dash.display()))
}

/// Run a command and treat non-zero exit status as an error.
fn run_status(cmd: &mut Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn {:?}: {e}", cmd.get_program()))?;
    if !status.success() {
        return Err(format!(
            "{:?} exited with status {status}",
            cmd.get_program()
        ));
    }
    Ok(())
}

/// Locate the workspace root by walking up from the xtask crate's manifest dir
/// until a Cargo.toml containing `[workspace]` is found.
///
/// `CARGO_MANIFEST_DIR` at build time points to `xtask/`; `cargo run --package
/// xtask` also sets it. The workspace root is its parent.
fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut dir = PathBuf::from(manifest_dir);
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            if let Ok(contents) = std::fs::read_to_string(&candidate) {
                if contents.contains("[workspace]") {
                    return Ok(dir);
                }
            }
        }
        if !dir.pop() {
            return Err(format!(
                "could not find workspace root starting from {manifest_dir}"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_finds_repo_toml() {
        let root = workspace_root().expect("workspace root resolvable from xtask manifest dir");
        assert!(root.join("Cargo.toml").is_file());
        let contents = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(contents.contains("[workspace]"));
    }

    #[test]
    fn workspace_root_contains_xtask_member() {
        let root = workspace_root().unwrap();
        assert!(root.join("xtask").join("Cargo.toml").is_file());
    }
}
