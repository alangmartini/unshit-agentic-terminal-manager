//! Guardrail: the CSS engine must understand every declaration the app's
//! stylesheet authors. Any declaration the parser silently drops (an
//! unrecognized property, or a value its parser rejects — e.g. a viewport unit
//! on a px-only pathway, or `calc()`) surfaces here, so a new engine gap fails
//! the build the moment someone writes unsupported CSS — instead of when a
//! screenshot looks wrong.
//!
//! When this fails with a NEW property: either teach the engine the dropped
//! declaration (preferred), or, if it is a deliberate, documented deferral, add
//! the property to [`KNOWN_UNSUPPORTED`] with a tracking note. The list is the
//! living inventory of CSS the engine does not yet support.

use std::collections::BTreeSet;

use unshit::core::style::parse::{CompiledStylesheet, DroppedDeclaration};

const STYLES: &str = include_str!("../assets/styles.css");

/// Properties the engine does not yet honor (their declarations drop harmlessly
/// today). Tracked so a *new* gap is caught while the existing backlog does not
/// fail the build. Shrinking this list as features land is the goal; growing it
/// should be a conscious, reviewed decision.
///
/// The narrative plan for each entry (why it drops, effort, and how to close it)
/// lives in `specs/css-engine-stylesheet-gaps.md`. When an item lands, remove it
/// from both this list and that spec.
const KNOWN_UNSUPPORTED: &[&str] = &[
    // Fully unimplemented properties.
    "filter", // non-backdrop filter (drop-shadow/brightness/none)
    "mix-blend-mode",
    "scroll-margin",
    "vertical-align",
    "word-break",
    // Partially supported — common values work, but the stylesheet uses a form
    // the parser rejects (masking note: a regression on the *supported* form of
    // these would not be caught here).
    "background", // `none` + first multi-layer paint work; some `ellipse <size>
    // at <pos>` radial-gradient forms still drop (gradient parser gap)
    "background-position",
    "background-size",
];

/// A dropped declaration is "expected" if its property is a known gap, or its
/// value uses a value form the engine does not support yet:
///   - the `inherit` keyword (e.g. `color: inherit`).
///
/// `calc()` is now supported for length values (resolving `px`/`vw`/`vh`), so
/// it is no longer a blanket allowance; a `calc()` that still drops (e.g. on an
/// unsupported property, or a `percent + length` form taffy can't represent)
/// must be covered by its property's `KNOWN_UNSUPPORTED` entry.
fn is_known_gap(d: &DroppedDeclaration) -> bool {
    KNOWN_UNSUPPORTED.contains(&d.property.as_str()) || d.value == "inherit"
}

#[test]
fn stylesheet_has_no_unknown_engine_gaps() {
    let sheet = CompiledStylesheet::parse(STYLES);

    // Custom-property definitions on non-:root selectors (every `.app.theme-*`
    // block, etc.) are a separate, known gap (cascade-aware custom properties),
    // not a per-property gap — reported but not failed on.
    let custom_count = sheet
        .dropped
        .iter()
        .filter(|d| d.is_custom_property())
        .count();

    let unexpected: Vec<&DroppedDeclaration> = sheet
        .dropped
        .iter()
        .filter(|d| !d.is_custom_property() && !is_known_gap(d))
        .collect();

    let distinct: BTreeSet<&str> = sheet
        .dropped
        .iter()
        .filter(|d| !d.is_custom_property())
        .map(|d| d.property.as_str())
        .collect();
    eprintln!(
        "stylesheet coverage: {} drops total ({custom_count} custom-property). \
         Distinct unsupported properties/values: {distinct:?}",
        sheet.dropped.len(),
    );

    assert!(
        unexpected.is_empty(),
        "The stylesheet has declarations the CSS engine silently drops that are \
         NOT in the known-gap inventory. Support them in the engine, or if \
         deliberately deferred add the property to KNOWN_UNSUPPORTED with a note.\n\
         Unexpected drops:\n{}",
        unexpected
            .iter()
            .map(|d| format!("  {} {{ {}: {} }}", d.selector, d.property, d.value))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
