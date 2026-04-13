//! Headless snapshot test for the terminal-manager visual shell (capillary
//! #132). The tree builder lives in `crates/unshit/examples/terminal_manager.rs`
//! next to the seed state, which is where the full layout invariants are
//! asserted (see the `#[cfg(test)] mod tests` block in that file). Exporting
//! the builder from an example binary into a separate integration test
//! crate would require either an awkward `path` attribute mod declaration
//! or turning the example into a library, so this file only covers what
//! can be asserted without importing the builder: the embedded stylesheet
//! parses into a populated `CompiledStylesheet`, the shell specific rules
//! survive the parse, and the custom properties that drive the shell
//! layout round trip to their reference values.

use unshit_core::style::parse::CompiledStylesheet;

const STYLES: &str = include_str!("../../unshit/examples/assets/terminal_manager/styles.css");

#[test]
fn stylesheet_parses_without_dropping_rules() {
    let sheet = CompiledStylesheet::parse(STYLES);
    // The shell uses roughly 220 class rules plus the reset. The exact
    // count drifts when dependent features land, so just assert the
    // stylesheet is non trivial.
    assert!(
        sheet.rules.len() > 150,
        "expected the terminal manager stylesheet to emit >150 rules, got {}",
        sheet.rules.len()
    );
}

#[test]
fn titlebar_sizing_tokens_are_present() {
    let sheet = CompiledStylesheet::parse(STYLES);
    let props = &sheet.custom_properties;
    assert_eq!(props.get("--titlebar-h").map(String::as_str), Some("34px"));
    assert_eq!(props.get("--sidebar-w").map(String::as_str), Some("252px"));
    assert_eq!(props.get("--tabbar-h").map(String::as_str), Some("38px"));
    assert_eq!(props.get("--statusbar-h").map(String::as_str), Some("24px"));
}

#[test]
fn amber_palette_tokens_are_present() {
    let sheet = CompiledStylesheet::parse(STYLES);
    let props = &sheet.custom_properties;
    for name in [
        "--amber-50",
        "--amber-100",
        "--amber-200",
        "--amber-300",
        "--amber-400",
        "--amber-500",
        "--bg-void",
        "--bg-base",
        "--bg-subtle",
        "--bg-elevated",
        "--fg-primary",
        "--fg-secondary",
        "--fg-tertiary",
        "--fg-muted",
    ] {
        assert!(props.contains_key(name), "expected custom property {} to be defined", name);
    }
}

#[test]
fn keyframes_pulse_dot_and_cursor_blink_exist() {
    let sheet = CompiledStylesheet::parse(STYLES);
    assert!(
        sheet.keyframes.contains_key("pulse-dot"),
        "pulse-dot keyframes must parse for the agents count badge"
    );
    assert!(
        sheet.keyframes.contains_key("cursor-blink"),
        "cursor-blink keyframes must parse for the term-cursor span"
    );
}
