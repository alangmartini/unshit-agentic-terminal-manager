use serde::{Deserialize, Serialize};

use crate::cell::Cell;
use crate::grid::Grid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub grid: Grid,
    pub scrollback: Vec<Vec<Cell>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trips_through_serde_json() {
        let grid = Grid::new(2, 3);
        let snap = Snapshot {
            grid: grid.clone(),
            scrollback: vec![vec![Cell::BLANK; 3]],
        };
        let j = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&j).unwrap();
        assert_eq!(back, snap);
    }
}
