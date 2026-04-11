# Required changes to unshit-rust-framework

These changes must be applied to the `unshit-rust-framework` repo for the terminal manager to work correctly. Each section is an independent PR.

## PR 1: CellGrid cursor + metrics support

**File:** `crates/unshit-core/src/cell_grid.rs`

Add imports and global atomics at top of file:
```rust
use std::sync::atomic::AtomicU32;
static GLOBAL_CELL_W: AtomicU32 = AtomicU32::new(0);
static GLOBAL_CELL_H: AtomicU32 = AtomicU32::new(0);
```

Add cursor and metrics fields to `CellGrid` struct:
```rust
pub struct CellGrid {
    rows: usize,
    cols: usize,
    cells: Vec<Cell>,
    dirty: Vec<bool>,
    cursor_row: usize,     // ADD
    cursor_col: usize,     // ADD
    cursor_visible: bool,  // ADD
}
```

Update `CellGrid::new()` to initialize them:
```rust
cursor_row: 0,
cursor_col: 0,
cursor_visible: true,
```

Add cursor methods (after `has_dirty_cells`):
```rust
pub fn set_cursor(&mut self, row: usize, col: usize) {
    self.cursor_row = row.min(self.rows.saturating_sub(1));
    self.cursor_col = col.min(self.cols.saturating_sub(1));
}

pub fn set_cursor_visible(&mut self, visible: bool) {
    self.cursor_visible = visible;
}

pub fn cursor_row(&self) -> usize { self.cursor_row }
pub fn cursor_col(&self) -> usize { self.cursor_col }
pub fn cursor_visible(&self) -> bool { self.cursor_visible }
```

## PR 2: Grid renderer fixes (font measurement + cursor rendering)

**File:** `crates/unshit-renderer/src/batch.rs`

### Change 1: Replace hardcoded 0.6 cell_w with actual measurement

In `walk_for_batch`, the `ElementContent::Grid` arm (~line 1261):
```rust
// BEFORE:
let cell_w = style.font_size * 0.6;

// AFTER:
let cell_w = measure_monospace_cell_width(font_system, style.font_size);
```

### Change 2: Use monospace font for glyph shaping

In `emit_grid_cells`, the `buffer.set_text` call (~line 1692):
```rust
// BEFORE:
cosmic_text::Attrs::new(),

// AFTER:
cosmic_text::Attrs::new().family(cosmic_text::Family::Monospace),
```

### Change 3: Add cursor rendering

At the end of `emit_grid_cells`, before the closing `}`:
```rust
// Draw cursor as a solid block if visible.
if grid.cursor_visible() {
    let cr = grid.cursor_row();
    let cc = grid.cursor_col();
    if cr < rows && cc < cols {
        let cx = origin_x + cc as f32 * cell_w;
        let cy = origin_y + cr as f32 * cell_h;
        let idx = cr * cols + cc;
        let cursor_color = cells[idx].fg.to_linear_f32();

        batch.quad_instances.push(QuadInstance {
            pos: [cx, cy],
            size: [cell_w, cell_h],
            color: cursor_color,
            border_color: [0.0; 4],
            border_width: [0.0; 4],
            border_radius: [0.0; 4],
            clip_rect,
            shadow_color: [0.0; 4],
            shadow_offset: [0.0; 2],
            shadow_params: [0.0; 2],
            shadow_spread: [0.0; 2],
            gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
            gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
            gradient_params: [0.0; 4],
            gradient_extra: EMPTY_GRADIENT_EXTRA,
        });
    }
}
```

### Change 4: Add the measurement function

Add before `is_wide_char`:
```rust
fn measure_monospace_cell_width(font_system: &mut FontSystem, font_size: f32) -> f32 {
    let metrics = cosmic_text::Metrics::new(font_size, font_size * 1.2);
    let mut buffer = cosmic_text::Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(font_size * 10.0), None);
    buffer.set_text(
        font_system,
        "M",
        cosmic_text::Attrs::new().family(cosmic_text::Family::Monospace),
        cosmic_text::Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);

    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            return glyph.w;
        }
    }
    font_size * 0.6
}
```

## PR 3: AppConfig callbacks (on_scale_factor + on_close)

**File:** `crates/unshit-app/src/app.rs`

Add to `AppConfig` struct:
```rust
pub on_scale_factor: Option<Arc<dyn Fn(f32) + Send + Sync>>,
pub on_close: Option<Arc<dyn Fn() + Send + Sync>>,
```

Add to `Default` impl:
```rust
on_scale_factor: None,
on_close: None,
```

In `resumed()`, after `let scale_factor = window.scale_factor() as f32;`:
```rust
if let Some(ref cb) = self.app.config.on_scale_factor {
    cb(scale_factor);
}
```

In `CloseRequested` handler, before `event_loop.exit()`:
```rust
if let Some(ref cb) = self.app.config.on_close {
    cb();
}
```

In the scale_factor change handler (SurfaceResized), after updating state.scale_factor:
```rust
if let Some(ref cb) = self.app.config.on_scale_factor {
    cb(new_scale);
}
```
