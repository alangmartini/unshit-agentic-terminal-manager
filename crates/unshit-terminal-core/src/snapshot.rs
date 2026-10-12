use serde::{Deserialize, Serialize};

use crate::cell::Cell;
use crate::grid::Grid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub grid: Grid,
    pub scrollback: Vec<Vec<Cell>>,
    /// Mouse protocol state must survive UI reattachment to daemon sessions.
    #[serde(default)]
    pub mouse_modes: MouseModes,
    /// DECSET 1004 focus-reporting state must survive UI reattachment so the
    /// bridge can continue sending focus-in/focus-out reports to the child.
    #[serde(default)]
    pub focus_reporting_1004: bool,
    /// DECSET 1007 alternate scroll must survive reattachment so the wheel
    /// keeps sending cursor keys to a program on the alternate screen.
    #[serde(default)]
    pub alternate_scroll_1007: bool,
    /// Whether the alternate screen was active. The snapshot carries only
    /// the visible grid, so the UI cannot rebuild the parked main screen,
    /// but it needs to know the program owns the screen.
    #[serde(default)]
    pub alt_screen: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseModes {
    pub report_1000: bool,
    pub report_1002: bool,
    pub report_1003: bool,
    pub sgr: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_snapshot_without_mouse_modes_remains_readable() {
        let old = serde_json::json!({
            "grid": Grid::new(2, 3),
            "scrollback": [],
        });
        let snapshot: Snapshot = serde_json::from_value(old).unwrap();
        assert_eq!(snapshot.mouse_modes, MouseModes::default());
        assert!(!snapshot.focus_reporting_1004);
        assert!(!snapshot.alternate_scroll_1007);
        assert!(!snapshot.alt_screen);
    }

    #[test]
    fn snapshot_round_trips_through_serde_json() {
        let grid = Grid::new(2, 3);
        let snap = Snapshot {
            mouse_modes: MouseModes::default(),
            focus_reporting_1004: true,
            alternate_scroll_1007: true,
            alt_screen: true,
            grid: grid.clone(),
            scrollback: vec![vec![Cell::BLANK; 3]],
        };
        let j = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&j).unwrap();
        assert_eq!(back, snap);
    }
}
