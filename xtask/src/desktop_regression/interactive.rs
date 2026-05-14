use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::diagnostics::{write_json_artifact, DiagnosticClient};
use crate::desktop_regression::launcher::AppSession;
use crate::desktop_regression::screenshots::capture_screen;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveCommand {
    Snapshot,
    Events,
    Screenshot,
    Rerun,
    Note(String),
    Continue,
    Abort,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractiveDecision {
    Continue,
    Abort,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveEffect {
    Artifact { path: String },
    Note { text: String },
    Message { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveRunResult {
    pub decision: InteractiveDecision,
    pub artifacts: Vec<String>,
}

pub trait InteractiveRuntime {
    fn capture_snapshot(&mut self) -> Result<InteractiveEffect, String>;
    fn capture_events(&mut self) -> Result<InteractiveEffect, String>;
    fn capture_screenshot(&mut self) -> Result<InteractiveEffect, String>;
    fn rerun_last_assertion(&mut self) -> Result<InteractiveEffect, String>;
    fn close_app(&mut self) -> Result<InteractiveEffect, String>;
}

pub struct SuiteInteractiveRuntime<'a> {
    run_dir: &'a Path,
    suite_id: &'a str,
    diagnostics: Option<&'a DiagnosticClient>,
    session: &'a mut AppSession,
    snapshot_count: u32,
    events_count: u32,
    screenshot_count: u32,
}

impl<'a> SuiteInteractiveRuntime<'a> {
    pub fn new(
        run_dir: &'a Path,
        suite_id: &'a str,
        diagnostics: Option<&'a DiagnosticClient>,
        session: &'a mut AppSession,
    ) -> Self {
        Self {
            run_dir,
            suite_id,
            diagnostics,
            session,
            snapshot_count: 0,
            events_count: 0,
            screenshot_count: 0,
        }
    }
}

impl InteractiveRuntime for SuiteInteractiveRuntime<'_> {
    fn capture_snapshot(&mut self) -> Result<InteractiveEffect, String> {
        let Some(diagnostics) = self.diagnostics else {
            return Ok(InteractiveEffect::Note {
                text: "snapshot unavailable: diagnostics are not enabled".to_owned(),
            });
        };

        self.snapshot_count += 1;
        let snapshot = diagnostics.snapshot("interactive failure snapshot")?;
        let artifact = write_json_artifact(
            self.run_dir,
            self.suite_id,
            &format!("interactive-snapshot-{}", self.snapshot_count),
            &snapshot,
        )?;
        Ok(InteractiveEffect::Artifact { path: artifact })
    }

    fn capture_events(&mut self) -> Result<InteractiveEffect, String> {
        let Some(diagnostics) = self.diagnostics else {
            return Ok(InteractiveEffect::Note {
                text: "event tail unavailable: diagnostics are not enabled".to_owned(),
            });
        };

        self.events_count += 1;
        let (_, flush_dropped) = diagnostics.flush()?;
        let (events, drain_dropped) = diagnostics.drain_events()?;
        let artifact = suite_artifact_name(
            self.suite_id,
            &format!("interactive-events-{}", self.events_count),
            "jsonl",
        );
        let mut body = String::new();
        for event in events {
            let line = serde_json::to_string(&event)
                .map_err(|e| format!("failed to serialize diagnostic event: {e}"))?;
            body.push_str(&line);
            body.push('\n');
        }
        if flush_dropped > 0 || drain_dropped > 0 {
            body.push_str(&format!(
                "{{\"type\":\"dropped-events\",\"flush_dropped\":{flush_dropped},\"drain_dropped\":{drain_dropped}}}\n"
            ));
        }
        std::fs::write(self.run_dir.join(&artifact), body)
            .map_err(|e| format!("failed to write {artifact}: {e}"))?;
        Ok(InteractiveEffect::Artifact { path: artifact })
    }

    fn capture_screenshot(&mut self) -> Result<InteractiveEffect, String> {
        self.screenshot_count += 1;
        let artifact = suite_artifact_name(
            self.suite_id,
            &format!("interactive-screenshot-{}", self.screenshot_count),
            "png",
        );
        capture_screen(&self.run_dir.join(&artifact))?;
        Ok(InteractiveEffect::Artifact { path: artifact })
    }

    fn rerun_last_assertion(&mut self) -> Result<InteractiveEffect, String> {
        Ok(InteractiveEffect::Note {
            text: "rerun last assertion is unsupported in interactive v1; original failure is preserved in results and the failure manifest".to_owned(),
        })
    }

    fn close_app(&mut self) -> Result<InteractiveEffect, String> {
        self.session.close_now()?;
        Ok(InteractiveEffect::Message {
            text: "app close requested".to_owned(),
        })
    }
}

pub fn parse_interactive_command(input: &str) -> Result<InteractiveCommand, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("enter a command".to_owned());
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default().to_ascii_lowercase();
    let rest = parts.next().unwrap_or_default().trim();

    match command.as_str() {
        "snapshot" => Ok(InteractiveCommand::Snapshot),
        "events" | "tail" => Ok(InteractiveCommand::Events),
        "screenshot" => Ok(InteractiveCommand::Screenshot),
        "rerun" => Ok(InteractiveCommand::Rerun),
        "note" if rest.is_empty() => Err("note requires text".to_owned()),
        "note" => Ok(InteractiveCommand::Note(rest.to_owned())),
        "continue" => Ok(InteractiveCommand::Continue),
        "abort" => Ok(InteractiveCommand::Abort),
        "close" => Ok(InteractiveCommand::Close),
        other => Err(format!(
            "unknown interactive command '{other}' (expected snapshot|events|tail|screenshot|rerun|note|continue|abort|close)"
        )),
    }
}

#[cfg(test)]
fn run_scripted_interactive_commands<I>(
    commands: I,
    run_dir: &Path,
    suite_id: &str,
    runtime: &mut dyn InteractiveRuntime,
) -> Result<InteractiveRunResult, String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut sink = std::io::sink();
    let mut notes = InteractiveNotes::new(run_dir, suite_id);
    let mut artifacts = Vec::new();

    for raw in commands {
        let command = parse_interactive_command(raw.as_ref())?;
        if let Some(decision) =
            execute_interactive_command(command, runtime, &mut notes, &mut artifacts, &mut sink)?
        {
            finalize_notes(&mut notes, &mut artifacts);
            return Ok(InteractiveRunResult {
                decision,
                artifacts,
            });
        }
    }

    Err("interactive command stream ended before continue, abort, or close".to_owned())
}

pub fn prompt_interactive_failure(
    run_dir: &Path,
    suite_id: &str,
    runtime: &mut dyn InteractiveRuntime,
) -> Result<InteractiveRunResult, String> {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    prompt_interactive_failure_with_io(run_dir, suite_id, runtime, &mut reader, &mut writer)
}

pub fn prompt_interactive_failure_with_io(
    run_dir: &Path,
    suite_id: &str,
    runtime: &mut dyn InteractiveRuntime,
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
) -> Result<InteractiveRunResult, String> {
    let mut notes = InteractiveNotes::new(run_dir, suite_id);
    let mut artifacts = Vec::new();
    writeln!(
        writer,
        "desktop-regression interactive failure pause for {suite_id}"
    )
    .map_err(|e| format!("failed to write interactive prompt: {e}"))?;
    writeln!(
        writer,
        "commands: snapshot, events/tail, screenshot, rerun, note <text>, continue, abort, close"
    )
    .map_err(|e| format!("failed to write interactive prompt: {e}"))?;

    loop {
        write!(writer, "desktop-regression[{suite_id}]> ")
            .map_err(|e| format!("failed to write interactive prompt: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("failed to flush interactive prompt: {e}"))?;

        let mut input = String::new();
        let read = reader
            .read_line(&mut input)
            .map_err(|e| format!("failed to read interactive command: {e}"))?;
        if read == 0 {
            let note = "interactive input ended; aborting failure workflow".to_owned();
            notes.append_system_note(&note)?;
            finalize_notes(&mut notes, &mut artifacts);
            return Ok(InteractiveRunResult {
                decision: InteractiveDecision::Abort,
                artifacts,
            });
        }

        let command = match parse_interactive_command(&input) {
            Ok(command) => command,
            Err(err) => {
                writeln!(writer, "{err}")
                    .map_err(|e| format!("failed to write interactive error: {e}"))?;
                continue;
            }
        };

        if let Some(decision) =
            execute_interactive_command(command, runtime, &mut notes, &mut artifacts, writer)?
        {
            finalize_notes(&mut notes, &mut artifacts);
            return Ok(InteractiveRunResult {
                decision,
                artifacts,
            });
        }
    }
}

fn execute_interactive_command(
    command: InteractiveCommand,
    runtime: &mut dyn InteractiveRuntime,
    notes: &mut InteractiveNotes,
    artifacts: &mut Vec<String>,
    writer: &mut dyn Write,
) -> Result<Option<InteractiveDecision>, String> {
    let effect = match command {
        InteractiveCommand::Snapshot => Some(runtime.capture_snapshot()?),
        InteractiveCommand::Events => Some(runtime.capture_events()?),
        InteractiveCommand::Screenshot => Some(runtime.capture_screenshot()?),
        InteractiveCommand::Rerun => Some(runtime.rerun_last_assertion()?),
        InteractiveCommand::Note(text) => {
            notes.append_user_note(&text)?;
            Some(InteractiveEffect::Message {
                text: format!("recorded note: {text}"),
            })
        }
        InteractiveCommand::Continue => {
            notes
                .append_system_note("continue selected; app cleanup will run via suite teardown")?;
            return Ok(Some(InteractiveDecision::Continue));
        }
        InteractiveCommand::Abort => {
            notes.append_system_note("abort selected; app cleanup will run via suite teardown")?;
            return Ok(Some(InteractiveDecision::Abort));
        }
        InteractiveCommand::Close => {
            let effect = runtime.close_app()?;
            apply_effect(effect, notes, artifacts, writer)?;
            notes
                .append_system_note("close selected; app was explicitly closed before teardown")?;
            return Ok(Some(InteractiveDecision::Close));
        }
    };

    if let Some(effect) = effect {
        apply_effect(effect, notes, artifacts, writer)?;
    }
    Ok(None)
}

fn apply_effect(
    effect: InteractiveEffect,
    notes: &mut InteractiveNotes,
    artifacts: &mut Vec<String>,
    writer: &mut dyn Write,
) -> Result<(), String> {
    match effect {
        InteractiveEffect::Artifact { path } => {
            push_artifact_once(artifacts, path.clone());
            writeln!(writer, "wrote artifact: {path}")
                .map_err(|e| format!("failed to write interactive output: {e}"))?;
        }
        InteractiveEffect::Note { text } => {
            notes.append_system_note(&text)?;
            writeln!(writer, "{text}")
                .map_err(|e| format!("failed to write interactive output: {e}"))?;
        }
        InteractiveEffect::Message { text } => {
            writeln!(writer, "{text}")
                .map_err(|e| format!("failed to write interactive output: {e}"))?;
        }
    }
    Ok(())
}

fn finalize_notes(notes: &mut InteractiveNotes, artifacts: &mut Vec<String>) {
    if let Some(name) = notes.artifact_name() {
        push_artifact_once(artifacts, name);
    }
}

fn push_artifact_once(artifacts: &mut Vec<String>, artifact: String) {
    if !artifacts.contains(&artifact) {
        artifacts.push(artifact);
    }
}

struct InteractiveNotes {
    artifact_name: String,
    path: PathBuf,
    written: bool,
}

impl InteractiveNotes {
    fn new(run_dir: &Path, suite_id: &str) -> Self {
        let artifact_name = suite_artifact_name(suite_id, "interactive-notes", "md");
        Self {
            path: run_dir.join(&artifact_name),
            artifact_name,
            written: false,
        }
    }

    fn append_user_note(&mut self, text: &str) -> Result<(), String> {
        self.append_line(&format!("- user: {text}"))
    }

    fn append_system_note(&mut self, text: &str) -> Result<(), String> {
        self.append_line(&format!("- system: {text}"))
    }

    fn append_line(&mut self, line: &str) -> Result<(), String> {
        let mut body = String::new();
        if !self.written {
            body.push_str("# Interactive failure notes\n\n");
        }
        body.push_str(line);
        body.push('\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| file.write_all(body.as_bytes()))
            .map_err(|e| format!("failed to write {}: {e}", self.path.display()))?;
        self.written = true;
        Ok(())
    }

    fn artifact_name(&self) -> Option<String> {
        self.written.then(|| self.artifact_name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeRuntime {
        close_called: bool,
    }

    impl InteractiveRuntime for FakeRuntime {
        fn capture_snapshot(&mut self) -> Result<InteractiveEffect, String> {
            Ok(InteractiveEffect::Note {
                text: "snapshot unavailable: diagnostics are not enabled".to_owned(),
            })
        }

        fn capture_events(&mut self) -> Result<InteractiveEffect, String> {
            Ok(InteractiveEffect::Artifact {
                path: "suite-interactive-events.jsonl".to_owned(),
            })
        }

        fn capture_screenshot(&mut self) -> Result<InteractiveEffect, String> {
            Ok(InteractiveEffect::Artifact {
                path: "suite-interactive-screenshot-1.png".to_owned(),
            })
        }

        fn rerun_last_assertion(&mut self) -> Result<InteractiveEffect, String> {
            Ok(InteractiveEffect::Note {
                text: "rerun last assertion is unsupported in interactive v1".to_owned(),
            })
        }

        fn close_app(&mut self) -> Result<InteractiveEffect, String> {
            self.close_called = true;
            Ok(InteractiveEffect::Message {
                text: "app close requested".to_owned(),
            })
        }
    }

    #[test]
    fn parses_interactive_commands() {
        assert_eq!(
            parse_interactive_command("snapshot").unwrap(),
            InteractiveCommand::Snapshot
        );
        assert_eq!(
            parse_interactive_command("tail").unwrap(),
            InteractiveCommand::Events
        );
        assert_eq!(
            parse_interactive_command("note inspect layout").unwrap(),
            InteractiveCommand::Note("inspect layout".to_owned())
        );
        assert_eq!(
            parse_interactive_command("continue").unwrap(),
            InteractiveCommand::Continue
        );
        assert!(parse_interactive_command("note").is_err());
        assert!(parse_interactive_command("bogus").is_err());
    }

    #[test]
    fn scripted_commands_write_notes_and_artifacts() {
        let dir =
            std::env::temp_dir().join(format!("xtask-dr-interactive-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut runtime = FakeRuntime::default();

        let result = run_scripted_interactive_commands(
            [
                "note saw stale rows",
                "snapshot",
                "screenshot",
                "rerun",
                "continue",
            ],
            &dir,
            "suite",
            &mut runtime,
        )
        .unwrap();

        assert_eq!(result.decision, InteractiveDecision::Continue);
        assert_eq!(
            result.artifacts,
            vec![
                "suite-interactive-screenshot-1.png".to_owned(),
                "suite-interactive-notes.md".to_owned()
            ]
        );
        let notes = std::fs::read_to_string(dir.join("suite-interactive-notes.md")).unwrap();
        assert!(notes.contains("- user: saw stale rows"));
        assert!(notes.contains("snapshot unavailable"));
        assert!(notes.contains("rerun last assertion is unsupported"));
        assert!(notes.contains("continue selected"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn close_command_requests_explicit_app_close() {
        let dir =
            std::env::temp_dir().join(format!("xtask-dr-interactive-close-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut runtime = FakeRuntime::default();

        let result =
            run_scripted_interactive_commands(["close"], &dir, "suite", &mut runtime).unwrap();

        assert_eq!(result.decision, InteractiveDecision::Close);
        assert!(runtime.close_called);
        let notes = std::fs::read_to_string(dir.join("suite-interactive-notes.md")).unwrap();
        assert!(notes.contains("close selected"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
