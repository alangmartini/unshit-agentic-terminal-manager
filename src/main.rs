// On Windows, build release as a "windows" (GUI) subsystem binary so launching
// the app from the installer shortcut or Explorer does not pop a console window
// next to it. Debug builds stay on the console subsystem so `cargo run` keeps
// surfacing logs during development. Release CLI/bench output is preserved via
// `attach_parent_console` below when the exe is started from a terminal.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod bench;
pub mod bridge;
pub mod command_palette;
pub mod daemon;
pub mod diagnostics;
pub mod drag;
pub mod git;
pub mod keybinds;
pub mod notifications;
pub mod persist;
pub mod pty;
pub mod quick_prompt;
pub mod shell;
pub mod state;
pub mod terminal;
pub mod theme;
pub mod ui;

use std::{path::PathBuf, sync::Arc};

use unshit::app::{App, AppConfig, FontSource};
use unshit::core::element::*;
use unshit::core::event::DragPhase;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{Background, Dimension};
use unshit::core::trace::{
    append_terminal_trace_line, terminal_trace_enabled, terminal_trace_file_path,
};

use crate::state::{
    dispatch, mutate_with, new_workspace, record_diagnostic_pty_event,
    record_diagnostic_renderer_frame, resize_all_terminals, seed_state, MutexExt, SharedState,
    UiSnapshot, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH,
};
use crate::ui::settings::build_settings_page;
use crate::ui::sidebar::{build_ctx_menu_overlay, build_sidebar};
use crate::ui::statusbar::build_statusbar;
use crate::ui::tabbar::build_tabbar;
use crate::ui::terminal_grid::build_terminal_grid;
use crate::ui::titlebar::build_titlebar;
use crate::ui::toasts::build_toast_overlay;

const STYLES: &str = include_str!("../assets/styles.css");
const JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf");
const JETBRAINS_MONO_SEMIBOLD: &[u8] =
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-SemiBold.ttf");
const JETBRAINS_MONO_BOLD: &[u8] =
    include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Bold.ttf");
const ENV_PTYD_SOCKET: &str = "TM_PTYD_SOCKET";
const ENV_PARITY_SHELL_PROGRAM: &str = "TM_PARITY_SHELL_PROGRAM";
const ENV_PARITY_SHELL_ARGS_JSON: &str = "TM_PARITY_SHELL_ARGS_JSON";
const ENV_PARITY_WINDOWS_TERMINAL_COLORS: &str = "TM_PARITY_WINDOWS_TERMINAL_COLORS";
const ENV_PARITY_FONT_SIZE_PT: &str = "TM_PARITY_FONT_SIZE_PT";
const ENV_PARITY_FONT_FAMILY: &str = "TM_PARITY_FONT_FAMILY";
const ENV_OPEN_SETTINGS: &str = "TM_OPEN_SETTINGS";
const ENV_OPEN_QUICK_PROMPT: &str = "TM_OPEN_QUICK_PROMPT";
/// Preview/screenshot hook: pre-attach the image file at this path to the
/// Quick Prompt on startup (opening the overlay if needed), exercising the
/// real `attach_dropped_images` drag-and-drop path. Used by
/// `scripts/qp-attach-shot.ps1` to verify the rendered image chip.
const ENV_QP_ATTACH_IMAGE: &str = "TM_QP_ATTACH_IMAGE";
const ENV_OPEN_CONFIRM_DIALOG: &str = "TM_OPEN_CONFIRM_DIALOG";
const ENV_SHOW_TEST_TOAST: &str = "TM_SHOW_TEST_TOAST";
const WINDOWS_TERMINAL_PARITY_FONT_SIZE_PT: u32 = 16;

// Heap profiling via dhat. Only compiled in when --features profiling is set.
//
// The profiler writes `target/profile/dhat-heap.json` on drop, which can be
// loaded into the dhat viewer. We keep the `Profiler` inside a static Mutex
// because all of our process-exit paths (`std::process::exit` in the ctrl-c
// handler, panic hook, and `on_close` callback) bypass normal stack unwinding
// and therefore skip Drop. `finalize_profiler` pulls the value out and drops
// it explicitly right before the exit call, flushing the JSON output.
#[cfg(feature = "profiling")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[cfg(feature = "profiling")]
static PROFILER: std::sync::Mutex<Option<dhat::Profiler>> = std::sync::Mutex::new(None);

/// Returns `true` if `init_profiler` should proceed to build a new
/// profiler. Extracted as a pure function so the idempotency guard
/// (refs #107) can be unit tested without engaging dhat's global
/// allocator, which cannot safely be re-initialized in-process.
#[cfg(feature = "profiling")]
fn should_init_profiler<T>(slot: &Option<T>) -> bool {
    slot.is_none()
}

#[cfg(feature = "profiling")]
fn init_profiler() {
    let mut guard = PROFILER.lock().unwrap();
    if !should_init_profiler(&*guard) {
        eprintln!("[profiling] init_profiler called twice, ignoring");
        return;
    }
    let out_dir = std::path::Path::new("target").join("profile");
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!(
            "[profiling] failed to create {}: {e}; dhat will panic on build()",
            out_dir.display()
        );
    }
    let out_path = out_dir.join("dhat-heap.json");
    let profiler = dhat::Profiler::builder().file_name(&out_path).build();
    *guard = Some(profiler);
    eprintln!(
        "[profiling] dhat heap profiling active; output: {}",
        out_path.display()
    );
}

#[cfg(all(test, feature = "profiling"))]
mod profiler_tests {
    use super::should_init_profiler;

    // Regression test for issue #107: init_profiler must early-return
    // when the PROFILER slot is already populated, otherwise a second
    // call would drop the in-flight profiler and flush a partial JSON.
    #[test]
    fn should_init_profiler_returns_true_when_slot_is_empty() {
        let slot: Option<u32> = None;
        assert!(should_init_profiler(&slot));
    }

    #[test]
    fn should_init_profiler_returns_false_when_slot_is_populated() {
        let slot: Option<u32> = Some(0);
        assert!(!should_init_profiler(&slot));
    }
}

#[cfg(feature = "profiling")]
pub(crate) fn finalize_profiler() {
    // Recover through poison: if we crash mid-flush we still want the JSON on disk.
    let mut guard = PROFILER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(p) = guard.take() {
        drop(p);
        eprintln!("[profiling] dhat heap profile flushed to target/profile/dhat-heap.json");
    }
}

#[cfg(not(feature = "profiling"))]
#[inline]
pub(crate) fn finalize_profiler() {}

/// Flush the heap profile (if the `profiling` feature is on) and exit
/// the process with status 0. Used by the close-app dialog buttons to
/// drive their own exit after routing through dispatch, since the
/// framework's `event_loop.exit()` path was vetoed by `on_close`.
pub(crate) fn shutdown_now() -> ! {
    finalize_profiler();
    std::process::exit(0);
}

/// Snapshot a terminal's display grid for the current render frame.
///
/// This is the per-terminal step run by `tree_fn` when it builds the
/// element tree. It takes the per-terminal mutex, clones the display
/// grid, hides the cursor on inactive panes, and returns the cloned
/// grid for the renderer to consume.
///
/// Issue #63: this function MUST NOT call `clear_dirty()` on the live
/// grid. The old code did, which produced a race where an interleaved
/// PTY chunk arriving between the clone and the clear would have its
/// damage wiped, dropping cells from the next frame. The clone already
/// owns its damage independently; leaving the live grid untouched means
/// the next snapshot still sees every write since the last clone. The
/// renderer's retained line quad cache (`LineQuadCache`) handles
/// replay for unchanged rows via the content-hash signature, so the
/// previous damage-skip optimisation is not needed for correctness or
/// performance.
fn snapshot_terminal_for_render(
    terminal: &mut crate::terminal::Terminal,
    pane_id: u32,
    is_active: bool,
    theme_id: &str,
    custom_theme: &crate::theme::CustomTheme,
    force_full_repaint: bool,
) -> unshit::core::cell_grid::CellGrid {
    let mut grid = terminal.display_grid();
    if !is_active {
        grid.set_cursor_visible(false);
    }
    if !parity_windows_terminal_colors_enabled() {
        let palette = crate::theme::terminal_palette_for(theme_id, custom_theme);
        crate::theme::apply_terminal_palette_to_grid(&mut grid, &palette);
        if force_full_repaint {
            grid.mark_all_dirty();
        }
    }
    if terminal_trace_enabled() && is_active {
        let rows = grid.debug_rows(4, 96);
        append_terminal_trace_line(&format!(
            "terminal-trace stage=main_snapshot pane={} active=true cursor=({}, {}) visible={} row0={:?} row1={:?} row2={:?} row3={:?}",
            pane_id,
            grid.cursor_row(),
            grid.cursor_col(),
            grid.cursor_visible(),
            rows.first().cloned().unwrap_or_default(),
            rows.get(1).cloned().unwrap_or_default(),
            rows.get(2).cloned().unwrap_or_default(),
            rows.get(3).cloned().unwrap_or_default(),
        ));
    }
    grid
}

fn custom_theme_active(snap: &UiSnapshot) -> bool {
    crate::theme::resolve_theme_id(&snap.theme) == crate::theme::CUSTOM_THEME_ID
}

fn with_custom_base_style(mut el: ElementDef, snap: &UiSnapshot) -> ElementDef {
    if custom_theme_active(snap) {
        el = el
            .with_style(StyleDeclaration::Background(Background::Color(
                snap.custom_theme.background,
            )))
            .with_style(StyleDeclaration::Color(snap.custom_theme.foreground));
    }
    el
}

fn with_custom_surface_style(mut el: ElementDef, snap: &UiSnapshot) -> ElementDef {
    if custom_theme_active(snap) {
        el = el
            .with_style(StyleDeclaration::Background(Background::Color(
                snap.custom_theme.surface,
            )))
            .with_style(StyleDeclaration::Color(snap.custom_theme.foreground));
    }
    el
}

fn build_tree(
    snap: &UiSnapshot,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
    window_events: Option<unshit::app::EventSink>,
) -> ElementTree {
    let sidebar = with_custom_surface_style(build_sidebar(snap, shared), snap)
        .with_style(StyleDeclaration::Width(Dimension::Px(snap.sidebar_width)))
        .with_style(StyleDeclaration::MinWidth(Dimension::Px(
            snap.sidebar_width,
        )));

    let drag_shared = shared.clone();
    let sidebar_resizer = ElementDef::new(Tag::Div)
        .with_class("sidebar-resizer")
        .on_drag(move |ev| match ev.phase {
            DragPhase::Start => {
                mutate_with(&drag_shared, |st| {
                    st.sidebar_drag_start = Some(st.sidebar_width);
                });
            }
            DragPhase::Update => {
                mutate_with(&drag_shared, |st| {
                    let start = match st.sidebar_drag_start {
                        Some(w) => w,
                        None => return,
                    };
                    st.sidebar_width =
                        (start + ev.total_delta_x).clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                });
            }
            DragPhase::End => {
                mutate_with(&drag_shared, |st| {
                    st.sidebar_drag_start = None;
                });
            }
        });

    let titlebar = with_custom_surface_style(build_titlebar(snap, shared, window_events), snap);
    let mut root = ElementDef::new(Tag::Div)
        .with_class("app")
        .with_class(crate::theme::theme_class_name(&snap.theme))
        .with_class(format!("density-{}", snap.ui_density.id()))
        .with_class(format!("tabs-width-{}", snap.tab_width_mode.id()))
        .with_class(format!("tabs-rows-{}", snap.tab_row_mode.id()))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("ambient-layer")
                .with_class("rust-glow"),
        )
        .with_child(titlebar);
    root = with_custom_base_style(root, snap);
    let config_font_scale =
        snap.config_font_size_pt as f32 / crate::state::DEFAULT_CONFIG_FONT_SIZE_PT as f32;
    if (config_font_scale - 1.0).abs() >= 0.001 {
        root = root.with_style(StyleDeclaration::FontScale(config_font_scale));
    }

    if snap.settings_open {
        root = root
            .with_class("settings")
            .with_child(build_settings_page(snap, shared))
            .with_child(with_custom_surface_style(build_statusbar(snap), snap));
    } else {
        root = root.with_child(
            ElementDef::new(Tag::Div)
                .with_class("layout")
                .with_child(sidebar)
                .with_child(sidebar_resizer)
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("content")
                        .with_class("role-main")
                        .with_child(build_tabbar(snap, shared))
                        .with_child(build_terminal_grid(snap, shared, grids))
                        .with_child(with_custom_surface_style(build_statusbar(snap), snap)),
                ),
        );
    }
    if parity_windows_terminal_colors_enabled() {
        root = root.with_class("parity-windows-terminal");
    }

    // The drop-zone overlay is rendered as a child inside each pane
    // (see `build_pane`); no root-level overlay is needed.
    if let Some(ghost) = crate::ui::drag_overlay::build_drag_overlay(snap) {
        root = root.with_child(ghost);
    }

    ElementTree {
        root: root
            .with_child(build_ctx_menu_overlay(snap, shared))
            .with_child(crate::ui::confirm_dialog::build_confirm_dialog_overlay(
                snap, shared,
            ))
            .with_child(crate::ui::command_palette::build_command_palette_overlay(
                snap, shared,
            ))
            .with_child(crate::quick_prompt::build_quick_prompt_overlay(
                snap, shared,
            ))
            .with_child(build_toast_overlay(snap, shared))
            .with_child(crate::ui::fps_overlay::build_fps_overlay()),
    }
}

fn user_shortcut_bindings() -> Vec<(String, String)> {
    let overrides = crate::keybinds::loader::load_if_installed();
    crate::keybinds::registry::shortcut_bindings_with_overrides(&overrides)
}

/// Whether `combo` is the Quick Prompt image-paste chord (Ctrl+V). Kept
/// as a named predicate so the key match is unit-testable and a future
/// edit cannot silently break it (matching uppercase `'V'`, the wrong
/// modifier set, etc.). The `on_raw_key` hook uses this to attach a
/// clipboard image when the overlay is open.
fn is_quick_prompt_paste_combo(combo: &unshit::core::shortcut::KeyCombo) -> bool {
    use unshit::core::event::{Key, Modifiers};
    combo.key == Key::Char('v') && combo.modifiers == Modifiers::CTRL
}

fn terminal_font_sources_from_value(value: Option<std::ffi::OsString>) -> Vec<FontSource> {
    let mut fonts = vec![
        FontSource::System("JetBrains Mono".to_string()),
        FontSource::Bytes(Arc::from(JETBRAINS_MONO_REGULAR)),
        FontSource::Bytes(Arc::from(JETBRAINS_MONO_SEMIBOLD)),
        FontSource::Bytes(Arc::from(JETBRAINS_MONO_BOLD)),
        FontSource::System("Berkeley Mono".to_string()),
        FontSource::System("SF Mono".to_string()),
        FontSource::System("Menlo".to_string()),
        FontSource::System("Consolas".to_string()),
    ];

    if let Some(value) = value {
        let raw = value.to_string_lossy();
        let family = raw.trim();
        if !family.is_empty() {
            fonts.retain(|source| match source {
                FontSource::System(name) => !name.eq_ignore_ascii_case(family),
                _ => true,
            });
            fonts.insert(0, FontSource::System(family.to_string()));
        }
    }

    fonts
}

fn terminal_font_sources() -> Vec<FontSource> {
    terminal_font_sources_from_value(std::env::var_os(ENV_PARITY_FONT_FAMILY))
}

fn ptyd_socket_path_from_env(value: Option<std::ffi::OsString>) -> PathBuf {
    value
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(unshit_ptyd::transport::default_socket_path)
}

fn ptyd_socket_path() -> PathBuf {
    ptyd_socket_path_from_env(std::env::var_os(ENV_PTYD_SOCKET))
}

fn truthy_env_value(value: Option<std::ffi::OsString>) -> bool {
    value
        .filter(|v| !v.is_empty())
        .map(|v| {
            let normalized = v.to_string_lossy().trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(false)
}

fn parity_windows_terminal_colors_enabled() -> bool {
    truthy_env_value(std::env::var_os(ENV_PARITY_WINDOWS_TERMINAL_COLORS))
}

fn parity_shell_spec_from_values(
    program: Option<std::ffi::OsString>,
    args_json: Option<std::ffi::OsString>,
) -> Result<Option<crate::shell::ShellSpec>, String> {
    let Some(program) = program else {
        return Ok(None);
    };
    if program.is_empty() {
        return Ok(None);
    }

    let program = program.to_string_lossy().trim().to_string();
    if program.is_empty() {
        return Ok(None);
    }

    let args = match args_json {
        Some(raw) if !raw.is_empty() => {
            let text = raw.to_string_lossy();
            serde_json::from_str::<Vec<String>>(&text)
                .map_err(|e| format!("invalid {ENV_PARITY_SHELL_ARGS_JSON}: {e}"))?
        }
        _ => Vec::new(),
    };

    Ok(Some(crate::shell::ShellSpec { program, args }))
}

fn parity_shell_spec_from_env() -> Option<crate::shell::ShellSpec> {
    parity_shell_spec_from_values(
        std::env::var_os(ENV_PARITY_SHELL_PROGRAM),
        std::env::var_os(ENV_PARITY_SHELL_ARGS_JSON),
    )
    .unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    })
}

fn parity_font_size_pt_from_value(value: Option<std::ffi::OsString>) -> Result<u32, String> {
    let Some(value) = value else {
        return Ok(WINDOWS_TERMINAL_PARITY_FONT_SIZE_PT);
    };
    let raw = value
        .into_string()
        .map_err(|_| format!("{ENV_PARITY_FONT_SIZE_PT} must be valid UTF-8"))?;
    let parsed: u32 = raw
        .parse()
        .map_err(|_| format!("{ENV_PARITY_FONT_SIZE_PT} must be an integer, got {raw:?}"))?;
    if !(crate::state::MIN_FONT_SIZE..=crate::state::MAX_FONT_SIZE).contains(&parsed) {
        return Err(format!(
            "{ENV_PARITY_FONT_SIZE_PT} must be between {} and {}, got {parsed}",
            crate::state::MIN_FONT_SIZE,
            crate::state::MAX_FONT_SIZE
        ));
    }
    Ok(parsed)
}

fn parity_font_size_pt_from_env() -> u32 {
    parity_font_size_pt_from_value(std::env::var_os(ENV_PARITY_FONT_SIZE_PT)).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    })
}

fn open_settings_on_startup_from_value(value: Option<std::ffi::OsString>) -> bool {
    value
        .as_deref()
        .and_then(|raw| raw.to_str())
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn open_settings_on_startup_from_env() -> bool {
    open_settings_on_startup_from_value(std::env::var_os(ENV_OPEN_SETTINGS))
}

fn open_quick_prompt_on_startup_from_value(value: Option<std::ffi::OsString>) -> bool {
    truthy_env_value(value)
}

fn open_quick_prompt_on_startup_from_env() -> bool {
    open_quick_prompt_on_startup_from_value(std::env::var_os(ENV_OPEN_QUICK_PROMPT))
}

/// Resolve the startup image-attach path from a raw env value. Returns
/// `None` for an unset or empty value so production launches (where this
/// is never set) skip the attach entirely.
fn quick_prompt_attach_image_from_value(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

fn quick_prompt_attach_image_from_env() -> Option<PathBuf> {
    quick_prompt_attach_image_from_value(std::env::var_os(ENV_QP_ATTACH_IMAGE))
}

fn open_confirm_dialog_on_startup_from_value(value: Option<std::ffi::OsString>) -> bool {
    truthy_env_value(value)
}

fn open_confirm_dialog_on_startup_from_env() -> bool {
    open_confirm_dialog_on_startup_from_value(std::env::var_os(ENV_OPEN_CONFIRM_DIALOG))
}

fn show_test_toast_on_startup_from_value(value: Option<std::ffi::OsString>) -> bool {
    truthy_env_value(value)
}

fn show_test_toast_on_startup_from_env() -> bool {
    show_test_toast_on_startup_from_value(std::env::var_os(ENV_SHOW_TEST_TOAST))
}

fn unix_epoch_millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn apply_parity_shell_override(state: &mut crate::state::AppState, spec: crate::shell::ShellSpec) {
    *state = seed_state();
    state.default_shell = spec;
    state.terminal_font_size_pt = parity_font_size_pt_from_env();
    state.sidebar_collapsed = true;
    state.sidebar_width = 48.0;
}

fn parse_bench_args() -> Option<crate::bench::BenchConfig> {
    let mut args = std::env::args().skip(1);
    let mut mode: Option<crate::bench::BenchMode> = None;
    let mut duration_secs: f64 = 10.0;
    let mut warmup_secs: f64 = 2.5;
    let mut out_path: std::path::PathBuf = "bench.json".into();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bench" => {
                if let Some(m) = args.next() {
                    mode = crate::bench::BenchMode::parse(&m);
                    if mode.is_none() {
                        eprintln!("unknown bench mode: {m}");
                        std::process::exit(2);
                    }
                }
            }
            "--duration" => {
                if let Some(s) = args.next() {
                    duration_secs = s.parse().unwrap_or(duration_secs);
                }
            }
            "--warmup" => {
                if let Some(s) = args.next() {
                    warmup_secs = s.parse().unwrap_or(warmup_secs);
                }
            }
            "--out" => {
                if let Some(s) = args.next() {
                    out_path = s.into();
                }
            }
            _ => {}
        }
    }
    mode.map(|mode| crate::bench::BenchConfig {
        mode,
        duration: std::time::Duration::from_secs_f64(duration_secs),
        warmup: std::time::Duration::from_secs_f64(warmup_secs),
        out_path,
    })
}

/// Reattach stdio to the parent terminal on Windows release builds.
///
/// Release is a "windows" subsystem binary (see the crate attribute above), so
/// it owns no console. When the user starts it from an existing terminal —
/// `terminal-manager --bench ...`, the `notify` CLI, etc. — this reconnects
/// stdout/stderr to that console so logs and bench output still appear. It is a
/// no-op when there is no parent console (the installer/Explorer launch), which
/// is exactly the no-extra-window behavior we want. Debug builds keep their own
/// console and skip this entirely.
fn attach_parent_console() {
    #[cfg(all(windows, not(debug_assertions)))]
    {
        // ATTACH_PARENT_PROCESS == (DWORD)-1.
        const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
        extern "system" {
            fn AttachConsole(dw_process_id: u32) -> i32;
        }
        // SAFETY: plain kernel32 call. Returns 0 (ignored) when the process has
        // no parent console, leaving the app window-free as intended.
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() {
    attach_parent_console();

    if let Some(code) = notifications::handle_cli_from_env(std::env::args_os().skip(1)) {
        std::process::exit(code);
    }

    #[cfg(feature = "profiling")]
    init_profiler();

    let bench_config = parse_bench_args();

    // Guard against ghost handles (#32): ensure the process exits
    // immediately on Ctrl+C or panic so spawn_blocking reader tasks
    // (bridge.rs) cannot keep the .exe locked on Windows (os error 32).
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        finalize_profiler();
        std::process::exit(1);
    }));
    ctrlc::set_handler(|| {
        finalize_profiler();
        std::process::exit(0);
    })
    .expect("failed to set Ctrl+C handler");

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(
            "info,wgpu_hal=error,wgpu_core=error,naga=error,unshit_app::app=error",
        ),
    )
    .init();

    let diagnostics_config = crate::diagnostics::DiagnosticConfig::from_env().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let diagnostics_enabled = diagnostics_config.is_some();

    if terminal_trace_enabled() {
        append_terminal_trace_line(&format!(
            "terminal-trace stage=startup trace_file={}",
            terminal_trace_file_path().display()
        ));
    }

    if let Some(path) = persist::default_config_path() {
        persist::install(path);
    }

    if let Some(path) = crate::keybinds::loader::keybinds_config_path() {
        crate::keybinds::loader::install(path);
    }

    if let Some(path) = crate::quick_prompt::state::default_config_path() {
        crate::quick_prompt::state::QuickPromptStore::install(path);
    }

    let mut initial_state = seed_state();
    if let Some(persisted) = persist::load_workspaces() {
        if persisted.has_layout() {
            // Full layout restore: rebuild every workspace's tab/pane tree
            // and the live fields so the startup reattach below can rejoin
            // each surviving daemon session keyed by `(workspace, pane)`.
            crate::state::restore_layout(&mut initial_state, &persisted);
        } else if !persisted.workspaces.is_empty() {
            // Legacy config (predates layout persistence): restore only the
            // workspace metadata and keep the seeded default terminal.
            initial_state.workspaces = persisted
                .workspaces
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let mut ws =
                        new_workspace((i + 1) as u32, entry.name.clone(), entry.path.clone());
                    ws.collapsed = entry.collapsed;
                    ws.shell = entry.shell.clone();
                    ws
                })
                .collect();
            let last = initial_state.workspaces.len() - 1;
            initial_state.active_workspace = persisted.active_workspace.min(last);
        }
        initial_state.toggles.insert(
            crate::state::ToggleKey::RememberCloseChoice,
            persisted.remember_close_choice,
        );
        initial_state.toggles.insert(
            crate::state::ToggleKey::KillAllOnClose,
            persisted.kill_all_on_close,
        );
        // Override the seed_state inference with whatever the user
        // last persisted. An upgrader without the field gets an
        // empty spec here, which keeps the daemon's `default_shell()`
        // floor exactly as before.
        initial_state.default_shell = persisted.default_shell;
    }
    // Bench mode needs a deterministic shell so the scroll workload
    // measures comparable output across runs. On Windows the bench
    // exercises `dir`, which only behaves on cmd.exe; route it through
    // the same `default_shell` channel the rest of the app uses instead
    // of mutating the SHELL env var (which would leak into any spawned
    // child).
    #[cfg(windows)]
    if bench_config.is_some() {
        initial_state.default_shell = crate::shell::ShellSpec {
            program: "cmd.exe".into(),
            args: Vec::new(),
        };
    }
    if let Some(spec) = parity_shell_spec_from_env() {
        apply_parity_shell_override(&mut initial_state, spec);
    }
    if open_settings_on_startup_from_env() {
        initial_state.settings_open = true;
    }
    if open_quick_prompt_on_startup_from_env() {
        initial_state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());
    }
    if let Some(image_path) = quick_prompt_attach_image_from_env() {
        // Preview/screenshot hook: drive the real drag-and-drop attach
        // path so the rendered chip can be verified. Open the overlay
        // first if it is not already open.
        if initial_state.quick_prompt.is_none() {
            initial_state.quick_prompt =
                Some(crate::quick_prompt::QuickPromptState::open_default());
        }
        crate::state::attach_dropped_images(&mut initial_state, std::slice::from_ref(&image_path));
    }
    if open_confirm_dialog_on_startup_from_env() {
        initial_state.confirm_dialog = Some(crate::state::ConfirmDialog::KillAll {
            count: initial_state.terminals.len().max(1),
        });
    }
    if show_test_toast_on_startup_from_env() {
        let workspace_id = crate::state::active_workspace_num(&initial_state);
        let pane_id = initial_state.active_pane.0;
        initial_state.toasts = unshit::core::toast::ToastStore::with_capacity(3, 60);
        crate::state::push_notification_toast(
            &mut initial_state,
            "test notification",
            "design-system toast smoke",
            workspace_id,
            pane_id,
        );
    }
    let shared: SharedState = Arc::new(std::sync::Mutex::new(initial_state));

    // Bring the unshit-ptyd daemon up and wire the UI's DaemonPty shim
    // to it. Uses a short-lived tokio runtime because connect_or_spawn
    // is async; the shim's own worker thread drives every subsequent
    // IPC call, so this runtime dies once the probe completes.
    {
        let socket_path = ptyd_socket_path();
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime for daemon probe");
        if let Err(err) = rt.block_on(daemon::connect_or_spawn(&socket_path)) {
            eprintln!("failed to connect or spawn unshit-ptyd daemon: {err}");
            std::process::exit(1);
        }
        drop(rt);
        let mut guard = shared.lock().unwrap();
        guard
            .pty_manager
            .connect_to(&socket_path)
            .expect("DaemonPty shim connect_to");
    }

    // Measure the actual monospace cell width ratio for later use (split
    // pane spawns, etc.). Do NOT pre-publish cell metrics to the global
    // atomics: the pre-published values differ slightly from what the
    // renderer measures (different FontSystem instance), causing the
    // on_resize handler to fire an intermediate resize with wrong column
    // count. Instead, let the renderer be the single source of truth:
    // on_resize stores last_grid_width (cell_w is 0 so no resize), then
    // on_cell_metrics fires with the renderer's exact cell_w and resizes
    // the PTY once to the correct dimensions.
    {
        let mut guard = shared.lock().unwrap();
        let font_size = guard.terminal_font_size_pt as f32 * guard.scale_factor;
        let line_height = font_size * crate::state::CSS_LINE_HEIGHT;
        guard.cell_width_ratio = crate::state::measure_cell_width_ratio_at(font_size, line_height);
    }

    // Reconcile the initial pane against any surviving daemon session
    // (slice 5): if a prior UI run left a session with a matching
    // `(workspace_id, pane_id)` on the daemon, reattach and replay its
    // snapshot so the user sees their shell exactly as they left it;
    // otherwise spawn a fresh one. This is load-bearing: without a live
    // terminal the CellGrid doesn't exist, the renderer can't publish
    // metrics, and the PTY never gets spawned (deadlock). Estimate
    // dimensions from the window size minus CSS chrome so the shell
    // greeting is formatted for roughly the right width; on_cell_metrics
    // corrects to exact values on the first frame.
    {
        let mut guard = shared.lock().unwrap();
        // CSS chrome in logical pixels (scale cancels: grid and cells both
        // scale equally).  sidebar(252) + resizer(4) + pane borders/margins
        // (4) + pane-body horizontal padding(24) = 284.  tabbar(38) +
        // statusbar(24) + pane-header(27) + pane borders/margins(4) +
        // pane-body vertical padding(16) = 109.
        let terminal_font_size = guard.terminal_font_size_pt as f32;
        let cell_w_est = terminal_font_size * guard.cell_width_ratio;
        let cell_h_est = terminal_font_size * crate::state::CSS_LINE_HEIGHT;
        let init_cols = ((1280.0_f32 - 284.0) / cell_w_est).max(1.0) as u16;
        let init_rows = ((800.0_f32 - 109.0) / cell_h_est).max(1.0) as u16;
        log::info!(
            "initial PTY estimate: {}x{} (cell_w_est={:.2}, cell_h_est={:.2})",
            init_cols,
            init_rows,
            cell_w_est,
            cell_h_est,
        );
        let pane_id = guard.active_pane.0;
        let workspace_id = crate::state::active_workspace_num(&guard);
        let cwd = crate::state::active_workspace_cwd(&guard);
        let shell = crate::shell::resolve(None, Some(&guard.default_shell));
        match guard.pty_manager.attach_or_spawn(
            pane_id,
            workspace_id,
            init_cols,
            init_rows,
            cwd.as_deref(),
            shell.as_ref(),
        ) {
            Ok((Some(snapshot), reader)) => {
                let rows = snapshot.grid.rows();
                let cols = snapshot.grid.cols();
                let mut terminal = crate::terminal::Terminal::new(rows, cols);
                terminal.apply_snapshot(&snapshot);
                guard.terminals.insert(
                    pane_id,
                    std::sync::Arc::new(std::sync::Mutex::new(terminal)),
                );
                crate::bridge::register_reader(pane_id, reader);
                if let Some(session_id) = guard.pty_manager.session_id(pane_id) {
                    record_diagnostic_pty_event(
                        &mut guard,
                        format!("attach pane={pane_id} session={session_id} source=initial"),
                    );
                }
                log::info!(
                    "reattached pane {} to surviving daemon session ({}x{})",
                    pane_id,
                    cols,
                    rows
                );
            }
            Ok((None, reader)) => {
                let terminal =
                    crate::terminal::Terminal::new(init_rows as usize, init_cols as usize);
                guard.terminals.insert(
                    pane_id,
                    std::sync::Arc::new(std::sync::Mutex::new(terminal)),
                );
                crate::bridge::register_reader(pane_id, reader);
                if let Some(session_id) = guard.pty_manager.session_id(pane_id) {
                    record_diagnostic_pty_event(
                        &mut guard,
                        format!("spawn pane={pane_id} session={session_id} source=initial"),
                    );
                }
            }
            Err(e) => {
                log::error!("failed to spawn initial PTY: {}", e);
                let mut terminal =
                    crate::terminal::Terminal::new(init_rows as usize, init_cols as usize);
                terminal.process_bytes(format!("Failed to spawn shell: {}\r\n", e).as_bytes());
                guard.terminals.insert(
                    pane_id,
                    std::sync::Arc::new(std::sync::Mutex::new(terminal)),
                );
            }
        }
    }

    // Reattach (or fresh-spawn) every *other* restored pane. The block
    // above brings up only the active pane (it must exist first so the
    // renderer can publish cell metrics). A restored layout can carry many
    // more panes across tabs and workspaces; each keeps a live terminal in
    // the runtime, so rejoin them here. A cache hit replays the surviving
    // daemon session's snapshot; a miss (the shell exited while we were
    // gone, or an upgrade) spawns a fresh shell in that pane.
    {
        let mut guard = shared.lock().unwrap();
        let terminal_font_size = guard.terminal_font_size_pt as f32;
        let cell_w_est = terminal_font_size * guard.cell_width_ratio;
        let cell_h_est = terminal_font_size * crate::state::CSS_LINE_HEIGHT;
        let init_cols = ((1280.0_f32 - 284.0) / cell_w_est).max(1.0) as u16;
        let init_rows = ((800.0_f32 - 109.0) / cell_h_est).max(1.0) as u16;
        let active_pane_id = guard.active_pane.0;

        // Snapshot the reattach targets up front so the immutable borrow of
        // `guard.workspaces` is released before we mutate `pty_manager` /
        // `terminals`. The active workspace's live tabs are mirrored into
        // `workspaces[active].tabs` by `restore_layout`, so iterating
        // `workspaces` covers every pane.
        let targets: Vec<(
            u32,
            u32,
            Option<std::path::PathBuf>,
            Option<crate::shell::ShellSpec>,
        )> = guard
            .workspaces
            .iter()
            .flat_map(|ws| {
                let ws_num = ws.num;
                let cwd = ws.path.clone();
                let shell = crate::shell::resolve(Some(&ws.shell), Some(&guard.default_shell));
                ws.tabs
                    .iter()
                    .flat_map(|tab| tab.panes.iter().flatten())
                    .filter(|pane| pane.id.0 != active_pane_id)
                    .map(move |pane| (ws_num, pane.id.0, cwd.clone(), shell.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();

        for (workspace_id, pane_id, cwd, shell) in targets {
            if guard.terminals.contains_key(&pane_id) {
                continue;
            }
            match guard.pty_manager.attach_or_spawn(
                pane_id,
                workspace_id,
                init_cols,
                init_rows,
                cwd.as_deref(),
                shell.as_ref(),
            ) {
                Ok((Some(snapshot), reader)) => {
                    let rows = snapshot.grid.rows();
                    let cols = snapshot.grid.cols();
                    let mut terminal = crate::terminal::Terminal::new(rows, cols);
                    terminal.apply_snapshot(&snapshot);
                    guard.terminals.insert(
                        pane_id,
                        std::sync::Arc::new(std::sync::Mutex::new(terminal)),
                    );
                    crate::bridge::register_reader(pane_id, reader);
                    log::info!(
                        "reattached background pane {} (workspace {}) to surviving session ({}x{})",
                        pane_id,
                        workspace_id,
                        cols,
                        rows
                    );
                }
                Ok((None, reader)) => {
                    let terminal =
                        crate::terminal::Terminal::new(init_rows as usize, init_cols as usize);
                    guard.terminals.insert(
                        pane_id,
                        std::sync::Arc::new(std::sync::Mutex::new(terminal)),
                    );
                    crate::bridge::register_reader(pane_id, reader);
                    log::info!(
                        "background pane {} (workspace {}) had no surviving session; spawned fresh",
                        pane_id,
                        workspace_id
                    );
                }
                Err(e) => {
                    log::error!(
                        "failed to reattach/spawn background pane {} (workspace {}): {}",
                        pane_id,
                        workspace_id,
                        e
                    );
                }
            }
        }
    }

    if let Some(cfg) = bench_config {
        crate::bench::start(cfg, shared.clone());
    }

    let tree_shared = shared.clone();
    let command_shared = shared.clone();
    let metrics_shared = shared.clone();
    let scale_shared = shared.clone();
    let window_state_shared = shared.clone();
    let close_shared = shared.clone();
    let sub_shared = shared.clone();
    let raw_key_shared = shared.clone();
    let file_drop_shared = shared.clone();
    let frame_metrics_shared = shared.clone();
    let scroll_metrics_shared = shared.clone();
    let scroll_tuning_shared = shared.clone();
    let window_event_sink: Arc<std::sync::OnceLock<unshit::app::EventSink>> =
        Arc::new(std::sync::OnceLock::new());
    let tree_window_event_sink = window_event_sink.clone();
    let fps_window_event_sink = window_event_sink.clone();

    let mut app = App::new(
        AppConfig {
            title: "terminal manager".to_string(),
            width: 1280,
            height: 800,
            decorations: false,
            css: STYLES.to_string(),
            fonts: terminal_font_sources(),
            user_shortcuts: user_shortcut_bindings(),
            on_command: Some(Arc::new(move |command: &str| -> bool {
                let mut guard = command_shared.lock_recover();
                dispatch(&mut guard, command)
            })),
            on_raw_key: Some(Arc::new(
                move |combo: &unshit::core::shortcut::KeyCombo| -> bool {
                    use unshit::core::event::Key;
                    let mut guard = raw_key_shared.lock().expect("state mutex poisoned");
                    // Recording mode owns the next key. Outside recording,
                    // plain Escape closes settings directly so repeated
                    // settings open/close cycles do not depend on shortcut
                    // resolver state.
                    if let Some(action) = guard.keybinds.recording {
                        if combo.key == Key::Escape && combo.modifiers.is_empty() {
                            dispatch(&mut guard, "keybind.cancel_record");
                        } else {
                            let cmd = format!("keybind.set:{}:{}", action.id(), combo);
                            dispatch(&mut guard, &cmd);
                        }
                        true
                    } else if crate::state::dispatch_palette_key(&mut guard, combo) {
                        true
                    } else if guard.quick_prompt.is_some() && is_quick_prompt_paste_combo(combo) {
                        // Quick Prompt is open: Ctrl+V attaches a clipboard
                        // image as a chip (spec U4/A4.1). Consume the event
                        // ONLY when an image was actually attached; otherwise
                        // return false so the framework falls through to its
                        // normal text paste and a plain Ctrl+V of text still
                        // lands in the input.
                        crate::state::try_attach_clipboard_image(&mut guard)
                    } else if guard.settings_open
                        && combo.key == Key::Escape
                        && combo.modifiers.is_empty()
                    {
                        dispatch(&mut guard, "modal.close")
                    } else {
                        false
                    }
                },
            )),
            // Approach 1: on_cell_metrics fires once after the first render
            // publishes valid cell dimensions. Resize all PTYs immediately.
            on_scale_factor: Some(Arc::new(move |scale: f32| {
                let mut guard = scale_shared.lock_recover();
                guard.scale_factor = scale;
                crate::state::sync_terminal_size_to_font_metrics(&mut guard);
            })),
            on_window_maximized: Some(Arc::new(move |maximized: bool| {
                let mut guard = window_state_shared.lock_recover();
                guard.window_maximized = maximized;
            })),
            on_close: Some(Arc::new(move || -> bool {
                // F7: when the user has not yet remembered a choice, veto
                // the framework's close and route through the confirm
                // dialog so they can pick "keep running" / "kill all" /
                // "cancel". Once a choice has been persisted it is applied
                // silently on close.
                //
                // Slice 5 session-survival policy still applies: keep-running
                // just drops local UI state (terminals map, readers) so the
                // current process can exit cleanly while the shells keep
                // running on the daemon. Kill-all routes through
                // `DaemonPty::destroy_all` before exit.
                //
                // Use .lock().ok() instead of .expect() so a poisoned mutex
                // (from a panic on another thread) does not prevent us from
                // reaching process::exit below.
                let action = {
                    let Ok(mut guard) = close_shared.lock() else {
                        // Mutex poisoned: skip the prompt path, fall back
                        // to the legacy "just exit" behaviour so we do not
                        // wedge the user holding an undismissable dialog.
                        finalize_profiler();
                        return true;
                    };
                    crate::state::resolve_close_action(&mut guard)
                };
                match action {
                    crate::state::CloseAction::Prompt => {
                        // Veto. The confirm dialog is now visible; the UI
                        // click handlers drive the real exit.
                        false
                    }
                    crate::state::CloseAction::KeepRunning => {
                        if let Ok(mut guard) = close_shared.lock() {
                            // Persist the live layout so the relaunch
                            // reattaches every surviving daemon session.
                            crate::persist::save_workspaces(&guard);
                            guard.terminals.clear();
                        }
                        finalize_profiler();
                        std::process::exit(0);
                    }
                    crate::state::CloseAction::KillAll => {
                        if let Ok(mut guard) = close_shared.lock() {
                            crate::state::mutate_kill_all_terminals(&mut guard);
                            crate::persist::save_workspaces(&guard);
                        }
                        finalize_profiler();
                        std::process::exit(0);
                    }
                }
            })),
            on_file_drop: Some(Arc::new(move |paths: &[std::path::PathBuf]| -> bool {
                // Native drag-and-drop. When the Quick Prompt overlay is
                // open, attach any dropped image files as chips (the
                // drag-and-drop counterpart to Ctrl+V). When it is closed,
                // `attach_dropped_images` is a no-op and we request no
                // rebuild, leaving terminal drops untouched.
                let mut guard = file_drop_shared.lock_recover();
                crate::state::attach_dropped_images(&mut guard, paths)
            })),
            on_cell_metrics: Some(Arc::new(move |cell_w: f32, cell_h: f32| {
                use unshit::core::cell_grid::CellGrid;
                let mut guard = metrics_shared.lock_recover();
                let (cols, rows) = CellGrid::take_pending_resize().unwrap_or_else(|| {
                    let w = guard.last_grid_width;
                    let h = guard.last_grid_height;
                    crate::state::compute_pty_dimensions(w, h, cell_w, cell_h)
                });
                log::info!(
                    "on_cell_metrics: cell={}x{} -> resize all PTYs to {}x{}",
                    cell_w,
                    cell_h,
                    cols,
                    rows
                );
                resize_all_terminals(&mut guard, cols, rows);
            })),
            on_frame_metrics: Some(Box::new(move |m| {
                crate::bench::record_frame(m);
                // record_frame returns true when the visible overlay is
                // due a rebuild (throttled to ~4Hz inside fps_overlay),
                // so a visible overlay no longer forces a full rebuild
                // every painted frame.
                let overlay_rebuild_due = crate::ui::fps_overlay::record_frame(m);
                if overlay_rebuild_due {
                    if let Some(sink) = fps_window_event_sink.get() {
                        let _ = sink.send(unshit::app::ExternalEvent::RequestRebuild);
                    }
                }
                if diagnostics_enabled {
                    let mut guard = frame_metrics_shared.lock_recover();
                    record_diagnostic_renderer_frame(&mut guard, unix_epoch_millis_now());
                }
            })),
            on_scroll_telemetry: diagnostics_enabled.then(|| {
                Box::new(move |sample: &unshit::app::ScrollTelemetry| {
                    let mut guard = scroll_metrics_shared.lock_recover();
                    guard.record_diagnostic_scroll_sample(sample);
                }) as Box<dyn Fn(&unshit::app::ScrollTelemetry) + Send>
            }),
            scroll_tuning: Some(Arc::new(move || {
                let guard = scroll_tuning_shared.lock_recover();
                unshit::app::ScrollTuning {
                    line_scroll_px: guard.scroll_line_px as f32,
                    smooth_scroll_duration_ms: guard.smooth_scroll_duration_ms as u64,
                }
            })),
            #[cfg(feature = "input-latency-histogram")]
            on_input_latency: Some(Box::new(|snap| crate::bench::record_input_latency(snap))),
            ..Default::default()
        },
        move || {
            // Grab the state mutex only long enough to snapshot the UI and
            // clone per-terminal `Arc<Mutex<Terminal>>` handles, then drop
            // it. The parser thread writing to any single terminal holds
            // only that terminal's mutex, so it never contends with this
            // closure on the state lock.
            let (
                snap,
                active_id,
                handles,
                force_terminal_theme_repaint,
                selections,
                selection_repaint,
            ): (
                crate::state::UiSnapshot,
                u32,
                Vec<(u32, crate::state::SharedTerminal)>,
                bool,
                std::collections::HashMap<u32, crate::state::TermSelection>,
                std::collections::HashSet<u32>,
            ) = {
                let mut guard = tree_shared.lock_recover();
                let snap = guard.ui_snapshot();
                let active_id = guard.active_pane.0;
                let handles: Vec<_> = guard
                    .terminals
                    .iter()
                    .map(|(&id, t)| (id, t.clone()))
                    .collect();
                let force_terminal_theme_repaint =
                    crate::state::take_terminal_theme_repaint_request(&mut guard);
                // Snapshot active selections and drain the per-pane
                // selection-changed set so the highlight below can force a
                // one-frame repaint of panes whose selection just changed.
                let selections = guard.terminal_selections.clone();
                let selection_repaint = std::mem::take(&mut guard.terminal_selection_repaint);
                (
                    snap,
                    active_id,
                    handles,
                    force_terminal_theme_repaint,
                    selections,
                    selection_repaint,
                )
            };

            // State mutex is released; take each per-terminal lock
            // independently to clone its display grid. `snapshot_terminal_for_render`
            // does NOT clear the live grid's dirty state (see issue #63):
            // interleaved PTY writes that land between two snapshots must
            // stay reflected as damage on the next clone, otherwise the
            // renderer can skip them and drop cells from the viewport.
            let grids: std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid> = handles
                .into_iter()
                .map(|(id, handle)| {
                    let mut t = handle.lock_recover();
                    let mut grid = snapshot_terminal_for_render(
                        &mut t,
                        id,
                        id == active_id,
                        &snap.theme,
                        &snap.custom_theme,
                        force_terminal_theme_repaint,
                    );
                    // Paint the selection highlight onto the per-frame clone
                    // (never the live buffer). Applied after the palette so
                    // the selection bg wins over the themed cell bg. The
                    // terminal maps the selection's absolute lines to current
                    // display rows, so it tracks content as the view scrolls.
                    if let Some(sel) = selections.get(&id) {
                        crate::state::apply_selection_highlight(&mut grid, &t, sel);
                    }
                    // Force a full repaint of this pane the frame its selection
                    // changed (added / moved / cleared) so the renderer's line
                    // cache re-emits the rows whose highlight just changed. A
                    // static selection re-applies the same bg each frame and
                    // needs no extra damage.
                    if selection_repaint.contains(&id) {
                        grid.mark_all_dirty();
                    }
                    (id, grid)
                })
                .collect();
            build_tree(
                &snap,
                &tree_shared,
                &grids,
                tree_window_event_sink.get().cloned(),
            )
        },
    );
    let _ = window_event_sink.set(app.event_sink());

    // Set up PTY output subscriptions.
    app.set_subscriptions(move || bridge::build_subscriptions(&sub_shared));

    if let Some(config) = diagnostics_config {
        let diagnostics_shared = shared.clone();
        app.spawn(async move {
            if let Err(err) = crate::diagnostics::server::run(config, diagnostics_shared).await {
                log::error!("diagnostic server stopped: {err}");
            }
        });
    }

    // Hand the framework's shared clipboard handle to AppState so
    // `terminal.paste` and any future paste callers reuse the same
    // underlying `arboard::Clipboard` instance. Concurrent arboard
    // handles in the same process can heap-corrupt on Windows; the
    // framework regression test `concurrent_clipboard_access_does_not_corrupt_heap`
    // documents that failure mode.
    {
        let app_clipboard = app.clipboard();
        let mut guard = shared.lock_recover();
        guard.clipboard = app_clipboard;
    }

    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SettingsSection};
    use crate::terminal::Terminal;
    use std::sync::{Arc, Mutex};
    use unshit::core::style::types::{Background, Color};
    use unshit_test::TestHarness;

    #[test]
    fn user_shortcut_bindings_includes_fps_overlay_toggle() {
        // Phase 0 of the 120fps perf work (refs #135) ships an in-app
        // FPS overlay toggled by Ctrl+Shift+F. Without this binding the
        // overlay is unreachable from the keyboard.
        let bindings = user_shortcut_bindings();
        assert!(
            bindings
                .iter()
                .any(|(s, c)| s == "Ctrl+Shift+F" && c == "fps_overlay.toggle"),
            "Ctrl+Shift+F must dispatch fps_overlay.toggle"
        );
    }

    #[test]
    fn stylesheet_prefers_design_system_font_stack_for_terminal_text() {
        let jetbrains = STYLES
            .find("'JetBrains Mono'")
            .expect("terminal font stack should include JetBrains Mono");
        let berkeley = STYLES
            .find("'Berkeley Mono'")
            .expect("terminal font stack should include Berkeley Mono");
        let consolas = STYLES
            .find("Consolas")
            .expect("terminal font stack should keep Consolas as a fallback");
        assert!(
            jetbrains < berkeley && berkeley < consolas,
            "font stack should follow the design-system order: JetBrains Mono, Berkeley Mono, then platform fallbacks"
        );
    }

    #[test]
    fn stylesheet_registers_bundled_jetbrains_mono_faces() {
        let stylesheet = unshit::core::style::parse::CompiledStylesheet::parse(STYLES);
        let jetbrains_faces = stylesheet
            .font_faces
            .iter()
            .filter(|face| face.family == "JetBrains Mono")
            .count();
        assert_eq!(
            jetbrains_faces, 3,
            "regular, semibold, and bold font faces should be available to the renderer"
        );
    }

    #[test]
    fn settings_route_controls_have_visible_scaled_layout() {
        let mut state = seed_state();
        state.settings_open = true;
        state.settings_section = SettingsSection::Appearance;
        let snap = state.ui_snapshot();
        let shared: SharedState = Arc::new(Mutex::new(state));
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let grids = std::collections::HashMap::new();
        let mut harness = TestHarness::new(
            STYLES,
            move || build_tree(&tree_snap, &tree_shared, &grids, None),
            1280.0,
            800.0,
        );
        harness.set_scale_factor(1.5);
        harness.step();

        for selector in [
            ".stepper",
            ".stepper-btn",
            ".set-inline-control",
            ".input-text",
            ".input-num",
            ".set-unit",
            ".preview-tile",
            ".set-page-savebar",
        ] {
            let snap = harness.query(selector).expect(selector);
            assert!(
                snap.layout_rect.width > 0.0 && snap.layout_rect.height > 0.0,
                "{selector} should have non-zero layout, got {:?}",
                snap.layout_rect
            );
            assert!(
                snap.layout_rect.x >= 0.0 && snap.layout_rect.x + snap.layout_rect.width <= 1280.0,
                "{selector} should be horizontally visible, got {:?}",
                snap.layout_rect
            );
        }
    }

    #[test]
    fn snapped_main_route_statusbar_stays_below_terminal_grid_with_actual_styles() {
        let state = seed_state();
        let active_pane = state.active_pane.0;
        let snap = state.ui_snapshot();
        let shared: SharedState = Arc::new(Mutex::new(state));
        let tree_snap = snap.clone();
        let tree_shared = shared.clone();
        let mut grids = std::collections::HashMap::new();
        grids.insert(active_pane, unshit::core::cell_grid::CellGrid::new(49, 79));
        let mut harness = TestHarness::new(
            STYLES,
            move || build_tree(&tree_snap, &tree_shared, &grids, None),
            1280.0,
            1368.0,
        );
        harness.set_scale_factor(1.5);
        harness.step();

        let content = harness.query(".content").expect("content exists");
        let terminal_grid = harness
            .query(".terminal-grid")
            .expect("terminal grid exists");
        let statusbar = harness.query(".statusbar").expect("statusbar exists");

        assert!(
            (statusbar.layout_rect.y + statusbar.layout_rect.height
                - (content.layout_rect.y + content.layout_rect.height))
                .abs()
                < 1.0,
            "statusbar should end at content bottom; content={:?} statusbar={:?}",
            content.layout_rect,
            statusbar.layout_rect
        );
        assert!(
            terminal_grid.layout_rect.y + terminal_grid.layout_rect.height
                <= statusbar.layout_rect.y + 1.0,
            "terminal grid should not cover statusbar; grid={:?} statusbar={:?}",
            terminal_grid.layout_rect,
            statusbar.layout_rect
        );
    }

    #[test]
    fn renderer_font_sources_prefer_jetbrains() {
        let fonts = terminal_font_sources_from_value(None);
        let first = fonts.first().expect("font source list must not be empty");
        match first {
            FontSource::System(name) => assert_eq!(name, "JetBrains Mono"),
            other => panic!("first terminal font source must be JetBrains Mono, got {other:?}"),
        }
    }

    #[test]
    fn renderer_font_sources_accept_parity_override() {
        let fonts =
            terminal_font_sources_from_value(Some(std::ffi::OsString::from("Cascadia Code")));
        let first = fonts.first().expect("font source list must not be empty");
        match first {
            FontSource::System(name) => assert_eq!(name, "Cascadia Code"),
            other => panic!("first terminal font source must be override, got {other:?}"),
        }

        let count = fonts
            .iter()
            .filter(|font| matches!(font, FontSource::System(name) if name == "Cascadia Code"))
            .count();
        assert_eq!(
            count, 1,
            "override should not duplicate an existing fallback"
        );
    }

    #[test]
    fn renderer_font_sources_bundle_jetbrains_weights() {
        let fonts = terminal_font_sources_from_value(None);
        let bundled = fonts
            .iter()
            .filter(|font| matches!(font, FontSource::Bytes(_)))
            .count();
        assert_eq!(
            bundled, 3,
            "regular, semibold, and bold weights must be bundled"
        );
    }

    #[test]
    fn ptyd_socket_path_from_env_uses_override() {
        let path = ptyd_socket_path_from_env(Some(std::ffi::OsString::from(
            r"\\.\pipe\unshit-ptyd-parity-test",
        )));
        assert_eq!(
            path,
            std::path::PathBuf::from(r"\\.\pipe\unshit-ptyd-parity-test")
        );
    }

    #[test]
    fn ptyd_socket_path_from_env_uses_default_when_missing() {
        assert_eq!(
            ptyd_socket_path_from_env(None),
            unshit_ptyd::transport::default_socket_path()
        );
    }

    #[test]
    fn truthy_env_value_accepts_common_enabled_values() {
        assert!(truthy_env_value(Some(std::ffi::OsString::from("1"))));
        assert!(truthy_env_value(Some(std::ffi::OsString::from("true"))));
        assert!(truthy_env_value(Some(std::ffi::OsString::from(
            "WindowsTerminal"
        ))));
    }

    #[test]
    fn truthy_env_value_rejects_common_disabled_values() {
        assert!(!truthy_env_value(None));
        assert!(!truthy_env_value(Some(std::ffi::OsString::from(""))));
        assert!(!truthy_env_value(Some(std::ffi::OsString::from("0"))));
        assert!(!truthy_env_value(Some(std::ffi::OsString::from("false"))));
        assert!(!truthy_env_value(Some(std::ffi::OsString::from("off"))));
        assert!(!truthy_env_value(Some(std::ffi::OsString::from("no"))));
    }

    #[test]
    fn open_settings_on_startup_accepts_enabled_values() {
        assert!(open_settings_on_startup_from_value(Some(
            std::ffi::OsString::from("1")
        )));
        assert!(open_settings_on_startup_from_value(Some(
            std::ffi::OsString::from("true")
        )));
        assert!(open_settings_on_startup_from_value(Some(
            std::ffi::OsString::from("on")
        )));
    }

    #[test]
    fn open_settings_on_startup_rejects_missing_or_disabled_values() {
        assert!(!open_settings_on_startup_from_value(None));
        assert!(!open_settings_on_startup_from_value(Some(
            std::ffi::OsString::from("0")
        )));
        assert!(!open_settings_on_startup_from_value(Some(
            std::ffi::OsString::from("false")
        )));
    }

    #[test]
    fn open_quick_prompt_on_startup_accepts_enabled_values() {
        assert!(open_quick_prompt_on_startup_from_value(Some(
            std::ffi::OsString::from("1")
        )));
        assert!(open_quick_prompt_on_startup_from_value(Some(
            std::ffi::OsString::from("true")
        )));
    }

    #[test]
    fn open_quick_prompt_on_startup_rejects_missing_or_disabled_values() {
        assert!(!open_quick_prompt_on_startup_from_value(None));
        assert!(!open_quick_prompt_on_startup_from_value(Some(
            std::ffi::OsString::from("0")
        )));
        assert!(!open_quick_prompt_on_startup_from_value(Some(
            std::ffi::OsString::from("false")
        )));
    }

    #[test]
    fn quick_prompt_attach_image_from_value_resolves_path_or_none() {
        assert_eq!(quick_prompt_attach_image_from_value(None), None);
        assert_eq!(
            quick_prompt_attach_image_from_value(Some(std::ffi::OsString::from(""))),
            None
        );
        assert_eq!(
            quick_prompt_attach_image_from_value(Some(std::ffi::OsString::from(
                "C:/tmp/shot.png"
            ))),
            Some(PathBuf::from("C:/tmp/shot.png"))
        );
    }

    #[test]
    fn open_confirm_dialog_on_startup_accepts_enabled_values() {
        assert!(open_confirm_dialog_on_startup_from_value(Some(
            std::ffi::OsString::from("1")
        )));
        assert!(open_confirm_dialog_on_startup_from_value(Some(
            std::ffi::OsString::from("true")
        )));
    }

    #[test]
    fn open_confirm_dialog_on_startup_rejects_missing_or_disabled_values() {
        assert!(!open_confirm_dialog_on_startup_from_value(None));
        assert!(!open_confirm_dialog_on_startup_from_value(Some(
            std::ffi::OsString::from("0")
        )));
        assert!(!open_confirm_dialog_on_startup_from_value(Some(
            std::ffi::OsString::from("false")
        )));
    }

    #[test]
    fn show_test_toast_on_startup_accepts_enabled_values() {
        assert!(show_test_toast_on_startup_from_value(Some(
            std::ffi::OsString::from("1")
        )));
        assert!(show_test_toast_on_startup_from_value(Some(
            std::ffi::OsString::from("true")
        )));
    }

    #[test]
    fn show_test_toast_on_startup_rejects_missing_or_disabled_values() {
        assert!(!show_test_toast_on_startup_from_value(None));
        assert!(!show_test_toast_on_startup_from_value(Some(
            std::ffi::OsString::from("0")
        )));
        assert!(!show_test_toast_on_startup_from_value(Some(
            std::ffi::OsString::from("false")
        )));
    }

    #[test]
    fn parity_shell_spec_from_values_returns_none_without_program() {
        let got = parity_shell_spec_from_values(None, None).expect("parse");
        assert!(got.is_none());
    }

    #[test]
    fn parity_shell_spec_from_values_parses_json_args() {
        let got = parity_shell_spec_from_values(
            Some(std::ffi::OsString::from("pwsh.exe")),
            Some(std::ffi::OsString::from(
                r#"["-NoLogo","-File","tools/parity/smoke-scene.ps1"]"#,
            )),
        )
        .expect("parse")
        .expect("spec");
        assert_eq!(got.program, "pwsh.exe");
        assert_eq!(
            got.args,
            vec![
                "-NoLogo".to_string(),
                "-File".to_string(),
                "tools/parity/smoke-scene.ps1".to_string()
            ]
        );
    }

    #[test]
    fn parity_shell_spec_from_values_rejects_invalid_json_args() {
        let err = parity_shell_spec_from_values(
            Some(std::ffi::OsString::from("pwsh.exe")),
            Some(std::ffi::OsString::from("{bad json")),
        )
        .expect_err("invalid JSON must be rejected");
        assert!(
            err.contains(ENV_PARITY_SHELL_ARGS_JSON),
            "error should name the env var, got {err}"
        );
    }

    #[test]
    fn apply_parity_shell_override_resets_persisted_workspace_shells() {
        let mut state = seed_state();
        state.active_workspace = 1;
        state.workspaces[0].shell = crate::shell::ShellSpec {
            program: "bash.exe".into(),
            args: vec!["--login".into()],
        };
        let parity_shell = crate::shell::ShellSpec {
            program: "pwsh.exe".into(),
            args: vec!["-File".into(), "tools/parity/smoke-scene.ps1".into()],
        };

        apply_parity_shell_override(&mut state, parity_shell.clone());

        assert_eq!(state.active_workspace, 0);
        assert_eq!(state.default_shell, parity_shell);
        assert_eq!(
            state.terminal_font_size_pt, WINDOWS_TERMINAL_PARITY_FONT_SIZE_PT,
            "parity mode should match Windows Terminal's 12pt-equivalent terminal font size"
        );
        assert!(
            state.sidebar_collapsed,
            "parity mode should focus capture space on the terminal content"
        );
        assert_eq!(
            state.sidebar_width, 48.0,
            "parity mode should narrow the sidebar layout width for screenshots"
        );
        assert!(
            state.workspaces.iter().all(|w| w.shell.is_empty()),
            "parity mode must clear workspace-specific shell overrides so the smoke shell wins"
        );
    }

    #[test]
    fn parity_font_size_pt_from_value_defaults_to_windows_terminal_equivalent() {
        let got = parity_font_size_pt_from_value(None).expect("default parity font size");
        assert_eq!(got, WINDOWS_TERMINAL_PARITY_FONT_SIZE_PT);
    }

    #[test]
    fn parity_font_size_pt_from_value_rejects_out_of_range_values() {
        let err =
            parity_font_size_pt_from_value(Some(std::ffi::OsString::from("99"))).expect_err("err");
        assert!(
            err.contains(ENV_PARITY_FONT_SIZE_PT),
            "error should name the env var, got {err}"
        );
    }

    #[test]
    fn parity_font_size_pt_from_value_rejects_non_integer_values() {
        let err = parity_font_size_pt_from_value(Some(std::ffi::OsString::from("16.5")))
            .expect_err("err");
        assert!(
            err.contains("must be an integer"),
            "error should describe the invalid integer parse, got {err}"
        );
    }

    #[test]
    fn stylesheet_applies_design_system_ambient_layers_to_app_root() {
        assert!(
            STYLES.contains(".app::before") && STYLES.contains("radial-gradient"),
            "design-system ambient amber radial glow should be applied to the app root"
        );
        assert!(
            STYLES.contains(".ambient-layer.rust-glow") && STYLES.contains("10% 100%"),
            "design-system lower-left rust glow should be represented as an app-level compatibility layer"
        );
        assert!(
            STYLES.contains(".app::after") && STYLES.contains("repeating-linear-gradient"),
            "design-system CRT scanline overlay should be applied to the app root"
        );
        assert!(
            STYLES.contains(".app.parity-windows-terminal::after"),
            "Windows Terminal parity mode should be able to disable the design-system scanline overlay"
        );
        assert!(
            STYLES.contains(".app.parity-windows-terminal > .ambient-layer"),
            "Windows Terminal parity mode should disable app-level ambient compatibility layers"
        );
    }

    #[test]
    fn stylesheet_collapsed_sidebar_reclaims_terminal_capture_space() {
        assert!(
            STYLES.contains(".sidebar.collapsed"),
            "parity mode depends on the collapsed sidebar class changing layout width"
        );
        assert!(
            STYLES.contains("min-width: 48px"),
            "collapsed sidebar should reclaim horizontal terminal capture space"
        );
    }

    #[test]
    fn stylesheet_has_windows_terminal_parity_theme() {
        assert!(
            STYLES.contains(".app.parity-windows-terminal"),
            "parity harness needs a class-gated theme override for Windows Terminal screenshots"
        );
        assert!(
            STYLES.contains("#0c0c0c") && STYLES.contains("#c4c4c4"),
            "parity theme should pin Windows Terminal default background and foreground"
        );
    }

    #[test]
    fn stylesheet_has_app_theme_classes_for_picker() {
        for theme in crate::theme::themes() {
            assert!(
                STYLES.contains(&format!(".app.theme-{}", theme.id)),
                "stylesheet should include app class for theme {}",
                theme.id
            );
            assert!(
                STYLES.contains(&format!(".theme-chip.{}", theme.id)),
                "stylesheet should include picker swatch for theme {}",
                theme.id
            );
        }
        assert!(STYLES.contains(".app.theme-custom"));
        assert!(STYLES.contains(".theme-chip.custom"));
        assert!(STYLES.contains(".custom-editor"));
        assert!(STYLES.contains(".theme-picker"));
        assert!(STYLES.contains(".theme-chip"));
        assert!(STYLES.contains(".theme-chip.active"));
    }

    #[test]
    fn settings_route_reflects_selected_theme_colors_after_rebuild() {
        let mut state = seed_state();
        state.settings_open = true;
        state.settings_section = SettingsSection::Appearance;
        state.theme = "catppuccin".to_string();
        let shared: SharedState = Arc::new(Mutex::new(state));
        let build_shared = shared.clone();
        let grids = std::collections::HashMap::new();
        let mut harness = TestHarness::new(
            STYLES,
            move || {
                let snap = build_shared.lock().unwrap().ui_snapshot();
                build_tree(&snap, &build_shared, &grids, None)
            },
            900.0,
            700.0,
        );

        shared.lock().unwrap().theme = "dracula".to_string();
        let rebuild_shared = shared.clone();
        let rebuild_grids = std::collections::HashMap::new();
        harness.rebuild(move || {
            let snap = rebuild_shared.lock().unwrap().ui_snapshot();
            build_tree(&snap, &rebuild_shared, &rebuild_grids, None)
        });

        let page = harness
            .query(".settings-page")
            .expect("settings page exists");
        assert_eq!(
            page.computed_style.background,
            Background::Color(Color::rgb(0x28, 0x2a, 0x36)),
            "changing AppState.theme should visibly restyle the settings route"
        );
        let content = harness
            .query(".set-page-content")
            .expect("settings content exists");
        assert_eq!(
            content.computed_style.background,
            Background::Color(Color::rgb(0x28, 0x2a, 0x36)),
            "theme background must reach the scrollable settings surface"
        );
    }

    #[test]
    fn settings_route_amber_surfaces_match_target_void_and_gradient() {
        let mut state = seed_state();
        state.settings_open = true;
        state.settings_section = SettingsSection::Appearance;
        state.theme = "amber".to_string();
        let shared: SharedState = Arc::new(Mutex::new(state));
        let build_shared = shared.clone();
        let grids = std::collections::HashMap::new();
        let harness = TestHarness::new(
            STYLES,
            move || {
                let snap = build_shared.lock().unwrap().ui_snapshot();
                build_tree(&snap, &build_shared, &grids, None)
            },
            925.0,
            540.0,
        );

        for selector in [".titlebar", ".statusbar"] {
            let node = harness.query(selector).expect(selector);
            assert_eq!(
                node.computed_style.background,
                Background::Color(Color::rgb(0x14, 0x11, 0x0c)),
                "{selector} should use the Claude target void surface"
            );
        }

        let savebar = harness.query(".set-page-savebar").expect("savebar");
        assert_eq!(
            savebar.computed_style.background,
            Background::Color(Color::rgb(0x17, 0x14, 0x11)),
            "savebar should use the current Claude target footer surface"
        );

        let header = harness.query(".set-page-header").expect("settings header");
        assert!(
            matches!(
                header.computed_style.background,
                Background::LinearGradient(_)
            ),
            "settings header should keep the subtle Claude gradient, got {:?}",
            header.computed_style.background
        );
    }

    #[test]
    fn settings_route_short_viewport_paints_theme_chip_content() {
        let mut state = seed_state();
        state.settings_open = true;
        state.settings_section = SettingsSection::Appearance;
        state.theme = "amber".to_string();
        let shared: SharedState = Arc::new(Mutex::new(state));
        let build_shared = shared.clone();
        let grids = std::collections::HashMap::new();
        let width = 1321u32;
        let height = 415u32;
        let mut harness = TestHarness::new(
            STYLES,
            move || {
                let snap = build_shared.lock().unwrap().ui_snapshot();
                build_tree(&snap, &build_shared, &grids, None)
            },
            width as f32,
            height as f32,
        );
        if !harness.try_with_gpu() {
            return;
        }
        harness.step();

        let screen = harness
            .query(".theme-chip.amber .theme-chip-screen")
            .expect("amber chip screen");
        assert!(
            screen.layout_rect.width > 100.0 && screen.layout_rect.height > 40.0,
            "amber chip screen should be laid out in short viewport, got {:?}",
            screen.layout_rect
        );

        let pixels = harness.render();
        let sample_x = (screen.layout_rect.x + screen.layout_rect.width * 0.5)
            .round()
            .clamp(0.0, (width - 1) as f32) as u32;
        let sample_y = (screen.layout_rect.y + screen.layout_rect.height * 0.72)
            .round()
            .clamp(0.0, (height - 1) as f32) as u32;
        let idx = ((sample_y * width + sample_x) * 4) as usize;
        let sample = [
            pixels[idx],
            pixels[idx + 1],
            pixels[idx + 2],
            pixels[idx + 3],
        ];
        assert!(
            sample[0].abs_diff(0x1c) <= 10
                && sample[1].abs_diff(0x18) <= 10
                && sample[2].abs_diff(0x12) <= 10,
            "amber chip screen should paint its dark preview at ({sample_x}, {sample_y}), got {sample:?}"
        );

        let name = harness
            .query(".theme-chip.amber .tcs-name")
            .expect("amber chip label");
        let x0 = name.layout_rect.x.floor().max(0.0) as u32;
        let y0 = name.layout_rect.y.floor().max(0.0) as u32;
        let x1 = (name.layout_rect.x + name.layout_rect.width)
            .ceil()
            .clamp(0.0, width as f32) as u32;
        let y1 = (name.layout_rect.y + name.layout_rect.height)
            .ceil()
            .clamp(0.0, height as f32) as u32;
        let mut saw_label_pixel = false;
        'scan: for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * width + x) * 4) as usize;
                if pixels[i].abs_diff(0xeb) <= 55
                    && pixels[i + 1].abs_diff(0xdc) <= 55
                    && pixels[i + 2].abs_diff(0xb6) <= 55
                {
                    saw_label_pixel = true;
                    break 'scan;
                }
            }
        }
        assert!(
            saw_label_pixel,
            "amber chip label should paint readable text in short viewport, got {:?}",
            name.layout_rect
        );
    }

    #[test]
    fn settings_route_visual_dump_when_requested() {
        let Some(path) = std::env::var_os("TM_SETTINGS_ROUTE_VISUAL_DUMP") else {
            return;
        };
        let width = std::env::var("TM_SETTINGS_ROUTE_VISUAL_WIDTH")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(924);
        let height = std::env::var("TM_SETTINGS_ROUTE_VISUAL_HEIGHT")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(540);

        let mut state = seed_state();
        state.settings_open = true;
        state.settings_section = SettingsSection::Appearance;
        state.theme = "amber".to_string();
        state.active_tab = 2;
        let shared: SharedState = Arc::new(Mutex::new(state));
        let build_shared = shared.clone();
        let grids = std::collections::HashMap::new();
        let mut harness = TestHarness::new(
            STYLES,
            move || {
                let snap = build_shared.lock().unwrap().ui_snapshot();
                build_tree(&snap, &build_shared, &grids, None)
            },
            width as f32,
            height as f32,
        );
        if !harness.try_with_gpu() {
            return;
        }
        harness.step();
        if let Some(scroll_y) = std::env::var("TM_SETTINGS_ROUTE_VISUAL_SCROLL_Y")
            .ok()
            .and_then(|raw| raw.parse::<f32>().ok())
        {
            harness.mouse_wheel(width as f32 * 0.5, height as f32 * 0.5, 0.0, -scroll_y);
            harness.step();
        }
        let screenshot = harness.screenshot();
        screenshot.save(path).expect("save screenshot");
    }

    #[test]
    fn main_route_reflects_selected_theme_colors_after_rebuild() {
        let mut state = seed_state();
        let active_pane = state.active_pane.0;
        state.theme = "catppuccin".to_string();
        let shared: SharedState = Arc::new(Mutex::new(state));
        let build_shared = shared.clone();
        let mut grids = std::collections::HashMap::new();
        grids.insert(active_pane, unshit::core::cell_grid::CellGrid::new(24, 80));
        let mut harness = TestHarness::new(
            STYLES,
            move || {
                let snap = build_shared.lock().unwrap().ui_snapshot();
                build_tree(&snap, &build_shared, &grids, None)
            },
            1280.0,
            800.0,
        );

        shared.lock().unwrap().theme = "dracula".to_string();
        let rebuild_shared = shared.clone();
        let mut rebuild_grids = std::collections::HashMap::new();
        rebuild_grids.insert(active_pane, unshit::core::cell_grid::CellGrid::new(24, 80));
        harness.rebuild(move || {
            let snap = rebuild_shared.lock().unwrap().ui_snapshot();
            build_tree(&snap, &rebuild_shared, &rebuild_grids, None)
        });

        let sidebar = harness.query(".sidebar").expect("sidebar exists");
        assert_eq!(
            sidebar.computed_style.background,
            Background::Color(Color::rgb(0x21, 0x22, 0x2c)),
            "changing AppState.theme should visibly restyle the sidebar"
        );
    }

    #[test]
    fn font_settings_restyle_immediately_after_rebuild() {
        let mut state = seed_state();
        let active_pane = state.active_pane.0;
        state.config_font_size_pt = 16;
        state.terminal_font_size_pt = 13;
        let shared: SharedState = Arc::new(Mutex::new(state));
        let build_shared = shared.clone();
        let mut grids = std::collections::HashMap::new();
        grids.insert(active_pane, unshit::core::cell_grid::CellGrid::new(24, 80));
        let mut harness = TestHarness::new(
            STYLES,
            move || {
                let snap = build_shared.lock().unwrap().ui_snapshot();
                build_tree(&snap, &build_shared, &grids, None)
            },
            1280.0,
            800.0,
        );
        harness.step();

        let breadcrumb = harness
            .query(".titlebar-breadcrumb")
            .expect("titlebar breadcrumb exists");
        assert!(
            breadcrumb.computed_style.font_size > 12.5,
            "config font size should scale app chrome text"
        );
        let terminal = harness
            .query(".terminal-content")
            .expect("terminal content exists");
        assert!(
            (terminal.computed_style.font_size - 13.0).abs() < 0.01,
            "config font size must not scale terminal text"
        );

        shared.lock().unwrap().terminal_font_size_pt = 18;
        let rebuild_shared = shared.clone();
        let mut rebuild_grids = std::collections::HashMap::new();
        rebuild_grids.insert(active_pane, unshit::core::cell_grid::CellGrid::new(24, 80));
        harness.rebuild(move || {
            let snap = rebuild_shared.lock().unwrap().ui_snapshot();
            build_tree(&snap, &rebuild_shared, &rebuild_grids, None)
        });
        let terminal = harness
            .query(".terminal-content")
            .expect("terminal content exists after rebuild");
        assert!(
            (terminal.computed_style.font_size - 18.0).abs() < 0.01,
            "terminal font size should restyle on the next rebuild"
        );
    }

    #[test]
    fn snapshot_terminal_for_render_applies_active_terminal_theme_palette() {
        let mut terminal = Terminal::new(1, 4);
        terminal.process_bytes(b"\x1b[31mR\x1b[0mD");

        let themed = snapshot_terminal_for_render(
            &mut terminal,
            0,
            true,
            "dracula",
            &crate::theme::default_custom_theme(),
            false,
        );
        let palette = crate::theme::terminal_palette("dracula");

        // Display snapshots carry one overscan row above the viewport,
        // so live row 0 sits at grid row 1.
        assert_eq!(
            themed.get_cell(1, 0).expect("red cell").fg,
            palette.ansi[1],
            "SGR red should render through the active theme ANSI palette"
        );
        assert_eq!(
            themed.get_cell(1, 1).expect("default cell").fg,
            palette.default_fg,
            "default text should render through the active theme foreground"
        );
    }

    #[test]
    fn snapshot_terminal_for_render_can_force_full_theme_repaint() {
        let mut terminal = Terminal::new(2, 4);
        terminal.process_bytes(b"x");
        terminal.grid_mut().clear_dirty();

        let themed = snapshot_terminal_for_render(
            &mut terminal,
            0,
            true,
            "dracula",
            &crate::theme::default_custom_theme(),
            true,
        );

        assert!(
            themed.line_damage().iter().all(|ld| !ld.is_clean()),
            "forced theme repaint should damage every displayed row"
        );
        assert!(
            themed.dirty_flags().iter().all(|dirty| *dirty),
            "forced theme repaint should mark every displayed cell dirty"
        );
    }

    #[test]
    fn snapshot_terminal_for_render_respects_terminal_cursor_hide() {
        let mut terminal = Terminal::new(1, 4);
        terminal.process_bytes(b"\x1b[?25l");

        let active = snapshot_terminal_for_render(
            &mut terminal,
            0,
            true,
            "catppuccin",
            &crate::theme::default_custom_theme(),
            false,
        );

        assert!(
            !active.cursor_visible(),
            "active snapshot must not re-show a cursor hidden by CSI ?25l"
        );
    }

    #[test]
    fn snapshot_terminal_for_render_hides_inactive_clone_without_mutating_live_cursor() {
        let mut terminal = Terminal::new(1, 4);
        assert!(terminal.grid().cursor_visible());

        let inactive = snapshot_terminal_for_render(
            &mut terminal,
            0,
            false,
            "catppuccin",
            &crate::theme::default_custom_theme(),
            false,
        );

        assert!(
            !inactive.cursor_visible(),
            "inactive snapshot should hide the rendered cursor"
        );
        assert!(
            terminal.grid().cursor_visible(),
            "inactive masking must not overwrite terminal-owned cursor visibility"
        );
    }

    /// Regression test for the clipboard paste keybind feature.
    ///
    /// Both Ctrl+V (Windows convention) and Ctrl+Shift+V (Linux
    /// terminal convention, where Ctrl+V is reserved by the shell for
    /// literal-input mode) MUST be registered against
    /// `terminal.paste`. If a future agent removes one binding the
    /// user loses paste from at least one platform's muscle memory;
    /// this test catches that before it ships.
    #[test]
    fn user_shortcut_bindings_wires_terminal_paste_to_both_combos() {
        let bindings = user_shortcut_bindings();
        let pasters: Vec<&str> = bindings
            .iter()
            .filter(|(_, c)| c == "terminal.paste")
            .map(|(s, _)| s.as_str())
            .collect();
        assert!(
            pasters.contains(&"Ctrl+V"),
            "Ctrl+V must dispatch terminal.paste; got {pasters:?}"
        );
        assert!(
            pasters.contains(&"Ctrl+Shift+V"),
            "Ctrl+Shift+V must dispatch terminal.paste; got {pasters:?}"
        );
    }

    /// The Quick Prompt image-paste hook fires only on a bare Ctrl+V.
    /// Uppercase `'V'` (the key combo is always lowercased), a missing
    /// Ctrl, or extra modifiers must NOT match, so plain typing and
    /// Ctrl+Shift+V (terminal literal paste) are never mistaken for an
    /// image paste.
    #[test]
    fn quick_prompt_paste_combo_matches_only_ctrl_v() {
        use unshit::core::event::{Key, Modifiers};
        use unshit::core::shortcut::KeyCombo;

        assert!(is_quick_prompt_paste_combo(&KeyCombo::new(
            Key::Char('v'),
            Modifiers::CTRL
        )));
        // Wrong / extra modifiers.
        assert!(!is_quick_prompt_paste_combo(&KeyCombo::plain(Key::Char('v'))));
        assert!(!is_quick_prompt_paste_combo(&KeyCombo::new(
            Key::Char('v'),
            Modifiers::CTRL | Modifiers::SHIFT
        )));
        // Different key.
        assert!(!is_quick_prompt_paste_combo(&KeyCombo::new(
            Key::Char('c'),
            Modifiers::CTRL
        )));
    }

    /// Regression test for issue #63.
    ///
    /// The render loop snapshots each terminal's display grid once per
    /// frame. The old code additionally called
    /// `terminal.grid_mut().clear_dirty()` on the LIVE grid right after
    /// the clone. If a PTY chunk wrote to the live grid between the
    /// clone and the clear (or, more generally, at any time before the
    /// next snapshot with a clear still pending), the clear would wipe
    /// the damage bumped by that interleaved write, and the next
    /// snapshot would report the affected row as clean. The renderer
    /// would then skip the row on a cache miss
    /// (see `crates/unshit-framework/crates/unshit-renderer/src/batch.rs`
    /// `emit_grid_cells` `row_is_clean` path) and cells would disappear
    /// from the viewport.
    ///
    /// This test simulates that exact ordering and asserts that the
    /// interleaved write's damage survives to the next snapshot.
    #[test]
    fn snapshot_terminal_for_render_preserves_interleaved_write_damage() {
        // Models the exact ordering the task spec calls out:
        //   1. `display_grid()` (snapshot) already happened in the
        //      previous frame.
        //   2. A PTY chunk lands on the live grid, bumping line damage.
        //   3. The render loop runs its post-snapshot reset (what the
        //      broken code did was `grid_mut().clear_dirty()` on the
        //      live grid).
        //   4. The next frame snapshots again.
        //
        // With the broken code, step 3 wiped the live grid's damage for
        // step 2's writes, so step 4's clone reported the row as clean
        // and the renderer skipped it on a cache miss, dropping cells.
        let mut terminal = Terminal::new(10, 40);
        terminal.process_bytes(b"first line");

        // Step 1: previous-frame snapshot (establishes the starting
        // state for the race).
        let _snap_previous_frame = snapshot_terminal_for_render(
            &mut terminal,
            0,
            true,
            "catppuccin",
            &crate::theme::default_custom_theme(),
            false,
        );

        // Step 2: a PTY chunk lands after the previous snapshot but
        // before the next frame, writing row 1.
        terminal.process_bytes(b"\r\nsecond line");

        // Step 3: production's "post-snapshot reset". This helper
        // mirrors whatever the main-loop does after taking a snapshot.
        // With the #63 fix the helper is a no-op; with the broken code
        // it used to call `grid_mut().clear_dirty()` on the live grid
        // and wipe the damage for step 2's writes.
        post_snapshot_reset_like_production(&mut terminal);

        // Step 4: the next frame snapshots the live grid. Before the
        // fix, row 1's damage was wiped by step 3 and this clone
        // reported the row as clean.
        let snap_next_frame = snapshot_terminal_for_render(
            &mut terminal,
            0,
            true,
            "catppuccin",
            &crate::theme::default_custom_theme(),
            false,
        );

        // Live row 1 sits at grid row 2 (one overscan row above the
        // viewport in every display snapshot).
        let row1 = snap_next_frame
            .line_damage_for(2)
            .expect("row 1 damage entry must exist");
        assert!(
            !row1.is_clean(),
            "row 1 damage is clean after an interleaved PTY write \
             followed by the production post-snapshot reset. The \
             renderer would then skip row 1 on a cache miss and drop \
             its cells (regression of issue #63)"
        );

        // Sanity: the freshly written cells actually live on row 1.
        let cell = snap_next_frame
            .get_cell(2, 0)
            .expect("row 1 col 0 must exist");
        assert_eq!(
            cell.ch, 's',
            "row 1 col 0 should hold the 's' of 'second line'"
        );
    }

    /// Mirrors the production "post-snapshot reset" step that used to
    /// be inlined at the end of every per-terminal iteration inside
    /// `tree_fn`. Before the #63 fix this called
    /// `terminal.grid_mut().clear_dirty()` on the live grid. The fix
    /// removed that call entirely, so this helper is now a no-op. If
    /// someone reintroduces the clear here or in
    /// `snapshot_terminal_for_render`, the regression tests below will
    /// trip.
    fn post_snapshot_reset_like_production(_terminal: &mut Terminal) {
        // Intentionally empty. See issue #63.
    }

    // refs #140 / Task 11: the bench mode used to mutate the SHELL env
    // var to force cmd.exe on Windows so `dir` produced real Windows
    // output. The default-shell pipeline now owns shell selection;
    // mutating SHELL would silently override the user's persisted
    // `default_shell` and produce bench numbers that diverge from the
    // configured shell. This regression scans the main.rs source so a
    // future commit reintroducing the env hack fails loudly.
    #[test]
    fn bench_does_not_mutate_shell_env_var() {
        let src = include_str!("main.rs");
        // Constructed from parts so the test source itself does not
        // match the needle and trip the assertion.
        let needle = concat!("set_", "var(\"SHELL\"");
        assert!(
            !src.contains(needle),
            "bench must not mutate the SHELL env var; route the bench shell through default_shell instead"
        );
    }

    /// Auxiliary regression for #63: the live grid's damage must keep
    /// accumulating across multiple snapshots so that every write ever
    /// performed (until the renderer has had a chance to replay it via
    /// the content-hash cache) shows up as damage on the latest
    /// snapshot. If someone reintroduces `clear_dirty()` on the live
    /// grid inside `snapshot_terminal_for_render`, this test also
    /// fails on the second row.
    #[test]
    fn snapshot_terminal_for_render_accumulates_damage_across_snapshots() {
        let mut terminal = Terminal::new(10, 40);

        terminal.process_bytes(b"alpha");
        let _s1 = snapshot_terminal_for_render(
            &mut terminal,
            0,
            true,
            "catppuccin",
            &crate::theme::default_custom_theme(),
            false,
        );

        terminal.process_bytes(b"\r\nbeta");
        let _s2 = snapshot_terminal_for_render(
            &mut terminal,
            0,
            true,
            "catppuccin",
            &crate::theme::default_custom_theme(),
            false,
        );

        terminal.process_bytes(b"\r\ngamma");
        let s3 = snapshot_terminal_for_render(
            &mut terminal,
            0,
            true,
            "catppuccin",
            &crate::theme::default_custom_theme(),
            false,
        );

        // Live rows 0..=2 sit at grid rows 1..=3 below the overscan row.
        for row in 1..=3 {
            let ld = s3
                .line_damage_for(row)
                .unwrap_or_else(|| panic!("damage entry for row {}", row));
            assert!(
                !ld.is_clean(),
                "row {} damage is clean; a previous snapshot cleared live \
                 grid damage and dropped this row's writes (#63)",
                row
            );
        }
    }
}
