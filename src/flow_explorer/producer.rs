//! The agent side of the Flow Explorer: the prompt the app hands to Claude
//! or Codex when the user asks it to explain a flow or review a change.
//!
//! The skill body ships in the repo (`assets/flow-explorer/SKILL.md`) and
//! is embedded verbatim, so the file users copy into `~/.claude/skills/`
//! and the prompt the app sends can never drift apart. The app only
//! prepends a short header (mode, request, output path); there is no
//! template substitution inside the skill.
//!
//! The full prompt is written to a file next to the future output and the
//! agent's argv carries one line pointing at it: the launch goes through
//! `claude.cmd` / `codex.cmd`, and cmd.exe cuts a command line at the
//! first newline.

use std::path::Path;
use std::sync::OnceLock;

use super::model::FlowMode;

/// The skill exactly as checked out, frontmatter included. Line endings
/// follow git's autocrlf setting; use [`skill`] for normalised text.
pub const SKILL: &str = include_str!("../../assets/flow-explorer/SKILL.md");

/// Longest request the dialog accepts; anything longer is a paste of the
/// wrong thing.
pub const MAX_REQUEST_CHARS: usize = 500;

/// The argv prompt is one short line; this guards against someone
/// growing it back into the multi-line prompt.
pub const MAX_LAUNCH_PROMPT_BYTES: usize = 1_000;

static SKILL_LF: OnceLock<String> = OnceLock::new();

/// The skill with `\n` line endings regardless of how git checked it out.
pub fn skill() -> &'static str {
    SKILL_LF.get_or_init(|| SKILL.replace("\r\n", "\n"))
}

/// The skill body without its YAML frontmatter.
pub fn skill_body() -> &'static str {
    let text = skill();
    let Some(rest) = text.strip_prefix("---") else {
        return text;
    };
    match rest.find("\n---") {
        Some(end) => rest[end + 4..].trim_start_matches(['\r', '\n']),
        None => text,
    }
}

/// One line, single spaces: the request is typed into a one-line input
/// but can arrive from a startup dispatch or a paste.
pub fn normalize_request(request: &str) -> String {
    request.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The request the agent gets when the user leaves the field blank.
pub fn default_request(mode: FlowMode) -> &'static str {
    match mode {
        FlowMode::Explain => "the main user-facing flow of this repository",
        FlowMode::Review => {
            "the current branch against the default branch (merge base..HEAD, including uncommitted changes)"
        }
    }
}

/// The full prompt for one launch. `request` is the user's text (a flow
/// name or a `base..head`); `output_path` is where the agent must write.
pub fn build_prompt(mode: FlowMode, request: &str, output_path: &Path) -> String {
    let request = normalize_request(request);
    let request = if request.is_empty() {
        default_request(mode)
    } else {
        request.as_str()
    };
    let mut prompt = String::with_capacity(skill().len() + 512);
    prompt.push_str("You are running the flow-explorer skill reproduced below.\n");
    prompt.push_str(&format!("Mode: {}\n", mode.as_str()));
    prompt.push_str(&format!("Request: {request}\n"));
    prompt.push_str(&format!("Write the result to: {}\n", output_path.display()));
    prompt.push_str(
        "Analyse the repository in the current working directory; it is a \
         read-only task (do not edit, commit or build). When the file is \
         written, say so in one line and stop.\n\n---\n\n",
    );
    prompt.push_str(skill_body());
    prompt
}

/// The one line the agent is started with: read the prompt file, write
/// the output. Both paths are under the profile's `flows/` directory.
pub fn launch_prompt(prompt_path: &Path, output_path: &Path) -> String {
    format!(
        "Read the file {} and follow the instructions in it exactly. It is a read-only \
         Flow Explorer task whose only deliverable is the JSON document it asks you to \
         write to {}.",
        prompt_path.display(),
        output_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn skill_ships_with_frontmatter_and_the_contract() {
        assert!(skill().starts_with("---\nname: flow-explorer\n"));
        let body = skill_body();
        assert!(
            !body.contains("name: flow-explorer"),
            "frontmatter stripped"
        );
        assert!(body.starts_with("# Flow Explorer producer"));
        for needle in [
            "schema_version",
            "handled_by",
            "hidden_children",
            "<path>.tmp",
            "calldiff",
            "\"error\"",
        ] {
            assert!(body.contains(needle), "skill body mentions {needle}");
        }
    }

    #[test]
    fn prompt_carries_mode_request_and_path() {
        let out = PathBuf::from("C:/data/flows/abc.json");
        let prompt = build_prompt(FlowMode::Review, "  main..feat/x  ", &out);
        assert!(prompt.starts_with("You are running the flow-explorer skill"));
        assert!(prompt.contains("Mode: review\n"));
        assert!(prompt.contains("Request: main..feat/x\n"));
        assert!(prompt.contains("Write the result to: C:/data/flows/abc.json\n"));
        assert!(prompt.ends_with(skill_body()));
        assert!(!prompt.contains('\r'), "normalised line endings");
    }

    #[test]
    fn blank_requests_get_a_mode_default_and_whitespace_collapses() {
        let out = PathBuf::from("x.json");
        assert!(build_prompt(FlowMode::Explain, "", &out)
            .contains("Request: the main user-facing flow of this repository\n"));
        assert!(build_prompt(FlowMode::Review, "   ", &out).contains("Request: the current branch"));
        assert!(build_prompt(FlowMode::Explain, "send\r\n  a\tprompt", &out)
            .contains("Request: send a prompt\n"));
        assert_eq!(normalize_request(" a \n b "), "a b");
    }

    #[test]
    fn launch_prompt_is_one_short_line_naming_both_files() {
        let prompt_path = PathBuf::from("C:/Users/Some One/AppData/flows/explain-1.prompt.md");
        let out = PathBuf::from("C:/Users/Some One/AppData/flows/explain-1.json");
        let line = launch_prompt(&prompt_path, &out);
        assert!(!line.contains('\n'), "argv prompt must be a single line");
        assert!(line.len() < MAX_LAUNCH_PROMPT_BYTES, "{} bytes", line.len());
        assert!(line.contains("explain-1.prompt.md"));
        assert!(line.ends_with("explain-1.json."));
    }
}
