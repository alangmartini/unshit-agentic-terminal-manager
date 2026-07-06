### Fixed

- **Quotes typed on dead-key layouts now reach the terminal.** On keyboard layouts where `'` and `"` are dead keys (US-International, ABNT2), the committed character was silently dropped: pressing the quote key twice produced nothing, and quote-then-space sent a plain space. Both paths now forward the composed text to the shell, so `'`, `"`, and other dead-key accents (`~`, `^`, `` ` ``) can be typed normally.
