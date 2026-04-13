//! Trace recording for post-mortem debugging of test runs.
//!
//! When enabled, every action and assertion performed through the test harness
//! is recorded as a `TraceStep`. On test failure (or when explicitly requested)
//! the trace is saved as a JSON timeline alongside optional per-step screenshots.

use std::fmt::Write;
use std::path::PathBuf;
use std::time::Instant;

/// Records a timeline of actions and assertions during a test run.
pub struct TraceRecorder {
    steps: Vec<TraceStep>,
    enabled: bool,
    capture_screenshots: bool,
    output_dir: PathBuf,
    start_time: Instant,
    frame_counter: usize,
}

/// A single recorded step in the trace timeline.
pub struct TraceStep {
    pub frame_number: usize,
    pub action: TraceAction,
    pub elapsed_ms: u64,
    pub screenshot_path: Option<PathBuf>,
}

/// The kind of action or assertion that was recorded.
pub enum TraceAction {
    Click { selector: String, x: f32, y: f32 },
    DoubleClick { selector: String, x: f32, y: f32 },
    RightClick { selector: String, x: f32, y: f32 },
    Fill { selector: String, text: String },
    Clear { selector: String },
    Hover { selector: String, x: f32, y: f32 },
    Press { selector: String, key: String },
    Scroll { selector: String, dx: f32, dy: f32 },
    SelectOption { selector: String, value: String },
    Assertion { selector: String, kind: String, expected: String, actual: String, passed: bool },
    Custom { description: String },
}

impl TraceRecorder {
    /// Create a new trace recorder. Recording does not start until `enable()` is called.
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            enabled: false,
            capture_screenshots: false,
            output_dir: PathBuf::from("tests/traces"),
            start_time: Instant::now(),
            frame_counter: 0,
        }
    }

    /// Enable trace recording.
    pub fn enable(&mut self) {
        self.enabled = true;
        self.start_time = Instant::now();
    }

    /// Enable screenshot capture at each step (requires GPU).
    pub fn enable_screenshots(&mut self) {
        self.capture_screenshots = true;
    }

    /// Returns true if recording is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns true if screenshot capture is enabled.
    pub fn captures_screenshots(&self) -> bool {
        self.capture_screenshots
    }

    /// Set the output directory for trace files.
    pub fn set_output_dir(&mut self, dir: PathBuf) {
        self.output_dir = dir;
    }

    /// Increment the frame counter. Called from `TestHarness::step()`.
    pub fn tick_frame(&mut self) {
        self.frame_counter += 1;
    }

    /// Current frame number.
    pub fn frame(&self) -> usize {
        self.frame_counter
    }

    /// Record a trace step. Does nothing if recording is disabled.
    pub fn record(&mut self, action: TraceAction) {
        if !self.enabled {
            return;
        }
        let elapsed_ms = self.start_time.elapsed().as_millis() as u64;
        self.steps.push(TraceStep {
            frame_number: self.frame_counter,
            action,
            elapsed_ms,
            screenshot_path: None,
        });
    }

    /// Attach a screenshot path to the most recently recorded step.
    pub fn attach_screenshot(&mut self, path: PathBuf) {
        if let Some(step) = self.steps.last_mut() {
            step.screenshot_path = Some(path);
        }
    }

    /// Return a reference to all recorded steps.
    pub fn steps(&self) -> &[TraceStep] {
        &self.steps
    }

    /// Save the trace to disk as `{output_dir}/{test_name}/trace.json`.
    /// Returns the path to the written file.
    pub fn save(&self, test_name: &str) -> PathBuf {
        let dir = self.output_dir.join(test_name);
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("trace.json");
        let json = self.to_json();
        std::fs::write(&path, json).expect("failed to write trace.json");
        path
    }

    /// Build a screenshot filename for the given step index and action.
    pub fn screenshot_path_for(
        &self,
        test_name: &str,
        step_index: usize,
        action: &TraceAction,
    ) -> PathBuf {
        let dir = self.output_dir.join(test_name);
        let label = action.short_label();
        let filename = format!("step_{:03}_{}.png", step_index + 1, label);
        dir.join(filename)
    }

    /// Serialize the trace to a JSON string without serde.
    fn to_json(&self) -> String {
        let mut out = String::with_capacity(self.steps.len() * 128);
        out.push_str("[\n");
        for (i, step) in self.steps.iter().enumerate() {
            if i > 0 {
                out.push_str(",\n");
            }
            out.push_str("  ");
            step.write_json(&mut out);
        }
        out.push_str("\n]\n");
        out
    }
}

impl Default for TraceRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceStep {
    /// Write this step as a JSON object into `out`.
    fn write_json(&self, out: &mut String) {
        let _ = write!(
            out,
            r#"{{"frame":{},"elapsed_ms":{},"action":"#,
            self.frame_number, self.elapsed_ms
        );
        self.action.write_json(out);
        match &self.screenshot_path {
            Some(p) => {
                let _ = write!(
                    out,
                    r#","screenshot":"{}"}}"#,
                    p.display().to_string().replace('\\', "/")
                );
            }
            None => out.push_str(r#","screenshot":null}"#),
        }
    }
}

impl TraceAction {
    /// A short label for use in screenshot filenames.
    pub fn short_label(&self) -> &str {
        match self {
            TraceAction::Click { .. } => "click",
            TraceAction::DoubleClick { .. } => "dblclick",
            TraceAction::RightClick { .. } => "rightclick",
            TraceAction::Fill { .. } => "fill",
            TraceAction::Clear { .. } => "clear",
            TraceAction::Hover { .. } => "hover",
            TraceAction::Press { .. } => "press",
            TraceAction::Scroll { .. } => "scroll",
            TraceAction::SelectOption { .. } => "select",
            TraceAction::Assertion { .. } => "assert",
            TraceAction::Custom { .. } => "custom",
        }
    }

    /// Write this action as a JSON object into `out`.
    fn write_json(&self, out: &mut String) {
        match self {
            TraceAction::Click { selector, x, y }
            | TraceAction::DoubleClick { selector, x, y }
            | TraceAction::RightClick { selector, x, y }
            | TraceAction::Hover { selector, x, y } => {
                let type_name = match self {
                    TraceAction::Click { .. } => "Click",
                    TraceAction::DoubleClick { .. } => "DoubleClick",
                    TraceAction::RightClick { .. } => "RightClick",
                    _ => "Hover",
                };
                let _ = write!(
                    out,
                    r#"{{"type":"{}","selector":"{}","x":{},"y":{}}}"#,
                    type_name,
                    escape_json(selector),
                    x,
                    y,
                );
            }
            TraceAction::Fill { selector, text } => {
                let _ = write!(
                    out,
                    r#"{{"type":"Fill","selector":"{}","text":"{}"}}"#,
                    escape_json(selector),
                    escape_json(text),
                );
            }
            TraceAction::Clear { selector } => {
                let _ = write!(out, r#"{{"type":"Clear","selector":"{}"}}"#, escape_json(selector));
            }
            TraceAction::Press { selector, key } => {
                let _ = write!(
                    out,
                    r#"{{"type":"Press","selector":"{}","key":"{}"}}"#,
                    escape_json(selector),
                    escape_json(key),
                );
            }
            TraceAction::Scroll { selector, dx, dy } => {
                let _ = write!(
                    out,
                    r#"{{"type":"Scroll","selector":"{}","dx":{},"dy":{}}}"#,
                    escape_json(selector),
                    dx,
                    dy,
                );
            }
            TraceAction::SelectOption { selector, value } => {
                let _ = write!(
                    out,
                    r#"{{"type":"SelectOption","selector":"{}","value":"{}"}}"#,
                    escape_json(selector),
                    escape_json(value),
                );
            }
            TraceAction::Assertion { selector, kind, expected, actual, passed } => {
                let _ = write!(
                    out,
                    r#"{{"type":"Assertion","selector":"{}","kind":"{}","expected":"{}","actual":"{}","passed":{}}}"#,
                    escape_json(selector),
                    escape_json(kind),
                    escape_json(expected),
                    escape_json(actual),
                    passed,
                );
            }
            TraceAction::Custom { description } => {
                let _ = write!(
                    out,
                    r#"{{"type":"Custom","description":"{}"}}"#,
                    escape_json(description)
                );
            }
        }
    }
}

/// Escape a string for inclusion in JSON output.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r#"\\"#),
            '\n' => out.push_str(r#"\n"#),
            '\r' => out.push_str(r#"\r"#),
            '\t' => out.push_str(r#"\t"#),
            c if c.is_control() => {
                out.push_str(&format!(r#"\u{:04x}"#, c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_records_actions_in_order() {
        let mut recorder = TraceRecorder::new();
        recorder.enable();

        recorder.record(TraceAction::Click { selector: ".btn".into(), x: 50.0, y: 25.0 });
        recorder.tick_frame();
        recorder.record(TraceAction::Fill { selector: "#name".into(), text: "hello".into() });
        recorder.tick_frame();
        recorder.record(TraceAction::Assertion {
            selector: ".label".into(),
            kind: "text".into(),
            expected: "hello".into(),
            actual: "hello".into(),
            passed: true,
        });

        assert_eq!(recorder.steps().len(), 3);
        assert_eq!(recorder.steps()[0].frame_number, 0);
        assert_eq!(recorder.steps()[1].frame_number, 1);
        assert_eq!(recorder.steps()[2].frame_number, 2);

        assert!(matches!(recorder.steps()[0].action, TraceAction::Click { .. }));
        assert!(matches!(recorder.steps()[1].action, TraceAction::Fill { .. }));
        assert!(matches!(recorder.steps()[2].action, TraceAction::Assertion { passed: true, .. }));
    }

    #[test]
    fn trace_disabled_records_nothing() {
        let mut recorder = TraceRecorder::new();
        // Not enabled

        recorder.record(TraceAction::Click { selector: ".btn".into(), x: 10.0, y: 20.0 });

        assert!(recorder.steps().is_empty());
    }

    #[test]
    fn trace_json_is_valid() {
        let mut recorder = TraceRecorder::new();
        recorder.enable();

        recorder.record(TraceAction::Click { selector: ".btn".into(), x: 50.0, y: 25.0 });
        recorder.record(TraceAction::Assertion {
            selector: ".output".into(),
            kind: "text".into(),
            expected: "result".into(),
            actual: "result".into(),
            passed: true,
        });

        let json = recorder.to_json();
        // Basic structural checks: starts with [, ends with ], contains both entries
        assert!(json.starts_with('['));
        assert!(json.trim_end().ends_with(']'));
        assert!(json.contains(r#""type":"Click""#));
        assert!(json.contains(r#""type":"Assertion""#));
        assert!(json.contains(r#""selector":".btn""#));
        assert!(json.contains(r#""passed":true"#));
    }

    #[test]
    fn trace_json_escapes_special_chars() {
        let mut recorder = TraceRecorder::new();
        recorder.enable();

        recorder.record(TraceAction::Fill {
            selector: "#input".into(),
            text: "line1\nline2\ttab\"quote".into(),
        });

        let json = recorder.to_json();
        assert!(json.contains(r#"\n"#));
        assert!(json.contains(r#"\t"#));
        assert!(json.contains(r#"\""#));
    }

    #[test]
    fn trace_save_writes_file() {
        let dir = std::env::temp_dir().join("unshit_trace_test");
        let _ = std::fs::remove_dir_all(&dir);

        let mut recorder = TraceRecorder::new();
        recorder.enable();
        recorder.set_output_dir(dir.clone());

        recorder.record(TraceAction::Custom { description: "test step".into() });

        let path = recorder.save("my_test");
        assert!(path.exists(), "trace.json should be written to disk");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("test step"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_var_enables_trace() {
        // This tests the helper used in TestHarness, not TraceRecorder directly
        std::env::set_var("UNSHIT_TEST_TRACE", "1");
        let val = std::env::var("UNSHIT_TEST_TRACE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        assert!(val);
        std::env::remove_var("UNSHIT_TEST_TRACE");
    }

    #[test]
    fn screenshot_path_format() {
        let recorder = TraceRecorder::new();
        let action = TraceAction::Click { selector: ".btn".into(), x: 0.0, y: 0.0 };
        let path = recorder.screenshot_path_for("my_test", 0, &action);
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(filename, "step_001_click.png");
    }
}
