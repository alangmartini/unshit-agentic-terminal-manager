//! Align contiguous replacement blocks before pagination. Indices reference the
//! original patch so switching views never copies source text or queries Git.
use super::git::Line;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub line: usize,
    pub note: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Shared(usize),
    Pair {
        old: Option<Cell>,
        new: Option<Cell>,
    },
}

pub fn align(lines: &[Line]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !matches!(lines[index].kind, "removed" | "added") {
            rows.push(Row::Shared(index));
            index += 1;
            continue;
        }
        let (mut old, mut new) = (Vec::<Cell>::new(), Vec::<Cell>::new());
        let mut old_side = true;
        while let Some(line) = lines.get(index) {
            match line.kind {
                "removed" | "added" => {
                    old_side = line.kind == "removed";
                    let side = if old_side { &mut old } else { &mut new };
                    side.push(Cell {
                        line: index,
                        note: None,
                    });
                }
                "meta" if line.text.starts_with("\\ ") => {
                    let side = if old_side { &mut old } else { &mut new };
                    if let Some(cell) = side.last_mut() {
                        cell.note = Some(index);
                    }
                }
                _ => break,
            }
            index += 1;
        }
        let count = old.len().max(new.len());
        let (mut old, mut new) = (old.into_iter(), new.into_iter());
        rows.extend((0..count).map(|_| Row::Pair {
            old: old.next(),
            new: new.next(),
        }));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_review::git::parse_patch;

    #[test]
    fn unequal_replacements_keep_context_and_hunks_aligned() {
        let lines =
            parse_patch("@@ -1,3 +1,4 @@\n-old\n+new\n+extra\n same\n@@ -9 +10 @@\n-gone\n");
        assert_eq!(
            align(&lines),
            vec![
                Row::Shared(0),
                Row::Pair {
                    old: Some(Cell {
                        line: 1,
                        note: None
                    }),
                    new: Some(Cell {
                        line: 2,
                        note: None
                    })
                },
                Row::Pair {
                    old: None,
                    new: Some(Cell {
                        line: 3,
                        note: None
                    })
                },
                Row::Shared(4),
                Row::Shared(5),
                Row::Pair {
                    old: Some(Cell {
                        line: 6,
                        note: None
                    }),
                    new: None
                },
            ]
        );
    }

    #[test]
    fn newline_notes_belong_to_their_side_without_shifting_replacements() {
        let lines = parse_patch("@@ -1 +1,2 @@\n-old\n\\ No newline at end of file\n+new\n+extra\n\\ No newline at end of file\n");
        assert_eq!(
            align(&lines),
            vec![
                Row::Shared(0),
                Row::Pair {
                    old: Some(Cell {
                        line: 1,
                        note: Some(2)
                    }),
                    new: Some(Cell {
                        line: 3,
                        note: None
                    })
                },
                Row::Pair {
                    old: None,
                    new: Some(Cell {
                        line: 4,
                        note: Some(5)
                    })
                },
            ]
        );
    }

    #[test]
    fn alignment_precedes_page_boundaries_and_preserves_metadata() {
        let mut patch = "Binary files differ\n@@ -1,205 +1,205 @@\n".to_string();
        for _ in 0..205 {
            patch.push_str("-old\n");
        }
        for _ in 0..205 {
            patch.push_str("+new\n");
        }
        let rows = align(&parse_patch(&patch));
        assert_eq!(rows.len(), 207);
        assert!(matches!(
            rows[200],
            Row::Pair {
                old: Some(_),
                new: Some(_)
            }
        ));
        assert_eq!(rows[0], Row::Shared(0));
        assert!(align(&[]).is_empty());
    }
}
