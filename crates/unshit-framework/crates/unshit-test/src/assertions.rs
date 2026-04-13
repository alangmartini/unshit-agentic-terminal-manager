use unshit_core::element::ElementContent;

use crate::query::ElementSnapshot;
use crate::trace::TraceAction;
use crate::TestHarness;

/// Default number of frames to retry before failing an assertion.
const DEFAULT_TIMEOUT_FRAMES: usize = 60;

/// Result of a single assertion check: either it passed (return early)
/// or it failed with a message to show if no further retries remain.
enum CheckResult {
    Pass,
    Fail(String),
}

impl TestHarness {
    /// Core retry loop. Calls `check` each frame; returns on Pass or panics
    /// after `max_frames` with the last Fail message.
    fn retry_until(&mut self, max_frames: usize, mut check: impl FnMut(&mut Self) -> CheckResult) {
        for _ in 0..max_frames {
            if matches!(check(self), CheckResult::Pass) {
                return;
            }
            self.step();
        }
        // Final attempt after the last step
        match check(self) {
            CheckResult::Pass => {}
            CheckResult::Fail(msg) => {
                panic!("Assertion failed after {} frames:\n{}", max_frames, msg,)
            }
        }
    }

    // -- Element existence and visibility ------------------------------------

    /// Assert that an element matching `selector` exists and has non-zero dimensions.
    pub fn expect_visible(&mut self, selector: &str) {
        self.expect_visible_with_timeout(selector, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_visible_with_timeout(&mut self, selector: &str, max_frames: usize) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) if s.layout_rect.width > 0.0 && s.layout_rect.height > 0.0 => {
                h.trace.record(TraceAction::Assertion {
                    selector: sel.clone(),
                    kind: "visible".into(),
                    expected: "visible".into(),
                    actual: format!("{}x{}", s.layout_rect.width, s.layout_rect.height),
                    passed: true,
                });
                CheckResult::Pass
            }
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: visible (non-zero dimensions)\n  Actual:   {}x{}\n  Element tag: {:?}\n  Element rect: ({}, {}, {}, {})",
                sel, s.layout_rect.width, s.layout_rect.height, s.tag,
                s.layout_rect.x, s.layout_rect.y, s.layout_rect.width, s.layout_rect.height,
            )),
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: element to exist and be visible\n  Actual:   element not found",
                sel,
            )),
        });
    }

    /// Assert that an element matching `selector` either does not exist or has zero dimensions.
    pub fn expect_hidden(&mut self, selector: &str) {
        self.expect_hidden_with_timeout(selector, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_hidden_with_timeout(&mut self, selector: &str, max_frames: usize) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            None => CheckResult::Pass,
            Some(s) if s.layout_rect.width <= 0.0 || s.layout_rect.height <= 0.0 => {
                CheckResult::Pass
            }
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: hidden (not found or zero dimensions)\n  Actual:   {}x{}\n  Element tag: {:?}\n  Element rect: ({}, {}, {}, {})",
                sel, s.layout_rect.width, s.layout_rect.height, s.tag,
                s.layout_rect.x, s.layout_rect.y, s.layout_rect.width, s.layout_rect.height,
            )),
        });
    }

    /// Assert that an element matching `selector` exists in the tree.
    pub fn expect_exists(&mut self, selector: &str) {
        self.expect_exists_with_timeout(selector, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_exists_with_timeout(&mut self, selector: &str, max_frames: usize) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(_) => {
                h.trace.record(TraceAction::Assertion {
                    selector: sel.clone(),
                    kind: "exists".into(),
                    expected: "exists".into(),
                    actual: "found".into(),
                    passed: true,
                });
                CheckResult::Pass
            }
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: element to exist\n  Actual:   not found",
                sel,
            )),
        });
    }

    /// Assert that no element matches `selector`.
    pub fn expect_not_exists(&mut self, selector: &str) {
        self.expect_not_exists_with_timeout(selector, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_not_exists_with_timeout(&mut self, selector: &str, max_frames: usize) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            None => CheckResult::Pass,
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: element to not exist\n  Actual:   found {:?} with id {:?}",
                sel, s.tag, s.id,
            )),
        });
    }

    // -- Text content --------------------------------------------------------

    /// Assert that the text content of the element matching `selector` equals `expected`.
    pub fn expect_text(&mut self, selector: &str, expected: &str) {
        self.expect_text_with_timeout(selector, expected, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_text_with_timeout(&mut self, selector: &str, expected: &str, max_frames: usize) {
        let sel = selector.to_owned();
        let exp = expected.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) => {
                let actual = h.collect_text_content(&s);
                if actual == exp {
                    h.trace.record(TraceAction::Assertion {
                        selector: sel.clone(),
                        kind: "text".into(),
                        expected: exp.clone(),
                        actual: actual.clone(),
                        passed: true,
                    });
                    CheckResult::Pass
                } else {
                    CheckResult::Fail(format!(
                        "  Selector: {}\n  Expected text: {:?}\n  Actual text:   {:?}\n  Element tag:   {:?}\n  Element rect:  ({}, {}, {}, {})",
                        sel, exp, actual, s.tag,
                        s.layout_rect.x, s.layout_rect.y, s.layout_rect.width, s.layout_rect.height,
                    ))
                }
            }
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected text: {:?}\n  Actual:   element not found",
                sel, exp,
            )),
        });
    }

    /// Assert that the text content of the element matching `selector` contains `substring`.
    pub fn expect_text_contains(&mut self, selector: &str, substring: &str) {
        self.expect_text_contains_with_timeout(selector, substring, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_text_contains_with_timeout(
        &mut self,
        selector: &str,
        substring: &str,
        max_frames: usize,
    ) {
        let sel = selector.to_owned();
        let sub = substring.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) => {
                let actual = h.collect_text_content(&s);
                if actual.contains(sub.as_str()) {
                    CheckResult::Pass
                } else {
                    CheckResult::Fail(format!(
                        "  Selector: {}\n  Expected to contain: {:?}\n  Actual text:   {:?}\n  Element tag:   {:?}\n  Element rect:  ({}, {}, {}, {})",
                        sel, sub, actual, s.tag,
                        s.layout_rect.x, s.layout_rect.y, s.layout_rect.width, s.layout_rect.height,
                    ))
                }
            }
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected to contain: {:?}\n  Actual:   element not found",
                sel, sub,
            )),
        });
    }

    // -- Element state -------------------------------------------------------

    /// Assert that the element matching `selector` has the given CSS class.
    pub fn expect_class(&mut self, selector: &str, class: &str) {
        self.expect_class_with_timeout(selector, class, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_class_with_timeout(&mut self, selector: &str, class: &str, max_frames: usize) {
        let sel = selector.to_owned();
        let cls = class.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) if s.classes.iter().any(|c| c == &cls) => CheckResult::Pass,
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected class: {:?}\n  Actual classes: {:?}\n  Element tag: {:?}",
                sel, cls, s.classes, s.tag,
            )),
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected class: {:?}\n  Actual:   element not found",
                sel, cls,
            )),
        });
    }

    /// Assert that the element matching `selector` does NOT have the given CSS class.
    pub fn expect_not_class(&mut self, selector: &str, class: &str) {
        self.expect_not_class_with_timeout(selector, class, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_not_class_with_timeout(
        &mut self,
        selector: &str,
        class: &str,
        max_frames: usize,
    ) {
        let sel = selector.to_owned();
        let cls = class.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            None => CheckResult::Pass,
            Some(s) if !s.classes.iter().any(|c| c == &cls) => CheckResult::Pass,
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected NOT to have class: {:?}\n  Actual classes: {:?}\n  Element tag: {:?}",
                sel, cls, s.classes, s.tag,
            )),
        });
    }

    /// Assert that the input matching `selector` is checked.
    pub fn expect_checked(&mut self, selector: &str) {
        self.expect_checked_with_timeout(selector, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_checked_with_timeout(&mut self, selector: &str, max_frames: usize) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) if s.checked == Some(true) => CheckResult::Pass,
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: checked\n  Actual checked: {:?}\n  Element tag: {:?}",
                sel, s.checked, s.tag,
            )),
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: checked\n  Actual:   element not found",
                sel,
            )),
        });
    }

    /// Assert that the input matching `selector` is NOT checked.
    pub fn expect_not_checked(&mut self, selector: &str) {
        self.expect_not_checked_with_timeout(selector, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_not_checked_with_timeout(&mut self, selector: &str, max_frames: usize) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) if s.checked != Some(true) => CheckResult::Pass,
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: not checked\n  Actual checked: {:?}\n  Element tag: {:?}",
                sel, s.checked, s.tag,
            )),
            None => CheckResult::Pass,
        });
    }

    /// Assert that the input matching `selector` has the given value.
    pub fn expect_value(&mut self, selector: &str, expected: &str) {
        self.expect_value_with_timeout(selector, expected, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_value_with_timeout(&mut self, selector: &str, expected: &str, max_frames: usize) {
        let sel = selector.to_owned();
        let exp = expected.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) if s.input_value.as_deref() == Some(exp.as_str()) => {
                h.trace.record(TraceAction::Assertion {
                    selector: sel.clone(),
                    kind: "value".into(),
                    expected: exp.clone(),
                    actual: s.input_value.clone().unwrap_or_default(),
                    passed: true,
                });
                CheckResult::Pass
            }
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected value: {:?}\n  Actual value:   {:?}\n  Element tag: {:?}",
                sel, exp, s.input_value, s.tag,
            )),
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected value: {:?}\n  Actual:   element not found",
                sel, exp,
            )),
        });
    }

    /// Assert that the element matching `selector` is currently focused.
    pub fn expect_focused(&mut self, selector: &str) {
        self.expect_focused_with_timeout(selector, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_focused_with_timeout(&mut self, selector: &str, max_frames: usize) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) if s.node_id == h.focused() => CheckResult::Pass,
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: focused\n  Element node_id: {:?}\n  Focused node_id: {:?}\n  Element tag: {:?}",
                sel, s.node_id, h.focused(), s.tag,
            )),
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Expected: focused\n  Actual:   element not found",
                sel,
            )),
        });
    }

    // -- Count ---------------------------------------------------------------

    /// Assert that exactly `expected` elements match `selector`.
    pub fn expect_count(&mut self, selector: &str, expected: usize) {
        self.expect_count_with_timeout(selector, expected, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_count_with_timeout(
        &mut self,
        selector: &str,
        expected: usize,
        max_frames: usize,
    ) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| {
            let count = h.query_all(&sel).len();
            if count == expected {
                CheckResult::Pass
            } else {
                CheckResult::Fail(format!(
                    "  Selector: {}\n  Expected count: {}\n  Actual count:   {}",
                    sel, expected, count,
                ))
            }
        });
    }

    // -- Custom predicate ----------------------------------------------------

    /// Assert that the element matching `selector` satisfies `predicate`.
    pub fn expect_element(&mut self, selector: &str, predicate: impl Fn(&ElementSnapshot) -> bool) {
        self.expect_element_with_timeout(selector, predicate, DEFAULT_TIMEOUT_FRAMES);
    }

    pub fn expect_element_with_timeout(
        &mut self,
        selector: &str,
        predicate: impl Fn(&ElementSnapshot) -> bool,
        max_frames: usize,
    ) {
        let sel = selector.to_owned();
        self.retry_until(max_frames, |h| match h.query(&sel) {
            Some(s) if predicate(&s) => CheckResult::Pass,
            Some(s) => CheckResult::Fail(format!(
                "  Selector: {}\n  Custom predicate returned false\n  Element tag: {:?}\n  Element rect: ({}, {}, {}, {})",
                sel, s.tag,
                s.layout_rect.x, s.layout_rect.y, s.layout_rect.width, s.layout_rect.height,
            )),
            None => CheckResult::Fail(format!(
                "  Selector: {}\n  Custom predicate could not be evaluated\n  Actual:   element not found",
                sel,
            )),
        });
    }

    // -- Helpers -------------------------------------------------------------

    /// Collect text content from an element and its descendants.
    /// If the element itself has Text content, returns that.
    /// Otherwise, walks children depth-first and concatenates all text found.
    fn collect_text_content(&self, snap: &ElementSnapshot) -> String {
        if let ElementContent::Text(ref text) = snap.content {
            return text.clone();
        }
        let mut result = String::new();
        self.collect_text_recursive(snap.node_id, &mut result);
        result
    }

    fn collect_text_recursive(&self, node_id: unshit_core::id::NodeId, out: &mut String) {
        if let Some(element) = self.arena.get(node_id) {
            if let ElementContent::Text(ref text) = element.content {
                out.push_str(text);
            }
            for child in self.arena.children(node_id) {
                self.collect_text_recursive(child, out);
            }
        }
    }
}
