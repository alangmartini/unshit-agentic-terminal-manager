//! Identification of agent executables and runtime entrypoints, never prompts.

use super::{profile, AgentProfile};

pub(super) fn executable_stem(image: &str) -> String {
    let name = image
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(image)
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

/// Only these runtimes need a command-line query. Shell arguments are not
/// proof that an agent is running: the shell may be idle or executing a script.
pub fn needs_command_line(image: &str) -> bool {
    let stem = executable_stem(image);
    matches!(stem.as_str(), "node" | "bun" | "python" | "pythonw")
        || stem.strip_prefix("python").is_some_and(|version| {
            !version.is_empty() && version.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
}

pub fn classify_process(image: &str, command_line: Option<&str>) -> Option<&'static AgentProfile> {
    let stem = executable_stem(image);
    if let Some(agent) = profile(&stem) {
        return Some(agent);
    }
    if stem == "gh-copilot" {
        return profile("copilot");
    }
    if !needs_command_line(image) {
        return None;
    }
    let mut remaining = command_line?;
    next_argument(&mut remaining)?; // runtime executable
    let mut entrypoint = next_argument(&mut remaining)?;
    if stem.starts_with("python") {
        while matches!(entrypoint, "-u" | "-B" | "-E" | "-s" | "-S" | "-I") {
            entrypoint = next_argument(&mut remaining)?;
        }
        if entrypoint == "-m" {
            return (next_argument(&mut remaining)? == "aider")
                .then(|| profile("aider"))
                .flatten();
        }
        let script = entrypoint.replace('\\', "/").to_ascii_lowercase();
        return (script.ends_with("/scripts/aider") || script.ends_with("/bin/aider"))
            .then(|| profile("aider"))
            .flatten();
    }
    // Skip known switches that cannot consume an entrypoint. Unknown options
    // fail closed instead of scanning subsequent prompt/JavaScript arguments.
    while matches!(entrypoint, "--no-warnings" | "--enable-source-maps" | "--") {
        entrypoint = next_argument(&mut remaining)?;
    }
    let script = entrypoint.replace('\\', "/").to_ascii_lowercase();
    for (suffix, id) in [
        ("/node_modules/@openai/codex/bin/codex.js", "codex"),
        ("/node_modules/@anthropic-ai/claude-code/cli.js", "claude"),
        ("/node_modules/@github/copilot/index.js", "copilot"),
        ("/node_modules/@github/copilot/npm-loader.js", "copilot"),
        ("/node_modules/@google/gemini-cli/dist/index.js", "gemini"),
        ("/node_modules/opencode-ai/bin/opencode", "opencode"),
    ] {
        if script.ends_with(suffix) || script == suffix[1..] {
            return profile(id);
        }
    }
    None
}

/// Read only the command's executable and entrypoint. Preserve Windows path
/// separators and quoted spaces; reject malformed/embedded quoting. This is
/// intentionally not a shell parser and never executes or expands anything.
pub(super) fn next_argument<'a>(remaining: &mut &'a str) -> Option<&'a str> {
    let text = remaining.trim_start();
    if let Some(quoted) = text.strip_prefix('"').or_else(|| text.strip_prefix('\'')) {
        let quote = text.chars().next()?;
        let end = quoted.find(quote)?;
        let rest = &quoted[end + 1..];
        if rest.chars().next().is_some_and(|c| !c.is_whitespace()) {
            return None;
        }
        *remaining = rest;
        Some(&quoted[..end])
    } else {
        let end = text.find(char::is_whitespace).unwrap_or(text.len());
        let word = &text[..end];
        if word.is_empty() || word.contains(['"', '\'']) {
            return None;
        }
        *remaining = &text[end..];
        Some(word)
    }
}

#[cfg(test)]
mod tests {
    use super::classify_process;

    fn classified_id(image: &str, command_line: Option<&str>) -> Option<&'static str> {
        classify_process(image, command_line).map(|profile| profile.id)
    }

    #[test]
    fn native_agent_processes_are_recognized_without_window_titles() {
        for agent in [
            "claude",
            "codex",
            "gemini",
            "opencode",
            "aider",
            "copilot",
            "openrouter",
        ] {
            assert_eq!(classified_id(agent, None), Some(agent), "{agent}");
            assert_eq!(
                classified_id(&format!(r"C:\Program Files\Agents\{agent}.exe"), None),
                Some(agent),
                "Windows executable {agent}"
            );
            assert_eq!(
                classified_id(&format!("/usr/local/bin/{agent}"), None),
                Some(agent),
                "Unix executable {agent}"
            );
        }
        assert_eq!(classified_id("CODEX.EXE", None), Some("codex"));
    }

    #[test]
    fn npm_agent_entrypoints_are_recognized_through_runtime_wrappers() {
        for (script, expected) in [
            ("@openai/codex/bin/codex.js", "codex"),
            ("@github/copilot/index.js", "copilot"),
            ("@github/copilot/npm-loader.js", "copilot"),
            ("@google/gemini-cli/dist/index.js", "gemini"),
            ("@anthropic-ai/claude-code/cli.js", "claude"),
            ("opencode-ai/bin/opencode", "opencode"),
        ] {
            for runtime in ["node", "bun"] {
                let command = format!("{runtime} /opt/node_modules/{script}");
                assert_eq!(
                    classified_id(runtime, Some(&command)),
                    Some(expected),
                    "{command}"
                );
            }
        }
    }

    #[test]
    fn quoted_windows_runtime_and_package_paths_preserve_spaces() {
        let command = r#""C:\Program Files\nodejs\node.exe" "C:\Users\Alan Beelink\AppData\Roaming\npm\node_modules\@github\copilot\npm-loader.js" --resume"#;
        assert_eq!(
            classified_id(r"C:\Program Files\nodejs\node.exe", Some(command)),
            Some("copilot")
        );
        let command = r#""C:\Program Files\nodejs\node.exe" "C:\Users\Alan Beelink\AppData\Roaming\npm\node_modules\@openai\codex\bin\codex.js""#;
        assert_eq!(classified_id("node.exe", Some(command)), Some("codex"));
    }

    #[test]
    fn python_module_entrypoint_identifies_aider() {
        for (image, command) in [
            ("python", "python -m aider"),
            ("python3", "python3 -m aider --model openrouter/example"),
            (
                "python.exe",
                r#""C:\Program Files\Python\python.exe" -m aider"#,
            ),
        ] {
            assert_eq!(classified_id(image, Some(command)), Some("aider"));
        }
    }

    #[test]
    fn names_in_unrelated_processes_paths_and_prompt_arguments_do_not_match() {
        for (image, command) in [
            ("codex-helper.exe", "codex-helper.exe"),
            ("mycopilot", "mycopilot"),
            ("node", "node /projects/codex/server.js"),
            ("node", "node /projects/copilot/index.js"),
            ("node", "node /projects/app.js --prompt codex"),
            (
                "node",
                "node /projects/app.js /opt/node_modules/@openai/codex/bin/codex.js",
            ),
            (
                "node",
                "node /opt/node_modules/@openai/codex/bin/unrelated.js",
            ),
            ("python", "python app.py -m aider"),
            ("python", "python -m aider_tools"),
            ("python", "python -c 'print(\"aider\")'"),
            ("pwsh.exe", "pwsh.exe -Command codex"),
            ("cmd.exe", "cmd.exe /c copilot"),
            ("bash", "bash -c claude"),
        ] {
            assert_eq!(classified_id(image, Some(command)), None, "{command}");
        }
    }

    #[test]
    fn runtime_without_readable_command_line_is_not_an_agent() {
        for runtime in ["node", "node.exe", "bun", "python", "python3"] {
            assert_eq!(classified_id(runtime, None), None);
            assert_eq!(classified_id(runtime, Some("")), None);
        }
    }
}
