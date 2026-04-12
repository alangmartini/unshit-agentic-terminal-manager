use std::sync::{Mutex, OnceLock};
use unshit_app::clipboard::{ClipboardContext, ClipboardError};

/// Shared clipboard context. On Windows, creating multiple arboard::Clipboard
/// instances concurrently from parallel test threads can trigger heap corruption.
/// Sharing a single instance and serializing access avoids this.
fn shared_ctx() -> &'static Mutex<&'static ClipboardContext> {
    static CTX: OnceLock<Mutex<&'static ClipboardContext>> = OnceLock::new();
    CTX.get_or_init(|| {
        let ctx = Box::leak(Box::new(ClipboardContext::new()));
        Mutex::new(ctx)
    })
}

/// Helper: try to access the clipboard, skip the test if the system
/// clipboard is not available (headless CI, etc.).
fn skip_if_unavailable(ctx: &ClipboardContext) -> bool {
    match ctx.read_text() {
        Ok(_) => false,
        Err(ClipboardError::Unavailable(_)) => {
            eprintln!("SKIP: system clipboard not available in this environment");
            true
        }
        Err(_) => false,
    }
}

#[test]
fn write_then_read_roundtrip() {
    let guard = shared_ctx().lock().unwrap();
    let ctx = *guard;
    if skip_if_unavailable(ctx) {
        return;
    }

    ctx.write_text("hello from unshit").unwrap();
    let text = ctx.read_text().unwrap();
    assert_eq!(text, "hello from unshit");
}

#[test]
fn empty_clipboard_returns_empty_string() {
    let guard = shared_ctx().lock().unwrap();
    let ctx = *guard;
    if skip_if_unavailable(ctx) {
        return;
    }

    ctx.clear().unwrap();
    let text = ctx.read_text().unwrap();
    assert!(text.is_empty(), "Expected empty string after clear, got: {:?}", text);
}

#[test]
fn unicode_cjk() {
    let guard = shared_ctx().lock().unwrap();
    let ctx = *guard;
    if skip_if_unavailable(ctx) {
        return;
    }

    let cjk = "\u{4F60}\u{597D}\u{4E16}\u{754C}"; // Chinese characters
    ctx.write_text(cjk).unwrap();
    let result = ctx.read_text().unwrap();
    assert_eq!(result, cjk);
}

#[test]
fn unicode_emoji() {
    let guard = shared_ctx().lock().unwrap();
    let ctx = *guard;
    if skip_if_unavailable(ctx) {
        return;
    }

    let emoji = "\u{1F600}\u{1F4CB}\u{2702}\u{FE0F}"; // grinning face, clipboard, scissors
    ctx.write_text(emoji).unwrap();
    let result = ctx.read_text().unwrap();
    assert_eq!(result, emoji);
}

#[test]
fn unicode_rtl() {
    let guard = shared_ctx().lock().unwrap();
    let ctx = *guard;
    if skip_if_unavailable(ctx) {
        return;
    }

    let rtl = "\u{0645}\u{0631}\u{062D}\u{0628}\u{0627}"; // Arabic "mrhba"
    ctx.write_text(rtl).unwrap();
    let result = ctx.read_text().unwrap();
    assert_eq!(result, rtl);
}

#[test]
fn clipboard_error_does_not_panic() {
    // Use the shared context to avoid concurrent initialization issues.
    // Even if the clipboard is unavailable, calling methods should return
    // Err variants, not panic.
    let guard = shared_ctx().lock().unwrap();
    let ctx = *guard;
    let _read = ctx.read_text();
    let _write = ctx.write_text("test");
    let _clear = ctx.clear();
    // If we reach here without panicking, the test passes.
}

#[test]
fn overwrite_replaces_previous_content() {
    let guard = shared_ctx().lock().unwrap();
    let ctx = *guard;
    if skip_if_unavailable(ctx) {
        return;
    }

    ctx.write_text("first").unwrap();
    ctx.write_text("second").unwrap();
    let text = ctx.read_text().unwrap();
    assert_eq!(text, "second");
}

#[test]
fn clear_after_write_empties_clipboard() {
    let guard = shared_ctx().lock().unwrap();
    let ctx = *guard;
    if skip_if_unavailable(ctx) {
        return;
    }

    ctx.write_text("some text").unwrap();
    ctx.clear().unwrap();
    let text = ctx.read_text().unwrap();
    assert!(text.is_empty(), "Expected empty after clear, got: {:?}", text);
}

/// Verify that ClipboardEvent variants exist and can be constructed.
#[test]
fn clipboard_event_variants_exist() {
    use unshit_core::event::ClipboardEvent;

    let _copy = ClipboardEvent::Copy;
    let _paste = ClipboardEvent::Paste("test".into());
    let _cut = ClipboardEvent::Cut;

    // Verify Debug and Clone
    let paste = ClipboardEvent::Paste("hello".into());
    let cloned = paste.clone();
    assert_eq!(paste, cloned);
}

/// Verify that the Event enum has a Clipboard variant.
#[test]
fn event_enum_has_clipboard_variant() {
    use unshit_core::event::{ClipboardEvent, Event};

    let evt = Event::Clipboard(ClipboardEvent::Copy);
    // Pattern matching should work
    match evt {
        Event::Clipboard(ClipboardEvent::Copy) => {}
        _ => panic!("Expected Event::Clipboard(Copy)"),
    }
}
