//! What the window shows between "it exists" and "the GPU can draw into it".
//!
//! GPU adapter and device creation costs over a second on a cold start, and
//! none of it can be skipped. The window, the stylesheet, the fonts, the
//! element tree and the layout are all ready long before that -- so the only
//! thing missing when the user is staring at a blank rectangle is the ability
//! to *rasterize*, not the knowledge of what to draw.
//!
//! This module turns the already-laid-out tree into a flat, back-to-front list
//! of solid rectangles and text runs that a platform blitter can put on screen
//! without any GPU at all. It is deliberately not a renderer: no gradients, no
//! rounded corners, no shadows, no glyph atlas, no terminal cells. It is the
//! app's real geometry in the app's real colors, which is enough to read as
//! "this is the application, starting" rather than "this is a dead window".
//!
//! Everything here is pure: tree in, commands out. The platform paint lives in
//! [`crate::splash_paint`], so this half is testable without a window.

use unshit_core::element::{Element, ElementContent};
use unshit_core::id::NodeId;
use unshit_core::style::types::{Background, Color, Display, Overflow};
use unshit_core::tree::NodeArena;

/// Deepest tree level the collector will descend.
///
/// The walk is recursive and runs on the event-loop thread during startup,
/// where a stack overflow would be indistinguishable from the hang this whole
/// feature exists to remove. Real UI trees are tens of levels deep; this bound
/// only ever trips on a cycle or a pathological build.
const MAX_DEPTH: u32 = 256;

/// Alpha below which a subtree is treated as invisible and skipped.
///
/// One 8-bit level is `1.0 / 255.0`; anything under that cannot change a pixel.
const ALPHA_EPSILON: f32 = 1.0 / 255.0;

/// An axis-aligned rectangle in physical window pixels, already clipped.
///
/// Integer because every consumer is a pixel blitter, and because rounding
/// once here keeps the painter from disagreeing with itself about edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplashRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl SplashRect {
    fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    fn intersect(&self, other: &SplashRect) -> SplashRect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        SplashRect { x, y, width: right - x, height: bottom - y }
    }

    fn from_layout(rect: &unshit_core::element::LayoutRect) -> SplashRect {
        // Round the edges rather than the origin and size independently, so
        // adjacent boxes that share an edge in layout space still share it in
        // pixel space instead of leaving a one-pixel seam.
        let left = rect.x.round() as i32;
        let top = rect.y.round() as i32;
        let right = (rect.x + rect.width).round() as i32;
        let bottom = (rect.y + rect.height).round() as i32;
        SplashRect { x: left, y: top, width: right - left, height: bottom - top }
    }
}

/// One thing to put on screen, in back-to-front order.
#[derive(Clone, Debug, PartialEq)]
pub enum SplashCommand {
    /// A solid fill. The color is already composited against everything behind
    /// it, so the painter can blit it opaquely and never needs to blend.
    Fill { rect: SplashRect, color: Color },
    /// A text run. Placement is the element's content box; the painter is
    /// expected to approximate, not to reproduce the real shaper.
    Text {
        rect: SplashRect,
        color: Color,
        /// Font size in physical pixels, as the cascade resolved it.
        font_size: f32,
        text: String,
    },
}

/// Flatten a laid-out, styled tree into paintable commands.
///
/// `surface` is the window's physical size and acts as the outermost clip, so
/// a tree laid out for a stale size cannot scribble outside the window.
///
/// The result is in painter's order: index 0 is furthest back. Colors are
/// pre-composited against `backdrop`, which should be whatever the platform
/// will clear the window to (and, ideally, whatever the GPU's first frame will
/// clear to -- matching them is what makes the eventual swap invisible).
pub fn collect(
    arena: &NodeArena,
    root: NodeId,
    surface: (u32, u32),
    backdrop: Color,
) -> Vec<SplashCommand> {
    let clip = SplashRect { x: 0, y: 0, width: surface.0 as i32, height: surface.1 as i32 };
    let mut out = Vec::new();
    if clip.is_empty() {
        return out;
    }
    walk(arena, root, clip, backdrop, 1.0, 0, &mut out);
    out
}

fn walk(
    arena: &NodeArena,
    id: NodeId,
    clip: SplashRect,
    under: Color,
    inherited_alpha: f32,
    depth: u32,
    out: &mut Vec<SplashCommand>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(el) = arena.get(id) else {
        return;
    };
    let style = &el.computed_style;
    if style.display == Display::None {
        return;
    }

    // CSS opacity applies to the element *and its subtree* as a group. This is
    // not that -- a true group opacity needs an offscreen layer -- but
    // multiplying down the tree gets the common case (a faded-in panel) right
    // and never produces something more opaque than the real renderer would.
    let alpha = inherited_alpha * style.opacity.clamp(0.0, 1.0);
    if alpha < ALPHA_EPSILON {
        return;
    }

    let node_rect = SplashRect::from_layout(&el.layout_rect).intersect(&clip);

    // The color descendants sit on top of. It only changes when this node
    // actually covers them with something.
    let mut under_for_children = under;

    if let Some(src) = background_color(&style.background) {
        let composited = composite(src, under, alpha);
        if !node_rect.is_empty() {
            out.push(SplashCommand::Fill { rect: node_rect, color: composited });
        }
        // Track the composited color even when the rect was clipped away: a
        // child that overflows into a visible region still sits on this
        // node's color conceptually, and guessing `under` is better than
        // guessing the grandparent's.
        under_for_children = composited;
    }

    if let ElementContent::Text(ref text) = el.content {
        if !text.trim().is_empty() && !node_rect.is_empty() {
            out.push(SplashCommand::Text {
                rect: node_rect,
                color: composite(style.color, under_for_children, alpha),
                font_size: style.font_size,
                text: text.clone(),
            });
        }
    }

    // Only a scroll container clips its children; under `overflow: visible` a
    // child is free to paint outside the parent box, so the clip must not
    // shrink.
    let clips = style.overflow_x != Overflow::Visible || style.overflow_y != Overflow::Visible;
    let child_clip = if clips { node_rect } else { clip };
    if child_clip.is_empty() {
        return;
    }

    for child in arena.children(id) {
        walk(arena, child, child_clip, under_for_children, alpha, depth + 1, out);
    }
}

/// The single flat color that best stands in for a background.
///
/// Gradients are averaged rather than skipped: a gradient-backed panel that
/// paints nothing would read as a hole in the layout, which is worse than a
/// panel in roughly the right color. `None` means "draw nothing here".
fn background_color(background: &Background) -> Option<Color> {
    let stops = match background {
        Background::Color(c) => return (c.a > 0).then_some(*c),
        Background::LinearGradient(g) => &g.stops,
        Background::RadialGradient(g) => &g.stops,
    };
    if stops.is_empty() {
        return None;
    }
    let n = stops.len() as u32;
    let mut acc = [0u32; 4];
    for stop in stops.iter() {
        acc[0] += stop.color.r as u32;
        acc[1] += stop.color.g as u32;
        acc[2] += stop.color.b as u32;
        acc[3] += stop.color.a as u32;
    }
    let mean = Color {
        r: (acc[0] / n) as u8,
        g: (acc[1] / n) as u8,
        b: (acc[2] / n) as u8,
        a: (acc[3] / n) as u8,
    };
    (mean.a > 0).then_some(mean)
}

/// `src` over `under`, with `src`'s own alpha scaled by `alpha`.
///
/// The result is always fully opaque, because `under` always is: the walk
/// starts from an opaque backdrop and every composite preserves that. This is
/// what lets the painter blit without blending.
fn composite(src: Color, under: Color, alpha: f32) -> Color {
    let a = (src.a as f32 / 255.0) * alpha.clamp(0.0, 1.0);
    let mix = |s: u8, u: u8| -> u8 {
        let v = s as f32 * a + u as f32 * (1.0 - a);
        v.round().clamp(0.0, 255.0) as u8
    };
    Color { r: mix(src.r, under.r), g: mix(src.g, under.g), b: mix(src.b, under.b), a: 255 }
}

/// The color the window should be cleared to before any command runs.
///
/// Prefers the root element's own background so the splash and the real first
/// frame agree, and falls back to the caller's value when the root is
/// transparent (which would otherwise show whatever the compositor had there).
pub fn backdrop_for(arena: &NodeArena, root: NodeId, fallback: Color) -> Color {
    arena
        .get(root)
        .and_then(|el: &Element| background_color(&el.computed_style.background))
        .filter(|c| c.a == 255)
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unshit_core::element::{Element, LayoutRect, Tag};
    use unshit_core::style::types::{GradientStop, GradientStopPosition, LinearGradient};

    const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 255 };
    const WHITE: Color = Color { r: 255, g: 255, b: 255, a: 255 };

    /// Build a node with a solid background at the given rect.
    fn node(rect: (f32, f32, f32, f32), bg: Option<Color>) -> Element {
        let mut el = Element::new(Tag::Div);
        el.layout_rect = LayoutRect { x: rect.0, y: rect.1, width: rect.2, height: rect.3 };
        if let Some(c) = bg {
            el.computed_style.background = Background::Color(c);
        }
        el
    }

    /// Wire `child` under `parent` in `arena`, appending to the child list.
    fn attach(arena: &mut NodeArena, parent: NodeId, child: NodeId) {
        let last = arena.get(parent).map(|p| p.last_child).unwrap_or(NodeId::DANGLING);
        if last.is_dangling() {
            if let Some(p) = arena.get_mut(parent) {
                p.first_child = child;
                p.last_child = child;
            }
        } else {
            if let Some(prev) = arena.get_mut(last) {
                prev.next_sibling = child;
            }
            if let Some(c) = arena.get_mut(child) {
                c.prev_sibling = last;
            }
            if let Some(p) = arena.get_mut(parent) {
                p.last_child = child;
            }
        }
        if let Some(c) = arena.get_mut(child) {
            c.parent = parent;
        }
    }

    fn fills(commands: &[SplashCommand]) -> Vec<(SplashRect, Color)> {
        commands
            .iter()
            .filter_map(|c| match c {
                SplashCommand::Fill { rect, color } => Some((*rect, *color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn emits_parent_before_child_so_the_painter_can_blit_in_order() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 100.0, 100.0), Some(BLACK)));
        let child = arena.alloc(node((10.0, 10.0, 20.0, 20.0), Some(WHITE)));
        attach(&mut arena, root, child);

        let out = collect(&arena, root, (100, 100), BLACK);
        let f = fills(&out);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].0, SplashRect { x: 0, y: 0, width: 100, height: 100 });
        assert_eq!(f[1].0, SplashRect { x: 10, y: 10, width: 20, height: 20 });
    }

    #[test]
    fn a_translucent_fill_is_composited_against_what_is_behind_it() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 10.0, 10.0), Some(BLACK)));
        let half = Color { r: 255, g: 255, b: 255, a: 128 };
        let child = arena.alloc(node((0.0, 0.0, 10.0, 10.0), Some(half)));
        attach(&mut arena, root, child);

        let f = fills(&collect(&arena, root, (10, 10), BLACK));
        // 128/255 of white over black; opaque, because the painter cannot blend.
        assert_eq!(f[1].1.a, 255);
        assert!((f[1].1.r as i32 - 128).abs() <= 1, "got {}", f[1].1.r);
    }

    #[test]
    fn the_surface_bounds_clip_a_tree_laid_out_for_a_bigger_window() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 400.0, 400.0), Some(BLACK)));

        let f = fills(&collect(&arena, root, (100, 50), BLACK));
        assert_eq!(f[0].0, SplashRect { x: 0, y: 0, width: 100, height: 50 });
    }

    #[test]
    fn overflow_visible_lets_a_child_paint_outside_its_parent() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 100.0, 100.0), Some(BLACK)));
        let mut clipper = node((0.0, 0.0, 10.0, 10.0), None);
        clipper.computed_style.overflow_x = Overflow::Visible;
        clipper.computed_style.overflow_y = Overflow::Visible;
        let clipper = arena.alloc(clipper);
        let child = arena.alloc(node((50.0, 50.0, 20.0, 20.0), Some(WHITE)));
        attach(&mut arena, root, clipper);
        attach(&mut arena, clipper, child);

        let f = fills(&collect(&arena, root, (100, 100), BLACK));
        assert_eq!(f.len(), 2);
        assert_eq!(f[1].0, SplashRect { x: 50, y: 50, width: 20, height: 20 });
    }

    #[test]
    fn overflow_hidden_clips_a_child_to_its_parent() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 100.0, 100.0), Some(BLACK)));
        let mut clipper = node((0.0, 0.0, 40.0, 40.0), None);
        clipper.computed_style.overflow_y = Overflow::Hidden;
        let clipper = arena.alloc(clipper);
        let child = arena.alloc(node((20.0, 20.0, 100.0, 100.0), Some(WHITE)));
        attach(&mut arena, root, clipper);
        attach(&mut arena, clipper, child);

        let f = fills(&collect(&arena, root, (100, 100), BLACK));
        assert_eq!(f[1].0, SplashRect { x: 20, y: 20, width: 20, height: 20 });
    }

    #[test]
    fn display_none_removes_the_node_and_everything_under_it() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 100.0, 100.0), Some(BLACK)));
        let mut hidden = node((0.0, 0.0, 50.0, 50.0), Some(WHITE));
        hidden.computed_style.display = Display::None;
        let hidden = arena.alloc(hidden);
        let child = arena.alloc(node((0.0, 0.0, 10.0, 10.0), Some(WHITE)));
        attach(&mut arena, root, hidden);
        attach(&mut arena, hidden, child);

        assert_eq!(fills(&collect(&arena, root, (100, 100), BLACK)).len(), 1);
    }

    #[test]
    fn a_fully_transparent_subtree_costs_nothing() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 100.0, 100.0), Some(BLACK)));
        let mut faded = node((0.0, 0.0, 50.0, 50.0), Some(WHITE));
        faded.computed_style.opacity = 0.0;
        let faded = arena.alloc(faded);
        let child = arena.alloc(node((0.0, 0.0, 10.0, 10.0), Some(WHITE)));
        attach(&mut arena, root, faded);
        attach(&mut arena, faded, child);

        assert_eq!(fills(&collect(&arena, root, (100, 100), BLACK)).len(), 1);
    }

    #[test]
    fn opacity_multiplies_down_the_tree() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 10.0, 10.0), Some(BLACK)));
        let mut group = node((0.0, 0.0, 10.0, 10.0), None);
        group.computed_style.opacity = 0.5;
        let group = arena.alloc(group);
        let mut child = node((0.0, 0.0, 10.0, 10.0), Some(WHITE));
        child.computed_style.opacity = 0.5;
        let child = arena.alloc(child);
        attach(&mut arena, root, group);
        attach(&mut arena, group, child);

        let f = fills(&collect(&arena, root, (10, 10), BLACK));
        // 0.5 * 0.5 of white over black.
        assert!((f[1].1.r as i32 - 64).abs() <= 1, "got {}", f[1].1.r);
    }

    #[test]
    fn a_gradient_becomes_its_mean_color_rather_than_a_hole() {
        let mut arena = NodeArena::new();
        let mut root = node((0.0, 0.0, 10.0, 10.0), None);
        root.computed_style.background = Background::LinearGradient(LinearGradient {
            angle_deg: 0.0,
            stops: [(BLACK, 0.0), (WHITE, 1.0)]
                .into_iter()
                .map(|(color, at)| GradientStop {
                    color,
                    position: GradientStopPosition::Percent(at),
                })
                .collect(),
            repeating: false,
        });
        let root = arena.alloc(root);

        let f = fills(&collect(&arena, root, (10, 10), BLACK));
        assert_eq!(f.len(), 1);
        assert!((f[0].1.r as i32 - 127).abs() <= 1, "got {}", f[0].1.r);
    }

    #[test]
    fn text_is_emitted_with_the_color_it_sits_on() {
        let mut arena = NodeArena::new();
        let mut root = node((0.0, 0.0, 100.0, 20.0), Some(BLACK));
        root.content = ElementContent::Text("main".into());
        root.computed_style.color = WHITE;
        root.computed_style.font_size = 13.0;
        let root = arena.alloc(root);

        let out = collect(&arena, root, (100, 20), BLACK);
        let text = out
            .iter()
            .find_map(|c| match c {
                SplashCommand::Text { text, color, font_size, .. } => {
                    Some((text.clone(), *color, *font_size))
                }
                _ => None,
            })
            .expect("expected a text command");
        assert_eq!(text.0, "main");
        assert_eq!(text.1, WHITE);
        assert_eq!(text.2, 13.0);
    }

    #[test]
    fn whitespace_only_text_is_not_worth_a_command() {
        let mut arena = NodeArena::new();
        let mut root = node((0.0, 0.0, 100.0, 20.0), Some(BLACK));
        root.content = ElementContent::Text("   \n\t".into());
        let root = arena.alloc(root);

        let out = collect(&arena, root, (100, 20), BLACK);
        assert!(out.iter().all(|c| matches!(c, SplashCommand::Fill { .. })));
    }

    #[test]
    fn adjacent_boxes_share_an_edge_instead_of_leaving_a_seam() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 100.0, 100.0), Some(BLACK)));
        // Fractional layout: 0..30.4 and 30.4..60.8 must meet exactly.
        let left = arena.alloc(node((0.0, 0.0, 30.4, 10.0), Some(WHITE)));
        let right = arena.alloc(node((30.4, 0.0, 30.4, 10.0), Some(WHITE)));
        attach(&mut arena, root, left);
        attach(&mut arena, root, right);

        let f = fills(&collect(&arena, root, (100, 100), BLACK));
        assert_eq!(f[1].0.x + f[1].0.width, f[2].0.x);
    }

    #[test]
    fn backdrop_falls_back_when_the_root_is_not_opaque() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 10.0, 10.0), None));
        assert_eq!(backdrop_for(&arena, root, WHITE), WHITE);

        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 10.0, 10.0), Some(BLACK)));
        assert_eq!(backdrop_for(&arena, root, WHITE), BLACK);
    }

    #[test]
    fn a_cycle_cannot_spin_the_walk_forever() {
        let mut arena = NodeArena::new();
        let root = arena.alloc(node((0.0, 0.0, 10.0, 10.0), Some(BLACK)));
        let child = arena.alloc(node((0.0, 0.0, 10.0, 10.0), Some(WHITE)));
        attach(&mut arena, root, child);
        // Point the child back at the root: a build bug, not a real tree.
        if let Some(c) = arena.get_mut(child) {
            c.first_child = root;
            c.last_child = root;
        }

        let out = collect(&arena, root, (10, 10), BLACK);
        assert!(out.len() <= MAX_DEPTH as usize + 2);
    }
}
