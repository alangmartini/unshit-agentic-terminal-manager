use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::desktop_regression::artifacts::format_utc_timestamp;

pub const ENVIRONMENT_METADATA_FILE: &str = "environment.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentMetadata {
    pub schema_version: String,
    pub captured_at_utc: String,
    pub runner: RunnerMetadata,
    pub binary: BinaryMetadata,
    pub source: SourceMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerMetadata {
    pub os: String,
    pub arch: String,
    pub current_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryMetadata {
    pub path: String,
    pub exists: bool,
    pub sha256: Option<String>,
    pub hash_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMetadata {
    pub schema_version: String,
    pub commit: Option<String>,
    pub dirty: bool,
    pub dirty_entries: Vec<String>,
    pub errors: Vec<String>,
}

pub fn collect_environment_metadata(
    workspace_root: &Path,
    binary_path: &Path,
) -> EnvironmentMetadata {
    let binary_hash = sha256_file(binary_path);
    let source = collect_source_metadata(workspace_root);

    EnvironmentMetadata {
        schema_version: "desktop-regression.environment/v1".to_owned(),
        captured_at_utc: format_utc_timestamp(SystemTime::now()),
        runner: RunnerMetadata {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            current_dir: std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|e| format!("unavailable: {e}")),
        },
        binary: BinaryMetadata {
            path: display_path_relative_to(workspace_root, binary_path),
            exists: binary_path.is_file(),
            sha256: binary_hash.as_ref().ok().cloned(),
            hash_error: binary_hash.err(),
        },
        source,
    }
}

pub fn write_environment_metadata(
    path: &Path,
    metadata: &EnvironmentMetadata,
) -> Result<(), String> {
    let json = serde_json::to_string_pretty(metadata)
        .map_err(|e| format!("failed to serialize environment metadata: {e}"))?;
    std::fs::write(path, json).map_err(|e| {
        format!(
            "failed to write environment metadata at {}: {e}",
            path.display()
        )
    })
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("failed to open {} for SHA-256: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("failed to read {} for SHA-256: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn collect_source_metadata(workspace_root: &Path) -> SourceMetadata {
    let mut errors = Vec::new();
    let commit = match git_output(workspace_root, &["rev-parse", "HEAD"]) {
        Ok(output) => Some(output),
        Err(err) => {
            errors.push(err);
            None
        }
    };
    let dirty_entries = match git_output(workspace_root, &["status", "--short"]) {
        Ok(output) => output
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
        Err(err) => {
            errors.push(err);
            Vec::new()
        }
    };

    SourceMetadata {
        schema_version: "desktop-regression.source/v1".to_owned(),
        commit,
        dirty: !dirty_entries.is_empty(),
        dirty_entries,
        errors,
    }
}

fn git_output(workspace_root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .map_err(|e| format!("failed to run git {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!(
            "git {} failed with status {}{}",
            args.join(" "),
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn display_path_relative_to(root: &Path, path: &Path) -> String {
    let absolute = if path.is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };

    absolute
        .strip_prefix(root)
        .unwrap_or(&absolute)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn hashes_binary_with_sha256() {
        let dir = std::env::temp_dir().join(format!("xtask-dr-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("binary.exe");
        std::fs::write(&path, b"abc").unwrap();

        let hash = sha256_file(&path).unwrap();

        assert_eq!(
            hash,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn metadata_records_missing_binary_without_losing_source_metadata() {
        let metadata = collect_environment_metadata(
            Path::new("C:/repo"),
            Path::new("target/debug/missing.exe"),
        );

        assert_eq!(metadata.binary.path, "target/debug/missing.exe");
        assert!(!metadata.binary.exists);
        assert!(metadata.binary.sha256.is_none());
        assert!(!metadata.source.schema_version.is_empty());
    }
}
