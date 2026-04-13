use std::time::Instant;

/// The visual shape of the text cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// Full-cell block cursor (semi-transparent overlay).
    Block,
    /// Thin vertical line at the left edge of the cell.
    Beam,
    /// Thin horizontal line at the bottom of the cell.
    Underline,
}

impl Default for CursorShape {
    fn default() -> Self {
        CursorShape::Beam
    }
}

/// Per-element cursor state, tracking blink timing and shape.
#[derive(Clone, Debug)]
pub struct CursorState {
    /// Visual shape of the cursor.
    pub shape: CursorShape,
    /// Whether the cursor is currently visible (toggles during blink).
    pub visible: bool,
    /// Milliseconds between blink toggles.
    pub blink_rate_ms: u32,
    /// When the visibility was last toggled.
    pub last_toggle: Instant,
    /// If true, blink is disabled and the cursor stays visible.
    pub steady: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            shape: CursorShape::default(),
            visible: true,
            blink_rate_ms: 530,
            last_toggle: Instant::now(),
            steady: false,
        }
    }
}

impl CursorState {
    /// Create a new CursorState with the given shape.
    pub fn with_shape(shape: CursorShape) -> Self {
        Self { shape, ..Default::default() }
    }

    /// Tick the blink timer. Returns true if visibility changed.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.steady {
            return false;
        }

        let elapsed = now.duration_since(self.last_toggle);
        let rate = std::time::Duration::from_millis(self.blink_rate_ms as u64);

        if elapsed >= rate {
            self.visible = !self.visible;
            self.last_toggle = now;
            true
        } else {
            false
        }
    }

    /// Reset the cursor to visible and restart the blink timer.
    /// Called on keystroke or focus gain.
    pub fn reset_blink(&mut self, now: Instant) {
        self.visible = true;
        self.last_toggle = now;
    }

    /// Compute the next instant at which the blink should toggle.
    /// Returns None if the cursor is steady (no blink).
    pub fn next_toggle_time(&self) -> Option<Instant> {
        if self.steady {
            return None;
        }
        let rate = std::time::Duration::from_millis(self.blink_rate_ms as u64);
        Some(self.last_toggle + rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn default_shape_is_beam() {
        let state = CursorState::default();
        assert_eq!(state.shape, CursorShape::Beam);
    }

    #[test]
    fn default_blink_rate() {
        let state = CursorState::default();
        assert_eq!(state.blink_rate_ms, 530);
    }

    #[test]
    fn starts_visible() {
        let state = CursorState::default();
        assert!(state.visible);
    }

    #[test]
    fn tick_toggles_visibility() {
        let start = Instant::now();
        let mut state = CursorState::default();
        state.last_toggle = start;

        // Before the rate elapses, no toggle
        let before = start + Duration::from_millis(500);
        assert!(!state.tick(before));
        assert!(state.visible);

        // After the rate elapses, toggle
        let after = start + Duration::from_millis(530);
        assert!(state.tick(after));
        assert!(!state.visible);

        // Toggle again
        let after2 = after + Duration::from_millis(530);
        assert!(state.tick(after2));
        assert!(state.visible);
    }

    #[test]
    fn steady_mode_prevents_blink() {
        let start = Instant::now();
        let mut state = CursorState::default();
        state.last_toggle = start;
        state.steady = true;

        let after = start + Duration::from_millis(1000);
        assert!(!state.tick(after));
        assert!(state.visible);
    }

    #[test]
    fn reset_blink_makes_visible() {
        let start = Instant::now();
        let mut state = CursorState::default();
        state.visible = false;
        state.last_toggle = start;

        let now = start + Duration::from_millis(100);
        state.reset_blink(now);
        assert!(state.visible);
        assert_eq!(state.last_toggle, now);
    }

    #[test]
    fn next_toggle_time_calculation() {
        let start = Instant::now();
        let mut state = CursorState::default();
        state.last_toggle = start;
        state.blink_rate_ms = 500;

        let expected = start + Duration::from_millis(500);
        assert_eq!(state.next_toggle_time(), Some(expected));
    }

    #[test]
    fn next_toggle_time_none_when_steady() {
        let mut state = CursorState::default();
        state.steady = true;
        assert_eq!(state.next_toggle_time(), None);
    }
}
