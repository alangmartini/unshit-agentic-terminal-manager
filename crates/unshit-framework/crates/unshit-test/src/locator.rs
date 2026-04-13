//! Playwright-inspired locator pattern for chainable, lazy element references.
//!
//! Locators are lazy: they do not resolve against the element tree until an
//! action or assertion is called. They re-resolve on every call, so they
//! always reflect the current state of the tree.
//!
//! ```ignore
//! let btn = harness.locator(".btn");
//! let card_title = harness.locator(".card").locator(".title");
//! let first_item = harness.locator(".list-item").nth(0);
//!
//! btn.click();
//! card_title.expect_text("Hello");
//! first_item.expect_visible();
//! ```

use unshit_core::element::{ElementContent, LayoutRect};
use unshit_core::id::NodeId;

use crate::query::{snapshot_from, ElementSnapshot};
use crate::selector;
use crate::TestHarness;

const DEFAULT_TIMEOUT_FRAMES: usize = 60;

#[derive(Clone)]
enum LocatorFilter {
    Nth(usize),
    TextContains(String),
    TextExact(String),
}

/// Describes what a locator should match, without borrowing the harness.
#[derive(Clone)]
struct SelectorChain {
    /// Each segment scopes the search to descendants of the previous matches.
    segments: Vec<SelectorSegment>,
}

#[derive(Clone)]
struct SelectorSegment {
    selector: String,
    filters: Vec<LocatorFilter>,
}

impl SelectorChain {
    fn new(selector: &str) -> Self {
        Self {
            segments: vec![SelectorSegment { selector: selector.to_owned(), filters: Vec::new() }],
        }
    }

    fn new_by_text(text: &str) -> Self {
        Self::new(&format!("text(\"{}\")", text))
    }

    fn new_by_text_contains(text: &str) -> Self {
        Self::new(&format!("has_text(\"{}\")", text))
    }

    fn chain(mut self, selector: &str) -> Self {
        self.segments.push(SelectorSegment { selector: selector.to_owned(), filters: Vec::new() });
        self
    }

    fn with_filter(mut self, filter: LocatorFilter) -> Self {
        if let Some(last) = self.segments.last_mut() {
            last.filters.push(filter);
        }
        self
    }

    fn describe(&self) -> String {
        self.segments
            .iter()
            .map(|seg| {
                let mut desc = format!("locator(\"{}\")", seg.selector);
                for f in &seg.filters {
                    match f {
                        LocatorFilter::Nth(n) => desc.push_str(&format!(".nth({})", n)),
                        LocatorFilter::TextContains(s) => {
                            desc.push_str(&format!(".filter_by_text(\"{}\")", s))
                        }
                        LocatorFilter::TextExact(s) => {
                            desc.push_str(&format!(".filter_by_exact_text(\"{}\")", s))
                        }
                    }
                }
                desc
            })
            .collect::<Vec<_>>()
            .join(".")
    }
}

fn resolve_all(harness: &TestHarness, chain: &SelectorChain) -> Vec<NodeId> {
    let mut roots = vec![harness.root()];

    for segment in &chain.segments {
        let query = selector::parse_query(&segment.selector)
            .unwrap_or_else(|e| panic!("invalid selector '{}': {}", segment.selector, e));

        let mut matches: Vec<NodeId> = Vec::new();
        for root in &roots {
            for id in selector::query_all(harness.arena(), *root, &query) {
                if !matches.contains(&id) {
                    matches.push(id);
                }
            }
        }

        for filter in &segment.filters {
            match filter {
                LocatorFilter::Nth(n) => {
                    if *n < matches.len() {
                        matches = vec![matches[*n]];
                    } else {
                        matches.clear();
                    }
                }
                LocatorFilter::TextContains(sub) => {
                    matches.retain(|id| collect_text_for_node(harness, *id).contains(sub.as_str()));
                }
                LocatorFilter::TextExact(exact) => {
                    matches.retain(|id| collect_text_for_node(harness, *id) == *exact);
                }
            }
        }

        roots = matches;
        if roots.is_empty() {
            break;
        }
    }

    roots
}

/// Resolve expecting exactly one match. Panics if zero or more than one.
fn resolve_one(harness: &TestHarness, chain: &SelectorChain) -> NodeId {
    let matches = resolve_all(harness, chain);
    let desc = chain.describe();
    match matches.len() {
        0 => panic!("locator resolved to 0 elements: {}", desc),
        1 => matches[0],
        n => panic!("locator resolved to {} elements (expected 1): {}", n, desc),
    }
}

/// Resolve one element and return its center coordinates.
fn resolve_center(harness: &TestHarness, chain: &SelectorChain) -> (f32, f32) {
    let node_id = resolve_one(harness, chain);
    let r = harness.arena().get(node_id).unwrap().layout_rect;
    (r.x + r.width / 2.0, r.y + r.height / 2.0)
}

fn collect_text_for_node(harness: &TestHarness, node_id: NodeId) -> String {
    let mut result = String::new();
    collect_text_recursive(harness, node_id, &mut result);
    result
}

fn collect_text_recursive(harness: &TestHarness, node_id: NodeId, out: &mut String) {
    if let Some(element) = harness.arena().get(node_id) {
        if let ElementContent::Text(ref text) = element.content {
            out.push_str(text);
        }
        for child in harness.arena().children(node_id) {
            collect_text_recursive(harness, child, out);
        }
    }
}

/// Resolve the first match from a chain, returning its NodeId.
/// Returns `None` if no elements match.
fn resolve_first(harness: &TestHarness, chain: &SelectorChain) -> Option<NodeId> {
    resolve_all(harness, chain).into_iter().next()
}

/// A lazy, chainable reference to one or more elements in the tree.
///
/// Does not resolve its selector until an action or assertion method is called.
/// Every call re-resolves against the current tree state.
pub struct Locator<'a> {
    harness: &'a mut TestHarness,
    chain: SelectorChain,
}

impl<'a> Locator<'a> {
    /// Narrow the search to descendants matching `selector`.
    pub fn locator(self, selector: &str) -> Locator<'a> {
        Locator { harness: self.harness, chain: self.chain.chain(selector) }
    }

    /// Pick the n-th match (0-based).
    pub fn nth(self, index: usize) -> Locator<'a> {
        Locator { harness: self.harness, chain: self.chain.with_filter(LocatorFilter::Nth(index)) }
    }

    /// Keep only matches whose text content contains `text`.
    pub fn filter_by_text(self, text: &str) -> Locator<'a> {
        Locator {
            harness: self.harness,
            chain: self.chain.with_filter(LocatorFilter::TextContains(text.to_owned())),
        }
    }

    /// Keep only matches whose text content equals `text` exactly.
    pub fn filter_by_exact_text(self, text: &str) -> Locator<'a> {
        Locator {
            harness: self.harness,
            chain: self.chain.with_filter(LocatorFilter::TextExact(text.to_owned())),
        }
    }

    /// Click on the single matching element. Panics if 0 or >1 matches.
    pub fn click(&mut self) {
        let (cx, cy) = resolve_center(self.harness, &self.chain);
        self.harness.click(cx, cy);
    }

    /// Double-click on the single matching element.
    pub fn double_click(&mut self) {
        let (cx, cy) = resolve_center(self.harness, &self.chain);
        self.harness.double_click(cx, cy);
    }

    /// Right-click on the single matching element.
    pub fn right_click(&mut self) {
        let (cx, cy) = resolve_center(self.harness, &self.chain);
        self.harness.right_click(cx, cy);
    }

    /// Hover over the single matching element.
    pub fn hover(&mut self) {
        let (cx, cy) = resolve_center(self.harness, &self.chain);
        self.harness.mouse_move(cx, cy);
        self.harness.step();
    }

    /// Focus the matching input, clear it, then type `text`.
    pub fn fill(&mut self, text: &str) {
        let node_id = resolve_one(self.harness, &self.chain);
        let (cx, cy) = {
            let r = self.harness.arena().get(node_id).unwrap().layout_rect;
            (r.x + r.width / 2.0, r.y + r.height / 2.0)
        };
        self.harness.click(cx, cy);
        self.harness.clear_input(node_id);
        self.harness.type_text(text);
        self.harness.step();
    }

    /// Press a key on the matching element (focuses it first).
    pub fn press(&mut self, key_str: &str) {
        let (cx, cy) = resolve_center(self.harness, &self.chain);
        self.harness.click(cx, cy);
        self.harness.press_key_str(key_str);
        self.harness.step();
    }

    /// Return the text content of the single matching element.
    pub fn text(&self) -> String {
        let node_id = resolve_one(self.harness, &self.chain);
        collect_text_for_node(self.harness, node_id)
    }

    /// Return a snapshot of the single matching element.
    pub fn snapshot(&self) -> ElementSnapshot {
        let node_id = resolve_one(self.harness, &self.chain);
        let e = self.harness.arena().get(node_id).unwrap();
        snapshot_from(node_id, e)
    }

    /// Return the bounding box of the single matching element.
    pub fn bounding_box(&self) -> LayoutRect {
        let node_id = resolve_one(self.harness, &self.chain);
        self.harness.arena().get(node_id).unwrap().layout_rect
    }

    /// Return the number of matching elements.
    pub fn count(&self) -> usize {
        resolve_all(self.harness, &self.chain).len()
    }

    /// Return snapshots for all matching elements.
    pub fn all_snapshots(&self) -> Vec<ElementSnapshot> {
        resolve_all(self.harness, &self.chain)
            .into_iter()
            .filter_map(|id| self.harness.arena().get(id).map(|e| snapshot_from(id, e)))
            .collect()
    }

    /// Return the input value of the single matching element.
    pub fn input_value(&self) -> Option<String> {
        self.snapshot().input_value
    }

    /// Assert the element is visible (exists and has non-zero dimensions).
    pub fn expect_visible(&mut self) {
        let chain = self.chain.clone();
        let desc = chain.describe();
        retry_until(self.harness, DEFAULT_TIMEOUT_FRAMES, move |h| {
            match resolve_first(h, &chain) {
                Some(id) => {
                    let e = h
                        .arena()
                        .get(id)
                        .ok_or_else(|| format!("  Locator: {}\n  Node not found in arena", desc))?;
                    if e.layout_rect.width > 0.0 && e.layout_rect.height > 0.0 {
                        Ok(())
                    } else {
                        Err(format!(
                            "  Locator: {}\n  Expected: visible\n  Actual: {}x{}",
                            desc, e.layout_rect.width, e.layout_rect.height
                        ))
                    }
                }
                None => {
                    Err(format!("  Locator: {}\n  Expected: visible\n  Actual: no matches", desc))
                }
            }
        });
    }

    /// Assert the element is hidden (not found or zero dimensions).
    pub fn expect_hidden(&mut self) {
        let chain = self.chain.clone();
        let desc = chain.describe();
        retry_until(self.harness, DEFAULT_TIMEOUT_FRAMES, move |h| {
            match resolve_first(h, &chain) {
                None => Ok(()),
                Some(id) => match h.arena().get(id) {
                    Some(e) if e.layout_rect.width > 0.0 && e.layout_rect.height > 0.0 => {
                        Err(format!(
                            "  Locator: {}\n  Expected: hidden\n  Actual: {}x{}",
                            desc, e.layout_rect.width, e.layout_rect.height
                        ))
                    }
                    _ => Ok(()),
                },
            }
        });
    }

    /// Assert the element's text content equals `expected`.
    pub fn expect_text(&mut self, expected: &str) {
        let chain = self.chain.clone();
        let desc = chain.describe();
        let exp = expected.to_owned();
        retry_until(self.harness, DEFAULT_TIMEOUT_FRAMES, move |h| {
            let id = resolve_first(h, &chain).ok_or_else(|| {
                format!("  Locator: {}\n  Expected text: {:?}\n  Actual: no matches", desc, exp)
            })?;
            let actual = collect_text_for_node(h, id);
            if actual == exp {
                Ok(())
            } else {
                Err(format!(
                    "  Locator: {}\n  Expected text: {:?}\n  Actual text: {:?}",
                    desc, exp, actual
                ))
            }
        });
    }

    /// Assert the element's text content contains `substring`.
    pub fn expect_text_contains(&mut self, substring: &str) {
        let chain = self.chain.clone();
        let desc = chain.describe();
        let sub = substring.to_owned();
        retry_until(self.harness, DEFAULT_TIMEOUT_FRAMES, move |h| {
            let id = resolve_first(h, &chain).ok_or_else(|| {
                format!(
                    "  Locator: {}\n  Expected to contain: {:?}\n  Actual: no matches",
                    desc, sub
                )
            })?;
            let actual = collect_text_for_node(h, id);
            if actual.contains(sub.as_str()) {
                Ok(())
            } else {
                Err(format!(
                    "  Locator: {}\n  Expected to contain: {:?}\n  Actual: {:?}",
                    desc, sub, actual
                ))
            }
        });
    }

    /// Assert that exactly `expected` elements match the locator.
    pub fn expect_count(&mut self, expected: usize) {
        let chain = self.chain.clone();
        let desc = chain.describe();
        retry_until(self.harness, DEFAULT_TIMEOUT_FRAMES, move |h| {
            let count = resolve_all(h, &chain).len();
            if count == expected {
                Ok(())
            } else {
                Err(format!(
                    "  Locator: {}\n  Expected count: {}\n  Actual count: {}",
                    desc, expected, count
                ))
            }
        });
    }

    /// Assert the element has a specific CSS class.
    pub fn expect_class(&mut self, class: &str) {
        let chain = self.chain.clone();
        let desc = chain.describe();
        let cls = class.to_owned();
        retry_until(self.harness, DEFAULT_TIMEOUT_FRAMES, move |h| {
            let id = resolve_first(h, &chain).ok_or_else(|| {
                format!("  Locator: {}\n  Expected class: {:?}\n  Actual: no matches", desc, cls)
            })?;
            let e = h
                .arena()
                .get(id)
                .ok_or_else(|| format!("  Locator: {}\n  Node not found in arena", desc))?;
            if e.classes.iter().any(|c| c == &cls) {
                Ok(())
            } else {
                Err(format!(
                    "  Locator: {}\n  Expected class: {:?}\n  Actual classes: {:?}",
                    desc,
                    cls,
                    e.classes.to_vec()
                ))
            }
        });
    }

    /// Assert the input value equals `expected`.
    pub fn expect_value(&mut self, expected: &str) {
        let chain = self.chain.clone();
        let desc = chain.describe();
        let exp = expected.to_owned();
        retry_until(self.harness, DEFAULT_TIMEOUT_FRAMES, move |h| {
            let id = resolve_first(h, &chain).ok_or_else(|| {
                format!("  Locator: {}\n  Expected value: {:?}\n  Actual: no matches", desc, exp)
            })?;
            let e = h
                .arena()
                .get(id)
                .ok_or_else(|| format!("  Locator: {}\n  Node not found in arena", desc))?;
            let snap = snapshot_from(id, e);
            if snap.input_value.as_deref() == Some(exp.as_str()) {
                Ok(())
            } else {
                Err(format!(
                    "  Locator: {}\n  Expected value: {:?}\n  Actual value: {:?}",
                    desc, exp, snap.input_value
                ))
            }
        });
    }
}

fn retry_until(
    harness: &mut TestHarness,
    max_frames: usize,
    mut check: impl FnMut(&TestHarness) -> Result<(), String>,
) {
    for _ in 0..max_frames {
        if check(harness).is_ok() {
            return;
        }
        harness.step();
    }
    if let Err(msg) = check(harness) {
        panic!("Assertion failed after {} frames:\n{}", max_frames, msg);
    }
}

impl TestHarness {
    /// Create a locator for elements matching `selector`.
    ///
    /// The locator is lazy: it does not resolve until an action or assertion
    /// is called. Chaining `.locator()` narrows the search to descendants.
    pub fn locator(&mut self, selector: &str) -> Locator<'_> {
        Locator { harness: self, chain: SelectorChain::new(selector) }
    }

    /// Create a locator matching elements whose text content equals `text`.
    pub fn locator_by_text(&mut self, text: &str) -> Locator<'_> {
        Locator { harness: self, chain: SelectorChain::new_by_text(text) }
    }

    /// Create a locator matching elements whose text content contains `text`.
    pub fn locator_by_text_contains(&mut self, text: &str) -> Locator<'_> {
        Locator { harness: self, chain: SelectorChain::new_by_text_contains(text) }
    }
}
