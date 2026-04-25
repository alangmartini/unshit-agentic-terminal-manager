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

/// Issue #129 regression: simulate a realistic split/unsplit round-trip.
/// Wide pane runs a sequence of prompts and outputs, splits (rows
/// shrink), runs another command in the narrow pane, then unsplits
/// (rows grow back). The visible grid must end with the live prompt
/// at the bottom and no blank gap above it; scrollback must contain
/// the older content in eviction order.
#[test]
fn split_unsplit_round_trip_keeps_prompt_anchored() {
    let mut t = Terminal::new(6, 12, 100);
    // Wide-pane phase: two "commands" with output, then a fresh prompt.
    t.process_bytes(b"$ ls\r\nfile1.txt\r\nfile2.txt\r\n");
    t.process_bytes(b"$ pwd\r\n/home/me\r\n");
    t.process_bytes(b"$ ");
    let prompt_cursor = t.grid().cursor();
    assert_eq!(t.grid().rows(), 6);

    // Split: pane shrinks by 2 rows. Top rows go to scrollback.
    t.resize(4, 12);
    let after_split_rows = t.grid().rows();
    assert_eq!(after_split_rows, 4);
    // Live prompt still at the bottom of the surviving pane.
    let bottom: String = t
        .grid()
        .row(after_split_rows - 1)
        .unwrap()
        .iter()
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string();
    assert_eq!(bottom, "$");

    // Run a command inside the narrow pane.
    t.process_bytes(b"echo hi\r\nhi\r\n$ ");

    // Unsplit: pane grows back to 6 rows. Scrollback content lifts up
    // to fill the new top rows.
    t.resize(6, 12);
    assert_eq!(t.grid().rows(), 6);

    // No blank gap before the live prompt: every row from cursor's row
    // upward to row 0 has visible content.
    let cursor_row = t.grid().cursor().0;
    for r in 0..cursor_row {
        let text: String = t
            .grid()
            .row(r)
            .unwrap()
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string();
        assert!(!text.is_empty(), "row {r} unexpectedly blank after unsplit");
    }

    // Live prompt is on the cursor's row.
    let cur_text: String = t
        .grid()
        .row(cursor_row)
        .unwrap()
        .iter()
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string();
    assert_eq!(cur_text, "$");

    // Cursor is below where it sat at the original wide prompt because
    // the narrow-pane "echo hi\r\nhi\r\n$ " sequence advanced it; what
    // matters for #129 is that the prompt is anchored to the cursor
    // row, not floating mid-grid.
    assert!(
        cursor_row >= prompt_cursor.0,
        "cursor regressed above its pre-split position"
    );
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
