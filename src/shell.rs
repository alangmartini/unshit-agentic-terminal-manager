//! Shell selection types and resolution.
//!
//! `ShellSpec` carries the program (path or PATH lookup name) plus its
//! launch args. Resolution prefers a per workspace override over the app
//! wide default; both empty means "let the daemon's `default_shell()`
//! decide", preserving the pre feature behavior.

use serde::{Deserialize, Serialize};

/// A shell program plus its launch args. Stored in `workspaces.json`
/// and forwarded across IPC as `(shell, shell_args)`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellSpec {
    /// Absolute path or PATH lookup name. Empty means "fall back".
    pub program: String,
    /// Args forwarded to the program before any daemon side cwd args.
    #[serde(default)]
    pub args: Vec<String>,
}

impl ShellSpec {
    /// Returns true when `program` is empty, regardless of args. An
    /// empty `program` is treated as "no preference set".
    pub fn is_empty(&self) -> bool {
        self.program.is_empty()
    }
}

/// Resolve which shell a pane should spawn with. Workspace override
/// wins over the app wide default; both `None` (or both `is_empty`)
/// means "let the daemon decide", preserving today's behavior.
pub fn resolve(workspace: Option<&ShellSpec>, app: Option<&ShellSpec>) -> Option<ShellSpec> {
    workspace
        .filter(|s| !s.is_empty())
        .or(app.filter(|s| !s.is_empty()))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: &[&str]) -> ShellSpec {
        ShellSpec {
            program: program.into(),
            args: args.iter().map(|a| (*a).into()).collect(),
        }
    }

    #[test]
    fn is_empty_returns_true_for_default() {
        assert!(ShellSpec::default().is_empty());
    }

    #[test]
    fn is_empty_returns_true_when_program_is_blank_even_with_args() {
        let s = spec("", &["--login"]);
        assert!(
            s.is_empty(),
            "is_empty should look only at program; args alone do not make a spec set"
        );
    }

    #[test]
    fn is_empty_returns_false_when_program_is_set() {
        let s = spec("pwsh.exe", &[]);
        assert!(!s.is_empty());
    }

    #[test]
    fn resolve_returns_workspace_override_when_both_set() {
        let ws = spec("pwsh.exe", &["-NoLogo"]);
        let app = spec("powershell.exe", &[]);
        let got = resolve(Some(&ws), Some(&app));
        assert_eq!(got, Some(ws));
    }

    #[test]
    fn resolve_falls_back_to_app_default_when_workspace_is_empty() {
        let ws = ShellSpec::default();
        let app = spec("pwsh.exe", &["-NoLogo"]);
        let got = resolve(Some(&ws), Some(&app));
        assert_eq!(got, Some(app));
    }

    #[test]
    fn resolve_returns_none_when_both_are_empty() {
        let ws = ShellSpec::default();
        let app = ShellSpec::default();
        let got = resolve(Some(&ws), Some(&app));
        assert!(
            got.is_none(),
            "both empty must yield None so the daemon falls back to default_shell()"
        );
    }

    #[test]
    fn resolve_returns_none_when_both_are_unset() {
        let got = resolve(None, None);
        assert!(got.is_none());
    }

    #[test]
    fn resolve_falls_back_to_app_when_workspace_is_none() {
        let app = spec("bash", &["--login"]);
        let got = resolve(None, Some(&app));
        assert_eq!(got, Some(app));
    }

    #[test]
    fn shell_spec_round_trips_through_serde_json() {
        let original = spec("pwsh.exe", &["-NoLogo", "-NoProfile"]);
        let s = serde_json::to_string(&original).unwrap();
        let back: ShellSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn shell_spec_deserializes_with_default_args_when_field_is_missing() {
        // Old configs (or hand edited ones) may omit the args field
        // entirely. Serde must default it to an empty vector so the
        // upgrade path is silent.
        let json = r#"{"program":"pwsh.exe"}"#;
        let got: ShellSpec = serde_json::from_str(json).unwrap();
        assert_eq!(got.program, "pwsh.exe");
        assert!(
            got.args.is_empty(),
            "missing args field must deserialize to an empty vector"
        );
    }
}
