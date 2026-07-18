use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::shell::ShellSpec;

pub mod discovery;
pub mod hooks;
pub mod telemetry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    Claude,
    Codex,
}

impl AgentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

impl From<crate::quick_prompt::Agent> for AgentKind {
    fn from(value: crate::quick_prompt::Agent) -> Self {
        match value {
            crate::quick_prompt::Agent::Claude => Self::Claude,
            crate::quick_prompt::Agent::Codex => Self::Codex,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResumeMode {
    #[default]
    Interactive,
    CodexExec,
}

impl AgentResumeMode {
    fn is_interactive(&self) -> bool {
        matches!(self, Self::Interactive)
    }
}

/// Durable launch phase for a recorded conversation.
///
/// This distinguishes a warm, already-confirmed agent from the ordinary
/// shell Terminal Manager starts while manual recovery is pending. Without
/// the phase, closing and reopening only the UI would reattach that temporary
/// shell and incorrectly hide the Resume button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLaunchPhase {
    /// A provider hook confirmed the agent session, or this record predates
    /// launch-phase persistence.
    #[default]
    Confirmed,
    /// A daemon-owned ordinary shell is intentionally waiting for the user to
    /// choose Resume.
    PendingManual,
    /// A structured resume command is alive but has not emitted SessionStart.
    ConfirmingResume,
}

impl AgentLaunchPhase {
    fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRestart {
    pub agent: AgentKind,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "AgentResumeMode::is_interactive")]
    pub resume_mode: AgentResumeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub observed_at_unix_ms: u64,
    #[serde(default)]
    pub managed: bool,
    #[serde(default, skip_serializing_if = "AgentLaunchPhase::is_confirmed")]
    pub launch_phase: AgentLaunchPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateConfidence {
    Exact,
    Discovered,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeCandidate {
    pub agent: AgentKind,
    pub cwd: PathBuf,
    pub resume_mode: AgentResumeMode,
    pub session_id: String,
    pub confidence: CandidateConfidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnIntent {
    Ordinary,
    PendingManual,
    AutoResume,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSpawnPlan {
    pub cwd: Option<PathBuf>,
    pub shell: Option<ShellSpec>,
    pub candidate: Option<ResumeCandidate>,
    pub intent: SpawnIntent,
}

pub fn spawn_plan(
    record: Option<&AgentRestart>,
    auto_resume: bool,
    fallback_cwd: Option<PathBuf>,
    fallback_shell: Option<ShellSpec>,
) -> AgentSpawnPlan {
    let Some(record) = record else {
        return AgentSpawnPlan {
            cwd: fallback_cwd,
            shell: fallback_shell,
            candidate: None,
            intent: SpawnIntent::Ordinary,
        };
    };
    let candidate = discovery::discover_candidate(record);
    spawn_plan_with_candidate(record, candidate, auto_resume, fallback_cwd, fallback_shell)
}

fn spawn_plan_with_candidate(
    record: &AgentRestart,
    candidate: Option<ResumeCandidate>,
    auto_resume: bool,
    _fallback_cwd: Option<PathBuf>,
    fallback_shell: Option<ShellSpec>,
) -> AgentSpawnPlan {
    let cwd = Some(normalized_cwd(&record.cwd));
    match candidate {
        Some(candidate) if auto_resume => AgentSpawnPlan {
            cwd,
            shell: Some(resume_shell_spec(&candidate)),
            candidate: Some(candidate),
            intent: SpawnIntent::AutoResume,
        },
        Some(candidate) => AgentSpawnPlan {
            cwd,
            shell: fallback_shell,
            candidate: Some(candidate),
            intent: SpawnIntent::PendingManual,
        },
        None => AgentSpawnPlan {
            cwd,
            shell: fallback_shell,
            candidate: None,
            intent: SpawnIntent::Ordinary,
        },
    }
}

pub fn is_valid_session_id(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => byte == b'-',
        _ => byte.is_ascii_hexdigit(),
    })
}

pub fn generate_session_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn mix64(mut value: u64) -> u64 {
        value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let process = u64::from(std::process::id());
    let high = mix64((now as u64) ^ process.rotate_left(17) ^ sequence);
    let low = mix64((now >> 64) as u64 ^ sequence.rotate_left(29) ^ process);
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&high.to_be_bytes());
    bytes[8..].copy_from_slice(&low.to_be_bytes());
    // RFC 9562 UUIDv4 layout. These ids are opaque correlation values,
    // not authentication secrets; uniqueness is supplied by time,
    // process id, and the process-local atomic sequence above.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

pub fn resume_shell_spec(candidate: &ResumeCandidate) -> ShellSpec {
    match candidate.agent {
        AgentKind::Claude => ShellSpec {
            program: if cfg!(windows) {
                "claude.cmd".to_string()
            } else {
                "claude".to_string()
            },
            args: vec!["--resume".to_string(), candidate.session_id.clone()],
        },
        AgentKind::Codex => ShellSpec {
            program: if cfg!(windows) {
                "codex.cmd".to_string()
            } else {
                "codex".to_string()
            },
            args: match candidate.resume_mode {
                AgentResumeMode::Interactive => vec![
                    "resume".to_string(),
                    "-C".to_string(),
                    candidate.cwd.display().to_string(),
                    candidate.session_id.clone(),
                ],
                AgentResumeMode::CodexExec => vec![
                    "exec".to_string(),
                    "resume".to_string(),
                    candidate.session_id.clone(),
                ],
            },
        },
    }
}

pub fn exact_candidate(record: &AgentRestart) -> Option<ResumeCandidate> {
    let session_id = record.session_id.as_deref()?;
    if !is_valid_session_id(session_id) {
        return None;
    }
    Some(ResumeCandidate {
        agent: record.agent,
        cwd: record.cwd.clone(),
        resume_mode: record.resume_mode,
        session_id: session_id.to_string(),
        confidence: CandidateConfidence::Exact,
    })
}

pub fn claude_initial_shell_spec(prompt: &str, session_id: &str) -> Option<ShellSpec> {
    is_valid_session_id(session_id).then(|| ShellSpec {
        program: if cfg!(windows) {
            "claude.cmd".to_string()
        } else {
            "claude".to_string()
        },
        args: vec![
            "--session-id".to_string(),
            session_id.to_string(),
            prompt.to_string(),
        ],
    })
}

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

pub fn normalized_cwd(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Apply one validated SessionStart observation. Repeated provider/id
/// observations retain the first launch directory but still return true
/// so persistence is retried after a prior transient write failure.
pub fn observe_session(
    state: &mut crate::state::AppState,
    pane_id: u32,
    workspace_id: u32,
    agent: AgentKind,
    session_id: &str,
    cwd: &Path,
    source: &'static str,
) -> bool {
    if !is_valid_session_id(session_id) || !cwd.is_absolute() {
        return false;
    }
    let normalized_id = session_id.to_ascii_lowercase();
    let same_identity = state.agent_restarts.get(&pane_id).is_some_and(|existing| {
        existing.agent == agent && existing.session_id.as_deref() == Some(normalized_id.as_str())
    });
    let unchanged = same_identity
        && state
            .agent_restarts
            .get(&pane_id)
            .is_some_and(|existing| existing.launch_phase == AgentLaunchPhase::Confirmed);
    if same_identity {
        // A resume confirmation is an observation of the launch already in
        // flight. Preserve its original cwd, launch mode, and managed marker;
        // only advance the durable phase. This matters for `codex exec
        // resume`, whose hook reports the same id as an interactive session.
        if let Some(record) = state.agent_restarts.get_mut(&pane_id) {
            record.launch_phase = AgentLaunchPhase::Confirmed;
            record.observed_at_unix_ms = now_unix_ms();
        }
    } else {
        let existing = state.agent_restarts.get(&pane_id);
        let managed = existing.map(|record| record.managed).unwrap_or(false);
        // Quick Prompt starts Codex Exec before its hook has supplied an id.
        // Preserve that launch mode only while filling this initial placeholder;
        // a later, distinct hook id represents a newly started interactive session.
        let resume_mode = existing
            .filter(|record| record.agent == agent && record.session_id.is_none())
            .map(|record| record.resume_mode)
            .unwrap_or_default();
        state.agent_restarts.insert(
            pane_id,
            AgentRestart {
                agent,
                cwd: normalized_cwd(cwd),
                resume_mode,
                session_id: Some(normalized_id),
                observed_at_unix_ms: now_unix_ms(),
                managed,
                launch_phase: AgentLaunchPhase::Confirmed,
            },
        );
    }
    state.pending_agent_resumes.remove(&pane_id);
    state.agent_resume_attempts.remove(&pane_id);
    let mut event = telemetry::RestoreEvent::new(
        &state.restore_correlation_id,
        telemetry::EventName::HookObserved,
        telemetry::Level::Info,
    );
    event.provider = Some(agent);
    event.workspace_id = Some(workspace_id);
    event.pane_id = Some(pane_id);
    event.source = Some(source);
    event.outcome = Some(if unchanged { "confirmed" } else { "observed" });
    telemetry::record(&event);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "019f75b0-e94f-71f0-b1ea-f478f8438c1a";

    #[test]
    fn validates_only_hyphenated_uuid_session_ids() {
        assert!(is_valid_session_id(ID));
        assert!(is_valid_session_id("24C31FC8-8200-4773-8A0B-0447BD64BCDC"));
        assert!(!is_valid_session_id("--last"));
        assert!(!is_valid_session_id("019f75b0e94f71f0b1eaf478f8438c1a"));
        assert!(!is_valid_session_id("../../conversation"));
        assert!(!is_valid_session_id("019f75b0-e94f-71f0-b1ea-f478f8438c1z"));
    }

    #[test]
    fn generated_ids_are_valid_and_distinct() {
        let first = generate_session_id();
        let second = generate_session_id();
        assert!(is_valid_session_id(&first), "{first}");
        assert!(is_valid_session_id(&second), "{second}");
        assert_ne!(first, second);
    }

    #[test]
    fn claude_resume_is_structured_argv() {
        let candidate = ResumeCandidate {
            agent: AgentKind::Claude,
            cwd: PathBuf::from(r"C:\dev\project"),
            resume_mode: AgentResumeMode::Interactive,
            session_id: ID.to_string(),
            confidence: CandidateConfidence::Exact,
        };
        let spec = resume_shell_spec(&candidate);
        assert_eq!(
            spec.program,
            if cfg!(windows) {
                "claude.cmd"
            } else {
                "claude"
            }
        );
        assert_eq!(spec.args, vec!["--resume", ID]);
    }

    #[test]
    fn codex_resume_is_structured_argv_with_recorded_cwd() {
        let cwd = PathBuf::from(r"C:\dev\project");
        let candidate = ResumeCandidate {
            agent: AgentKind::Codex,
            cwd: cwd.clone(),
            resume_mode: AgentResumeMode::Interactive,
            session_id: ID.to_string(),
            confidence: CandidateConfidence::Exact,
        };
        let spec = resume_shell_spec(&candidate);
        assert_eq!(
            spec.program,
            if cfg!(windows) { "codex.cmd" } else { "codex" }
        );
        assert_eq!(
            spec.args,
            vec![
                "resume".to_string(),
                "-C".to_string(),
                cwd.display().to_string(),
                ID.to_string(),
            ]
        );
    }

    #[test]
    fn codex_exec_resume_preserves_non_interactive_session_mode() {
        let candidate = ResumeCandidate {
            agent: AgentKind::Codex,
            cwd: PathBuf::from(r"C:\dev\project"),
            resume_mode: AgentResumeMode::CodexExec,
            session_id: ID.to_string(),
            confidence: CandidateConfidence::Exact,
        };
        let spec = resume_shell_spec(&candidate);
        assert_eq!(spec.args, vec!["exec", "resume", ID]);
    }

    #[test]
    fn claude_initial_session_rejects_invalid_id_and_keeps_prompt_one_arg() {
        assert!(claude_initial_shell_spec("secret prompt", "--continue").is_none());
        let spec = claude_initial_shell_spec("line one\nline two", ID).expect("valid spec");
        assert_eq!(
            spec.args,
            vec![
                "--session-id".to_string(),
                ID.to_string(),
                "line one\nline two".to_string(),
            ]
        );
    }

    #[test]
    fn exact_candidate_fails_closed_for_invalid_persisted_id() {
        let record = AgentRestart {
            agent: AgentKind::Claude,
            cwd: PathBuf::from("project"),
            resume_mode: AgentResumeMode::Interactive,
            session_id: Some("--continue".to_string()),
            observed_at_unix_ms: 1,
            managed: false,
            launch_phase: AgentLaunchPhase::Confirmed,
        };
        assert!(exact_candidate(&record).is_none());
    }

    #[test]
    fn spawn_plan_uses_resume_only_when_auto_is_enabled() {
        let cwd = std::env::current_dir().expect("cwd");
        let record = AgentRestart {
            agent: AgentKind::Claude,
            cwd: cwd.clone(),
            resume_mode: AgentResumeMode::Interactive,
            session_id: Some(ID.into()),
            observed_at_unix_ms: 1,
            managed: true,
            launch_phase: AgentLaunchPhase::Confirmed,
        };
        let candidate = exact_candidate(&record).expect("candidate");
        let ordinary_shell = ShellSpec {
            program: "pwsh".into(),
            args: vec![],
        };

        let manual = spawn_plan_with_candidate(
            &record,
            Some(candidate.clone()),
            false,
            None,
            Some(ordinary_shell.clone()),
        );
        assert_eq!(manual.intent, SpawnIntent::PendingManual);
        assert_eq!(manual.shell, Some(ordinary_shell));
        assert_eq!(manual.cwd, Some(cwd.canonicalize().expect("canonical cwd")));

        let automatic = spawn_plan_with_candidate(&record, Some(candidate), true, None, None);
        assert_eq!(automatic.intent, SpawnIntent::AutoResume);
        assert_eq!(
            automatic.shell.expect("resume shell").args,
            vec!["--resume", ID]
        );
    }

    #[test]
    fn missing_candidate_starts_an_ordinary_shell_in_recorded_cwd() {
        let cwd = std::env::current_dir().expect("cwd");
        let record = AgentRestart {
            agent: AgentKind::Codex,
            cwd: cwd.clone(),
            resume_mode: AgentResumeMode::Interactive,
            session_id: None,
            observed_at_unix_ms: u64::MAX,
            managed: true,
            launch_phase: AgentLaunchPhase::Confirmed,
        };
        let shell = ShellSpec {
            program: "shell".into(),
            args: vec![],
        };
        let plan = spawn_plan_with_candidate(&record, None, true, None, Some(shell.clone()));
        assert_eq!(plan.intent, SpawnIntent::Ordinary);
        assert_eq!(plan.shell, Some(shell));
        assert_eq!(plan.cwd, Some(cwd.canonicalize().expect("canonical cwd")));
    }

    #[test]
    fn resume_confirmation_preserves_codex_exec_mode_and_original_cwd() {
        let mut state = crate::state::seed_state();
        let original_cwd = std::env::current_dir().expect("cwd");
        let hook_cwd = original_cwd.join("hook-reported-subdirectory");
        state.agent_restarts.insert(
            1,
            AgentRestart {
                agent: AgentKind::Codex,
                cwd: original_cwd.clone(),
                resume_mode: AgentResumeMode::CodexExec,
                session_id: Some(ID.to_string()),
                observed_at_unix_ms: 1,
                managed: true,
                launch_phase: AgentLaunchPhase::ConfirmingResume,
            },
        );

        assert!(observe_session(
            &mut state,
            1,
            1,
            AgentKind::Codex,
            ID,
            &hook_cwd,
            "resume",
        ));

        let record = &state.agent_restarts[&1];
        assert_eq!(record.resume_mode, AgentResumeMode::CodexExec);
        assert_eq!(record.cwd, original_cwd);
        assert!(record.managed);
        assert_eq!(record.launch_phase, AgentLaunchPhase::Confirmed);
    }
}
