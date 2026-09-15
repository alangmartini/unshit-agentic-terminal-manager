//! Guards the grid movement cost paid by terminal output. Include periodic
//! contiguous snapshots so deferred copying is counted as part of the work.
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use unshit::core::cell_grid::{Cell, CellGrid};

fn terminal_scroll(c: &mut Criterion) {
    for (rows, cols) in [(37, 125), (80, 240)] {
        c.bench_function(
            &format!("terminal_scroll/{rows}x{cols}/64_lines_and_snapshot"),
            |b| {
                let mut grid = CellGrid::new(rows, cols);
                b.iter(|| {
                    for _ in 0..64 {
                        grid.shift_rows(0, 1, rows - 1);
                        for col in 0..cols {
                            let ch = if col < 78 { 'M' } else { ' ' };
                            grid.set_cell(rows - 1, col, Cell::with_char(ch));
                        }
                        grid.reset_line_identity(rows - 1);
                    }
                    let snapshot = grid.clone();
                    black_box(snapshot.cells());
                    black_box(snapshot.line_ids());
                });
            },
        );
    }
}

fn terminal_row_clear(c: &mut Criterion) {
    for (rows, cols) in [(37, 125), (80, 240)] {
        for (name, bulk) in [("per_cell", false), ("bulk", true)] {
            c.bench_function(&format!("terminal_row_clear/{rows}x{cols}/{name}"), |b| {
                let mut grid = CellGrid::new(rows, cols);
                let blank = Cell::with_char(' ');
                b.iter(|| {
                    for _ in 0..64 {
                        grid.shift_rows(0, 1, rows - 1);
                        if bulk {
                            grid.fill_row(rows - 1, blank);
                        } else {
                            for col in 0..cols {
                                grid.set_cell(rows - 1, col, blank);
                            }
                        }
                        grid.reset_line_identity(rows - 1);
                    }
                    let snapshot = grid.clone();
                    black_box(snapshot.cells());
                    black_box(snapshot.line_ids());
                });
            });
        }
    }
}

fn terminal_row_copy(c: &mut Criterion) {
    for (rows, cols) in [(37, 125), (80, 240)] {
        for (name, bulk) in [("per_cell", false), ("bulk", true)] {
            c.bench_function(&format!("terminal_row_copy/{rows}x{cols}/{name}"), |b| {
                let mut grid = CellGrid::new(rows, cols);
                for row in 0..rows {
                    grid.fill_row(row, Cell::with_char((b'a' + (row % 26) as u8) as char));
                }
                b.iter(|| {
                    for _ in 0..64 {
                        let row = if bulk {
                            grid.row_cells(0).unwrap().to_vec()
                        } else {
                            let mut row = Vec::with_capacity(cols);
                            for col in 0..cols {
                                row.push(grid.get_cell(0, col).copied().unwrap_or_default());
                            }
                            row
                        };
                        black_box(row);
                        grid.shift_rows(0, 1, rows - 1);
                    }
                });
            });
        }
    }
}

fn daemon_terminal_parse(c: &mut Criterion) {
    let mut input = Vec::with_capacity(80 * 1024);
    for _ in 0..1024 {
        input.extend_from_slice(
            b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789ABCDEF\r\n",
        );
    }
    c.bench_function("daemon_terminal_parse/1024_lines", |b| {
        let mut terminal = unshit_terminal_core::Terminal::new(37, 125, 10_000);
        b.iter(|| {
            for chunk in input.chunks(4096) {
                terminal.process_bytes(black_box(chunk));
            }
            black_box(terminal.grid());
        });
    });
}

fn terminal_row_write(c: &mut Criterion) {
    for count in [1, 8, 32, 78, 128] {
        let cells: Vec<_> = (0..count)
            .map(|i| Cell::with_char((b'A' + (i % 26) as u8) as char))
            .collect();
        for (name, bulk) in [("per_cell", false), ("bulk", true)] {
            c.bench_function(&format!("terminal_row_write/{count}_cells/{name}"), |b| {
                let mut grid = CellGrid::new(37, 240);
                b.iter(|| {
                    for _ in 0..64 {
                        grid.shift_rows(0, 1, 36);
                        let cells = black_box(cells.as_slice());
                        if bulk {
                            grid.set_row_cells(36, 0, cells);
                        } else {
                            for (col, cell) in cells.iter().enumerate() {
                                grid.set_cell(36, col, *cell);
                            }
                        }
                    }
                    black_box(grid.clone());
                });
            });
        }
    }
}

criterion_group!(
    benches,
    terminal_scroll,
    terminal_row_clear,
    terminal_row_copy,
    terminal_row_write,
    daemon_terminal_parse
);
criterion_main!(benches);
