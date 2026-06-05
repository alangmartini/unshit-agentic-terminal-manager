# Fix: parser drops a declaration after the overflow shorthand / no-op accepts

## Fixed

- Scroll containers (`overflow: scroll` / `overflow: auto`) stopped scrolling.
  `parse_rule`'s declaration loop does not auto-drain on success, so each
  declaration arm must leave the parser at the start of the next declaration.
  Two arms added in the tier-1 engine work regressed this:
  - the `overflow` shorthand early-returned **without consuming its trailing
    `;`**, so the *following* declaration was dropped — e.g.
    `overflow: scroll; height: 200px` silently lost `height`, leaving the
    container sized to its content (no overflow), so it never scrolled and
    rendered no scrollbar.
  - the recognized-but-inert accepts (`appearance`, `-webkit-appearance`,
    `-webkit-font-smoothing`, `border-collapse`, `background-repeat`,
    `font-feature-settings`, `font-variant-numeric`, `scrollbar-width`) drained
    with `while parser.next().is_ok() {}`, consuming the rest of the **block**
    and dropping every declaration after them in the same rule.

  Both now stop exactly at the declaration terminator. Restores the 12
  `unshit-test` scroll tests; the full `unshit-test` suite is green.
