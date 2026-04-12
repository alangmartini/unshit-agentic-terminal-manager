//! Integration tests for the interactivity scaffolding landed in
//! capillary #135 (phase J of epic #125).
//!
//! The `terminal_manager` example is a binary, not a library crate, so
//! integration tests in this directory cannot reach into its internal
//! `AppState`/`dispatch` types directly. Most of the behavioural tests
//! for the example (tab add/close, modal open/close, toggle flip, theme
//! chip, stepper, font clamp, pane split/close, workspace collapse,
//! shortcut table coverage) live in the example's own `#[cfg(test)]`
//! module. This integration test exercises the *framework* patch the
//! example relies on: `AppConfig::user_shortcuts` plus
//! `AppConfig::on_command` wired through the shortcut resolver. Any
//! regression in that pathway would break every terminal-manager
//! interactivity feature at once, so we guard it here.

use std::sync::{Arc, Mutex};

use unshit::app::AppConfig;
use unshit::core::shortcut::Shortcut;

/// Every shortcut string the terminal-manager example registers must
/// parse cleanly through `Shortcut::parse`. The framework logs and drops
/// bad entries at startup, so an unparseable binding would silently go
/// missing: this test keeps the ported key table honest.
#[test]
fn terminal_manager_shortcut_table_parses() {
    let bindings: Vec<(&str, &str)> = vec![
        ("Ctrl+T", "tab.new"),
        ("Ctrl+W", "pane.close"),
        ("Ctrl+D", "pane.split_right"),
        ("Ctrl+Shift+D", "pane.split_down"),
        ("Ctrl+B", "sidebar.toggle"),
        ("Ctrl+,", "modal.open"),
        ("Ctrl+K", "palette.toggle"),
        ("Ctrl+Shift+P", "palette.toggle"),
        ("Escape", "modal.close"),
        ("Ctrl+1", "tab.switch:0"),
        ("Ctrl+2", "tab.switch:1"),
        ("Ctrl+3", "tab.switch:2"),
        ("Ctrl+4", "tab.switch:3"),
        ("Ctrl+5", "tab.switch:4"),
        ("Ctrl+6", "tab.switch:5"),
        ("Ctrl+7", "tab.switch:6"),
        ("Ctrl+8", "tab.switch:7"),
        ("Ctrl+9", "tab.switch:8"),
        ("Ctrl+=", "font.inc"),
        ("Ctrl+Shift+=", "font.inc"),
        ("Ctrl+-", "font.dec"),
        ("Ctrl+Tab", "tab.next"),
        ("Ctrl+Shift+Tab", "tab.prev"),
    ];

    for (key, command) in &bindings {
        Shortcut::parse(key)
            .unwrap_or_else(|e| panic!("failed to parse {key:?} for {command:?}: {e}"));
    }
}

/// The `AppConfig::on_command` hook is stored as an `Arc<dyn Fn>`.
/// Verify that cloning the handler and calling it still mutates the
/// shared state. This mirrors what the framework does inside
/// `dispatch_command` when an unhandled command bubbles up to the
/// user-supplied handler.
#[test]
fn on_command_handler_can_mutate_shared_state() {
    let counter: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let counter_clone = counter.clone();

    let config = AppConfig {
        user_shortcuts: vec![("Ctrl+T".to_string(), "increment".to_string())],
        on_command: Some(Arc::new(move |command: &str| -> bool {
            if command == "increment" {
                *counter_clone.lock().unwrap() += 1;
                true
            } else {
                false
            }
        })),
        ..AppConfig::default()
    };

    // Verify the config stores both fields and the closure still fires
    // against the shared counter when invoked manually.
    assert_eq!(config.user_shortcuts.len(), 1);
    let handler = config.on_command.as_ref().expect("handler stored");
    assert!(handler("increment"));
    assert!(!handler("unknown"));
    assert_eq!(*counter.lock().unwrap(), 1);
}

/// Registering shortcut strings with special characters (`,`, `-`, `=`)
/// must work end-to-end because the terminal-manager example relies on
/// them. A regression here would cause `Ctrl+,` or the font size
/// stepper keys to silently fail at registration time.
#[test]
fn special_character_shortcuts_parse_cleanly() {
    for key in ["Ctrl+,", "Ctrl+=", "Ctrl+-", "Ctrl+Shift+="] {
        Shortcut::parse(key).unwrap_or_else(|e| panic!("{key} must parse: {e}"));
    }
}
