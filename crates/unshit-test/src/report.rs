//! HTML test report generation with screenshots and diff images.
//!
//! Set `UNSHIT_TEST_REPORT=1` before running tests to enable report collection.
//! Reports are written as self-contained HTML files with inline CSS/JS and
//! base64-encoded screenshots.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::trace::TraceStep;

/// Status of a single test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

impl TestStatus {
    fn label(self) -> &'static str {
        match self {
            TestStatus::Passed => "passed",
            TestStatus::Failed => "failed",
            TestStatus::Skipped => "skipped",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            TestStatus::Passed => "&#x2714;",
            TestStatus::Failed => "&#x2718;",
            TestStatus::Skipped => "&#x25CB;",
        }
    }
}

/// A single test entry in the report.
pub struct TestReportEntry {
    pub name: String,
    pub status: TestStatus,
    pub duration: Duration,
    pub trace: Option<Vec<TraceStep>>,
    pub failure_message: Option<String>,
    pub screenshot_paths: Vec<PathBuf>,
}

/// Collects test entries and generates an HTML report.
pub struct TestReport {
    entries: Vec<TestReportEntry>,
    output_dir: PathBuf,
}

impl TestReport {
    /// Create a new report that will be written to `output_dir`.
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            entries: Vec::new(),
            output_dir: output_dir.into(),
        }
    }

    /// Add a test entry to the report.
    pub fn add_entry(&mut self, entry: TestReportEntry) {
        self.entries.push(entry);
    }

    /// Return a reference to all entries.
    pub fn entries(&self) -> &[TestReportEntry] {
        &self.entries
    }

    /// Returns true if report collection is enabled via the environment variable.
    pub fn is_enabled() -> bool {
        crate::test_app::env_is_truthy("UNSHIT_TEST_REPORT")
    }

    /// Generate the HTML report file. Returns the path to the written file.
    pub fn generate(&self) -> PathBuf {
        std::fs::create_dir_all(&self.output_dir).ok();
        let path = self.output_dir.join("index.html");
        let html = self.render_html();
        std::fs::write(&path, html).expect("failed to write report HTML");
        path
    }

    fn render_html(&self) -> String {
        let mut html = String::with_capacity(8192);

        let (passed, failed, skipped) = self.count_statuses();
        let total = self.entries.len();
        let total_duration = self.total_duration();

        html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
        html.push_str("<meta charset=\"utf-8\">\n");
        html.push_str("<title>Test Report</title>\n");
        html.push_str("<style>\n");
        write_css(&mut html);
        html.push_str("</style>\n</head>\n<body>\n");

        html.push_str("<div class=\"summary\">\n");
        let _ = write!(
            html,
            "<h1>Test Report</h1>\n\
             <div class=\"stats\">\
             <span class=\"stat total\">{total} total</span> \
             <span class=\"stat passed\">{passed} passed</span> \
             <span class=\"stat failed\">{failed} failed</span> \
             <span class=\"stat skipped\">{skipped} skipped</span> \
             <span class=\"stat duration\">{:.2}s</span>\
             </div>\n",
            total_duration.as_secs_f64(),
        );

        html.push_str(
            "<div class=\"filters\">\
             <button onclick=\"filterTests('all')\" class=\"active\">All</button>\
             <button onclick=\"filterTests('failed')\">Failed</button>\
             <button onclick=\"filterTests('passed')\">Passed</button>\
             </div>\n",
        );
        html.push_str("</div>\n");

        html.push_str("<div class=\"entries\" id=\"entries\">\n");
        for entry in &self.entries {
            self.render_entry(&mut html, entry);
        }
        html.push_str("</div>\n");

        html.push_str("<script>\n");
        write_js(&mut html);
        html.push_str("</script>\n");

        html.push_str("</body>\n</html>\n");
        html
    }

    fn render_entry(&self, html: &mut String, entry: &TestReportEntry) {
        let status = entry.status.label();
        let icon = entry.status.icon();

        let _ = write!(
            html,
            "<details class=\"entry {status}\">\n\
             <summary>\
             <span class=\"icon\">{icon}</span> \
             <span class=\"name\">{}</span> \
             <span class=\"duration\">{:.3}s</span>\
             </summary>\n\
             <div class=\"detail\">\n",
            escape_html(&entry.name),
            entry.duration.as_secs_f64(),
        );

        if let Some(msg) = &entry.failure_message {
            let _ = write!(
                html,
                "<div class=\"failure\"><pre>{}</pre></div>\n",
                escape_html(msg),
            );
        }

        if !entry.screenshot_paths.is_empty() {
            html.push_str("<div class=\"screenshots\">\n");
            for path in &entry.screenshot_paths {
                if let Some(data_uri) = encode_image_as_data_uri(path) {
                    let _ = write!(
                        html,
                        "<div class=\"screenshot\"><img src=\"{data_uri}\" alt=\"{}\"></div>\n",
                        escape_html(&path.display().to_string()),
                    );
                }
            }
            html.push_str("</div>\n");
        }

        if let Some(steps) = &entry.trace {
            if !steps.is_empty() {
                html.push_str("<div class=\"trace\">\n<h3>Trace Timeline</h3>\n<table>\n");
                html.push_str("<tr><th>Frame</th><th>Time</th><th>Action</th></tr>\n");
                for step in steps {
                    let _ = write!(
                        html,
                        "<tr><td>{}</td><td>{}ms</td><td>{}</td></tr>\n",
                        step.frame_number,
                        step.elapsed_ms,
                        escape_html(step.action.short_label()),
                    );
                }
                html.push_str("</table>\n</div>\n");
            }
        }

        html.push_str("</div>\n</details>\n");
    }

    fn count_statuses(&self) -> (usize, usize, usize) {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        for e in &self.entries {
            match e.status {
                TestStatus::Passed => passed += 1,
                TestStatus::Failed => failed += 1,
                TestStatus::Skipped => skipped += 1,
            }
        }
        (passed, failed, skipped)
    }

    fn total_duration(&self) -> Duration {
        self.entries.iter().map(|e| e.duration).sum()
    }
}

fn write_css(out: &mut String) {
    out.push_str(
        "\
* { margin: 0; padding: 0; box-sizing: border-box; }
body { background: #1a1a2e; color: #e0e0e0; font-family: 'Fira Code', 'Cascadia Code', monospace; padding: 20px; }
.summary { background: #16213e; padding: 20px; border-radius: 8px; margin-bottom: 20px; }
.summary h1 { font-size: 1.4em; margin-bottom: 12px; color: #f0f0f0; }
.stats { display: flex; gap: 16px; flex-wrap: wrap; margin-bottom: 12px; }
.stat { padding: 4px 12px; border-radius: 4px; font-size: 0.9em; }
.stat.total { background: #2a2a4a; }
.stat.passed { background: #1b4332; color: #95d5b2; }
.stat.failed { background: #641220; color: #fca5a5; }
.stat.skipped { background: #3a3a3a; color: #aaa; }
.stat.duration { background: #1e3a5f; color: #93c5fd; }
.filters { display: flex; gap: 8px; }
.filters button { background: #2a2a4a; color: #ccc; border: 1px solid #444; padding: 6px 16px; border-radius: 4px; cursor: pointer; font-family: inherit; font-size: 0.85em; }
.filters button:hover { background: #3a3a5a; }
.filters button.active { background: #4a4a7a; color: #fff; border-color: #6a6aaa; }
.entries { display: flex; flex-direction: column; gap: 8px; }
.entry { background: #16213e; border-radius: 6px; border-left: 4px solid #555; }
.entry.passed { border-left-color: #2d6a4f; }
.entry.failed { border-left-color: #e63946; }
.entry.skipped { border-left-color: #666; }
.entry summary { padding: 12px 16px; cursor: pointer; display: flex; align-items: center; gap: 10px; list-style: none; }
.entry summary::-webkit-details-marker { display: none; }
.entry summary .icon { font-size: 1.1em; }
.entry.passed summary .icon { color: #95d5b2; }
.entry.failed summary .icon { color: #fca5a5; }
.entry.skipped summary .icon { color: #888; }
.entry summary .name { flex: 1; }
.entry summary .duration { color: #888; font-size: 0.85em; }
.detail { padding: 12px 16px; border-top: 1px solid #2a2a4a; }
.failure { background: #2d0a0a; border: 1px solid #5c1a1a; border-radius: 4px; padding: 12px; margin-bottom: 12px; }
.failure pre { white-space: pre-wrap; word-break: break-word; color: #fca5a5; font-size: 0.85em; }
.screenshots { display: flex; gap: 12px; flex-wrap: wrap; margin-bottom: 12px; }
.screenshot img { max-width: 400px; border: 1px solid #333; border-radius: 4px; }
.trace table { width: 100%; border-collapse: collapse; font-size: 0.85em; }
.trace th, .trace td { padding: 6px 10px; text-align: left; border-bottom: 1px solid #2a2a4a; }
.trace th { color: #93c5fd; }
.trace h3 { margin-bottom: 8px; font-size: 1em; color: #93c5fd; }
.hidden { display: none; }
",
    );
}

fn write_js(out: &mut String) {
    out.push_str(
        "\
function filterTests(status) {
  var entries = document.querySelectorAll('.entry');
  var buttons = document.querySelectorAll('.filters button');
  for (var i = 0; i < buttons.length; i++) {
    buttons[i].classList.remove('active');
    if (buttons[i].textContent.toLowerCase() === status) {
      buttons[i].classList.add('active');
    }
  }
  for (var j = 0; j < entries.length; j++) {
    if (status === 'all') {
      entries[j].classList.remove('hidden');
    } else if (entries[j].classList.contains(status)) {
      entries[j].classList.remove('hidden');
    } else {
      entries[j].classList.add('hidden');
    }
  }
}
",
    );
}

/// Escape HTML special characters.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Read an image file and return a data URI string with base64-encoded PNG data.
/// Returns `None` if the file cannot be read.
fn encode_image_as_data_uri(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let encoded = base64_encode(&data);
    Some(format!("data:image/png;base64,{encoded}"))
}

/// Base64 encode a byte slice using the standard alphabet.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let chunks = input.chunks(3);

    for chunk in chunks {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }

        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encode_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn base64_encode_standard_vectors() {
        // RFC 4648 test vectors
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn empty_report_generates_valid_html() {
        let report = TestReport::new("tests/report");
        let html = report.render_html();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
        assert!(html.contains("0 total"));
        assert!(html.contains("0 passed"));
        assert!(html.contains("0 failed"));
        assert!(html.contains("0 skipped"));
    }

    #[test]
    fn report_records_entries() {
        let mut report = TestReport::new("tests/report");
        report.add_entry(TestReportEntry {
            name: "my_test".into(),
            status: TestStatus::Passed,
            duration: Duration::from_millis(150),
            trace: None,
            failure_message: None,
            screenshot_paths: vec![],
        });
        report.add_entry(TestReportEntry {
            name: "failing_test".into(),
            status: TestStatus::Failed,
            duration: Duration::from_millis(300),
            trace: None,
            failure_message: Some("assertion failed".into()),
            screenshot_paths: vec![],
        });

        assert_eq!(report.entries().len(), 2);
        assert_eq!(report.entries()[0].status, TestStatus::Passed);
        assert_eq!(report.entries()[1].status, TestStatus::Failed);
    }

    #[test]
    fn report_html_contains_entry_data() {
        let mut report = TestReport::new("tests/report");
        report.add_entry(TestReportEntry {
            name: "counter_test".into(),
            status: TestStatus::Passed,
            duration: Duration::from_millis(42),
            trace: None,
            failure_message: None,
            screenshot_paths: vec![],
        });
        report.add_entry(TestReportEntry {
            name: "broken_test".into(),
            status: TestStatus::Failed,
            duration: Duration::from_millis(100),
            trace: None,
            failure_message: Some("expected 5 got 3".into()),
            screenshot_paths: vec![],
        });

        let html = report.render_html();
        assert!(html.contains("counter_test"));
        assert!(html.contains("broken_test"));
        assert!(html.contains("expected 5 got 3"));
        assert!(html.contains("1 passed"));
        assert!(html.contains("1 failed"));
    }

    #[test]
    fn report_html_has_correct_filter_classes() {
        let mut report = TestReport::new("tests/report");
        report.add_entry(TestReportEntry {
            name: "pass".into(),
            status: TestStatus::Passed,
            duration: Duration::default(),
            trace: None,
            failure_message: None,
            screenshot_paths: vec![],
        });
        report.add_entry(TestReportEntry {
            name: "fail".into(),
            status: TestStatus::Failed,
            duration: Duration::default(),
            trace: None,
            failure_message: None,
            screenshot_paths: vec![],
        });
        report.add_entry(TestReportEntry {
            name: "skip".into(),
            status: TestStatus::Skipped,
            duration: Duration::default(),
            trace: None,
            failure_message: None,
            screenshot_paths: vec![],
        });

        let html = report.render_html();
        assert!(html.contains("class=\"entry passed\""));
        assert!(html.contains("class=\"entry failed\""));
        assert!(html.contains("class=\"entry skipped\""));
    }

    #[test]
    fn report_html_contains_trace_timeline() {
        use crate::trace::TraceAction;

        let mut report = TestReport::new("tests/report");
        report.add_entry(TestReportEntry {
            name: "traced_test".into(),
            status: TestStatus::Passed,
            duration: Duration::from_millis(50),
            trace: Some(vec![
                TraceStep {
                    frame_number: 0,
                    action: TraceAction::Click {
                        selector: ".btn".into(),
                        x: 10.0,
                        y: 20.0,
                    },
                    elapsed_ms: 5,
                    screenshot_path: None,
                },
                TraceStep {
                    frame_number: 1,
                    action: TraceAction::Assertion {
                        selector: ".out".into(),
                        kind: "text".into(),
                        expected: "1".into(),
                        actual: "1".into(),
                        passed: true,
                    },
                    elapsed_ms: 12,
                    screenshot_path: None,
                },
            ]),
            failure_message: None,
            screenshot_paths: vec![],
        });

        let html = report.render_html();
        assert!(html.contains("Trace Timeline"));
        assert!(html.contains("click"));
        assert!(html.contains("assert"));
    }

    #[test]
    fn report_generate_writes_file() {
        let dir = std::env::temp_dir().join("unshit_report_test");
        let _ = std::fs::remove_dir_all(&dir);

        let mut report = TestReport::new(&dir);
        report.add_entry(TestReportEntry {
            name: "write_test".into(),
            status: TestStatus::Passed,
            duration: Duration::from_millis(10),
            trace: None,
            failure_message: None,
            screenshot_paths: vec![],
        });

        let path = report.generate();
        assert!(path.exists(), "index.html should be written");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("write_test"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn escape_html_works() {
        assert_eq!(escape_html("<b>\"hi\"</b>"), "&lt;b&gt;&quot;hi&quot;&lt;/b&gt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("it's"), "it&#39;s");
    }

    #[test]
    fn test_status_labels() {
        assert_eq!(TestStatus::Passed.label(), "passed");
        assert_eq!(TestStatus::Failed.label(), "failed");
        assert_eq!(TestStatus::Skipped.label(), "skipped");
    }
}
