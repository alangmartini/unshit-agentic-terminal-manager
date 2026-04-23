use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::cell::Cell;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scrollback {
    lines: VecDeque<Vec<Cell>>,
    max_lines: usize,
}

impl Scrollback {
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            max_lines,
        }
    }

    pub fn max_lines(&self) -> usize {
        self.max_lines
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn push(&mut self, line: Vec<Cell>) {
        if self.max_lines == 0 {
            return;
        }
        self.lines.push_back(line);
        while self.lines.len() > self.max_lines {
            self.lines.pop_front();
        }
    }

    pub fn lines(&self) -> impl Iterator<Item = &Vec<Cell>> {
        self.lines.iter()
    }

    pub fn tail(&self, n: usize) -> Vec<Vec<Cell>> {
        let start = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(start).cloned().collect()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ch: char) -> Vec<Cell> {
        vec![Cell { ch, ..Cell::BLANK }]
    }

    #[test]
    fn push_beyond_max_lines_evicts_oldest() {
        let mut sb = Scrollback::new(3);
        sb.push(sample('a'));
        sb.push(sample('b'));
        sb.push(sample('c'));
        sb.push(sample('d'));
        assert_eq!(sb.len(), 3);
        let collected: Vec<char> = sb.lines().map(|l| l[0].ch).collect();
        assert_eq!(collected, vec!['b', 'c', 'd']);
    }

    #[test]
    fn tail_returns_at_most_n_oldest_first() {
        let mut sb = Scrollback::new(10);
        for ch in ['a', 'b', 'c', 'd', 'e'] {
            sb.push(sample(ch));
        }
        let t2 = sb.tail(2);
        assert_eq!(t2.len(), 2);
        assert_eq!(t2[0][0].ch, 'd');
        assert_eq!(t2[1][0].ch, 'e');

        let t_all = sb.tail(100);
        assert_eq!(t_all.len(), 5);
        assert_eq!(t_all.first().unwrap()[0].ch, 'a');
        assert_eq!(t_all.last().unwrap()[0].ch, 'e');
    }

    #[test]
    fn tail_zero_is_empty() {
        let mut sb = Scrollback::new(10);
        sb.push(sample('a'));
        assert!(sb.tail(0).is_empty());
    }

    #[test]
    fn len_and_is_empty_stay_consistent() {
        let mut sb = Scrollback::new(5);
        assert!(sb.is_empty());
        assert_eq!(sb.len(), 0);
        sb.push(sample('a'));
        assert!(!sb.is_empty());
        assert_eq!(sb.len(), 1);
        sb.clear();
        assert!(sb.is_empty());
        assert_eq!(sb.len(), 0);
    }

    #[test]
    fn zero_max_rejects_pushes() {
        let mut sb = Scrollback::new(0);
        sb.push(sample('a'));
        assert!(sb.is_empty());
    }

    #[test]
    fn max_lines_reports_configured_value() {
        let sb = Scrollback::new(42);
        assert_eq!(sb.max_lines(), 42);
    }

    #[test]
    fn lines_iter_yields_in_insertion_order() {
        let mut sb = Scrollback::new(10);
        for ch in ['x', 'y', 'z'] {
            sb.push(sample(ch));
        }
        let out: Vec<char> = sb.lines().map(|l| l[0].ch).collect();
        assert_eq!(out, vec!['x', 'y', 'z']);
    }
}
