//! Shared helpers for unshit benchmarks.

use taffy::TaffyTree;
use unshit_core::build::build_tree_from_def;
pub use unshit_core::build::{resolve_all_styles, run_layout_pipeline};
use unshit_core::element::{ElementDef, Tag};
use unshit_core::id::NodeId;
use unshit_core::layout::TextMeasureCtx;
use unshit_core::tree::NodeArena;

/// Build a realistic element tree definition with ~500 nodes.
///
/// Structure: a root container with 10 sections, each containing 5 rows,
/// each row containing ~10 leaf elements (mix of text spans and buttons).
pub fn build_large_tree_def() -> ElementDef {
    build_large_tree_def_inner(false)
}

fn build_large_tree_def_inner(keyed: bool) -> ElementDef {
    let mut root = ElementDef::new(Tag::Div).with_class("root").with_id("app");

    for section_i in 0..10 {
        let mut section = ElementDef::new(Tag::Div)
            .with_class("section")
            .with_class(if section_i % 2 == 0 { "even" } else { "odd" });
        if keyed {
            section = section.with_id(format!("section-{}", section_i));
        }

        let header = ElementDef::new(Tag::Div).with_class("header").with_child(
            ElementDef::new(Tag::Text)
                .with_class("title")
                .with_text(format!("Section {}", section_i + 1)),
        );
        section = section.with_child(header);

        for row_i in 0..5 {
            let mut row = ElementDef::new(Tag::Div)
                .with_class("row")
                .with_class(if row_i % 2 == 0 { "row-even" } else { "row-odd" });
            if keyed {
                row = row.with_id(format!("row-{}-{}", section_i, row_i));
            }

            row =
                row.with_child(ElementDef::new(Tag::Span).with_class("label").with_text(format!(
                    "Item {}.{}",
                    section_i + 1,
                    row_i + 1
                )));

            for col_i in 0..5 {
                row = row.with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("cell")
                        .with_class(if col_i % 2 == 0 { "primary" } else { "secondary" })
                        .with_text(format!("Value {}", col_i * 10 + row_i)),
                );
            }

            for btn_i in 0..3 {
                row = row.with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class(match btn_i {
                            0 => "btn-primary",
                            1 => "btn-secondary",
                            _ => "btn-danger",
                        })
                        .with_text(match btn_i {
                            0 => "Edit".to_string(),
                            1 => "Copy".to_string(),
                            _ => "Delete".to_string(),
                        }),
                );
            }

            section = section.with_child(row);
        }

        root = root.with_child(section);
    }

    root
}

/// Generate a realistic CSS stylesheet string with ~100 rules.
pub fn generate_large_css() -> String {
    let mut css = String::with_capacity(8192);

    css.push_str(
        r#"
.root {
    display: flex;
    flex-direction: column;
    width: 100%;
    padding: 16px;
    gap: 12px;
    background: #1a1a2e;
    color: #e0e0e0;
    font-size: 14px;
    line-height: 1.5;
}

.section {
    display: flex;
    flex-direction: column;
    padding: 12px;
    gap: 8px;
    background: #16213e;
    border-radius: 8px;
    border-width: 1px;
    border-color: #2a2a4a;
}

.section.even {
    background: #1a1a3e;
}

.section.odd {
    background: #16213e;
}

.header {
    display: flex;
    flex-direction: row;
    align-items: center;
    padding: 8px 12px;
    background: #0f3460;
    border-radius: 4px;
}

.title {
    font-size: 18px;
    font-weight: bold;
    color: #e94560;
}

.row {
    display: flex;
    flex-direction: row;
    align-items: center;
    padding: 6px 8px;
    gap: 8px;
    border-radius: 4px;
}

.row-even {
    background: rgba(255, 255, 255, 0.03);
}

.row-odd {
    background: rgba(255, 255, 255, 0.06);
}

.label {
    font-weight: bold;
    color: #a8d8ea;
    min-width: 80px;
    padding: 4px 8px;
}

.cell {
    padding: 4px 12px;
    border-radius: 4px;
    min-width: 60px;
}

.cell.primary {
    background: rgba(233, 69, 96, 0.1);
    color: #e94560;
}

.cell.secondary {
    background: rgba(168, 216, 234, 0.1);
    color: #a8d8ea;
}

.btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 4px 12px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: bold;
    cursor: pointer;
}

.btn-primary {
    background: #e94560;
    color: white;
}

.btn-secondary {
    background: #0f3460;
    color: #a8d8ea;
    border-width: 1px;
    border-color: #a8d8ea;
}

.btn-danger {
    background: #c0392b;
    color: white;
}
"#,
    );

    css.push_str(
        r#"
.btn:hover {
    opacity: 0.85;
}

.btn-primary:hover {
    background: #d63851;
}

.btn-secondary:hover {
    background: #1a4a7a;
}

.btn-danger:hover {
    background: #e74c3c;
}

.row:hover {
    background: rgba(233, 69, 96, 0.08);
}

.section:hover {
    border-color: #e94560;
}
"#,
    );

    for i in 0..40 {
        css.push_str(&format!(
            r#"
.custom-class-{i} {{
    padding: {p}px;
    margin: {m}px;
    font-size: {fs}px;
    color: #{r:02x}{g:02x}{b:02x};
    background: #{br:02x}{bg:02x}{bb:02x};
    border-radius: {rad}px;
    gap: {gap}px;
    opacity: {op};
}}
"#,
            i = i,
            p = 4 + (i % 8),
            m = 2 + (i % 6),
            fs = 12 + (i % 10),
            r = 50 + (i * 5) % 200,
            g = 80 + (i * 3) % 170,
            b = 100 + (i * 7) % 150,
            br = 20 + (i * 4) % 60,
            bg = 20 + (i * 2) % 60,
            bb = 30 + (i * 3) % 80,
            rad = 2 + (i % 12),
            gap = 4 + (i % 8),
            op = format!("{:.2}", 0.5 + (i as f32 % 6.0) * 0.1),
        ));
    }

    css
}

/// Generate a CSS string with ~50 rules for cascade benchmarks.
pub fn generate_cascade_css() -> String {
    let mut css = String::with_capacity(4096);

    css.push_str(
        r#"
.root {
    display: flex;
    flex-direction: column;
    padding: 16px;
    gap: 12px;
}

.section {
    display: flex;
    flex-direction: column;
    padding: 12px;
    gap: 8px;
}

.header {
    display: flex;
    padding: 8px;
    font-size: 18px;
    font-weight: bold;
}

.row {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    padding: 4px;
}

.label {
    font-weight: bold;
    min-width: 80px;
}

.cell {
    padding: 4px 8px;
}

.btn {
    padding: 4px 12px;
    border-radius: 4px;
    cursor: pointer;
}

.title { color: #e94560; }
.primary { color: #e94560; }
.secondary { color: #a8d8ea; }
.btn-primary { background: #e94560; }
.btn-secondary { background: #0f3460; }
.btn-danger { background: #c0392b; }
.even { background: #1a1a3e; }
.odd { background: #16213e; }
.row-even { background: rgba(255, 255, 255, 0.03); }
.row-odd { background: rgba(255, 255, 255, 0.06); }

.btn:hover { opacity: 0.85; }
.btn-primary:hover { background: #d63851; }
.row:hover { background: rgba(233, 69, 96, 0.08); }
"#,
    );

    for i in 0..30 {
        css.push_str(&format!(
            ".extra-rule-{i} {{ padding: {p}px; color: #{c:02x}{c:02x}{c:02x}; }}\n",
            i = i,
            p = 4 + i % 12,
            c = 80 + (i * 7) % 170,
        ));
    }

    css
}

/// Build an arena + taffy tree from an `ElementDef`, returning (arena, taffy, root_id).
pub fn materialize_tree(def: &ElementDef) -> (NodeArena, taffy::TaffyTree<TextMeasureCtx>, NodeId) {
    let mut arena = NodeArena::new();
    let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
    let root = build_tree_from_def(def, &mut arena, &mut taffy, NodeId::DANGLING);
    (arena, taffy, root)
}

/// Same as `build_large_tree_def` but with id attributes on sections and rows
/// for keyed reconciliation testing.
pub fn build_large_tree_def_with_keys() -> ElementDef {
    build_large_tree_def_inner(true)
}

/// Run reconciliation on an existing tree against a new definition.
pub fn reconcile_tree(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    root: NodeId,
    new_def: &ElementDef,
) {
    unshit_core::reconcile::reconcile(arena, taffy, root, new_def);
}
