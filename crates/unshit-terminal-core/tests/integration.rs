use unshit_terminal_core::{color_256, CellAttrs, Color, Snapshot, Terminal};

#[test]
fn full_session_reconstructs_from_snapshot() {
    let mut t = Terminal::new(3, 10, 100);
    t.process_bytes(b"\x1b[1;31mhello\x1b[0m\r\n");
    t.process_bytes(b"world\r\n");
    t.process_bytes(b"done");

    let snap = t.snapshot(1000);
    let json = serde_json::to_string(&snap).expect("snapshot should serialize");
    let back: Snapshot = serde_json::from_str(&json).expect("snapshot should deserialize");
    assert_eq!(back, snap);

    assert_eq!(snap.grid.rows(), 3);
    assert_eq!(snap.grid.cols(), 10);
    assert_eq!(snap.grid.cursor(), (2, 4));
}

#[test]
fn scrollback_captures_rows_in_order_of_eviction() {
    let mut t = Terminal::new(2, 10, 50);
    t.process_bytes(b"AAAAA\r\nBBBBB\r\nCCCCC\r\nDDDDD");

    let r0: String = t.grid().row(0).unwrap().iter().map(|c| c.ch).collect();
    let r1: String = t.grid().row(1).unwrap().iter().map(|c| c.ch).collect();
    assert!(r0.starts_with("CCCCC"));
    assert!(r1.starts_with("DDDDD"));

    // Text-bearing rows are evicted in order. With a wide grid the non-text
    // scroll noise from line wrapping goes away, so scrollback is exactly the
    // two oldest lines.
    let sb: Vec<String> = t
        .scrollback()
        .lines()
        .map(|line| line.iter().map(|c| c.ch).collect())
        .collect();
    assert_eq!(sb.len(), 2);
    assert!(sb[0].starts_with("AAAAA"));
    assert!(sb[1].starts_with("BBBBB"));
}

#[test]
fn truecolor_and_256_colour_sgr_persist_in_cells() {
    let mut t = Terminal::new(1, 8, 10);
    t.process_bytes(b"\x1b[38;2;10;20;30mA");
    t.process_bytes(b"\x1b[38;5;123mB");
    t.process_bytes(b"\x1b[1;4mC");
    assert_eq!(t.grid().get(0, 0).unwrap().fg, Color::rgb(10, 20, 30));
    assert_eq!(t.grid().get(0, 1).unwrap().fg, color_256(123));
    assert_eq!(
        t.grid().get(0, 2).unwrap().attrs,
        CellAttrs::BOLD | CellAttrs::UNDERLINE,
    );
}
