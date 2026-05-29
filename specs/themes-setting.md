# Spec: Terminal Theme Setting

## Objective
Add an Appearance settings control that lets the user switch between curated terminal themes. The active theme must update the application shell and the rendered terminal color palette immediately, including default text and ANSI colors in already-rendered terminal grids.

## Tech Stack
Rust 2021 application using the local `unshit` framework, `CellGrid` terminal rendering, app state in `src/state.rs`, settings UI in `src/ui/settings.rs`, and static styles in `assets/styles.css`.

## Commands
- Format check: `cargo fmt --check`
- Focused tests: `cargo test theme`
- App/settings tests: `cargo test settings_page_appearance theme`
- Full tests: `cargo test`
- Clippy: `cargo clippy -- -D warnings`
- Manual launch: `cargo run`

## Project Structure
- `src/theme.rs`: Theme catalog and terminal palette mapping.
- `src/main.rs`: Applies theme class and maps terminal grid snapshots before rendering.
- `src/state.rs`: Stores the selected theme id.
- `src/ui/settings.rs`: Renders the Appearance theme picker and applies changes.
- `assets/styles.css`: App shell and settings picker theme styles.
- `assets/themes.json`: Human-readable theme metadata matching the static catalog.

## Code Style
Keep theme ids short, lowercase, and CSS-safe:

```rust
pub fn theme_class_name(raw: &str) -> String {
    format!("theme-{}", resolve_theme_id(raw))
}
```

Theme logic should be pure and unit-testable. UI callbacks may mutate `AppState::theme`, but no terminal write path should block on theme selection.

## Testing Strategy
Use small Rust unit tests for the theme catalog and palette mapping. Use settings UI tree tests to verify the picker renders, click handlers update state, and active classes move. Use stylesheet smoke tests to verify every catalog theme has a matching `.app.theme-*` class and picker styling.

## Theme Sources
Preserve the original Amber palette as the default, then offer established palette families as optional choices: Catppuccin Mocha, Tokyo Night, Nord, Dracula, Everforest, and Rose Pine Moon. Keep Rust ANSI constants, CSS variables, and `assets/themes.json` aligned.

## Boundaries
- Always: Keep `DaemonPty::write()` fire-and-forget; use render-time mapping for theme color changes.
- Always: Preserve existing PTY spawning and resize behavior.
- Ask first: New dependencies, persistence format changes, or framework-level renderer changes.
- Never: Revert unrelated user work, remove eager PTY spawning, or add synchronous IPC to rendering.

## Success Criteria
- Appearance settings show multiple concrete theme examples, including the original Amber theme plus Catppuccin, Tokyo Night, Nord, Dracula, Everforest, and Rose Pine.
- Selecting a theme immediately changes `AppState::theme`, root app theme class, active picker state, and rendered terminal default/ANSI colors.
- Unknown theme ids resolve to the default theme safely.
- Theme catalog and CSS classes stay in sync.
- Focused tests and reasonable quality gates pass or any unrun gate is explicitly reported.

## Open Questions
None for the first slice. Theme persistence can be handled separately unless the existing persistence layer already supports this field.
