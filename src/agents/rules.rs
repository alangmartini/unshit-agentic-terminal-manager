//! User rules supplement built-in detection, without executing commands.

use super::process::{executable_stem, needs_command_line, next_argument};
use serde::Deserialize;

const MAX_FILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectionRules {
    rules: Vec<DetectionRule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetectionRule {
    profile: String,
    executable: String,
    #[serde(default)]
    args_prefix: Vec<String>,
}

pub fn valid_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.as_bytes()[0].is_ascii_alphanumeric()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

impl DetectionRules {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_FILE_BYTES {
            return Err("file exceeds 64 KiB".into());
        }
        // Do not echo configuration contents in diagnostics.
        let mut parsed: Self =
            serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| {
                format!(
                    "invalid configuration at line {}, column {}",
                    e.line(),
                    e.column()
                )
            })?;
        if parsed.rules.len() > 64 {
            return Err("at most 64 rules are allowed".into());
        }
        for (index, rule) in parsed.rules.iter_mut().enumerate() {
            let invalid = if !valid_profile_id(&rule.profile) {
                Some("profile must be a lowercase ID of 1–64 letters, digits, underscores or hyphens, starting with a letter or digit")
            } else if rule.executable.is_empty()
                || rule.executable.len() > 128
                || rule.executable.trim() != rule.executable
                || rule
                    .executable
                    .chars()
                    .any(|c| c.is_control() || "/\\:*?\"'<>|".contains(c))
                || executable_stem(&rule.executable).is_empty()
            {
                Some("executable must be a basename of 1–128 bytes, without paths or patterns")
            } else if rule.args_prefix.len() > 16
                || rule.args_prefix.iter().any(|arg| {
                    arg.is_empty() || arg.len() > 1024 || arg.chars().any(char::is_control)
                })
            {
                Some("args_prefix allows at most 16 nonempty arguments of at most 1024 bytes, without control characters")
            } else if rule.args_prefix.is_empty() && is_runtime_or_shell(&rule.executable) {
                Some("shells and runtimes require args_prefix to identify the harness")
            } else {
                None
            };
            if let Some(reason) = invalid {
                return Err(format!("rule {}: {reason}", index + 1));
            }
            rule.executable = executable_stem(&rule.executable);
        }
        Ok(parsed)
    }

    pub fn classify(&self, image: &str, command_line: Option<&str>) -> Option<&str> {
        let stem = executable_stem(image);
        self.rules
            .iter()
            .find(|rule| {
                if rule.executable != stem {
                    return false;
                }
                if rule.args_prefix.is_empty() {
                    return true;
                }
                let Some(mut remaining) = command_line else {
                    return false;
                };
                if next_argument(&mut remaining).is_none() {
                    return false;
                }
                rule.args_prefix.iter().all(|expected| {
                    next_argument(&mut remaining).is_some_and(|actual| {
                        if expected.contains(['/', '\\']) {
                            actual
                                .replace('\\', "/")
                                .eq_ignore_ascii_case(&expected.replace('\\', "/"))
                        } else {
                            actual == expected
                        }
                    })
                })
            })
            .map(|rule| rule.profile.as_str())
    }

    pub fn needs_command_line(&self, image: &str) -> bool {
        let stem = executable_stem(image);
        self.rules
            .iter()
            .any(|rule| rule.executable == stem && !rule.args_prefix.is_empty())
    }
}

fn is_runtime_or_shell(image: &str) -> bool {
    needs_command_line(image)
        || matches!(
            executable_stem(image).as_str(),
            "pwsh"
                | "powershell"
                | "cmd"
                | "bash"
                | "sh"
                | "zsh"
                | "fish"
                | "deno"
                | "java"
                | "dotnet"
                | "ruby"
                | "perl"
                | "php"
        )
}

pub struct RulesFile {
    path: Option<std::path::PathBuf>,
    pub rules: DetectionRules,
    last_text: Option<String>,
    last_error: Option<String>,
}

impl RulesFile {
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        Self {
            path,
            rules: DetectionRules::default(),
            last_text: None,
            last_error: None,
        }
    }

    /// Called only by the background monitor, outside the app-state lock.
    /// Bounded reads also detect edits whose length or timestamp did not change.
    pub fn reload(&mut self) -> Result<bool, String> {
        use std::io::Read;
        let Some(path) = &self.path else {
            return Ok(false);
        };
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let changed = !self.rules.rules.is_empty();
                self.rules = DetectionRules::default();
                self.last_text = None;
                return Ok(changed);
            }
            Err(error) => return Err(format!("cannot open rules file: {:?}", error.kind())),
        };
        let mut text = String::new();
        file.take((MAX_FILE_BYTES + 1) as u64)
            .read_to_string(&mut text)
            .map_err(|error| format!("cannot read rules file: {:?}", error.kind()))?;
        if self.last_text.as_ref() == Some(&text) {
            return Ok(false);
        }
        let parsed = DetectionRules::parse(&text)?;
        let changed = self.rules != parsed;
        self.rules = parsed;
        self.last_text = Some(text);
        Ok(changed)
    }

    /// Reload once per monitor tick. Transitions land in
    /// `agent-events.jsonl` as `agent.rules_loaded` (the rule set changed, or
    /// a rejected file became valid again) and `agent.rules_rejected` (the
    /// file could not be read or parsed; the previous rules stay in force).
    /// Nothing is emitted while the file is unchanged or stays broken, so a
    /// steady state costs no writes.
    pub fn refresh(&mut self) {
        use super::telemetry::{record, AgentEventRecord};
        match self.reload() {
            Ok(changed) => {
                if changed || self.last_error.is_some() {
                    log::info!(
                        "agent detection rules loaded: {} rules",
                        self.rules.rules.len()
                    );
                    let correlation_id = format!("process-{}", std::process::id());
                    let mut event =
                        AgentEventRecord::new("agent.rules_loaded", "info", &correlation_id);
                    event.rule_count = Some(self.rules.rules.len());
                    event.reason = Some(if self.last_error.is_some() {
                        "recovered"
                    } else {
                        "changed"
                    });
                    record(&event);
                }
                self.last_error = None;
            }
            Err(error) => {
                if self.last_error.as_ref() != Some(&error) {
                    log::warn!("agent-detection.json: {error}; keeping last valid rules");
                    let correlation_id = format!("process-{}", std::process::id());
                    let mut event =
                        AgentEventRecord::new("agent.rules_rejected", "warn", &correlation_id);
                    event.rule_count = Some(self.rules.rules.len());
                    event.detail = Some(error.clone());
                    record(&event);
                }
                self.last_error = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DetectionRules;
    use serde_json::json;

    #[test]
    fn rule_file_reloads_keeps_last_good_on_error_and_clears_on_deletion() {
        let path = std::env::temp_dir().join(format!(
            "tm-agent-rules-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut file = super::RulesFile::new(Some(path.clone()));
        assert!(!file.reload().unwrap(), "missing file means no rules");
        std::fs::write(&path, rule_document("first", "custom", &[])).unwrap();
        assert!(file.reload().unwrap());
        assert_eq!(file.rules.classify("custom.exe", None), Some("first"));
        assert!(!file.reload().unwrap(), "unchanged file has no transition");
        std::fs::write(&path, "{partial edit").unwrap();
        assert!(file.reload().is_err());
        assert_eq!(file.rules.classify("custom.exe", None), Some("first"));
        std::fs::write(&path, "x".repeat(super::MAX_FILE_BYTES + 1)).unwrap();
        assert!(file.reload().is_err());
        assert_eq!(file.rules.classify("custom.exe", None), Some("first"));
        std::fs::write(&path, rule_document("second", "custom", &[])).unwrap();
        assert!(file.reload().unwrap());
        assert_eq!(file.rules.classify("custom.exe", None), Some("second"));
        std::fs::remove_file(&path).unwrap();
        assert!(file.reload().unwrap());
        assert_eq!(file.rules.classify("custom.exe", None), None);
    }

    #[test]
    fn shipped_rule_example_is_valid_and_utf8_bom_is_accepted() {
        let example = include_str!("../../assets/agent-detection.example.json");
        assert!(DetectionRules::parse(example).is_ok());
        assert!(DetectionRules::parse(&format!("\u{feff}{example}")).is_ok());
    }

    fn rule_document(profile: &str, executable: &str, args: &[&str]) -> String {
        json!({"rules": [{
            "profile": profile,
            "executable": executable,
            "args_prefix": args
        }]})
        .to_string()
    }

    #[test]
    fn native_rules_allow_custom_profiles_and_existing_profile_ids() {
        let rules = DetectionRules::parse(
            r#"{"rules":[{"profile":"my-agent_2","executable":"my-agent.exe"},{"profile":"openrouter","executable":"router-agent"}]}"#,
        )
        .unwrap();
        assert_eq!(
            rules.classify(r"C:\Program Files\Custom\MY-AGENT.EXE", None),
            Some("my-agent_2")
        );
        assert_eq!(rules.classify("my-agent", None), Some("my-agent_2"));
        assert_eq!(
            rules.classify("/usr/bin/router-agent", None),
            Some("openrouter")
        );
        assert_eq!(rules.classify("my-agent-helper.exe", None), None);
        assert_eq!(
            rules.classify("unrelated.exe", Some("unrelated.exe my-agent")),
            None
        );
    }

    #[test]
    fn runtime_prefix_matches_quoted_paths_and_preserves_argument_case() {
        let rules = DetectionRules::parse(&rule_document(
            "openrouter",
            "node.exe",
            &["C:/Tools/My Router/cli.js", "--mode", "Agent"],
        ))
        .unwrap();
        assert_eq!(
            rules.classify(
                "NODE.EXE",
                Some(r#""C:\Program Files\nodejs\node.exe" "c:\tools\my router\CLI.JS" --mode Agent --prompt "write code""#),
            ),
            Some("openrouter")
        );
        for command in [
            r#"node "C:/Tools/My Router/cli.js" --mode agent"#,
            r#"node "C:/Tools/My Router/cli.js" --MODE Agent"#,
            r#"node "C:/Tools/My Router/cli.js" --mode"#,
            r#"node unrelated.js "C:/Tools/My Router/cli.js" --mode Agent"#,
            r#"node --eval "C:/Tools/My Router/cli.js" --mode Agent"#,
            r#"node "C:/Tools/My Router/cli.js.bak" --mode Agent"#,
            r#"node "C:/Tools/My Router/cli.js"suffix --mode Agent"#,
            r#"node "C:/Tools/My Router/cli.js --mode Agent"#,
            "",
        ] {
            assert_eq!(rules.classify("node.exe", Some(command)), None, "{command}");
        }
        assert_eq!(rules.classify("node.exe", None), None);
        assert_eq!(
            rules.classify(
                "bun.exe",
                Some(r#"bun "C:/Tools/My Router/cli.js" --mode Agent"#)
            ),
            None
        );
    }

    #[test]
    fn first_matching_rule_wins_and_command_queries_are_scoped_to_images() {
        let rules = DetectionRules::parse(
            r#"{"rules":[
                {"profile":"first","executable":"node","args_prefix":["router.js"]},
                {"profile":"second","executable":"node.exe","args_prefix":["router.js","--special"]},
                {"profile":"native","executable":"custom-agent"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(
            rules.classify("node", Some("node router.js --special")),
            Some("first")
        );
        assert_eq!(rules.classify("node", Some("node ROUTER.JS")), None);
        assert!(rules.needs_command_line(r"C:\Program Files\nodejs\NODE.EXE"));
        assert!(!rules.needs_command_line("custom-agent.exe"));
        assert!(!rules.needs_command_line("python.exe"));
        assert!(!rules.needs_command_line("node-helper.exe"));
        let empty = DetectionRules::parse(r#"{"rules":[]}"#).unwrap();
        assert_eq!(empty.classify("codex.exe", None), None);
        assert!(!empty.needs_command_line("node.exe"));
    }

    #[test]
    fn invalid_profiles_and_executable_patterns_are_rejected() {
        for profile in [
            "",
            "Upper",
            "-leading",
            "_leading",
            "a.b",
            "two words",
            "a/b",
            "caf\u{e9}",
            "a\n",
        ] {
            assert!(
                DetectionRules::parse(&rule_document(profile, "agent", &[])).is_err(),
                "{profile:?}"
            );
        }
        assert!(DetectionRules::parse(&rule_document(&"a".repeat(64), "agent", &[])).is_ok());
        assert!(DetectionRules::parse(&rule_document(&"a".repeat(65), "agent", &[])).is_err());
        for executable in [
            "",
            "C:/tools/agent.exe",
            r"C:\tools\agent.exe",
            "agent*",
            "agent?",
            "\"agent\"",
            "'agent'",
            "agent\n",
            "agent\0",
        ] {
            assert!(
                DetectionRules::parse(&rule_document("custom", executable, &[])).is_err(),
                "{executable:?}"
            );
        }
        assert!(DetectionRules::parse(&rule_document("custom", &"a".repeat(128), &[])).is_ok());
        assert!(DetectionRules::parse(&rule_document("custom", &"a".repeat(129), &[])).is_err());
    }

    #[test]
    fn bare_shells_and_runtimes_require_a_discriminating_argument_prefix() {
        for executable in [
            "node",
            "NODE.EXE",
            "bun",
            "python",
            "python3",
            "python3.12",
            "pythonw",
            "pwsh",
            "powershell",
            "cmd",
            "bash",
            "sh",
            "zsh",
            "fish",
            "deno",
            "java",
            "dotnet",
            "ruby",
            "perl",
            "php",
        ] {
            assert!(
                DetectionRules::parse(&rule_document("custom", executable, &[])).is_err(),
                "{executable}"
            );
            let without_args =
                json!({"rules":[{"profile":"custom","executable":executable}]}).to_string();
            assert!(
                DetectionRules::parse(&without_args).is_err(),
                "omitted prefix: {executable}"
            );
            assert!(
                DetectionRules::parse(&rule_document("custom", executable, &["entrypoint"]))
                    .is_ok(),
                "specific prefix: {executable}"
            );
        }
    }

    #[test]
    fn configuration_limits_and_unknown_fields_fail_validation() {
        let args16 = vec!["argument"; 16];
        assert!(DetectionRules::parse(&rule_document("custom", "agent", &args16)).is_ok());
        assert!(
            DetectionRules::parse(&rule_document("custom", "agent", &["argument"; 17])).is_err()
        );
        assert!(DetectionRules::parse(&rule_document("custom", "agent", &[""])).is_err());
        assert!(
            DetectionRules::parse(&rule_document("custom", "agent", &[&"a".repeat(1024)])).is_ok()
        );
        assert!(
            DetectionRules::parse(&rule_document("custom", "agent", &[&"a".repeat(1025)])).is_err()
        );
        assert!(
            DetectionRules::parse(&rule_document("custom", "agent", &[&"\u{e9}".repeat(513)]))
                .is_err()
        );
        let rule = json!({"profile":"custom","executable":"agent"});
        assert!(DetectionRules::parse(&json!({"rules":vec![rule.clone();64]}).to_string()).is_ok());
        assert!(DetectionRules::parse(&json!({"rules":vec![rule;65]}).to_string()).is_err());
        for document in [
            r#"{"rules":[],"extra":true}"#,
            r#"{"rules":[{"profile":"custom","executable":"agent","arg_prefix":["script"]}]}"#,
            r#"{"rules":[{"profile":"custom"}]}"#,
            r#"{"rules":[{"executable":"agent"}]}"#,
            r#"{"rules":[{"profile":"custom","executable":"agent","args_prefix":"script"}]}"#,
            "not json",
        ] {
            assert!(DetectionRules::parse(document).is_err(), "{document}");
        }
    }
}
