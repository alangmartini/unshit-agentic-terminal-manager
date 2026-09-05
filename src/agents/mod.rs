//! Agent registry and classification for the sidebar **agents** subtab.
//!
//! A pane is an *agent pane* when it runs an agent CLI (Claude Code, Codex,
//! Gemini CLI, ...). Membership comes from three signals, strongest first:
//!
//! 1. **Launched** — the app started the agent itself (New agent, Quick
//!    Prompt). Recorded at spawn time.
//! 2. **Hook** — a provider SessionStart hook reported the session
//!    (`terminal-manager session-hook`). Lives in `AppState::agent_restarts`.
//! 3. **Title** — the guest program's window title (OSC 0/2) identifies a
//!    known agent. This is the only signal that needs no setup, so it is
//!    the primary path for a user who types `claude` into a shell. It is
//!    also the only signal that clears again: when the title stops
//!    matching (the agent exited back to the shell) the pane returns to
//!    the terminals list.
//!
//! The profile table below is static on purpose: adding an agent is one
//! row, and nothing here touches the resume machinery in
//! `crate::agent_restore`, which keeps its own two-variant `AgentKind`.

use serde::{Deserialize, Serialize};

pub mod telemetry;

/// How a pane earned its agent tag. Order is significance: a `Launched`
/// tag is never overwritten by a `Title` observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTagSource {
    Launched,
    Hook,
    Title,
}

impl AgentTagSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Launched => "launched",
            Self::Hook => "hook",
            Self::Title => "title",
        }
    }
}

/// The agent a pane is running, as far as the app can tell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTag {
    /// `AgentProfile::id` of the recognised agent.
    pub profile: String,
    pub source: AgentTagSource,
}

impl AgentTag {
    pub fn new(profile: &str, source: AgentTagSource) -> Self {
        Self {
            profile: profile.to_string(),
            source,
        }
    }

    /// Display label for the tag's profile; falls back to the raw id for
    /// a tag persisted by a newer build with an unknown profile.
    pub fn label(&self) -> String {
        profile(&self.profile)
            .map(|p| p.label.to_string())
            .unwrap_or_else(|| self.profile.clone())
    }
}

/// One known agent CLI.
#[derive(Debug)]
pub struct AgentProfile {
    /// Stable id used in dispatch strings, the CLI, and persistence.
    pub id: &'static str,
    /// Human label for menus, palette rows and pane titles.
    pub label: &'static str,
    /// Executable to launch, without the Windows `.cmd` suffix; empty
    /// when the profile is recognised by title only and cannot be
    /// launched from the app.
    pub program: &'static str,
    /// Extra launch arguments.
    pub args: &'static [&'static str],
    /// Lower-case words that identify the agent in a window title.
    /// Matched on word boundaries against non-path tokens only.
    pub title_needles: &'static [&'static str],
    /// Resume/recovery provider, when `crate::agent_restore` knows how to
    /// resume this agent.
    pub restore_kind: Option<crate::agent_restore::AgentKind>,
}

impl AgentProfile {
    /// True when the app can start this agent itself.
    pub fn launchable(&self) -> bool {
        !self.program.is_empty()
    }

    /// Program name as the daemon should spawn it. Windows resolves
    /// `claude.cmd` through PathExt the way the standard installers lay
    /// the shims out; other platforms use the bare name.
    pub fn program_name(&self) -> String {
        if cfg!(windows) && !self.program.is_empty() {
            format!("{}.cmd", self.program)
        } else {
            self.program.to_string()
        }
    }
}

/// Every agent the app recognises, in menu order. Launchable profiles
/// come first.
pub const PROFILES: &[AgentProfile] = &[
    AgentProfile {
        id: "claude",
        label: "Claude Code",
        program: "claude",
        args: &[],
        title_needles: &["claude"],
        restore_kind: Some(crate::agent_restore::AgentKind::Claude),
    },
    AgentProfile {
        id: "codex",
        label: "Codex",
        program: "codex",
        args: &[],
        title_needles: &["codex"],
        restore_kind: Some(crate::agent_restore::AgentKind::Codex),
    },
    AgentProfile {
        id: "gemini",
        label: "Gemini CLI",
        program: "gemini",
        args: &[],
        title_needles: &["gemini"],
        restore_kind: None,
    },
    AgentProfile {
        id: "opencode",
        label: "OpenCode",
        program: "opencode",
        args: &[],
        title_needles: &["opencode"],
        restore_kind: None,
    },
    AgentProfile {
        id: "aider",
        label: "Aider",
        program: "aider",
        args: &[],
        title_needles: &["aider"],
        restore_kind: None,
    },
    AgentProfile {
        id: "copilot",
        label: "Copilot CLI",
        program: "copilot",
        args: &[],
        title_needles: &["copilot"],
        restore_kind: None,
    },
    // Title-only: there is no single `openrouter` executable, but agent
    // CLIs backed by OpenRouter commonly put the name in their title.
    AgentProfile {
        id: "openrouter",
        label: "OpenRouter agent",
        program: "",
        args: &[],
        title_needles: &["openrouter"],
        restore_kind: None,
    },
];

/// Profile id used by the bare `agent.new` dispatch (hotkey, palette,
/// CLI without an argument) when no launchable agent is installed.
pub const DEFAULT_PROFILE_ID: &str = "claude";

pub fn profile(id: &str) -> Option<&'static AgentProfile> {
    PROFILES.iter().find(|p| p.id == id)
}

/// Profile for a resume provider: the pane an agent hook confirmed is
/// listed under the same label the launcher uses.
pub fn profile_for_restore_kind(
    kind: crate::agent_restore::AgentKind,
) -> Option<&'static AgentProfile> {
    PROFILES.iter().find(|p| p.restore_kind == Some(kind))
}

pub fn launchable_profiles() -> impl Iterator<Item = &'static AgentProfile> {
    PROFILES.iter().filter(|p| p.launchable())
}

/// Launchable profiles whose program resolves on `PATH`. Scans PATH once
/// per call, the same way `crate::shell::discover_installed` does for
/// shells; callers build menus from it, never hot paths.
pub fn installed_profiles() -> Vec<&'static AgentProfile> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path_var).collect();
    let exts = executable_extensions();
    launchable_profiles()
        .filter(|p| program_on_path(p.program, &dirs, &exts))
        .collect()
}

/// Profiles offered in the "New agent" flyout: the installed ones, or
/// every launchable profile when nothing is installed so the menu never
/// renders empty.
pub fn menu_profiles() -> Vec<&'static AgentProfile> {
    let installed = installed_profiles();
    if installed.is_empty() {
        launchable_profiles().collect()
    } else {
        installed
    }
}

/// Profile launched by the bare `agent.new`: the first installed
/// launchable agent, else [`DEFAULT_PROFILE_ID`].
pub fn default_profile() -> &'static AgentProfile {
    installed_profiles()
        .into_iter()
        .next()
        .or_else(|| profile(DEFAULT_PROFILE_ID))
        .expect("default profile exists")
}

fn executable_extensions() -> Vec<String> {
    if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    }
}

fn program_on_path(program: &str, dirs: &[std::path::PathBuf], exts: &[String]) -> bool {
    dirs.iter().any(|dir| {
        exts.iter()
            .any(|ext| dir.join(format!("{program}{ext}")).is_file())
    })
}

/// Why a title matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleMatch {
    /// The title starts with one of Claude Code's status glyphs.
    Glyph,
    /// A profile needle appeared as a whole word in a non-path token.
    Needle,
}

impl TitleMatch {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Glyph => "glyph",
            Self::Needle => "needle",
        }
    }
}

/// Status glyphs Claude Code puts in front of its window title: the
/// idle asterisk family and the quarter-circle spinner while it works.
const CLAUDE_TITLE_GLYPHS: &[char] = &[
    '\u{2733}', // ✳
    '\u{2736}', // ✶
    '\u{273B}', // ✻
    '\u{273D}', // ✽
    '\u{2722}', // ✢
    '\u{00B7}', // ·
    '\u{25D0}', // ◐
    '\u{25D3}', // ◓
    '\u{25D1}', // ◑
    '\u{25D2}', // ◒
];

/// Classify a guest window title. Returns the recognised profile and why.
///
/// Path-like tokens (anything with a `\` or `/`) are skipped for needle
/// matching so a shell prompt that sets the title to the cwd
/// (`C:\Users\me\.claude\projects`) does not count as Claude. A bare
/// executable path (`...\claude.exe`) is collapsed to its stem first,
/// which is how ConPTY reports a freshly started program.
pub fn classify_title(title: &str) -> Option<(&'static AgentProfile, TitleMatch)> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first = trimmed.chars().next()?;
    if CLAUDE_TITLE_GLYPHS.contains(&first) {
        return profile("claude").map(|p| (p, TitleMatch::Glyph));
    }
    let lowered = exe_stem(trimmed).to_ascii_lowercase();
    for token in lowered.split_whitespace() {
        if token.contains('\\') || token.contains('/') {
            continue;
        }
        for profile in PROFILES {
            if profile
                .title_needles
                .iter()
                .any(|needle| contains_word(token, needle))
            {
                return Some((profile, TitleMatch::Needle));
            }
        }
    }
    None
}

/// `...\claude.exe` -> `claude`; anything else unchanged.
fn exe_stem(title: &str) -> &str {
    let looks_like_exe_path = title.to_ascii_lowercase().ends_with(".exe")
        && !title.contains(' ')
        && (title.contains('\\') || title.contains('/'));
    if looks_like_exe_path {
        if let Some(stem) = std::path::Path::new(title)
            .file_stem()
            .and_then(|s| s.to_str())
        {
            return stem;
        }
    }
    title
}

/// Whole-word containment: `needle` bounded by non-alphanumerics (or the
/// token ends). `claude-code` and `claude.exe` match `claude`;
/// `concluded` does not.
fn contains_word(token: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = token[start..].find(needle) {
        let at = start + pos;
        let end = at + needle.len();
        let before_ok = at == 0
            || !token[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric());
        let after_ok = end == token.len()
            || !token[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = at + 1;
        if start >= token.len() {
            break;
        }
    }
    false
}

/// Parse a profile id typed by a user (CLI argument, dispatch suffix).
/// Case-insensitive and tolerant of the common aliases; only launchable
/// profiles are accepted because the caller is about to start one.
pub fn parse_launchable_id(raw: &str) -> Option<&'static AgentProfile> {
    let key = raw.trim().to_ascii_lowercase();
    let id = match key.as_str() {
        "claude-code" | "claudecode" | "cc" => "claude",
        "gemini-cli" => "gemini",
        "copilot-cli" => "copilot",
        other => other,
    };
    profile(id).filter(|p| p.launchable())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classified(title: &str) -> Option<(&'static str, TitleMatch)> {
        classify_title(title).map(|(p, m)| (p.id, m))
    }

    #[test]
    fn claude_status_glyph_titles_classify_as_claude() {
        assert_eq!(
            classified("\u{2733} Claude Code"),
            Some(("claude", TitleMatch::Glyph))
        );
        assert_eq!(
            classified("\u{25D0} Refactoring the sidebar"),
            Some(("claude", TitleMatch::Glyph))
        );
        assert_eq!(
            classified("  \u{273B} thinking"),
            Some(("claude", TitleMatch::Glyph))
        );
    }

    #[test]
    fn plain_agent_names_classify_by_needle() {
        assert_eq!(
            classified("Claude Code"),
            Some(("claude", TitleMatch::Needle))
        );
        assert_eq!(classified("codex"), Some(("codex", TitleMatch::Needle)));
        assert_eq!(
            classified("Codex \u{2014} C:\\dev\\project"),
            Some(("codex", TitleMatch::Needle))
        );
        assert_eq!(classified("gemini"), Some(("gemini", TitleMatch::Needle)));
        assert_eq!(
            classified("aider v0.86"),
            Some(("aider", TitleMatch::Needle))
        );
        assert_eq!(
            classified("opencode: fix tests"),
            Some(("opencode", TitleMatch::Needle))
        );
        assert_eq!(
            classified("my openrouter agent"),
            Some(("openrouter", TitleMatch::Needle))
        );
    }

    #[test]
    fn exe_paths_collapse_to_the_program_stem() {
        assert_eq!(
            classified(r"C:\Users\me\.local\bin\claude.exe"),
            Some(("claude", TitleMatch::Needle))
        );
        assert_eq!(
            classified(r"C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe"),
            None
        );
    }

    #[test]
    fn cwd_prompts_mentioning_an_agent_directory_are_not_agents() {
        assert_eq!(classified(r"C:\Users\me\.claude\projects"), None);
        assert_eq!(classified("~/.codex/sessions"), None);
        assert_eq!(classified(r"PS C:\dev\claude-tools"), None);
    }

    #[test]
    fn needles_match_whole_words_only() {
        assert_eq!(classified("concluded"), None);
        assert_eq!(classified("decodex"), None);
        assert_eq!(
            classified("claude-code"),
            Some(("claude", TitleMatch::Needle))
        );
        assert_eq!(classified("[codex]"), Some(("codex", TitleMatch::Needle)));
    }

    #[test]
    fn ordinary_shell_titles_are_not_agents() {
        for title in [
            "",
            "   ",
            "Windows PowerShell",
            "bash",
            "cargo build --release",
            "vim src/main.rs",
            "Administrator: cmd",
        ] {
            assert_eq!(classified(title), None, "{title:?}");
        }
    }

    #[test]
    fn profile_ids_are_unique_and_launchables_lead() {
        let mut seen = std::collections::HashSet::new();
        for p in PROFILES {
            assert!(seen.insert(p.id), "duplicate profile id {}", p.id);
            assert!(!p.label.is_empty());
            assert!(!p.title_needles.is_empty());
        }
        let first_unlaunchable = PROFILES.iter().position(|p| !p.launchable());
        if let Some(pos) = first_unlaunchable {
            assert!(
                PROFILES[pos..].iter().all(|p| !p.launchable()),
                "launchable profiles must precede title-only ones"
            );
        }
        assert!(profile(DEFAULT_PROFILE_ID).is_some_and(|p| p.launchable()));
    }

    #[test]
    fn restore_kinds_map_back_to_profiles() {
        assert_eq!(
            profile_for_restore_kind(crate::agent_restore::AgentKind::Claude).map(|p| p.id),
            Some("claude")
        );
        assert_eq!(
            profile_for_restore_kind(crate::agent_restore::AgentKind::Codex).map(|p| p.id),
            Some("codex")
        );
    }

    #[test]
    fn parse_launchable_id_accepts_aliases_and_rejects_title_only() {
        assert_eq!(parse_launchable_id("Claude").map(|p| p.id), Some("claude"));
        assert_eq!(parse_launchable_id("cc").map(|p| p.id), Some("claude"));
        assert_eq!(parse_launchable_id(" codex ").map(|p| p.id), Some("codex"));
        assert!(parse_launchable_id("openrouter").is_none());
        assert!(parse_launchable_id("nope").is_none());
    }

    #[test]
    fn program_name_adds_cmd_shim_on_windows() {
        let claude = profile("claude").unwrap();
        if cfg!(windows) {
            assert_eq!(claude.program_name(), "claude.cmd");
        } else {
            assert_eq!(claude.program_name(), "claude");
        }
        assert_eq!(profile("openrouter").unwrap().program_name(), "");
    }

    #[test]
    fn tag_serializes_with_snake_case_source() {
        let tag = AgentTag::new("claude", AgentTagSource::Title);
        let json = serde_json::to_string(&tag).unwrap();
        assert_eq!(json, r#"{"profile":"claude","source":"title"}"#);
        let back: AgentTag = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tag);
        assert_eq!(tag.label(), "Claude Code");
        assert_eq!(
            AgentTag::new("future", AgentTagSource::Hook).label(),
            "future"
        );
    }

    #[test]
    fn default_and_menu_profiles_never_come_back_empty() {
        assert!(default_profile().launchable());
        assert!(!menu_profiles().is_empty());
        assert!(menu_profiles().iter().all(|p| p.launchable()));
    }
}
