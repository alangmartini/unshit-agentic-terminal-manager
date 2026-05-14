use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct ArtifactLayout {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub results_path: PathBuf,
}

pub fn create_run_layout(
    workspace_root: &Path,
    artifact_root: &Path,
) -> Result<ArtifactLayout, String> {
    create_run_layout_at(workspace_root, artifact_root, SystemTime::now())
}

fn create_run_layout_at(
    workspace_root: &Path,
    artifact_root: &Path,
    now: SystemTime,
) -> Result<ArtifactLayout, String> {
    let base_dir = workspace_root.join(artifact_root);
    std::fs::create_dir_all(&base_dir).map_err(|e| {
        format!(
            "failed to create artifact root directory {}: {e}",
            base_dir.display()
        )
    })?;

    let base_run_id = make_run_id(now);
    for attempt in 0..100 {
        let run_id = if attempt == 0 {
            base_run_id.clone()
        } else {
            format!("{base_run_id}-{attempt}")
        };
        let run_dir = base_dir.join(&run_id);
        match std::fs::create_dir(&run_dir) {
            Ok(()) => {
                let results_path = run_dir.join("results.json");
                return Ok(ArtifactLayout {
                    run_id,
                    run_dir,
                    results_path,
                });
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!(
                    "failed to create artifact directory {}: {err}",
                    run_dir.display()
                ));
            }
        }
    }

    Err(format!(
        "failed to allocate unique artifact directory under {}",
        base_dir.display()
    ))
}

pub fn suite_artifact_name(suite_id: &str, name: &str, extension: &str) -> String {
    let safe_name = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("{suite_id}-{safe_name}.{extension}")
}

pub fn make_run_id(now: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(now);
    let millis = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .subsec_millis();
    format!(
        "{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}-{millis:03}-p{}",
        std::process::id()
    )
}

pub fn format_utc_timestamp(now: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(now);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn utc_parts(now: SystemTime) -> (i64, i64, i64, i64, i64, i64) {
    let duration = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let seconds = duration.as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;

    (year, month, day, hour, minute, second)
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096).div_euclid(365);
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let d = doy - (153 * mp + 2).div_euclid(5) + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_run_directory_under_artifact_root() {
        let root = std::env::temp_dir().join(format!("xtask-dr-artifacts-{}", std::process::id()));
        let artifact_root = PathBuf::from("artifacts/windows/desktop-regression");
        let layout = create_run_layout(&root, &artifact_root).unwrap();

        assert!(layout.run_dir.starts_with(root.join(&artifact_root)));
        assert!(layout.run_dir.is_dir());
        assert_eq!(layout.results_path, layout.run_dir.join("results.json"));
        assert!(!layout.run_id.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn run_id_uses_utc_timestamp_layout() {
        let time = UNIX_EPOCH + Duration::from_secs(1_778_434_212);
        assert_eq!(
            make_run_id(time),
            format!("20260510-173012-000-p{}", std::process::id())
        );
    }

    #[test]
    fn create_run_layout_retries_on_run_id_collision() {
        let root = std::env::temp_dir().join(format!(
            "xtask-dr-artifact-collision-{}",
            std::process::id()
        ));
        let artifact_root = PathBuf::from("artifacts/windows/desktop-regression");
        let time = UNIX_EPOCH + Duration::from_secs(1_778_434_212);
        let first_id = make_run_id(time);
        std::fs::create_dir_all(root.join(&artifact_root).join(&first_id)).unwrap();

        let layout = create_run_layout_at(&root, &artifact_root, time).unwrap();

        assert_eq!(layout.run_id, format!("{first_id}-1"));
        assert!(layout.run_dir.is_dir());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn formats_schema_timestamp_as_utc_iso_like_string() {
        let time = UNIX_EPOCH + Duration::from_secs(1_778_434_212);
        assert_eq!(format_utc_timestamp(time), "2026-05-10T17:30:12Z");
    }

    #[test]
    fn suite_artifact_name_sanitizes_dynamic_parts() {
        assert_eq!(
            suite_artifact_name("edge-resize-stability", "after drag", "png"),
            "edge-resize-stability-after-drag.png"
        );
    }
}
