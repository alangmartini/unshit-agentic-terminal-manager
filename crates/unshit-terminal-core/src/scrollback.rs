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

    /// Pop up to `n` newest lines off the back of the buffer and return
    /// them in oldest-first order so callers can place them top-to-bottom
    /// in a grid. Returns an empty vec when `n == 0` or the buffer is
    /// empty. Used by `Terminal::resize` to lift scrollback into newly
    /// available rows on grow.
    pub fn pop_back_n(&mut self, n: usize) -> Vec<Vec<Cell>> {
        if n == 0 {
            return Vec::new();
        }
        let take = n.min(self.lines.len());
        let mut popped: Vec<Vec<Cell>> = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(line) = self.lines.pop_back() {
                popped.push(line);
            }
        }
        popped.reverse();
        popped
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

    #[test]
    fn pop_back_n_returns_newest_n_oldest_first() {
        let mut sb = Scrollback::new(10);
        for ch in ['a', 'b', 'c', 'd', 'e'] {
            sb.push(sample(ch));
        }
        let popped = sb.pop_back_n(2);
        assert_eq!(popped.len(), 2);
        // Newest two were 'd' and 'e'; returned with the older first so
        // callers can place them top-to-bottom in a grid.
        assert_eq!(popped[0][0].ch, 'd');
        assert_eq!(popped[1][0].ch, 'e');
        assert_eq!(sb.len(), 3);
        let remaining: Vec<char> = sb.lines().map(|l| l[0].ch).collect();
        assert_eq!(remaining, vec!['a', 'b', 'c']);
    }

    #[test]
    fn pop_back_n_more_than_len_drains_all() {
        let mut sb = Scrollback::new(10);
        sb.push(sample('a'));
        sb.push(sample('b'));
        let popped = sb.pop_back_n(10);
        assert_eq!(popped.len(), 2);
        assert_eq!(popped[0][0].ch, 'a');
        assert_eq!(popped[1][0].ch, 'b');
        assert!(sb.is_empty());
    }

    #[test]
    fn pop_back_n_zero_is_noop() {
        let mut sb = Scrollback::new(10);
        sb.push(sample('a'));
        let popped = sb.pop_back_n(0);
        assert!(popped.is_empty());
        assert_eq!(sb.len(), 1);
    }

    #[test]
    fn pop_back_n_on_empty_returns_empty() {
        let mut sb = Scrollback::new(10);
        let popped = sb.pop_back_n(3);
        assert!(popped.is_empty());
    }
}
