//! Bell and notification support.
//!
//! Provides window attention requests, visual bell overlays, and an optional
//! OS-level notification interface gated behind the `notifications` feature.

use std::time::Instant;

/// Window attention urgency level, mapping to winit's `UserAttentionType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttentionUrgency {
    /// Informational: subtle hint (e.g. taskbar flash).
    Informational,
    /// Critical: more aggressive alert (e.g. flashing titlebar).
    Critical,
}

impl AttentionUrgency {
    /// Convert to the winit `UserAttentionType` variant.
    pub fn to_winit(self) -> winit::window::UserAttentionType {
        match self {
            AttentionUrgency::Informational => winit::window::UserAttentionType::Informational,
            AttentionUrgency::Critical => winit::window::UserAttentionType::Critical,
        }
    }
}

/// Controls how bell signals are delivered (mirrors the CSS `bell-style`
/// property from `unshit_core::style::types::BellStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BellStyle {
    Visual,
    Attention,
    Both,
    None,
}

impl Default for BellStyle {
    fn default() -> Self {
        BellStyle::Both
    }
}

impl std::fmt::Display for BellStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BellStyle::Visual => write!(f, "visual"),
            BellStyle::Attention => write!(f, "attention"),
            BellStyle::Both => write!(f, "both"),
            BellStyle::None => write!(f, "none"),
        }
    }
}

/// Configuration for the bell subsystem.
#[derive(Clone, Debug)]
pub struct BellConfig {
    /// Which outputs to activate when a bell fires.
    pub style: BellStyle,
    /// Minimum interval between bells in milliseconds. Bells arriving faster
    /// than this are silently suppressed to prevent bell storms.
    pub rate_limit_ms: u32,
}

impl Default for BellConfig {
    fn default() -> Self {
        Self { style: BellStyle::Both, rate_limit_ms: 100 }
    }
}

/// Runtime state for the bell subsystem, stored inside `AppState`.
pub struct BellState {
    pub config: BellConfig,
    /// Timestamp of the last bell that was actually delivered.
    last_bell: Option<Instant>,
    /// Whether a visual bell overlay is currently active (fading out).
    pub visual_bell_active: bool,
    /// Timestamp when the current visual bell started (for fade-out timing).
    pub visual_bell_start: Option<Instant>,
}

impl BellState {
    pub fn new(config: BellConfig) -> Self {
        Self { config, last_bell: None, visual_bell_active: false, visual_bell_start: None }
    }

    /// Duration of the visual bell overlay fade-out in milliseconds.
    pub const VISUAL_BELL_DURATION_MS: u32 = 100;

    /// Try to fire a bell. Returns `true` if the bell was accepted (not
    /// rate-limited). Returns `false` if suppressed.
    pub fn try_bell(&mut self) -> bool {
        if self.config.style == BellStyle::None {
            return false;
        }

        let now = Instant::now();
        if let Some(last) = self.last_bell {
            let elapsed_ms = now.duration_since(last).as_millis() as u32;
            if elapsed_ms < self.config.rate_limit_ms {
                return false;
            }
        }

        self.last_bell = Some(now);

        // Activate visual bell if the style includes it.
        if self.config.style == BellStyle::Visual || self.config.style == BellStyle::Both {
            self.visual_bell_active = true;
            self.visual_bell_start = Some(now);
        }

        true
    }

    /// Returns true if the style includes a window attention request.
    pub fn should_request_attention(&self) -> bool {
        matches!(self.config.style, BellStyle::Attention | BellStyle::Both)
    }

    /// Returns true if the style includes a visual bell overlay.
    pub fn should_show_visual(&self) -> bool {
        matches!(self.config.style, BellStyle::Visual | BellStyle::Both)
    }

    /// Compute the current overlay opacity (0.0 to 0.1) based on elapsed
    /// time since the bell fired. Returns 0.0 if no visual bell is active.
    pub fn visual_bell_opacity(&self) -> f32 {
        let Some(start) = self.visual_bell_start else {
            return 0.0;
        };
        if !self.visual_bell_active {
            return 0.0;
        }

        let elapsed_ms = Instant::now().duration_since(start).as_millis() as u32;
        if elapsed_ms >= Self::VISUAL_BELL_DURATION_MS {
            return 0.0;
        }

        // Linear fade from 0.1 (10% opacity) down to 0.0 over the duration.
        let progress = elapsed_ms as f32 / Self::VISUAL_BELL_DURATION_MS as f32;
        0.1 * (1.0 - progress)
    }

    /// Tick the visual bell state. Call each frame. Returns true if the
    /// overlay should still be rendered.
    pub fn tick(&mut self) -> bool {
        if !self.visual_bell_active {
            return false;
        }

        let opacity = self.visual_bell_opacity();
        if opacity <= 0.0 {
            self.visual_bell_active = false;
            self.visual_bell_start = None;
            return false;
        }

        true
    }
}

// -------------------------------------------------------------------------
// OS-level notifications (feature-gated)
// -------------------------------------------------------------------------

/// Send an OS-level notification. This is a best-effort operation; failures
/// are logged but do not propagate.
///
/// Requires the `notifications` Cargo feature. Without the feature, calls
/// compile but do nothing.
#[cfg(feature = "notifications")]
pub fn send_os_notification(title: &str, body: &str) {
    // TODO: integrate platform-specific notification crates
    // (notify-rust on Linux/macOS, winrt-notification on Windows).
    // For now, log the notification intent.
    log::info!("OS notification: title={title:?} body={body:?}");
}

#[cfg(not(feature = "notifications"))]
pub fn send_os_notification(_title: &str, _body: &str) {
    // Feature not enabled; no-op.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bell_config() {
        let cfg = BellConfig::default();
        assert_eq!(cfg.style, BellStyle::Both);
        assert_eq!(cfg.rate_limit_ms, 100);
    }

    #[test]
    fn bell_style_display() {
        assert_eq!(BellStyle::Visual.to_string(), "visual");
        assert_eq!(BellStyle::Attention.to_string(), "attention");
        assert_eq!(BellStyle::Both.to_string(), "both");
        assert_eq!(BellStyle::None.to_string(), "none");
    }

    #[test]
    fn attention_urgency_to_winit() {
        assert_eq!(
            AttentionUrgency::Informational.to_winit(),
            winit::window::UserAttentionType::Informational,
        );
        assert_eq!(
            AttentionUrgency::Critical.to_winit(),
            winit::window::UserAttentionType::Critical,
        );
    }

    #[test]
    fn bell_state_none_suppresses_everything() {
        let mut state = BellState::new(BellConfig { style: BellStyle::None, rate_limit_ms: 100 });
        assert!(!state.try_bell(), "BellStyle::None should suppress bells");
        assert!(!state.visual_bell_active);
    }

    #[test]
    fn bell_state_rate_limiting() {
        let mut state = BellState::new(BellConfig { style: BellStyle::Both, rate_limit_ms: 100 });

        // First bell should succeed.
        assert!(state.try_bell());

        // Immediate second bell should be rate-limited.
        assert!(!state.try_bell(), "rapid repeat should be suppressed");
    }

    #[test]
    fn bell_state_visual_only_no_attention() {
        let state = BellState::new(BellConfig { style: BellStyle::Visual, rate_limit_ms: 100 });
        assert!(state.should_show_visual());
        assert!(!state.should_request_attention());
    }

    #[test]
    fn bell_state_attention_only_no_visual() {
        let state = BellState::new(BellConfig { style: BellStyle::Attention, rate_limit_ms: 100 });
        assert!(!state.should_show_visual());
        assert!(state.should_request_attention());
    }

    #[test]
    fn visual_bell_opacity_starts_near_ten_percent() {
        let mut state = BellState::new(BellConfig { style: BellStyle::Visual, rate_limit_ms: 100 });
        state.try_bell();

        let opacity = state.visual_bell_opacity();
        // Should be close to 0.1 immediately after bell fires.
        assert!(opacity > 0.05 && opacity <= 0.1, "initial opacity {opacity} should be near 0.1");
    }

    #[test]
    fn tick_deactivates_after_duration() {
        let mut state = BellState::new(BellConfig { style: BellStyle::Visual, rate_limit_ms: 0 });
        state.try_bell();
        assert!(state.visual_bell_active);

        // Fake the start time far in the past so the bell has expired.
        state.visual_bell_start = Some(Instant::now() - std::time::Duration::from_millis(200));
        let still_active = state.tick();
        assert!(!still_active, "should deactivate after duration elapses");
        assert!(!state.visual_bell_active);
    }
}
