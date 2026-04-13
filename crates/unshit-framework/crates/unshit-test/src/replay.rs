use crate::TestHarness;

/// A synthetic event for replay-based testing.
/// Represents the kinds of inputs a real user produces.
#[derive(Clone, Debug)]
pub enum TestEvent {
    /// Move cursor to (x, y).
    CursorMoved { x: f32, y: f32 },
    /// Press the left mouse button at current cursor position.
    MouseDown,
    /// Release the left mouse button.
    MouseUp,
    /// Scroll wheel with (dx, dy) delta.
    MouseWheel { dx: f32, dy: f32 },
    /// Advance N frames without input (simulates continuous redraw loop).
    Wait { frames: usize },
    /// Assert that hover state is stable for N additional frames.
    AssertHoverStable { frames: usize },
    /// Assert pixel-level render stability for N frames (requires GPU).
    AssertRenderStable { frames: usize },
}

impl TestEvent {
    /// Load a recorded event sequence from a JSON file.
    ///
    /// Parses the format produced by the app's event recording feature
    /// (enabled via `UNSHIT_RECORD_EVENTS=1`). Uses basic string parsing
    /// instead of serde to avoid extra dependencies.
    ///
    /// Expected format:
    /// ```json
    /// [
    ///   {"type":"CursorMoved","x":400.5,"y":300.2,"time_ms":0},
    ///   {"type":"MouseDown","time_ms":500},
    ///   {"type":"MouseUp","time_ms":550},
    ///   {"type":"MouseWheel","dx":0,"dy":-3,"time_ms":700}
    /// ]
    /// ```
    pub fn load_recording(path: &str) -> Vec<TestEvent> {
        let Ok(content) = std::fs::read_to_string(path) else {
            return Vec::new();
        };

        let mut events = Vec::new();

        // Split on `},{` to get individual event objects
        // First strip the outer array brackets
        let trimmed = content.trim();
        let inner = if trimmed.starts_with('[') && trimmed.ends_with(']') {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        };

        for chunk in split_json_objects(inner) {
            let chunk = chunk.trim();
            if chunk.is_empty() {
                continue;
            }

            if let Some(event) = parse_event_object(chunk) {
                events.push(event);
            }
        }

        events
    }
}

/// Split a string containing comma-separated JSON objects.
/// Handles the fact that objects themselves contain commas.
fn split_json_objects(s: &str) -> Vec<&str> {
    let mut results = Vec::new();
    let mut depth = 0;
    let mut start = 0;

    for (i, ch) in s.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    results.push(&s[start..=i]);
                }
            }
            _ => {}
        }
    }

    results
}

/// Parse a single JSON object string into a TestEvent.
fn parse_event_object(s: &str) -> Option<TestEvent> {
    let event_type = extract_string_field(s, "type")?;

    match event_type {
        "CursorMoved" => {
            let x = extract_number_field(s, "x")?;
            let y = extract_number_field(s, "y")?;
            Some(TestEvent::CursorMoved { x, y })
        }
        "MouseDown" => Some(TestEvent::MouseDown),
        "MouseUp" => Some(TestEvent::MouseUp),
        "MouseWheel" => {
            let dx = extract_number_field(s, "dx").unwrap_or(0.0);
            let dy = extract_number_field(s, "dy").unwrap_or(0.0);
            Some(TestEvent::MouseWheel { dx, dy })
        }
        _ => None,
    }
}

/// Extract a string value for a given key from a JSON object string.
/// e.g., extract_string_field(r#"{"type":"CursorMoved"}"#, "type") => Some("CursorMoved")
fn extract_string_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!(r#""{}":""#, key);
    let start = json.find(&pattern)? + pattern.len();
    let end = json[start..].find('"')? + start;
    Some(&json[start..end])
}

/// Extract a numeric value for a given key from a JSON object string.
/// e.g., extract_number_field(r#"{"x":400.5}"#, "x") => Some(400.5)
fn extract_number_field(json: &str, key: &str) -> Option<f32> {
    let pattern = format!(r#""{}":"#, key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..];
    // The number ends at the next comma, closing brace, or whitespace
    let end = rest.find([',', '}', ' ', '\n']).unwrap_or(rest.len());
    rest[..end].trim().parse::<f32>().ok()
}

impl TestHarness {
    /// Replay a sequence of events, executing each in order.
    pub fn replay(&mut self, events: &[TestEvent]) {
        for event in events {
            match event {
                TestEvent::CursorMoved { x, y } => {
                    self.mouse_move(*x, *y);
                }
                TestEvent::MouseDown => {
                    let (x, y) = self.cursor_pos();
                    self.mouse_down(x, y);
                }
                TestEvent::MouseUp => {
                    let (x, y) = self.cursor_pos();
                    self.mouse_up(x, y);
                }
                TestEvent::MouseWheel { dx, dy } => {
                    let (x, y) = self.cursor_pos();
                    self.mouse_wheel(x, y, *dx, *dy);
                }
                TestEvent::Wait { frames } => {
                    for _ in 0..*frames {
                        self.step();
                    }
                }
                TestEvent::AssertHoverStable { frames } => {
                    self.assert_hover_stable(*frames);
                }
                TestEvent::AssertRenderStable { frames } => {
                    self.assert_render_stable(*frames);
                }
            }
        }
    }

    /// Get the current cursor position.
    pub fn cursor_pos(&self) -> (f32, f32) {
        self.interaction.last_cursor_pos
    }
}
