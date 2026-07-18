use std::io::Read;
use std::path::{Path, PathBuf};

use super::{
    exact_candidate, is_valid_session_id, normalized_cwd, AgentKind, AgentRestart,
    CandidateConfidence, ResumeCandidate,
};

const MAX_FILES: usize = 512;
const MAX_ENTRIES: usize = 2_048;
const MAX_DEPTH: usize = 5;
const MAX_METADATA_BYTES: u64 = 128 * 1024;
const MAX_METADATA_LINES: usize = 64;
const RECENCY_WINDOW_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

pub fn discover_candidate(record: &AgentRestart) -> Option<ResumeCandidate> {
    if let Some(candidate) = exact_candidate(record) {
        return Some(candidate);
    }
    if !record.managed || record.session_id.is_some() {
        return None;
    }
    let home = dirs::home_dir()?;
    discover_candidate_in(
        record,
        &home.join(".claude/projects"),
        &home.join(".codex/sessions"),
        super::now_unix_ms(),
    )
}

fn discover_candidate_in(
    record: &AgentRestart,
    claude_projects: &Path,
    codex_sessions: &Path,
    now_unix_ms: u64,
) -> Option<ResumeCandidate> {
    if let Some(candidate) = exact_candidate(record) {
        return Some(candidate);
    }
    if !record.managed || record.session_id.is_some() {
        return None;
    }
    let root = match record.agent {
        AgentKind::Claude => claude_projects,
        AgentKind::Codex => codex_sessions,
    };
    let not_before = record
        .observed_at_unix_ms
        .saturating_sub(CLOCK_SKEW_MS)
        .max(now_unix_ms.saturating_sub(RECENCY_WINDOW_MS));
    let not_after = now_unix_ms.saturating_add(CLOCK_SKEW_MS);
    let wanted_cwd = normalized_cwd(&record.cwd);

    let scan = collect_jsonl_files(root);
    candidate_from_complete_scan(
        record,
        wanted_cwd,
        not_before,
        not_after,
        scan,
        transcript_identity,
    )
}

fn candidate_from_complete_scan(
    record: &AgentRestart,
    wanted_cwd: PathBuf,
    not_before: u64,
    not_after: u64,
    scan: CandidateScan,
    mut identity: impl FnMut(AgentKind, &Path) -> Option<(String, PathBuf)>,
) -> Option<ResumeCandidate> {
    if !scan.complete {
        return None;
    }
    let mut matches = Vec::new();
    for entry in scan
        .files
        .into_iter()
        .filter(|entry| entry.modified_unix_ms >= not_before && entry.modified_unix_ms <= not_after)
    {
        // A relevant file that cannot be opened/read/identified may hide a
        // second matching conversation. Never turn that uncertainty into a
        // unique automatic candidate.
        let (session_id, cwd) = identity(record.agent, &entry.path)?;
        if same_cwd(&wanted_cwd, &cwd) {
            matches.push((entry.modified_unix_ms, session_id));
        }
    }
    matches.sort_by(|left, right| left.1.cmp(&right.1));
    matches.dedup_by(|left, right| left.1 == right.1);
    if matches.len() != 1 {
        return None;
    }
    let (_, session_id) = matches.pop()?;
    Some(ResumeCandidate {
        agent: record.agent,
        cwd: wanted_cwd,
        resume_mode: record.resume_mode,
        session_id,
        confidence: CandidateConfidence::Discovered,
    })
}

struct CandidateFile {
    path: PathBuf,
    modified_unix_ms: u64,
}

struct CandidateScan {
    files: Vec<CandidateFile>,
    complete: bool,
}

fn collect_jsonl_files(root: &Path) -> CandidateScan {
    match std::fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return CandidateScan {
                files: Vec::new(),
                complete: false,
            };
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CandidateScan {
                files: Vec::new(),
                complete: true,
            };
        }
        Err(_) => {
            return CandidateScan {
                files: Vec::new(),
                complete: false,
            };
        }
    }
    let mut directories = vec![(root.to_path_buf(), 0usize)];
    let mut files = Vec::new();
    let mut entries_seen = 0usize;
    while let Some((directory, depth)) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return CandidateScan {
                files,
                complete: false,
            };
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return CandidateScan {
                    files,
                    complete: false,
                };
            };
            if entries_seen >= MAX_ENTRIES || files.len() >= MAX_FILES {
                return CandidateScan {
                    files,
                    complete: false,
                };
            }
            entries_seen += 1;
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                return CandidateScan {
                    files,
                    complete: false,
                };
            };
            if file_type.is_dir() {
                if depth >= MAX_DEPTH {
                    return CandidateScan {
                        files,
                        complete: false,
                    };
                }
                directories.push((path, depth + 1));
                continue;
            }
            if file_type.is_symlink() {
                return CandidateScan {
                    files,
                    complete: false,
                };
            }
            if !file_type.is_file()
                || path.extension().and_then(|extension| extension.to_str()) != Some("jsonl")
            {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
                return CandidateScan {
                    files,
                    complete: false,
                };
            };
            let Ok(modified) = modified.duration_since(std::time::UNIX_EPOCH) else {
                return CandidateScan {
                    files,
                    complete: false,
                };
            };
            let modified_unix_ms = modified.as_millis().min(u64::MAX as u128) as u64;
            files.push(CandidateFile {
                path,
                modified_unix_ms,
            });
        }
    }
    CandidateScan {
        files,
        complete: true,
    }
}

fn transcript_identity(agent: AgentKind, path: &Path) -> Option<(String, PathBuf)> {
    let file = std::fs::File::open(path).ok()?;
    let mut body = String::new();
    file.take(MAX_METADATA_BYTES)
        .read_to_string(&mut body)
        .ok()?;
    match agent {
        AgentKind::Claude => claude_identity(path, &body),
        AgentKind::Codex => codex_identity(&body),
    }
}

fn claude_identity(path: &Path, body: &str) -> Option<(String, PathBuf)> {
    let mut session_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|value| is_valid_session_id(value))
        .map(ToOwned::to_owned);
    let mut cwd = None;
    for line in body.lines().take(MAX_METADATA_LINES) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if session_id.is_none() {
            session_id = value
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .filter(|value| is_valid_session_id(value))
                .map(ToOwned::to_owned);
        }
        if cwd.is_none() {
            cwd = value
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .filter(|value| Path::new(value).is_absolute())
                .map(PathBuf::from);
        }
        if session_id.is_some() && cwd.is_some() {
            break;
        }
    }
    Some((session_id?, cwd?))
}

fn codex_identity(body: &str) -> Option<(String, PathBuf)> {
    for line in body.lines().take(MAX_METADATA_LINES) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(serde_json::Value::as_str) != Some("session_meta") {
            continue;
        }
        let payload = value.get("payload")?;
        let session_id = payload
            .get("id")
            .or_else(|| payload.get("session_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| is_valid_session_id(value))?;
        let cwd = payload
            .get("cwd")
            .and_then(serde_json::Value::as_str)
            .filter(|value| Path::new(value).is_absolute())?;
        return Some((session_id.to_string(), PathBuf::from(cwd)));
    }
    None
}

fn same_cwd(left: &Path, right: &Path) -> bool {
    let left = normalized_cwd(left).to_string_lossy().to_string();
    let right = normalized_cwd(right).to_string_lossy().to_string();
    if cfg!(windows) {
        left.eq_ignore_ascii_case(&right)
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_restore::{AgentKind, CandidateConfidence};
    use std::sync::atomic::{AtomicU64, Ordering};

    const ID: &str = "24c31fc8-8200-4773-8a0b-0447bd64bcdc";

    fn fixture_root(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let count = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tm-agent-discovery-{tag}-{}-{count}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("fixture root");
        path
    }

    fn record(agent: AgentKind, cwd: PathBuf) -> AgentRestart {
        AgentRestart {
            agent,
            cwd,
            resume_mode: Default::default(),
            session_id: None,
            observed_at_unix_ms: 0,
            managed: true,
            launch_phase: crate::agent_restore::AgentLaunchPhase::Confirmed,
        }
    }

    #[test]
    fn discovers_claude_id_from_bounded_metadata_for_matching_cwd() {
        let root = fixture_root("claude");
        let cwd = root.join("project");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let claude = root.join("claude");
        let project_bucket = claude.join("encoded-project");
        std::fs::create_dir_all(&project_bucket).expect("bucket");
        std::fs::write(
            project_bucket.join(format!("{ID}.jsonl")),
            format!(
                "{{\"type\":\"user\",\"sessionId\":\"{ID}\",\"cwd\":{},\"prompt\":\"must not escape\"}}\n",
                serde_json::to_string(&cwd).expect("cwd json")
            ),
        )
        .expect("transcript");

        let candidate = discover_candidate_in(
            &record(AgentKind::Claude, cwd.clone()),
            &claude,
            &root.join("codex"),
            crate::agent_restore::now_unix_ms(),
        )
        .expect("candidate");
        assert_eq!(candidate.session_id, ID);
        assert_eq!(candidate.cwd, cwd.canonicalize().expect("canonical cwd"));
        assert_eq!(candidate.confidence, CandidateConfidence::Discovered);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn discovers_codex_session_meta_and_ignores_unknown_payload_fields() {
        let root = fixture_root("codex");
        let cwd = root.join("project");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let codex_day = root.join("codex/2026/07/18");
        std::fs::create_dir_all(&codex_day).expect("codex day");
        std::fs::write(
            codex_day.join(format!("rollout-{ID}.jsonl")),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{ID}\",\"cwd\":{},\"instructions\":\"must not escape\"}}}}\n",
                serde_json::to_string(&cwd).expect("cwd json")
            ),
        )
        .expect("transcript");

        let mut restart = record(AgentKind::Codex, cwd.clone());
        restart.resume_mode = crate::agent_restore::AgentResumeMode::CodexExec;
        let candidate = discover_candidate_in(
            &restart,
            &root.join("claude"),
            &root.join("codex"),
            crate::agent_restore::now_unix_ms(),
        )
        .expect("candidate");
        assert_eq!(candidate.session_id, ID);
        assert_eq!(candidate.agent, AgentKind::Codex);
        assert_eq!(
            candidate.resume_mode,
            crate::agent_restore::AgentResumeMode::CodexExec
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mismatched_or_corrupt_transcripts_fail_closed() {
        let root = fixture_root("reject");
        let cwd = root.join("wanted");
        let other = root.join("other");
        std::fs::create_dir_all(&cwd).expect("cwd");
        std::fs::create_dir_all(&other).expect("other");
        let bucket = root.join("claude/bucket");
        std::fs::create_dir_all(&bucket).expect("bucket");
        std::fs::write(
            bucket.join(format!("{ID}.jsonl")),
            format!(
                "{{\"sessionId\":\"{ID}\",\"cwd\":{}}}\n",
                serde_json::to_string(&other).expect("cwd json")
            ),
        )
        .expect("mismatch");
        std::fs::write(bucket.join("broken.jsonl"), b"not-json\n").expect("broken");

        assert!(discover_candidate_in(
            &record(AgentKind::Claude, cwd),
            &root.join("claude"),
            &root.join("codex"),
            crate::agent_restore::now_unix_ms(),
        )
        .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn multiple_matching_sessions_are_ambiguous_even_with_different_mtimes() {
        let root = fixture_root("ambiguous");
        let cwd = root.join("project");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let bucket = root.join("claude/bucket");
        std::fs::create_dir_all(&bucket).expect("bucket");
        for id in [
            "24c31fc8-8200-4773-8a0b-0447bd64bcdc",
            "3b9436f8-7a22-4289-bf8a-bce97853cb79",
        ] {
            std::fs::write(
                bucket.join(format!("{id}.jsonl")),
                format!(
                    "{{\"sessionId\":\"{id}\",\"cwd\":{}}}\n",
                    serde_json::to_string(&cwd).expect("cwd json")
                ),
            )
            .expect("transcript");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(discover_candidate_in(
            &record(AgentKind::Claude, cwd),
            &root.join("claude"),
            &root.join("codex"),
            crate::agent_restore::now_unix_ms(),
        )
        .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_ids_and_unmanaged_records_never_fall_back_to_discovery() {
        let root = fixture_root("unmanaged");
        let cwd = root.join("project");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let bucket = root.join("claude/bucket");
        std::fs::create_dir_all(&bucket).expect("bucket");
        std::fs::write(
            bucket.join(format!("{ID}.jsonl")),
            format!(
                "{{\"sessionId\":\"{ID}\",\"cwd\":{}}}\n",
                serde_json::to_string(&cwd).expect("cwd json")
            ),
        )
        .expect("transcript");

        let mut unmanaged = record(AgentKind::Claude, cwd.clone());
        unmanaged.managed = false;
        assert!(discover_candidate_in(
            &unmanaged,
            &root.join("claude"),
            &root.join("codex"),
            crate::agent_restore::now_unix_ms(),
        )
        .is_none());

        let mut invalid = record(AgentKind::Claude, cwd);
        invalid.session_id = Some("--continue".to_string());
        assert!(discover_candidate_in(
            &invalid,
            &root.join("claude"),
            &root.join("codex"),
            crate::agent_restore::now_unix_ms(),
        )
        .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn incomplete_file_scan_never_returns_a_partial_unique_match() {
        let root = fixture_root("file-cap");
        let cwd = root.join("project");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let bucket = root.join("claude/bucket");
        std::fs::create_dir_all(&bucket).expect("bucket");
        for index in 0..=MAX_FILES {
            let id = if index == 0 { ID } else { "invalid" };
            std::fs::write(
                bucket.join(format!("entry-{index:04}-{id}.jsonl")),
                if index == 0 {
                    format!(
                        "{{\"sessionId\":\"{ID}\",\"cwd\":{}}}\n",
                        serde_json::to_string(&cwd).expect("cwd json")
                    )
                } else {
                    "{}\n".to_string()
                },
            )
            .expect("candidate file");
        }

        assert!(discover_candidate_in(
            &record(AgentKind::Claude, cwd),
            &root.join("claude"),
            &root.join("codex"),
            crate::agent_restore::now_unix_ms(),
        )
        .is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn unreadable_relevant_identity_never_makes_visible_match_look_unique() {
        let root = fixture_root("identity-error");
        let cwd = root.join("project");
        std::fs::create_dir_all(&cwd).expect("cwd");
        let scan = CandidateScan {
            files: vec![
                CandidateFile {
                    path: PathBuf::from("visible.jsonl"),
                    modified_unix_ms: 10,
                },
                CandidateFile {
                    path: PathBuf::from("unreadable.jsonl"),
                    modified_unix_ms: 11,
                },
            ],
            complete: true,
        };
        let candidate = candidate_from_complete_scan(
            &record(AgentKind::Claude, cwd.clone()),
            cwd.clone(),
            0,
            20,
            scan,
            |_agent, path| {
                (path == Path::new("visible.jsonl")).then(|| (ID.to_string(), cwd.clone()))
            },
        );

        assert!(candidate.is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
