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

criterion_group!(benches, terminal_scroll, daemon_terminal_parse);
criterion_main!(benches);
