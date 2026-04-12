use unshit_app::notification::{AttentionUrgency, BellConfig, BellState, BellStyle};
use unshit_core::style::parse::CompiledStylesheet;
use unshit_core::style::types;

// ---------------------------------------------------------------------------
// Test: request_attention maps to correct winit urgency level
// ---------------------------------------------------------------------------

#[test]
fn attention_urgency_maps_to_winit_informational() {
    let winit_type = AttentionUrgency::Informational.to_winit();
    assert_eq!(winit_type, winit::window::UserAttentionType::Informational);
}

#[test]
fn attention_urgency_maps_to_winit_critical() {
    let winit_type = AttentionUrgency::Critical.to_winit();
    assert_eq!(winit_type, winit::window::UserAttentionType::Critical);
}

// ---------------------------------------------------------------------------
// Test: visual bell overlay has correct opacity (white at ~10%)
// ---------------------------------------------------------------------------

#[test]
fn visual_bell_overlay_initial_opacity_near_ten_percent() {
    let mut state = BellState::new(BellConfig { style: BellStyle::Visual, rate_limit_ms: 0 });
    state.try_bell();

    let opacity = state.visual_bell_opacity();
    // Immediately after firing, the opacity should be close to 0.1 (10%).
    assert!(opacity > 0.05 && opacity <= 0.1, "expected opacity near 0.1, got {opacity}");
}

// ---------------------------------------------------------------------------
// Test: bell rate limiting suppresses rapid repeated bells within 100ms
// ---------------------------------------------------------------------------

#[test]
fn rate_limiting_suppresses_rapid_bells() {
    let mut state = BellState::new(BellConfig { style: BellStyle::Both, rate_limit_ms: 100 });

    // First bell succeeds.
    assert!(state.try_bell(), "first bell should succeed");

    // Immediate repeat should be rate-limited.
    assert!(!state.try_bell(), "second bell within 100ms should be suppressed");

    // Third immediate repeat also suppressed.
    assert!(!state.try_bell(), "third bell within 100ms should be suppressed");
}

#[test]
fn rate_limiting_allows_bell_after_window_elapses() {
    let mut state = BellState::new(BellConfig { style: BellStyle::Both, rate_limit_ms: 10 });

    assert!(state.try_bell(), "first bell should succeed");

    // Simulate time passing by backdating the last bell.
    // The internal last_bell is set on try_bell; we sleep briefly.
    std::thread::sleep(std::time::Duration::from_millis(15));

    assert!(state.try_bell(), "bell after rate limit window should succeed");
}

// ---------------------------------------------------------------------------
// Test: BellConfig respects visual-only mode (no attention request)
// ---------------------------------------------------------------------------

#[test]
fn bell_config_visual_only_no_attention() {
    let state = BellState::new(BellConfig { style: BellStyle::Visual, rate_limit_ms: 100 });
    assert!(state.should_show_visual(), "visual-only should show visual");
    assert!(!state.should_request_attention(), "visual-only should NOT request attention");
}

#[test]
fn bell_config_attention_only_no_visual() {
    let state = BellState::new(BellConfig { style: BellStyle::Attention, rate_limit_ms: 100 });
    assert!(!state.should_show_visual(), "attention-only should NOT show visual");
    assert!(state.should_request_attention(), "attention-only should request attention");
}

#[test]
fn bell_config_both_enables_visual_and_attention() {
    let state = BellState::new(BellConfig { style: BellStyle::Both, rate_limit_ms: 100 });
    assert!(state.should_show_visual());
    assert!(state.should_request_attention());
}

// ---------------------------------------------------------------------------
// Test: bell-style CSS property parses correctly for all variants
// ---------------------------------------------------------------------------

#[test]
fn css_bell_style_visual_parses() {
    let css = ".test { bell-style: visual; }";
    let sheet = CompiledStylesheet::parse(css);
    assert!(!sheet.rules.is_empty(), "should have parsed a rule");

    let mut style = types::ComputedStyle::default();
    for decl in &sheet.rules[0].declarations {
        unshit_core::style::parse::apply_declaration(&mut style, decl);
    }
    assert_eq!(style.bell_style, types::BellStyle::Visual);
}

#[test]
fn css_bell_style_attention_parses() {
    let css = ".test { bell-style: attention; }";
    let sheet = CompiledStylesheet::parse(css);
    assert!(!sheet.rules.is_empty());

    let mut style = types::ComputedStyle::default();
    for decl in &sheet.rules[0].declarations {
        unshit_core::style::parse::apply_declaration(&mut style, decl);
    }
    assert_eq!(style.bell_style, types::BellStyle::Attention);
}

#[test]
fn css_bell_style_both_parses() {
    let css = ".test { bell-style: both; }";
    let sheet = CompiledStylesheet::parse(css);
    assert!(!sheet.rules.is_empty());

    let mut style = types::ComputedStyle::default();
    for decl in &sheet.rules[0].declarations {
        unshit_core::style::parse::apply_declaration(&mut style, decl);
    }
    assert_eq!(style.bell_style, types::BellStyle::Both);
}

#[test]
fn css_bell_style_none_parses() {
    let css = ".test { bell-style: none; }";
    let sheet = CompiledStylesheet::parse(css);
    assert!(!sheet.rules.is_empty());

    let mut style = types::ComputedStyle::default();
    for decl in &sheet.rules[0].declarations {
        unshit_core::style::parse::apply_declaration(&mut style, decl);
    }
    assert_eq!(style.bell_style, types::BellStyle::None);
}

// ---------------------------------------------------------------------------
// Test: bell-style: none suppresses all bell output
// ---------------------------------------------------------------------------

#[test]
fn bell_style_none_suppresses_all() {
    let mut state = BellState::new(BellConfig { style: BellStyle::None, rate_limit_ms: 0 });

    assert!(!state.try_bell(), "BellStyle::None should suppress bell");
    assert!(!state.visual_bell_active, "no visual bell should activate");
    assert!(!state.should_request_attention(), "no attention request with None");
    assert!(!state.should_show_visual(), "no visual with None");
}

// ---------------------------------------------------------------------------
// Test: default BellConfig works without explicit setup
// ---------------------------------------------------------------------------

#[test]
fn default_bell_config_is_both_with_100ms_rate_limit() {
    let cfg = BellConfig::default();
    assert_eq!(cfg.style, BellStyle::Both);
    assert_eq!(cfg.rate_limit_ms, 100);
}

#[test]
fn default_bell_state_fires_correctly() {
    let mut state = BellState::new(BellConfig::default());
    // Default config is Both + 100ms rate limit.
    assert!(state.try_bell(), "first bell with default config should fire");
    assert!(state.visual_bell_active, "visual bell should be active");
    assert!(state.should_request_attention(), "should request attention");
    assert!(state.should_show_visual(), "should show visual");
}

// ---------------------------------------------------------------------------
// Test: BellStyle enum serialization/display
// ---------------------------------------------------------------------------

#[test]
fn bell_style_display_round_trip() {
    let variants = [
        (BellStyle::Visual, "visual"),
        (BellStyle::Attention, "attention"),
        (BellStyle::Both, "both"),
        (BellStyle::None, "none"),
    ];
    for (variant, expected_str) in variants {
        assert_eq!(
            variant.to_string(),
            expected_str,
            "BellStyle::{variant:?} should display as {expected_str:?}"
        );
    }
}

#[test]
fn core_bell_style_display() {
    // The core types::BellStyle also implements Display.
    assert_eq!(types::BellStyle::Visual.to_string(), "visual");
    assert_eq!(types::BellStyle::Attention.to_string(), "attention");
    assert_eq!(types::BellStyle::Both.to_string(), "both");
    assert_eq!(types::BellStyle::None.to_string(), "none");
}

// ---------------------------------------------------------------------------
// Test: visual bell tick deactivates after duration
// ---------------------------------------------------------------------------

#[test]
fn visual_bell_deactivates_after_duration() {
    let mut state = BellState::new(BellConfig { style: BellStyle::Visual, rate_limit_ms: 0 });
    state.try_bell();
    assert!(state.visual_bell_active);

    // Fast-forward the start time into the past so the bell has expired.
    state.visual_bell_start =
        Some(std::time::Instant::now() - std::time::Duration::from_millis(200));

    let still_active = state.tick();
    assert!(!still_active, "tick should return false after expiry");
    assert!(!state.visual_bell_active, "visual_bell_active should be cleared");
}

// ---------------------------------------------------------------------------
// Test: ComputedStyle default bell_style is Both
// ---------------------------------------------------------------------------

#[test]
fn computed_style_default_bell_style_is_both() {
    let style = types::ComputedStyle::default();
    assert_eq!(style.bell_style, types::BellStyle::Both);
}
