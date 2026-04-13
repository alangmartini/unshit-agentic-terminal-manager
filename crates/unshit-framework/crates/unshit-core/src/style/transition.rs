use smallvec::SmallVec;
use std::time::{Duration, Instant};

use crate::id::NodeId;
use crate::style::types::*;

// ---------------------------------------------------------------------------
// Easing / Timing functions
// ---------------------------------------------------------------------------

/// Easing function for transitions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimingFunction {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Custom cubic-bezier(x1, y1, x2, y2).
    CubicBezier(f32, f32, f32, f32),
}

impl TimingFunction {
    /// Evaluate the easing curve at linear progress `t` in [0, 1].
    pub fn evaluate(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            TimingFunction::Linear => t,
            TimingFunction::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0, t),
            TimingFunction::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
            TimingFunction::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
            TimingFunction::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
            TimingFunction::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
        }
    }
}

/// Evaluate a cubic-bezier curve at linear progress `t`.
///
/// The curve is defined by control points (0,0), (x1,y1), (x2,y2), (1,1).
/// We need to find the parameter `s` such that `bezier_x(s) = t`, then
/// return `bezier_y(s)`.
///
/// Uses Newton-Raphson iteration to solve for `s`, converging in roughly
/// 8 iterations to 1e-7 precision.
fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }

    // For linear curves, skip the solve.
    if (x1 - y1).abs() < 1e-6 && (x2 - y2).abs() < 1e-6 {
        return t;
    }

    // Newton-Raphson to find s where bezier_x(s) = t.
    let mut s = t; // initial guess
    for _ in 0..8 {
        let x = sample_bezier(x1, x2, s) - t;
        let dx = sample_bezier_derivative(x1, x2, s);
        if dx.abs() < 1e-10 {
            break;
        }
        s -= x / dx;
        s = s.clamp(0.0, 1.0);
    }

    // If Newton did not converge, fall back to bisection.
    let x_at_s = sample_bezier(x1, x2, s);
    if (x_at_s - t).abs() > 1e-5 {
        let mut lo = 0.0_f32;
        let mut hi = 1.0_f32;
        s = t;
        for _ in 0..20 {
            let x = sample_bezier(x1, x2, s);
            if (x - t).abs() < 1e-7 {
                break;
            }
            if x < t {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) * 0.5;
        }
    }

    sample_bezier(y1, y2, s)
}

/// Sample a 1D cubic bezier with control points 0, p1, p2, 1 at parameter s.
#[inline]
fn sample_bezier(p1: f32, p2: f32, s: f32) -> f32 {
    // B(s) = 3(1-s)^2 s p1 + 3(1-s) s^2 p2 + s^3
    let inv = 1.0 - s;
    3.0 * inv * inv * s * p1 + 3.0 * inv * s * s * p2 + s * s * s
}

/// Derivative of the 1D cubic bezier at parameter s.
#[inline]
fn sample_bezier_derivative(p1: f32, p2: f32, s: f32) -> f32 {
    let inv = 1.0 - s;
    3.0 * inv * inv * p1 + 6.0 * inv * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
}

// ---------------------------------------------------------------------------
// Animatable property identifiers
// ---------------------------------------------------------------------------

/// Which CSS property (or all) to transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransitionProperty {
    All,
    Opacity,
    Background,
    Color,
    BorderColor,
    BorderWidth,
    BorderRadius,
    Padding,
    Margin,
    Width,
    Height,
    Gap,
    FontSize,
    OutlineColor,
    OutlineWidth,
    BoxShadow,
    LetterSpacing,
    LineHeight,
    MinWidth,
    MaxWidth,
    MinHeight,
    MaxHeight,
}

impl TransitionProperty {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "all" => Some(Self::All),
            "opacity" => Some(Self::Opacity),
            "background" | "background-color" => Some(Self::Background),
            "color" => Some(Self::Color),
            "border-color" => Some(Self::BorderColor),
            "border-width" => Some(Self::BorderWidth),
            "border-radius" => Some(Self::BorderRadius),
            "padding" => Some(Self::Padding),
            "margin" => Some(Self::Margin),
            "width" => Some(Self::Width),
            "height" => Some(Self::Height),
            "gap" => Some(Self::Gap),
            "font-size" => Some(Self::FontSize),
            "outline-color" => Some(Self::OutlineColor),
            "outline-width" => Some(Self::OutlineWidth),
            "box-shadow" => Some(Self::BoxShadow),
            "letter-spacing" => Some(Self::LetterSpacing),
            "line-height" => Some(Self::LineHeight),
            "min-width" => Some(Self::MinWidth),
            "max-width" => Some(Self::MaxWidth),
            "min-height" => Some(Self::MinHeight),
            "max-height" => Some(Self::MaxHeight),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Transition definitions (parsed from CSS)
// ---------------------------------------------------------------------------

/// Definition of a transition for a single property, parsed from CSS.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionDef {
    pub property: TransitionProperty,
    pub duration: Duration,
    pub timing_function: TimingFunction,
    pub delay: Duration,
}

// ---------------------------------------------------------------------------
// Animatable values and interpolation
// ---------------------------------------------------------------------------

/// A snapshot of a property value that can be interpolated.
#[derive(Clone, Debug)]
pub enum AnimatableValue {
    Float(f32),
    Color(Color),
    Edges(Edges),
    Corners(Corners),
    Dimension(Dimension),
    Background(Background),
    BoxShadow(SmallVec<[BoxShadow; 2]>),
}

impl AnimatableValue {
    /// Linearly interpolate between `self` and `other` at `t` in [0, 1].
    pub fn lerp(&self, other: &AnimatableValue, t: f32) -> AnimatableValue {
        match (self, other) {
            (AnimatableValue::Float(a), AnimatableValue::Float(b)) => {
                AnimatableValue::Float(lerp_f32(*a, *b, t))
            }
            (AnimatableValue::Color(a), AnimatableValue::Color(b)) => {
                AnimatableValue::Color(lerp_color_oklab(*a, *b, t))
            }
            (AnimatableValue::Edges(a), AnimatableValue::Edges(b)) => {
                AnimatableValue::Edges(lerp_edges(a, b, t))
            }
            (AnimatableValue::Corners(a), AnimatableValue::Corners(b)) => {
                AnimatableValue::Corners(lerp_corners(a, b, t))
            }
            (AnimatableValue::Dimension(a), AnimatableValue::Dimension(b)) => {
                AnimatableValue::Dimension(lerp_dimension(a, b, t))
            }
            (AnimatableValue::Background(a), AnimatableValue::Background(b)) => {
                AnimatableValue::Background(lerp_background(a, b, t))
            }
            (AnimatableValue::BoxShadow(a), AnimatableValue::BoxShadow(b)) => {
                AnimatableValue::BoxShadow(lerp_box_shadow_list(a, b, t))
            }
            // Mismatched types: snap to `other` at t >= 0.5.
            _ => {
                if t >= 0.5 {
                    other.clone()
                } else {
                    self.clone()
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Interpolation helpers
// ---------------------------------------------------------------------------

#[inline]
pub fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_edges(a: &Edges, b: &Edges, t: f32) -> Edges {
    Edges {
        top: lerp_f32(a.top, b.top, t),
        right: lerp_f32(a.right, b.right, t),
        bottom: lerp_f32(a.bottom, b.bottom, t),
        left: lerp_f32(a.left, b.left, t),
    }
}

fn lerp_corners(a: &Corners, b: &Corners, t: f32) -> Corners {
    Corners {
        top_left: lerp_f32(a.top_left, b.top_left, t),
        top_right: lerp_f32(a.top_right, b.top_right, t),
        bottom_right: lerp_f32(a.bottom_right, b.bottom_right, t),
        bottom_left: lerp_f32(a.bottom_left, b.bottom_left, t),
    }
}

fn lerp_dimension(a: &Dimension, b: &Dimension, t: f32) -> Dimension {
    match (a, b) {
        (Dimension::Px(a_v), Dimension::Px(b_v)) => Dimension::Px(lerp_f32(*a_v, *b_v, t)),
        (Dimension::Percent(a_v), Dimension::Percent(b_v)) => {
            Dimension::Percent(lerp_f32(*a_v, *b_v, t))
        }
        // Different units or Auto: snap.
        _ => {
            if t >= 0.5 {
                *b
            } else {
                *a
            }
        }
    }
}

fn lerp_background(a: &Background, b: &Background, t: f32) -> Background {
    match (a, b) {
        (Background::Color(ca), Background::Color(cb)) => {
            Background::Color(lerp_color_oklab(*ca, *cb, t))
        }
        // Mixed background types (or any gradient): snap. Full gradient
        // interpolation is out of scope; CSS does not define gradient to
        // gradient or gradient to color interpolation in a useful way.
        _ => {
            if t >= 0.5 {
                b.clone()
            } else {
                a.clone()
            }
        }
    }
}

fn lerp_shadow(sa: &BoxShadow, sb: &BoxShadow, t: f32) -> BoxShadow {
    BoxShadow {
        offset_x: lerp_f32(sa.offset_x, sb.offset_x, t),
        offset_y: lerp_f32(sa.offset_y, sb.offset_y, t),
        blur_radius: lerp_f32(sa.blur_radius, sb.blur_radius, t),
        spread_radius: lerp_f32(sa.spread_radius, sb.spread_radius, t),
        color: lerp_color_oklab(sa.color, sb.color, t),
        // `inset` cannot be interpolated, snap using t.
        inset: if t >= 0.5 { sb.inset } else { sa.inset },
    }
}

fn transparent_shadow(inset: bool) -> BoxShadow {
    BoxShadow {
        offset_x: 0.0,
        offset_y: 0.0,
        blur_radius: 0.0,
        spread_radius: 0.0,
        color: Color::TRANSPARENT,
        inset,
    }
}

fn lerp_box_shadow_list(
    a: &SmallVec<[BoxShadow; 2]>,
    b: &SmallVec<[BoxShadow; 2]>,
    t: f32,
) -> SmallVec<[BoxShadow; 2]> {
    let len = a.len().max(b.len());
    let mut out: SmallVec<[BoxShadow; 2]> = SmallVec::with_capacity(len);
    for i in 0..len {
        let sa = a.get(i);
        let sb = b.get(i);
        let shadow = match (sa, sb) {
            (Some(sa), Some(sb)) => lerp_shadow(sa, sb, t),
            (None, Some(sb)) => lerp_shadow(&transparent_shadow(sb.inset), sb, t),
            (Some(sa), None) => lerp_shadow(sa, &transparent_shadow(sa.inset), t),
            (None, None) => continue,
        };
        out.push(shadow);
    }
    out
}

// ---------------------------------------------------------------------------
// Oklab color interpolation
// ---------------------------------------------------------------------------

/// Interpolate two sRGB colors in Oklab space for perceptually smooth transitions.
pub fn lerp_color_oklab(a: Color, b: Color, t: f32) -> Color {
    let a_oklab = srgb_to_oklab(a);
    let b_oklab = srgb_to_oklab(b);

    let l = lerp_f32(a_oklab[0], b_oklab[0], t);
    let a_ch = lerp_f32(a_oklab[1], b_oklab[1], t);
    let b_ch = lerp_f32(a_oklab[2], b_oklab[2], t);
    let alpha = lerp_f32(a.a as f32 / 255.0, b.a as f32 / 255.0, t);

    oklab_to_srgb(l, a_ch, b_ch, alpha)
}

/// Convert sRGB u8 color to Oklab [L, a, b].
fn srgb_to_oklab(c: Color) -> [f32; 3] {
    // sRGB to linear
    let r = srgb_to_linear(c.r as f32 / 255.0);
    let g = srgb_to_linear(c.g as f32 / 255.0);
    let b = srgb_to_linear(c.b as f32 / 255.0);

    // Linear RGB to LMS (using Oklab M1 matrix)
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;

    // Cube root
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    // LMS to Oklab (M2 matrix)
    let ok_l = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
    let ok_a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
    let ok_b = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_;

    [ok_l, ok_a, ok_b]
}

/// Convert Oklab [L, a, b] + alpha back to sRGB u8 Color.
fn oklab_to_srgb(l: f32, a: f32, b: f32, alpha: f32) -> Color {
    // Oklab to LMS (inverse M2)
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;

    // Cube
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    // LMS to linear RGB (inverse M1)
    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;

    // Linear to sRGB
    let r = linear_to_srgb(r);
    let g = linear_to_srgb(g);
    let b = linear_to_srgb(b);

    Color::rgba(
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
        (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

#[inline]
fn srgb_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
fn linear_to_srgb(x: f32) -> f32 {
    if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

// ---------------------------------------------------------------------------
// Running transitions
// ---------------------------------------------------------------------------

/// State of a currently running transition on a single property.
#[derive(Clone, Debug)]
pub struct RunningTransition {
    pub property: TransitionProperty,
    pub start_value: AnimatableValue,
    pub end_value: AnimatableValue,
    pub start_time: Instant,
    pub duration: Duration,
    pub delay: Duration,
    pub timing_fn: TimingFunction,
}

impl RunningTransition {
    /// Compute the current interpolated value at the given instant.
    /// Returns `None` if we are still in the delay period.
    pub fn sample(&self, now: Instant) -> Option<AnimatableValue> {
        let elapsed = now.duration_since(self.start_time);
        if elapsed < self.delay {
            return Some(self.start_value.clone());
        }

        let active_elapsed = elapsed - self.delay;
        if self.duration.is_zero() {
            return Some(self.end_value.clone());
        }

        let linear_t = (active_elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        let eased_t = self.timing_fn.evaluate(linear_t);
        Some(self.start_value.lerp(&self.end_value, eased_t))
    }

    /// Returns true if this transition has completed.
    pub fn is_complete(&self, now: Instant) -> bool {
        let elapsed = now.duration_since(self.start_time);
        elapsed >= self.delay + self.duration
    }
}

// ---------------------------------------------------------------------------
// Extract animatable value from a computed style
// ---------------------------------------------------------------------------

/// Extract the current animatable value for a given property from a style.
pub fn extract_value(style: &ComputedStyle, prop: TransitionProperty) -> AnimatableValue {
    match prop {
        TransitionProperty::Opacity => AnimatableValue::Float(style.opacity),
        TransitionProperty::Background => AnimatableValue::Background(style.background.clone()),
        TransitionProperty::Color => AnimatableValue::Color(style.color),
        TransitionProperty::BorderColor => AnimatableValue::Color(style.border_color),
        TransitionProperty::BorderWidth => AnimatableValue::Edges(style.border_width),
        TransitionProperty::BorderRadius => AnimatableValue::Corners(style.border_radius),
        TransitionProperty::Padding => AnimatableValue::Edges(style.padding),
        TransitionProperty::Margin => AnimatableValue::Edges(style.margin),
        TransitionProperty::Width => AnimatableValue::Dimension(style.width),
        TransitionProperty::Height => AnimatableValue::Dimension(style.height),
        TransitionProperty::Gap => AnimatableValue::Float(style.row_gap),
        TransitionProperty::FontSize => AnimatableValue::Float(style.font_size),
        TransitionProperty::OutlineColor => AnimatableValue::Color(style.outline_color),
        TransitionProperty::OutlineWidth => AnimatableValue::Float(style.outline_width),
        TransitionProperty::BoxShadow => AnimatableValue::BoxShadow(style.box_shadow.clone()),
        TransitionProperty::LetterSpacing => AnimatableValue::Float(style.letter_spacing),
        TransitionProperty::LineHeight => AnimatableValue::Float(style.line_height),
        TransitionProperty::MinWidth => AnimatableValue::Dimension(style.min_width),
        TransitionProperty::MaxWidth => AnimatableValue::Dimension(style.max_width),
        TransitionProperty::MinHeight => AnimatableValue::Dimension(style.min_height),
        TransitionProperty::MaxHeight => AnimatableValue::Dimension(style.max_height),
        // All is handled by expanding to individual properties.
        TransitionProperty::All => AnimatableValue::Float(0.0),
    }
}

/// Apply an interpolated animatable value back onto a computed style.
pub fn apply_value(style: &mut ComputedStyle, prop: TransitionProperty, value: &AnimatableValue) {
    match (prop, value) {
        (TransitionProperty::Opacity, AnimatableValue::Float(v)) => style.opacity = *v,
        (TransitionProperty::Background, AnimatableValue::Background(v)) => {
            style.background = v.clone();
        }
        (TransitionProperty::Color, AnimatableValue::Color(v)) => style.color = *v,
        (TransitionProperty::BorderColor, AnimatableValue::Color(v)) => style.border_color = *v,
        (TransitionProperty::BorderWidth, AnimatableValue::Edges(v)) => style.border_width = *v,
        (TransitionProperty::BorderRadius, AnimatableValue::Corners(v)) => {
            style.border_radius = *v;
        }
        (TransitionProperty::Padding, AnimatableValue::Edges(v)) => style.padding = *v,
        (TransitionProperty::Margin, AnimatableValue::Edges(v)) => style.margin = *v,
        (TransitionProperty::Width, AnimatableValue::Dimension(v)) => style.width = *v,
        (TransitionProperty::Height, AnimatableValue::Dimension(v)) => style.height = *v,
        (TransitionProperty::Gap, AnimatableValue::Float(v)) => {
            style.row_gap = *v;
            style.column_gap = *v;
        }
        (TransitionProperty::FontSize, AnimatableValue::Float(v)) => style.font_size = *v,
        (TransitionProperty::OutlineColor, AnimatableValue::Color(v)) => style.outline_color = *v,
        (TransitionProperty::OutlineWidth, AnimatableValue::Float(v)) => style.outline_width = *v,
        (TransitionProperty::BoxShadow, AnimatableValue::BoxShadow(v)) => {
            style.box_shadow = v.clone();
        }
        (TransitionProperty::LetterSpacing, AnimatableValue::Float(v)) => {
            style.letter_spacing = *v;
        }
        (TransitionProperty::LineHeight, AnimatableValue::Float(v)) => style.line_height = *v,
        (TransitionProperty::MinWidth, AnimatableValue::Dimension(v)) => style.min_width = *v,
        (TransitionProperty::MaxWidth, AnimatableValue::Dimension(v)) => style.max_width = *v,
        (TransitionProperty::MinHeight, AnimatableValue::Dimension(v)) => style.min_height = *v,
        (TransitionProperty::MaxHeight, AnimatableValue::Dimension(v)) => style.max_height = *v,
        _ => {}
    }
}

/// All individually animatable properties (everything except `All`).
pub const ALL_ANIMATABLE: &[TransitionProperty] = &[
    TransitionProperty::Opacity,
    TransitionProperty::Background,
    TransitionProperty::Color,
    TransitionProperty::BorderColor,
    TransitionProperty::BorderWidth,
    TransitionProperty::BorderRadius,
    TransitionProperty::Padding,
    TransitionProperty::Margin,
    TransitionProperty::Width,
    TransitionProperty::Height,
    TransitionProperty::Gap,
    TransitionProperty::FontSize,
    TransitionProperty::OutlineColor,
    TransitionProperty::OutlineWidth,
    TransitionProperty::BoxShadow,
    TransitionProperty::LetterSpacing,
    TransitionProperty::LineHeight,
    TransitionProperty::MinWidth,
    TransitionProperty::MaxWidth,
    TransitionProperty::MinHeight,
    TransitionProperty::MaxHeight,
];

// ---------------------------------------------------------------------------
// Transition engine: diff styles and manage running transitions
// ---------------------------------------------------------------------------

/// Check if two animatable values are "equal enough" to not warrant a transition.
fn values_equal(a: &AnimatableValue, b: &AnimatableValue) -> bool {
    match (a, b) {
        (AnimatableValue::Float(a), AnimatableValue::Float(b)) => (a - b).abs() < 1e-6,
        (AnimatableValue::Color(a), AnimatableValue::Color(b)) => a == b,
        (AnimatableValue::Edges(a), AnimatableValue::Edges(b)) => a == b,
        (AnimatableValue::Corners(a), AnimatableValue::Corners(b)) => a == b,
        (AnimatableValue::Dimension(a), AnimatableValue::Dimension(b)) => a == b,
        (AnimatableValue::Background(a), AnimatableValue::Background(b)) => a == b,
        (AnimatableValue::BoxShadow(a), AnimatableValue::BoxShadow(b)) => a == b,
        _ => false,
    }
}

/// Given old and new computed styles, plus transition definitions, start new
/// transitions for properties that changed. Existing running transitions for
/// the same property are replaced (the current animated value becomes the new
/// start value for a smooth reversal).
pub fn start_transitions(
    old_style: &ComputedStyle,
    new_style: &ComputedStyle,
    defs: &[TransitionDef],
    running: &mut SmallVec<[RunningTransition; 4]>,
    now: Instant,
) {
    for def in defs {
        let props: SmallVec<[TransitionProperty; 17]> = if def.property == TransitionProperty::All {
            ALL_ANIMATABLE.iter().copied().collect()
        } else {
            smallvec::smallvec![def.property]
        };

        for prop in props {
            let old_val = extract_value(old_style, prop);
            let new_val = extract_value(new_style, prop);

            if values_equal(&old_val, &new_val) {
                continue;
            }

            // If there is already a running transition for this property,
            // use its current animated value as the start (smooth reversal).
            let start_value = if let Some(idx) = running.iter().position(|r| r.property == prop) {
                let current = running[idx].sample(now).unwrap_or(old_val);
                running.remove(idx);
                current
            } else {
                old_val
            };

            running.push(RunningTransition {
                property: prop,
                start_value,
                end_value: new_val,
                start_time: now,
                duration: def.duration,
                delay: def.delay,
                timing_fn: def.timing_function,
            });
        }
    }
}

/// Tick all running transitions: apply interpolated values to the computed style,
/// remove completed transitions. Returns true if any transitions are still active.
pub fn tick_transitions(
    style: &mut ComputedStyle,
    running: &mut SmallVec<[RunningTransition; 4]>,
    now: Instant,
) -> bool {
    let mut i = 0;
    while i < running.len() {
        if running[i].is_complete(now) {
            let t = running.remove(i);
            apply_value(style, t.property, &t.end_value);
            // don't increment i; the next element slid into this slot.
        } else {
            if let Some(val) = running[i].sample(now) {
                apply_value(style, running[i].property, &val);
            }
            i += 1;
        }
    }

    !running.is_empty()
}

/// Global tracker of elements with active transitions so we can tick only them
/// instead of scanning the whole arena.
#[derive(Clone, Debug, Default)]
pub struct ActiveTransitions {
    pub nodes: SmallVec<[NodeId; 8]>,
}

impl ActiveTransitions {
    pub fn has_active(&self) -> bool {
        !self.nodes.is_empty()
    }

    pub fn add(&mut self, id: NodeId) {
        if !self.nodes.contains(&id) {
            self.nodes.push(id);
        }
    }

    pub fn remove(&mut self, id: NodeId) {
        if let Some(pos) = self.nodes.iter().position(|n| *n == id) {
            self.nodes.swap_remove(pos);
        }
    }

    /// Compute the soonest instant at which any running transition needs a new
    /// frame sample, or `None` when no transitions are active.
    ///
    /// For each `RunningTransition` on every tracked node:
    /// - If still in the delay window, wake at `start_time + delay`.
    /// - If active (past the delay), wake one frame budget (16 ms) from `now`
    ///   so the animation progresses at roughly 60 fps without busy-polling.
    ///
    /// This feeds into the unified `ControlFlow::WaitUntil` calculation so
    /// the event loop sleeps between frames instead of spinning.
    pub fn next_wake(
        arena: &crate::tree::NodeArena,
        active: &ActiveTransitions,
        now: Instant,
    ) -> Option<Instant> {
        if active.nodes.is_empty() {
            return None;
        }

        let frame_budget = Duration::from_millis(16);
        let mut out: Option<Instant> = None;

        for &node_id in &active.nodes {
            let Some(el) = arena.get(node_id) else {
                continue;
            };
            for tr in &el.running_transitions {
                let elapsed = now.duration_since(tr.start_time);
                let wake = if elapsed < tr.delay {
                    // Still in the delay phase: wake exactly when the delay expires.
                    tr.start_time + tr.delay
                } else {
                    // Active phase: schedule one frame ahead.
                    now + frame_budget
                };
                out = Some(match out {
                    Some(current) if current <= wake => current,
                    _ => wake,
                });
            }
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_easing_boundaries() {
        let tf = TimingFunction::Linear;
        assert!((tf.evaluate(0.0) - 0.0).abs() < 1e-6);
        assert!((tf.evaluate(0.5) - 0.5).abs() < 1e-6);
        assert!((tf.evaluate(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_linear_easing_clamping() {
        let tf = TimingFunction::Linear;
        assert!((tf.evaluate(-0.5) - 0.0).abs() < 1e-6);
        assert!((tf.evaluate(1.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_ease_endpoints() {
        let tf = TimingFunction::Ease;
        assert!((tf.evaluate(0.0) - 0.0).abs() < 1e-4);
        assert!((tf.evaluate(1.0) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_ease_in_slow_start() {
        let tf = TimingFunction::EaseIn;
        // At t=0.25, ease-in should be less than 0.25 (slower start).
        let val = tf.evaluate(0.25);
        assert!(val < 0.25, "ease-in at 0.25 should be < 0.25, got {}", val);
    }

    #[test]
    fn test_ease_out_fast_start() {
        let tf = TimingFunction::EaseOut;
        // At t=0.25, ease-out should be more than 0.25 (faster start).
        let val = tf.evaluate(0.25);
        assert!(val > 0.25, "ease-out at 0.25 should be > 0.25, got {}", val);
    }

    #[test]
    fn test_ease_in_out_symmetry() {
        let tf = TimingFunction::EaseInOut;
        let at_half = tf.evaluate(0.5);
        assert!(
            (at_half - 0.5).abs() < 0.05,
            "ease-in-out at 0.5 should be near 0.5, got {}",
            at_half
        );
    }

    #[test]
    fn test_custom_cubic_bezier() {
        // cubic-bezier(0.0, 0.0, 1.0, 1.0) is equivalent to linear.
        let tf = TimingFunction::CubicBezier(0.0, 0.0, 1.0, 1.0);
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let val = tf.evaluate(t);
            assert!(
                (val - t).abs() < 0.02,
                "linear cubic-bezier at {} should be ~{}, got {}",
                t,
                t,
                val
            );
        }
    }

    #[test]
    fn test_cubic_bezier_monotonic_for_standard_curves() {
        // Ease should be monotonically increasing.
        let tf = TimingFunction::Ease;
        let mut prev = 0.0;
        for i in 1..=100 {
            let t = i as f32 / 100.0;
            let val = tf.evaluate(t);
            assert!(val >= prev - 1e-6, "ease not monotonic at t={}: {} < {}", t, val, prev);
            prev = val;
        }
    }

    #[test]
    fn test_lerp_f32_basics() {
        assert!((lerp_f32(0.0, 1.0, 0.0) - 0.0).abs() < 1e-6);
        assert!((lerp_f32(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
        assert!((lerp_f32(0.0, 1.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((lerp_f32(10.0, 20.0, 0.3) - 13.0).abs() < 1e-6);
    }

    #[test]
    fn test_lerp_color_oklab_same_color() {
        let c = Color::rgb(100, 150, 200);
        let result = lerp_color_oklab(c, c, 0.5);
        // Should be approximately the same color.
        assert!((result.r as i32 - 100).abs() <= 1);
        assert!((result.g as i32 - 150).abs() <= 1);
        assert!((result.b as i32 - 200).abs() <= 1);
    }

    #[test]
    fn test_lerp_color_oklab_endpoints() {
        let black = Color::rgb(0, 0, 0);
        let white = Color::rgb(255, 255, 255);

        let at_0 = lerp_color_oklab(black, white, 0.0);
        assert_eq!(at_0.r, 0);
        assert_eq!(at_0.g, 0);
        assert_eq!(at_0.b, 0);

        let at_1 = lerp_color_oklab(black, white, 1.0);
        assert_eq!(at_1.r, 255);
        assert_eq!(at_1.g, 255);
        assert_eq!(at_1.b, 255);
    }

    #[test]
    fn test_lerp_color_oklab_midpoint_not_muddy() {
        // In sRGB, red + cyan midpoint is gray. In Oklab, it should be lighter.
        let red = Color::rgb(255, 0, 0);
        let cyan = Color::rgb(0, 255, 255);
        let mid = lerp_color_oklab(red, cyan, 0.5);
        // The midpoint should have reasonable brightness (not dark gray).
        let brightness = mid.r as f32 * 0.299 + mid.g as f32 * 0.587 + mid.b as f32 * 0.114;
        assert!(
            brightness > 80.0,
            "Oklab midpoint should not be dark; brightness = {}",
            brightness
        );
    }

    #[test]
    fn test_lerp_dimension_same_unit() {
        let a = Dimension::Px(10.0);
        let b = Dimension::Px(30.0);
        let result = lerp_dimension(&a, &b, 0.5);
        assert_eq!(result, Dimension::Px(20.0));
    }

    #[test]
    fn test_lerp_dimension_different_units_snaps() {
        let a = Dimension::Px(10.0);
        let b = Dimension::Percent(50.0);
        let result = lerp_dimension(&a, &b, 0.6);
        assert_eq!(result, Dimension::Percent(50.0)); // t >= 0.5, snaps to b.
    }

    #[test]
    fn test_lerp_edges() {
        let a = Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 };
        let b = Edges { top: 10.0, right: 20.0, bottom: 30.0, left: 40.0 };
        let mid = lerp_edges(&a, &b, 0.5);
        assert!((mid.top - 5.0).abs() < 1e-6);
        assert!((mid.right - 10.0).abs() < 1e-6);
        assert!((mid.bottom - 15.0).abs() < 1e-6);
        assert!((mid.left - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_running_transition_sample() {
        let now = Instant::now();
        let t = RunningTransition {
            property: TransitionProperty::Opacity,
            start_value: AnimatableValue::Float(0.0),
            end_value: AnimatableValue::Float(1.0),
            start_time: now,
            duration: Duration::from_millis(1000),
            delay: Duration::ZERO,
            timing_fn: TimingFunction::Linear,
        };

        // At start.
        let val = t.sample(now).unwrap();
        if let AnimatableValue::Float(v) = val {
            assert!((v - 0.0).abs() < 1e-6);
        } else {
            panic!("expected Float");
        }

        // Halfway through.
        let mid = now + Duration::from_millis(500);
        let val = t.sample(mid).unwrap();
        if let AnimatableValue::Float(v) = val {
            assert!((v - 0.5).abs() < 1e-3);
        } else {
            panic!("expected Float");
        }

        // At end.
        let end = now + Duration::from_millis(1000);
        let val = t.sample(end).unwrap();
        if let AnimatableValue::Float(v) = val {
            assert!((v - 1.0).abs() < 1e-6);
        } else {
            panic!("expected Float");
        }
    }

    #[test]
    fn test_running_transition_with_delay() {
        let now = Instant::now();
        let t = RunningTransition {
            property: TransitionProperty::Opacity,
            start_value: AnimatableValue::Float(0.0),
            end_value: AnimatableValue::Float(1.0),
            start_time: now,
            duration: Duration::from_millis(500),
            delay: Duration::from_millis(200),
            timing_fn: TimingFunction::Linear,
        };

        // During delay: should return start value.
        let val = t.sample(now + Duration::from_millis(100)).unwrap();
        if let AnimatableValue::Float(v) = val {
            assert!((v - 0.0).abs() < 1e-6);
        } else {
            panic!("expected Float");
        }

        // At delay + half duration = 200 + 250 = 450ms.
        let val = t.sample(now + Duration::from_millis(450)).unwrap();
        if let AnimatableValue::Float(v) = val {
            assert!((v - 0.5).abs() < 1e-3);
        } else {
            panic!("expected Float");
        }

        // Complete at 200 + 500 = 700ms.
        assert!(t.is_complete(now + Duration::from_millis(700)));
        assert!(!t.is_complete(now + Duration::from_millis(699)));
    }

    #[test]
    fn test_transition_property_from_str() {
        assert_eq!(TransitionProperty::from_str("all"), Some(TransitionProperty::All));
        assert_eq!(TransitionProperty::from_str("opacity"), Some(TransitionProperty::Opacity));
        assert_eq!(
            TransitionProperty::from_str("background-color"),
            Some(TransitionProperty::Background)
        );
        assert_eq!(TransitionProperty::from_str("nonsense"), None);
    }

    #[test]
    fn test_start_transitions_creates_transition() {
        let old = ComputedStyle::default();
        let mut new = ComputedStyle::default();
        new.opacity = 0.5;

        let defs = vec![TransitionDef {
            property: TransitionProperty::Opacity,
            duration: Duration::from_millis(300),
            timing_function: TimingFunction::Ease,
            delay: Duration::ZERO,
        }];

        let now = Instant::now();
        let mut running = SmallVec::new();
        start_transitions(&old, &new, &defs, &mut running, now);

        assert_eq!(running.len(), 1);
        assert_eq!(running[0].property, TransitionProperty::Opacity);
    }

    #[test]
    fn test_start_transitions_no_change_no_transition() {
        let style = ComputedStyle::default();

        let defs = vec![TransitionDef {
            property: TransitionProperty::Opacity,
            duration: Duration::from_millis(300),
            timing_function: TimingFunction::Ease,
            delay: Duration::ZERO,
        }];

        let now = Instant::now();
        let mut running = SmallVec::new();
        start_transitions(&style, &style, &defs, &mut running, now);

        assert_eq!(running.len(), 0);
    }

    #[test]
    fn test_start_transitions_all_expands() {
        let old = ComputedStyle::default();
        let mut new = ComputedStyle::default();
        new.opacity = 0.5;
        new.row_gap = 10.0;
        new.column_gap = 10.0;

        let defs = vec![TransitionDef {
            property: TransitionProperty::All,
            duration: Duration::from_millis(300),
            timing_function: TimingFunction::Ease,
            delay: Duration::ZERO,
        }];

        let now = Instant::now();
        let mut running = SmallVec::new();
        start_transitions(&old, &new, &defs, &mut running, now);

        // Should create transitions for the two changed properties.
        assert_eq!(running.len(), 2);
        let props: Vec<_> = running.iter().map(|r| r.property).collect();
        assert!(props.contains(&TransitionProperty::Opacity));
        assert!(props.contains(&TransitionProperty::Gap));
    }

    #[test]
    fn test_tick_transitions_applies_and_removes() {
        let now = Instant::now();
        let mut style = ComputedStyle::default();
        style.opacity = 1.0;

        let mut running: SmallVec<[RunningTransition; 4]> =
            smallvec::smallvec![RunningTransition {
                property: TransitionProperty::Opacity,
                start_value: AnimatableValue::Float(1.0),
                end_value: AnimatableValue::Float(0.0),
                start_time: now,
                duration: Duration::from_millis(100),
                delay: Duration::ZERO,
                timing_fn: TimingFunction::Linear,
            }];

        // Mid-transition.
        let mid = now + Duration::from_millis(50);
        let still_active = tick_transitions(&mut style, &mut running, mid);
        assert!(still_active);
        assert!((style.opacity - 0.5).abs() < 0.05);

        // After completion.
        let after = now + Duration::from_millis(200);
        let still_active = tick_transitions(&mut style, &mut running, after);
        assert!(!still_active);
        assert!((style.opacity - 0.0).abs() < 1e-6);
        assert!(running.is_empty());
    }

    #[test]
    fn test_smooth_reversal() {
        let now = Instant::now();

        let old = ComputedStyle::default(); // opacity = 1.0
        let mut new = ComputedStyle::default();
        new.opacity = 0.0;

        let defs = vec![TransitionDef {
            property: TransitionProperty::Opacity,
            duration: Duration::from_millis(1000),
            timing_function: TimingFunction::Linear,
            delay: Duration::ZERO,
        }];

        let mut running = SmallVec::new();
        start_transitions(&old, &new, &defs, &mut running, now);
        assert_eq!(running.len(), 1);

        // Advance to 500ms: opacity should be ~0.5.
        let mid = now + Duration::from_millis(500);
        let mut style = new.clone();
        tick_transitions(&mut style, &mut running, mid);

        // Now reverse: new target is back to opacity 1.0.
        let reversed_target = ComputedStyle::default(); // opacity 1.0
        start_transitions(&new, &reversed_target, &defs, &mut running, mid);

        // The new transition should start from ~0.5 (current animated value).
        assert_eq!(running.len(), 1);
        if let AnimatableValue::Float(v) = &running[0].start_value {
            assert!(
                (*v - 0.5).abs() < 0.05,
                "reversed transition should start near 0.5, got {}",
                v
            );
        } else {
            panic!("expected Float start_value");
        }
    }

    // ---------------------------------------------------------------------------
    // ActiveTransitions::next_wake tests
    // ---------------------------------------------------------------------------

    fn make_arena_with_transition(tr: RunningTransition) -> (crate::tree::NodeArena, NodeId) {
        use crate::element::Element;
        use crate::element::Tag;
        let mut arena = crate::tree::NodeArena::new();
        let mut el = Element::new(Tag::Div);
        el.running_transitions.push(tr);
        let id = arena.alloc(el);
        (arena, id)
    }

    #[test]
    fn test_active_transitions_next_wake_none_when_empty() {
        let arena = crate::tree::NodeArena::new();
        let active = ActiveTransitions::default();
        let now = Instant::now();
        assert!(ActiveTransitions::next_wake(&arena, &active, now).is_none());
    }

    #[test]
    fn test_active_transitions_next_wake_active_phase_returns_near_future() {
        // Transition already past delay: next_wake should return ~16 ms from now.
        let now = Instant::now();
        let tr = RunningTransition {
            property: TransitionProperty::Opacity,
            start_value: AnimatableValue::Float(0.0),
            end_value: AnimatableValue::Float(1.0),
            start_time: now - Duration::from_millis(50),
            duration: Duration::from_millis(300),
            delay: Duration::ZERO,
            timing_fn: TimingFunction::Linear,
        };
        let (arena, id) = make_arena_with_transition(tr);
        let mut active = ActiveTransitions::default();
        active.add(id);

        let wake = ActiveTransitions::next_wake(&arena, &active, now)
            .expect("should return Some during active phase");
        let diff = wake.duration_since(now);
        // Should be roughly one frame budget (16 ms), give 1 ms tolerance.
        assert!(
            diff.as_millis() >= 15 && diff.as_millis() <= 17,
            "expected ~16ms wake, got {}ms",
            diff.as_millis()
        );
    }

    #[test]
    fn test_active_transitions_next_wake_delay_phase_returns_delay_end() {
        // Transition still in delay: next_wake should be start_time + delay.
        let now = Instant::now();
        let delay = Duration::from_millis(200);
        let tr = RunningTransition {
            property: TransitionProperty::Opacity,
            start_value: AnimatableValue::Float(0.0),
            end_value: AnimatableValue::Float(1.0),
            start_time: now,
            duration: Duration::from_millis(300),
            delay,
            timing_fn: TimingFunction::Linear,
        };
        let expected_wake = now + delay;
        let (arena, id) = make_arena_with_transition(tr);
        let mut active = ActiveTransitions::default();
        active.add(id);

        let wake = ActiveTransitions::next_wake(&arena, &active, now)
            .expect("should return Some during delay phase");
        // Should match start_time + delay exactly.
        let diff_ns = (wake.duration_since(expected_wake).as_nanos() as i64).abs();
        assert!(diff_ns < 1_000_000, "wake should equal start_time + delay, diff = {}ns", diff_ns);
    }

    #[test]
    fn test_active_transitions_next_wake_returns_minimum_across_multiple() {
        // Two transitions: one still in delay (wakes later), one active (wakes in ~16ms).
        // The active one's wake should win because it is sooner.
        let now = Instant::now();

        // Active transition (past delay).
        let tr_active = RunningTransition {
            property: TransitionProperty::Opacity,
            start_value: AnimatableValue::Float(0.0),
            end_value: AnimatableValue::Float(1.0),
            start_time: now - Duration::from_millis(50),
            duration: Duration::from_millis(300),
            delay: Duration::ZERO,
            timing_fn: TimingFunction::Linear,
        };
        // Delay transition (wakes in 500ms).
        let tr_delayed = RunningTransition {
            property: TransitionProperty::Opacity,
            start_value: AnimatableValue::Float(0.0),
            end_value: AnimatableValue::Float(1.0),
            start_time: now,
            duration: Duration::from_millis(300),
            delay: Duration::from_millis(500),
            timing_fn: TimingFunction::Linear,
        };

        let mut arena = crate::tree::NodeArena::new();
        let mut el1 = crate::element::Element::new(crate::element::Tag::Div);
        el1.running_transitions.push(tr_active);
        let id1 = arena.alloc(el1);
        let mut el2 = crate::element::Element::new(crate::element::Tag::Div);
        el2.running_transitions.push(tr_delayed);
        let id2 = arena.alloc(el2);

        let mut active = ActiveTransitions::default();
        active.add(id1);
        active.add(id2);

        let wake = ActiveTransitions::next_wake(&arena, &active, now).expect("should return Some");
        // The minimum across both should be the active-phase budget (~16ms),
        // not the 500ms delay end.
        let diff = wake.duration_since(now);
        assert!(
            diff.as_millis() <= 20,
            "minimum wake should be the near-future frame budget, got {}ms",
            diff.as_millis()
        );
    }
}
