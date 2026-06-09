use smallvec::SmallVec;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255, a: 255 };
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0, a: 255 };

    pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Normalize to [0,1] without gamma conversion.
    /// We blend in sRGB space to match CSS compositing behavior.
    pub fn to_linear_f32(self) -> [f32; 4] {
        [self.r as f32 / 255.0, self.g as f32 / 255.0, self.b as f32 / 255.0, self.a as f32 / 255.0]
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const ZERO: Edges = Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 };

    pub fn all(v: f32) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }

    pub fn any_nonzero(&self) -> bool {
        self.top != 0.0 || self.right != 0.0 || self.bottom != 0.0 || self.left != 0.0
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.top, self.right, self.bottom, self.left]
    }
}

impl Default for Edges {
    fn default() -> Self {
        Self::ZERO
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeAutoFlags {
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub left: bool,
}

impl EdgeAutoFlags {
    pub const NONE: EdgeAutoFlags =
        EdgeAutoFlags { top: false, right: false, bottom: false, left: false };

    pub fn any(self) -> bool {
        self.top || self.right || self.bottom || self.left
    }
}

impl Default for EdgeAutoFlags {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub const ZERO: Corners =
        Corners { top_left: 0.0, top_right: 0.0, bottom_right: 0.0, bottom_left: 0.0 };

    pub fn all(v: f32) -> Self {
        Self { top_left: v, top_right: v, bottom_right: v, bottom_left: v }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.top_left, self.top_right, self.bottom_right, self.bottom_left]
    }
}

impl Default for Corners {
    fn default() -> Self {
        Self::ZERO
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub spread_radius: f32,
    pub color: Color,
    pub inset: bool,
}

/// A single CSS `text-shadow` layer (`<offset-x> <offset-y> <blur>? <color>?`).
/// Simpler than [`BoxShadow`]: no spread and no inset. The renderer paints it
/// as a colored glow behind the text (see the stacked-tap emit in the batch
/// builder).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_radius: f32,
    pub color: Color,
}

/// Individual CSS filter function entry stored inside `BackdropFilter`.
///
/// Only `Blur` is honored by the renderer today. The enum is open so other
/// filter functions (`brightness`, `contrast`, ...) can be added in a future
/// pass without a parser rewrite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterFunction {
    /// Gaussian blur radius in CSS pixels. Parser clamps this to `[0, 64]`.
    Blur(f32),
}

/// Parsed value of the CSS `backdrop-filter` property.
///
/// The declaration stores a list of filter entries even though the renderer
/// only honors `Blur` entries in this pass. Keeping the list shape allows the
/// parser grammar to accept the full comma separated form from day one.
#[derive(Clone, Debug, PartialEq)]
pub struct BackdropFilter {
    pub filters: smallvec::SmallVec<[FilterFunction; 2]>,
}

/// Unit aware position of a single gradient stop.
///
/// CSS allows stop positions in either `%` (fraction of the projected axis
/// length) or `px` (absolute distance in pixels along the projection axis).
/// The unit is preserved until batch build time because absolute pixel stops
/// need the element's projected axis length to normalize into the 0..1 range
/// that the shader samples in.
///
/// Issue #128 (`repeating-linear-gradient`) introduced the `Px` variant so
/// the terminal-manager CRT scanline overlay (`0, 2px, 2px, 3px`) can be
/// expressed without quantizing the 3 pixel tile to a percentage of the
/// element height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientStopPosition {
    /// Fraction of the projected axis length in the closed interval
    /// `[0.0, 1.0]`. `0.5` means halfway along the gradient.
    Percent(f32),
    /// Absolute distance in CSS pixels from the first stop along the
    /// projection axis. Batch build time converts this to a fraction by
    /// dividing by the element's projected axis length.
    Px(f32),
}

impl GradientStopPosition {
    /// Normalize this position into `[0.0, 1.0]` given the projected axis
    /// length along the gradient direction. Pixel positions are clamped
    /// against negative values so the shader never sees a position below
    /// zero. Percent positions are passed through unchanged.
    pub fn resolve(self, axis_length: f32) -> f32 {
        match self {
            GradientStopPosition::Percent(v) => v,
            GradientStopPosition::Px(v) => {
                if axis_length <= 0.0 {
                    0.0
                } else {
                    (v / axis_length).max(0.0)
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub color: Color,
    pub position: GradientStopPosition,
}

/// A CSS `linear-gradient` with N color stops (2 or more).
///
/// Stops are stored inline up to 4 entries, which covers the current
/// terminal-manager corpus with zero heap allocation. Gradients with more
/// than 4 stops spill to the heap. The renderer caps the GPU side at 8 stops.
///
/// Parse time invariants: the stop list is always non empty, has a length of
/// at least 2, and positions are fully populated and monotonic (see the
/// fixup pass in `parse::parse_linear_gradient` per CSS Images Level 3).
/// When all stops are percentages the parser guarantees positions in
/// `[0.0, 1.0]`; when pixel positions are mixed in, normalization happens
/// at batch build time against the element's projected axis length.
///
/// The `repeating` flag (issue #128) selects between the non repeating and
/// repeating gradient sampling branches in the fragment shader. When true,
/// the shader wraps the projected coordinate with `fract` so the gradient
/// tiles along the axis.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub angle_deg: f32,
    pub stops: SmallVec<[GradientStop; 4]>,
    pub repeating: bool,
}

impl Default for LinearGradient {
    fn default() -> Self {
        Self { angle_deg: 180.0, stops: SmallVec::new(), repeating: false }
    }
}

/// Length or percentage value used by the `radial-gradient` grammar for
/// explicit radii and position coordinates. Percentages are stored as a
/// unit fraction (50% becomes `0.5`), pixel lengths are stored verbatim.
///
/// Percentages resolve against the element box at paint time: the x axis
/// against the rect width and the y axis against the rect height. A negative
/// explicit radius is rejected by the parser per CSS Images Level 3.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthOrPercent {
    Px(f32),
    Percent(f32),
}

/// Shape of a `radial-gradient`. Ellipse is the CSS default when the shape
/// is omitted and explicit sizing is either absent or uses two values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadialShape {
    Circle,
    Ellipse,
}

/// Sizing hint for a `radial-gradient`. The keyword variants are resolved
/// against the element box at paint time in the renderer. `Explicit`
/// carries a pair of user provided length or percentage values; for a
/// circle the parser collapses a single value into `rx == ry`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadialSize {
    ClosestSide,
    ClosestCorner,
    FarthestSide,
    FarthestCorner,
    Explicit { rx: LengthOrPercent, ry: LengthOrPercent },
}

/// Center position for a `radial-gradient`. A percentage coordinate resolves
/// linearly against the box width or height. Values outside `[0, 100]` are
/// allowed and do not get clamped, so the gradient center can legitimately
/// lie outside the box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadialPosition {
    pub x: LengthOrPercent,
    pub y: LengthOrPercent,
}

impl RadialPosition {
    pub const CENTER: RadialPosition =
        RadialPosition { x: LengthOrPercent::Percent(0.5), y: LengthOrPercent::Percent(0.5) };
}

/// A CSS `radial-gradient` with N color stops (2 or more).
///
/// Stops reuse the exact same container that `LinearGradient` uses so the
/// shared parser helpers, position fixup pass, and GPU stop buffer helpers
/// from the linear gradient work land on both variants without a fork.
///
/// `shape`, `size`, and `center` carry the grammar as parsed; the renderer
/// resolves `size` and `center` against the element rect at paint time.
/// This matches the CSS model, where `closest-side` and friends depend on
/// the box, which is not known until after layout runs.
///
/// Parse time invariants: the stop list always has at least 2 entries and
/// its positions are monotonic in `[0.0, 1.0]`, just like `LinearGradient`.
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    pub shape: RadialShape,
    pub size: RadialSize,
    pub center: RadialPosition,
    pub stops: SmallVec<[GradientStop; 4]>,
}

/// Resolved center and radii of a `RadialGradient` in element local pixels.
///
/// Produced by [`RadialGradient::resolve`] at paint time. `rx` and `ry` are
/// always non negative. `shape` carries through so the shader can pick the
/// correct distance function (`true` circle means isotropic distance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedRadial {
    pub center_x: f32,
    pub center_y: f32,
    pub rx: f32,
    pub ry: f32,
    pub shape: RadialShape,
}

impl LengthOrPercent {
    /// Resolve against a reference length (rect width for x, rect height for y).
    /// Percentages multiply the reference, pixel values pass through.
    pub fn resolve(self, reference: f32) -> f32 {
        match self {
            LengthOrPercent::Px(v) => v,
            LengthOrPercent::Percent(p) => p * reference,
        }
    }
}

/// Per-corner `border-radius` values that retain their CSS unit (px / percent)
/// until paint, when the element box is known. `ComputedStyle.border_radius`
/// (resolved f32 `Corners`) stays the value the transition / scale code reads;
/// `border_radius_src: CornersDim` is the source of truth the renderer resolves
/// against the box so circular forms (`border-radius: 50%`) round correctly.
///
/// This mirrors the `padding` / `padding_src` dual-write: the f32 mirror holds
/// 0 for percent corners (paint resolves the real value from this source), which
/// is harmless for the only mirror consumers — transition lerp and DPI scaling,
/// both of which only ever see pure-px radii in the current corpus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornersDim {
    pub top_left: LengthOrPercent,
    pub top_right: LengthOrPercent,
    pub bottom_right: LengthOrPercent,
    pub bottom_left: LengthOrPercent,
}

impl CornersDim {
    pub const ZERO: CornersDim = CornersDim {
        top_left: LengthOrPercent::Px(0.0),
        top_right: LengthOrPercent::Px(0.0),
        bottom_right: LengthOrPercent::Px(0.0),
        bottom_left: LengthOrPercent::Px(0.0),
    };

    /// Lift a resolved f32 `Corners` into the unit-preserving form (all px).
    pub fn from_px(c: Corners) -> Self {
        Self {
            top_left: LengthOrPercent::Px(c.top_left),
            top_right: LengthOrPercent::Px(c.top_right),
            bottom_right: LengthOrPercent::Px(c.bottom_right),
            bottom_left: LengthOrPercent::Px(c.bottom_left),
        }
    }

    /// Resolve each corner against a reference length to produce the
    /// `[top_left, top_right, bottom_right, bottom_left]` pixel array the quad
    /// SDF shader consumes. CSS resolves a `border-radius` percentage against
    /// the box; callers pass `min(width, height)` so a `50%` radius stays
    /// circular on non-square boxes.
    pub fn resolve(self, reference: f32) -> [f32; 4] {
        [
            self.top_left.resolve(reference),
            self.top_right.resolve(reference),
            self.bottom_right.resolve(reference),
            self.bottom_left.resolve(reference),
        ]
    }
}

impl Default for CornersDim {
    fn default() -> Self {
        Self::ZERO
    }
}

impl RadialGradient {
    /// Resolve this gradient against a box of `(width, height)` to produce
    /// a center and a pair of radii in element local pixels.
    ///
    /// Size keyword rules (per CSS Images Level 3):
    /// * `ClosestSide` ellipse: `rx = min(cx, w - cx)`, `ry = min(cy, h - cy)`.
    ///   Circle: `r = min(cx, w - cx, cy, h - cy)`.
    /// * `FarthestSide` ellipse: `rx = max(cx, w - cx)`, `ry = max(cy, h - cy)`.
    ///   Circle: `r = max(cx, w - cx, cy, h - cy)`.
    /// * `ClosestCorner`: nearest corner distance. Ellipse scales by the
    ///   closest side aspect ratio so the curve passes through the corner.
    /// * `FarthestCorner`: farthest corner distance. Same aspect scaling
    ///   rule as `ClosestCorner`. This is the CSS default.
    /// * `Explicit`: both values resolve against width for `rx`, height for
    ///   `ry`. For a circle the parser has already collapsed the single
    ///   value into `rx == ry`.
    pub fn resolve(&self, width: f32, height: f32) -> ResolvedRadial {
        let cx = self.center.x.resolve(width);
        let cy = self.center.y.resolve(height);

        let (rx, ry) = match self.size {
            RadialSize::Explicit { rx, ry } => {
                let rx_px = rx.resolve(width).max(0.0);
                let ry_px = ry.resolve(height).max(0.0);
                (rx_px, ry_px)
            }
            RadialSize::ClosestSide => {
                let dx = cx.min(width - cx).max(0.0);
                let dy = cy.min(height - cy).max(0.0);
                match self.shape {
                    RadialShape::Circle => {
                        let r = dx.min(dy);
                        (r, r)
                    }
                    RadialShape::Ellipse => (dx, dy),
                }
            }
            RadialSize::FarthestSide => {
                let dx = cx.abs().max((width - cx).abs());
                let dy = cy.abs().max((height - cy).abs());
                match self.shape {
                    RadialShape::Circle => {
                        let r = dx.max(dy);
                        (r, r)
                    }
                    RadialShape::Ellipse => (dx, dy),
                }
            }
            RadialSize::ClosestCorner => {
                // Find the closest corner by picking the closest side on
                // each axis, then the corner distance is the hypotenuse of
                // those two sides.
                let dx = cx.min(width - cx).max(0.0);
                let dy = cy.min(height - cy).max(0.0);
                match self.shape {
                    RadialShape::Circle => {
                        let r = (dx * dx + dy * dy).sqrt();
                        (r, r)
                    }
                    RadialShape::Ellipse => {
                        // Ellipse that passes through the closest corner
                        // preserves the `dx / dy` aspect: rx = dx * sqrt(2),
                        // ry = dy * sqrt(2). This is the CSS Images Level 3
                        // definition in terms of the `closest-side` rectangle.
                        let k = std::f32::consts::SQRT_2;
                        (dx * k, dy * k)
                    }
                }
            }
            RadialSize::FarthestCorner => {
                let dx = cx.abs().max((width - cx).abs());
                let dy = cy.abs().max((height - cy).abs());
                match self.shape {
                    RadialShape::Circle => {
                        let r = (dx * dx + dy * dy).sqrt();
                        (r, r)
                    }
                    RadialShape::Ellipse => {
                        let k = std::f32::consts::SQRT_2;
                        (dx * k, dy * k)
                    }
                }
            }
        };

        ResolvedRadial { center_x: cx, center_y: cy, rx, ry, shape: self.shape }
    }
}

/// Value of `transform: translateX(...)` as stored on a computed style.
///
/// CSS `translateX` accepts either an absolute length in pixels or a
/// A `<length-percentage>` translation component of a CSS `transform`.
///
/// Used for both `translateX` (percentage resolves against the element's
/// own width) and `translateY` (percentage resolves against its height); the
/// caller picks which box dimension to resolve against. The name is kept for
/// backward compatibility with the transition machinery that references it.
///
/// The whole transform is applied at paint time as a post-layout render
/// effect: siblings do not shift, only the transformed element (and its
/// subtree) appear moved. This mirrors CSS's `transform` semantics where
/// transforms do not participate in flow layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformX {
    Px(f32),
    /// Percentage of the relevant box axis, stored as a unit fraction
    /// (e.g. `50%` becomes `0.5`).
    Percent(f32),
}

impl TransformX {
    /// Resolve the translation to an absolute pixel offset given the
    /// element's own size along the relevant axis. Pixel values pass through;
    /// percentages multiply the axis length.
    pub fn resolve(self, axis_len: f32) -> f32 {
        match self {
            TransformX::Px(v) => v,
            TransformX::Percent(p) => p * axis_len,
        }
    }
}

/// A parsed CSS `transform` function list, stored in decomposed component
/// form rather than a baked matrix so transitions / keyframes can interpolate
/// each component independently (matching CSS for matching function lists).
///
/// The renderer composes these into a 2x3 affine about the element's
/// `transform-origin` (default `50% 50%`, the box center) in the fixed
/// canonical order `Translate · Rotate · Scale` — i.e. a point is scaled,
/// then rotated, then translated. This captures every form the app stylesheet
/// authors (`scale(..)`, `rotate(..)`, `translateY(..)`, and the combined
/// `translateY(..) scale(..)`); arbitrary `matrix()` / `skew()` and
/// non-canonical orderings are out of scope.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// `translateX` (or the X of `translate`). `None` is zero.
    pub translate_x: Option<TransformX>,
    /// `translateY` (or the Y of `translate`). `None` is zero.
    pub translate_y: Option<TransformX>,
    /// `scaleX` (or uniform `scale`). `1.0` is identity.
    pub scale_x: f32,
    /// `scaleY` (or uniform `scale`). `1.0` is identity.
    pub scale_y: f32,
    /// `rotate`, in radians, clockwise (screen-space, Y-down). `0.0` is none.
    pub rotate: f32,
}

impl Transform {
    /// The identity transform (no visual change).
    pub const IDENTITY: Transform =
        Transform { translate_x: None, translate_y: None, scale_x: 1.0, scale_y: 1.0, rotate: 0.0 };

    /// True when this transform paints identically to no transform at all, so
    /// the renderer can keep its fast (matrix-free) path.
    pub fn is_identity(&self) -> bool {
        self.translate_x.is_none()
            && self.translate_y.is_none()
            && self.scale_x == 1.0
            && self.scale_y == 1.0
            && self.rotate == 0.0
    }
}

impl Default for Transform {
    fn default() -> Self {
        Transform::IDENTITY
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Background {
    Color(Color),
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
}

impl Default for Background {
    fn default() -> Self {
        Background::Color(Color::TRANSPARENT)
    }
}

impl Background {
    pub fn is_visible(&self) -> bool {
        match self {
            Background::Color(c) => c.a > 0,
            Background::LinearGradient(g) => g.stops.iter().any(|s| s.color.a > 0),
            Background::RadialGradient(g) => g.stops.iter().any(|s| s.color.a > 0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Dimension {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
    /// Viewport height unit: 1vh = 1% of viewport height.
    Vh(f32),
    /// Viewport width unit: 1vw = 1% of viewport width.
    Vw(f32),
    /// A `calc()` expression reduced to a linear combination of a constant and
    /// viewport fractions: `px + vw/100 * viewport_w + vh/100 * viewport_h`.
    /// Resolves to an absolute pixel length against the viewport at layout time
    /// (taffy never sees it). Percentage-of-container terms are intentionally
    /// excluded — taffy cannot represent `length + percent`, and the app authors
    /// no such form on a supported property; the parser rejects them.
    Calc {
        px: f32,
        vw: f32,
        vh: f32,
    },
}

impl Dimension {
    /// Resolve a `Calc` (or any dimension with viewport units) to absolute
    /// pixels given the viewport. Returns `None` for `Auto`/`Percent`, which
    /// need taffy's container-relative resolution.
    pub fn calc_to_px(px: f32, vw: f32, vh: f32, viewport_w: f32, viewport_h: f32) -> f32 {
        px + vw / 100.0 * viewport_w + vh / 100.0 * viewport_h
    }
}

/// Per-side edge values that retain their CSS unit (px / percent / vh / vw)
/// until layout, when the viewport is known. `ComputedStyle.padding` (resolved
/// f32 `Edges`) stays the value the renderer / hit-test / transition code reads;
/// `padding_src: EdgesDim` is the source of truth `to_taffy_style` resolves so
/// viewport-relative padding (e.g. `padding-top: 12vh`) lays out correctly.
/// The f32 mirror holds 0 for non-px units (paint has no viewport), which is
/// harmless for the only consumers today — content-less full-viewport scrims.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgesDim {
    pub top: Dimension,
    pub right: Dimension,
    pub bottom: Dimension,
    pub left: Dimension,
}

impl EdgesDim {
    pub const ZERO: EdgesDim = EdgesDim {
        top: Dimension::Px(0.0),
        right: Dimension::Px(0.0),
        bottom: Dimension::Px(0.0),
        left: Dimension::Px(0.0),
    };

    /// Lift a resolved f32 `Edges` into the unit-preserving form (all px).
    pub fn from_px(e: Edges) -> Self {
        Self {
            top: Dimension::Px(e.top),
            right: Dimension::Px(e.right),
            bottom: Dimension::Px(e.bottom),
            left: Dimension::Px(e.left),
        }
    }
}

impl Default for EdgesDim {
    fn default() -> Self {
        Self::ZERO
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Display {
    #[default]
    Block,
    Flex,
    InlineFlex,
    InlineBlock,
    Grid,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GridAutoFlow {
    #[default]
    Row,
    Column,
    RowDense,
    ColumnDense,
}

/// A single grid track sizing value (maps to taffy's NonRepeatedTrackSizingFunction).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridTrackSize {
    pub min: GridMinTrackSize,
    pub max: GridMaxTrackSize,
}

impl GridTrackSize {
    pub fn fixed_px(v: f32) -> Self {
        Self { min: GridMinTrackSize::Px(v), max: GridMaxTrackSize::Px(v) }
    }

    pub fn fixed_percent(v: f32) -> Self {
        Self { min: GridMinTrackSize::Percent(v), max: GridMaxTrackSize::Percent(v) }
    }

    pub fn fr(v: f32) -> Self {
        Self { min: GridMinTrackSize::Auto, max: GridMaxTrackSize::Fr(v) }
    }

    pub fn auto() -> Self {
        Self { min: GridMinTrackSize::Auto, max: GridMaxTrackSize::Auto }
    }

    pub fn min_content() -> Self {
        Self { min: GridMinTrackSize::MinContent, max: GridMaxTrackSize::MinContent }
    }

    pub fn max_content() -> Self {
        Self { min: GridMinTrackSize::MaxContent, max: GridMaxTrackSize::MaxContent }
    }

    pub fn minmax(min: GridMinTrackSize, max: GridMaxTrackSize) -> Self {
        Self { min, max }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridMinTrackSize {
    Px(f32),
    Percent(f32),
    Auto,
    MinContent,
    MaxContent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridMaxTrackSize {
    Px(f32),
    Percent(f32),
    Auto,
    MinContent,
    MaxContent,
    Fr(f32),
    FitContent(f32),
    FitContentPercent(f32),
}

/// A track definition in a grid template, which can be a single track or a repeat().
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrackDef {
    Single(GridTrackSize),
    Repeat(GridRepeatCount, Vec<GridTrackSize>),
}

/// The repeat count for repeat() in grid templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridRepeatCount {
    Count(u16),
    AutoFill,
    AutoFit,
}

/// Grid item placement value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GridPlacement {
    #[default]
    Auto,
    Line(i16),
    Span(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
    RowReverse,
    ColumnReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignItems {
    Start,
    End,
    Center,
    #[default]
    Stretch,
    Baseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignSelf {
    #[default]
    Auto,
    Start,
    End,
    Center,
    Stretch,
    Baseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    Normal,
    Start,
    End,
    Center,
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlignContent {
    Start,
    End,
    Center,
    #[default]
    Stretch,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    PreLine,
}

/// CSS `text-overflow`. Controls how text that overflows its container's
/// content box is signaled when it cannot wrap (`white-space: nowrap`).
/// `Clip` hard-cuts at the clip rect (the default); `Ellipsis` truncates on a
/// grapheme-cluster boundary and appends an ellipsis so the cut never lands
/// mid-glyph. This property is NOT inherited (per the CSS spec).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
    W(u16),
}

/// CSS `font-style`. `Oblique` is treated the same as `Italic` by the text
/// pipeline (both request a slanted face and fall back to the render-time
/// skew when no slanted face resolves).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CssPosition {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorStyle {
    #[default]
    Default,
    None,
    Pointer,
    Text,
    Grab,
    Grabbing,
    NotAllowed,
    Crosshair,
    Move,
    Wait,
    Help,
    Progress,
    ColResize,
    RowResize,
    NResize,
    SResize,
    EResize,
    WResize,
    NeResize,
    NwResize,
    SeResize,
    SwResize,
    NsResize,
    EwResize,
    NeswResize,
    NwseResize,
    ZoomIn,
    ZoomOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CssResize {
    #[default]
    None,
    Both,
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectPosition {
    /// Horizontal position as a percentage (0.0 = left, 50.0 = center, 100.0 = right).
    pub x: f32,
    /// Vertical position as a percentage (0.0 = top, 50.0 = center, 100.0 = bottom).
    pub y: f32,
}

impl Default for ObjectPosition {
    fn default() -> Self {
        Self { x: 50.0, y: 50.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BoxSizing {
    ContentBox,
    #[default]
    BorderBox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
}

impl Default for Visibility {
    fn default() -> Self {
        Visibility::Visible
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerEvents {
    Auto,
    None,
}

impl Default for PointerEvents {
    fn default() -> Self {
        PointerEvents::Auto
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UserSelect {
    #[default]
    Auto,
    None,
    Text,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AppRegion {
    #[default]
    Auto,
    Drag,
    NoDrag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextDecoration {
    #[default]
    None,
    Underline,
    LineThrough,
    Overline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum Layer {
    Background = 0,
    #[default]
    Content = 1,
    Popover = 2,
    Modal = 3,
    Overlay = 4,
    Tooltip = 5,
    Debug = 6,
}

impl Layer {
    pub const COUNT: usize = 7;
    pub const ALL: [Layer; 7] = [
        Layer::Background,
        Layer::Content,
        Layer::Popover,
        Layer::Modal,
        Layer::Overlay,
        Layer::Tooltip,
        Layer::Debug,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RenderTarget {
    #[default]
    Inline,
    Portal(Layer),
}

/// Controls how bell/alert signals are delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BellStyle {
    /// Render a visual bell overlay only.
    Visual,
    /// Request window attention only (no overlay).
    Attention,
    /// Both visual overlay and window attention request.
    #[default]
    Both,
    /// Suppress all bell output.
    None,
}

impl std::fmt::Display for BellStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BellStyle::Visual => write!(f, "visual"),
            BellStyle::Attention => write!(f, "attention"),
            BellStyle::Both => write!(f, "both"),
            BellStyle::None => write!(f, "none"),
        }
    }
}

use std::sync::Arc;
use std::time::Duration;

use crate::cursor::CursorShape;
use crate::resize_handle::ResizeAxis;
use crate::style::transition::{TimingFunction, TransitionDef};

/// A single keyframe inside an `@keyframes` rule.
///
/// Each keyframe carries a normalized offset in the `0.0..=1.0` range plus a
/// list of property declarations that apply at that offset. Multi selector
/// blocks like `0%, 100% { opacity: 1; }` are flattened at parse time into
/// one `Keyframe` per offset that share the same declaration list.
#[derive(Clone, Debug)]
pub struct Keyframe {
    /// Normalized offset in `0.0..=1.0`.
    pub offset: f32,
    /// Raw style declarations active at this offset. We reuse the regular
    /// declaration type so every property already understood by the cascade
    /// is automatically supported inside keyframes.
    pub declarations: Vec<crate::style::parse::StyleDeclaration>,
}

/// A parsed `@keyframes <name> { ... }` at rule.
///
/// Frames are stored sorted by ascending offset so the driver can look them
/// up with a simple binary search.
#[derive(Clone, Debug)]
pub struct KeyframesRule {
    pub name: String,
    pub frames: Vec<Keyframe>,
}

/// Number of iterations an animation should run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IterationCount {
    /// Finite iteration count (spec allows fractional values).
    Finite(f32),
    /// Runs forever until the animation is removed.
    Infinite,
}

impl Default for IterationCount {
    fn default() -> Self {
        IterationCount::Finite(1.0)
    }
}

/// Direction in which the animation plays each iteration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AnimationDirection {
    #[default]
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// What to do with the computed style before and after the animation is
/// active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AnimationFillMode {
    #[default]
    None,
    Forwards,
    Backwards,
    Both,
}

/// Whether the animation is running or paused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AnimationPlayState {
    #[default]
    Running,
    Paused,
}

/// A single entry in the `animation:` shorthand or its longhand equivalents.
///
/// Carries everything the driver needs to know to run one animation on one
/// element. Multiple `AnimationDef` values can live on the same element when
/// the shorthand lists several comma separated animations.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationDef {
    /// Animation name. `None` means the entry is inert (either the author
    /// wrote `none` or the shorthand omitted a name).
    pub name: Option<Arc<str>>,
    pub duration: Duration,
    pub timing_function: TimingFunction,
    pub delay: Duration,
    /// Delay as signed nanoseconds; negative values start the animation
    /// already in progress and are preserved separately from `delay`, which
    /// is clamped to zero.
    pub delay_nanos: i64,
    pub iteration_count: IterationCount,
    pub direction: AnimationDirection,
    pub fill_mode: AnimationFillMode,
    pub play_state: AnimationPlayState,
}

impl Default for AnimationDef {
    fn default() -> Self {
        Self {
            name: None,
            duration: Duration::ZERO,
            timing_function: TimingFunction::Ease,
            delay: Duration::ZERO,
            delay_nanos: 0,
            iteration_count: IterationCount::default(),
            direction: AnimationDirection::default(),
            fill_mode: AnimationFillMode::default(),
            play_state: AnimationPlayState::default(),
        }
    }
}

/// Value of the CSS `content` property for pseudo elements.
///
/// The first pass supports `none`, `normal`, a literal string, and
/// `attr(name)` lookups. Other forms (counters, `url(...)`, quotes) are out
/// of scope and parse as errors.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ContentValue {
    None,
    Normal,
    Literal(String),
    Attr(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

pub fn apply_text_transform(text: &str, transform: TextTransform) -> std::borrow::Cow<'_, str> {
    match transform {
        TextTransform::None => std::borrow::Cow::Borrowed(text),
        TextTransform::Uppercase => std::borrow::Cow::Owned(text.to_ascii_uppercase()),
        TextTransform::Lowercase => std::borrow::Cow::Owned(text.to_ascii_lowercase()),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(text.len());
            let mut word_start = true;
            for ch in text.chars() {
                if ch.is_ascii_alphanumeric() {
                    if word_start {
                        out.push(ch.to_ascii_uppercase());
                        word_start = false;
                    } else {
                        out.push(ch);
                    }
                } else {
                    out.push(ch);
                    word_start = true;
                }
            }
            std::borrow::Cow::Owned(out)
        }
    }
}

impl Default for ContentValue {
    fn default() -> Self {
        ContentValue::Normal
    }
}

impl ContentValue {
    /// Returns true if this value should produce a visible pseudo element,
    /// i.e. a string the pseudo resolver can stamp onto a synthetic node.
    pub fn produces_box(&self) -> bool {
        matches!(self, ContentValue::Literal(_) | ContentValue::Attr(_))
    }
}

/// Style overrides applied to selected text via `::selection`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SelectionStyle {
    pub color: Option<Color>,
    pub background_color: Option<Color>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComputedStyle {
    // Pseudo element content (only meaningful on synthetic ::before / ::after nodes,
    // or as the last-applied source of truth during pseudo resolution).
    pub content: ContentValue,

    // Transitions (parsed from CSS `transition` property)
    pub transitions: SmallVec<[TransitionDef; 2]>,

    // Animations (parsed from CSS `animation` shorthand and longhands).
    //
    // Multiple animations per element are supported; the last entry in source
    // order wins on property conflicts at sample time, matching the CSS spec.
    pub animations: SmallVec<[AnimationDef; 2]>,

    // Layout
    pub display: Display,
    pub flex_direction: FlexDirection,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Dimension,
    pub align_items: AlignItems,
    pub align_self: AlignSelf,
    pub justify_content: JustifyContent,
    pub flex_wrap: FlexWrap,
    pub align_content: AlignContent,
    pub width: Dimension,
    pub height: Dimension,
    pub min_width: Dimension,
    pub min_height: Dimension,
    pub max_width: Dimension,
    pub max_height: Dimension,
    pub padding: Edges,
    /// Unit-preserving source for `padding`, resolved against the viewport in
    /// `to_taffy_style`. Always kept in sync with `padding` by the cascade.
    pub padding_src: EdgesDim,
    pub margin: Edges,
    pub margin_auto: EdgeAutoFlags,
    pub row_gap: f32,
    pub column_gap: f32,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    pub box_sizing: BoxSizing,
    pub aspect_ratio: Option<f32>,
    pub object_fit: ObjectFit,
    pub object_position: ObjectPosition,

    // Grid container properties
    pub grid_template_columns: Vec<GridTrackDef>,
    pub grid_template_rows: Vec<GridTrackDef>,
    pub grid_auto_columns: Vec<GridTrackSize>,
    pub grid_auto_rows: Vec<GridTrackSize>,
    pub grid_auto_flow: GridAutoFlow,

    // Grid item properties
    pub grid_column_start: GridPlacement,
    pub grid_column_end: GridPlacement,
    pub grid_row_start: GridPlacement,
    pub grid_row_end: GridPlacement,
    pub position: CssPosition,
    pub top: Option<Dimension>,
    pub right: Option<Dimension>,
    pub bottom: Option<Dimension>,
    pub left: Option<Dimension>,
    pub z_index: i32,

    // Visual
    pub background: Background,
    pub border_color: Color,
    pub border_width: Edges,
    pub border_radius: Corners,
    /// Unit-preserving source for `border_radius`, resolved against the element
    /// box at paint time. Always kept in sync with `border_radius` by the
    /// cascade (px corners write both; percent corners write here and leave the
    /// f32 mirror at 0).
    pub border_radius_src: CornersDim,
    pub opacity: f32,
    pub box_shadow: SmallVec<[BoxShadow; 2]>,
    /// CSS `text-shadow` layers (empty = none). Painted as a colored glow
    /// behind the element's text.
    pub text_shadow: SmallVec<[TextShadow; 2]>,
    /// Optional `backdrop-filter` value. `None` means the element does not
    /// request a backdrop filter and the renderer stays on its fast path.
    pub backdrop_filter: Option<BackdropFilter>,

    // Text
    pub color: Color,
    pub font_size: f32,
    /// Inherited runtime text scale applied to explicit font-size declarations.
    pub font_size_scale: f32,
    /// True when `font_size` came from a declaration on this element rather
    /// than from the parent. Used so inherited scaled text is not multiplied
    /// again as the cascade walks down the tree.
    pub font_size_explicit: bool,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub font_family: String,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub text_align: TextAlign,
    pub text_transform: TextTransform,
    pub text_decoration: TextDecoration,
    pub text_decoration_color: Option<Color>,
    pub white_space: WhiteSpace,
    pub text_overflow: TextOverflow,

    // Input / Cursor
    pub caret_color: Color,
    pub caret_shape: CursorShape,
    pub caret_blink_rate: u32,
    pub placeholder_color: Color,

    // Outline
    pub outline_color: Color,
    pub outline_width: f32,
    pub outline_offset: f32,

    // Interaction
    pub cursor: CursorStyle,

    // Visibility / pointer behavior
    pub visibility: Visibility,
    pub pointer_events: PointerEvents,
    pub user_select: UserSelect,
    pub app_region: AppRegion,

    // Keyboard capture
    pub keyboard_capture: bool,

    // Layer / overlay
    pub layer: Layer,
    pub render_target: RenderTarget,

    // Resize
    pub resize: CssResize,
    pub resize_axis: Option<ResizeAxis>,

    // Bell / notification
    pub bell_style: BellStyle,

    /// Parsed `transform` (translate / scale / rotate). `Transform::IDENTITY`
    /// keeps the renderer on its fast (matrix-free) path; any non-identity
    /// value is composed into a 2x3 affine about the element's center and
    /// applied at paint time, propagating to the subtree without disturbing
    /// layout flow.
    pub transform: Transform,

    /// Parsed `mask-image: linear-gradient(...)` mask.
    ///
    /// `None` means no mask is attached to this element and the renderer
    /// emits its background quad through the normal solid / gradient path.
    /// When set, the gradient is baked into the quad instance as an
    /// auxiliary stop list and the fragment shader multiplies the final
    /// output alpha by the mask's alpha channel.
    pub mask_image: Option<LinearGradient>,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            content: ContentValue::Normal,
            transitions: SmallVec::new(),
            animations: SmallVec::new(),
            display: Display::Block,
            flex_direction: FlexDirection::Row,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Dimension::Auto,
            align_items: AlignItems::Stretch,
            align_self: AlignSelf::Auto,
            justify_content: JustifyContent::Normal,
            flex_wrap: FlexWrap::NoWrap,
            align_content: AlignContent::Stretch,
            width: Dimension::Auto,
            height: Dimension::Auto,
            min_width: Dimension::Auto,
            min_height: Dimension::Auto,
            max_width: Dimension::Auto,
            max_height: Dimension::Auto,
            padding: Edges::ZERO,
            padding_src: EdgesDim::ZERO,
            margin: Edges::ZERO,
            margin_auto: EdgeAutoFlags::NONE,
            row_gap: 0.0,
            column_gap: 0.0,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            box_sizing: BoxSizing::BorderBox,
            aspect_ratio: None,
            object_fit: ObjectFit::Fill,
            object_position: ObjectPosition::default(),
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_auto_columns: Vec::new(),
            grid_auto_rows: Vec::new(),
            grid_auto_flow: GridAutoFlow::Row,
            grid_column_start: GridPlacement::Auto,
            grid_column_end: GridPlacement::Auto,
            grid_row_start: GridPlacement::Auto,
            grid_row_end: GridPlacement::Auto,
            position: CssPosition::Static,
            top: None,
            right: None,
            bottom: None,
            left: None,
            z_index: 0,
            background: Background::default(),
            border_color: Color::TRANSPARENT,
            border_width: Edges::ZERO,
            border_radius: Corners::ZERO,
            border_radius_src: CornersDim::ZERO,
            opacity: 1.0,
            box_shadow: SmallVec::new(),
            text_shadow: SmallVec::new(),
            backdrop_filter: None,
            color: Color::BLACK,
            font_size: 16.0,
            font_size_scale: 1.0,
            font_size_explicit: true,
            font_weight: FontWeight::Normal,
            font_style: FontStyle::Normal,
            font_family: String::new(),
            line_height: 1.2,
            letter_spacing: 0.0,
            text_align: TextAlign::Left,
            text_transform: TextTransform::None,
            text_decoration: TextDecoration::None,
            text_decoration_color: None,
            white_space: WhiteSpace::Normal,
            text_overflow: TextOverflow::Clip,
            caret_color: Color::BLACK,
            caret_shape: CursorShape::default(),
            caret_blink_rate: 530,
            placeholder_color: Color::rgba(128, 128, 128, 255),
            outline_color: Color::TRANSPARENT,
            outline_width: 0.0,
            outline_offset: 0.0,
            cursor: CursorStyle::Default,
            visibility: Visibility::Visible,
            pointer_events: PointerEvents::Auto,
            user_select: UserSelect::Auto,
            app_region: AppRegion::Auto,
            keyboard_capture: false,
            layer: Layer::Content,
            render_target: RenderTarget::Inline,
            resize: CssResize::None,
            resize_axis: None,
            bell_style: BellStyle::Both,
            transform: Transform::IDENTITY,
            mask_image: None,
        }
    }
}

impl ComputedStyle {
    pub fn inherit_from(&mut self, parent: &ComputedStyle) {
        self.color = parent.color;
        self.font_size = parent.font_size;
        self.font_size_scale = parent.font_size_scale;
        self.font_size_explicit = false;
        self.font_weight = parent.font_weight;
        self.font_style = parent.font_style;
        self.font_family = parent.font_family.clone();
        self.line_height = parent.line_height;
        self.letter_spacing = parent.letter_spacing;
        self.text_align = parent.text_align;
        self.text_transform = parent.text_transform;
        self.text_decoration = parent.text_decoration;
        self.text_decoration_color = parent.text_decoration_color;
        self.white_space = parent.white_space;
        self.caret_color = parent.caret_color;
        self.caret_shape = parent.caret_shape;
        self.caret_blink_rate = parent.caret_blink_rate;
        self.cursor = parent.cursor;
        self.visibility = parent.visibility;
        self.pointer_events = parent.pointer_events;
        self.user_select = parent.user_select;
    }

    /// Build the computed style for an anonymous text box from its host's
    /// style. The anonymous child carries the host's text when the host has
    /// both text content and element children, so it must render the text
    /// exactly as the host would have: everything `inherit_from` copies,
    /// plus the painter-read properties inheritance misses (`opacity` is
    /// applied per emitting node, not composed down to descendants;
    /// `text_shadow` and `text_overflow` are non-inherited but read by the
    /// text draw). Box properties stay at their defaults (zero
    /// padding/border/margin, transparent background) so the text box adds
    /// no inset of its own, and it flexes to fill the host's content box
    /// (`flex_grow: 1`) while remaining shrinkable below its text's
    /// min-content size (`flex_shrink: 1`, `min_width: 0`) so `text-align`
    /// and `text-overflow: ellipsis` keep working against the host's
    /// content width.
    ///
    /// NOTE for maintainers: if you add an inherited property or a property
    /// the text painter reads off the emitting node, update this derivation
    /// alongside `inherit_from`.
    pub fn derive_anonymous_text(host: &ComputedStyle) -> ComputedStyle {
        let mut style = ComputedStyle::default();
        style.inherit_from(host);
        style.opacity = host.opacity;
        style.text_shadow = host.text_shadow.clone();
        style.text_overflow = host.text_overflow;
        style.flex_grow = 1.0;
        style.flex_shrink = 1.0;
        style.min_width = Dimension::Px(0.0);
        style
    }

    pub fn to_taffy_style(&self, viewport_w: f32, viewport_h: f32) -> taffy::Style {
        taffy::Style {
            display: match self.display {
                Display::Flex | Display::InlineFlex => taffy::Display::Flex,
                Display::Block | Display::InlineBlock => taffy::Display::Block,
                Display::Grid => taffy::Display::Grid,
                Display::None => taffy::Display::None,
            },
            flex_direction: match self.flex_direction {
                FlexDirection::Row => taffy::FlexDirection::Row,
                FlexDirection::Column => taffy::FlexDirection::Column,
                FlexDirection::RowReverse => taffy::FlexDirection::RowReverse,
                FlexDirection::ColumnReverse => taffy::FlexDirection::ColumnReverse,
            },
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            flex_basis: dim_to_taffy(self.flex_basis, viewport_w, viewport_h),
            align_items: Some(align_items_to_taffy(self.align_items)),
            align_self: align_self_to_taffy(self.align_self),
            // CSS Grid's initial `justify-items: normal` behaves as stretch
            // for ordinary grid items. Taffy's implicit fallback may shrink
            // items with intrinsic width, which leaves unpainted strips in a
            // single-column grid container.
            justify_items: Some(taffy::AlignItems::Stretch),
            justify_content: Some(justify_content_to_taffy(self.justify_content, self.display)),
            flex_wrap: match self.flex_wrap {
                FlexWrap::NoWrap => taffy::FlexWrap::NoWrap,
                FlexWrap::Wrap => taffy::FlexWrap::Wrap,
                FlexWrap::WrapReverse => taffy::FlexWrap::WrapReverse,
            },
            align_content: Some(match self.align_content {
                AlignContent::Start => taffy::AlignContent::FlexStart,
                AlignContent::End => taffy::AlignContent::FlexEnd,
                AlignContent::Center => taffy::AlignContent::Center,
                AlignContent::Stretch => taffy::AlignContent::Stretch,
                AlignContent::SpaceBetween => taffy::AlignContent::SpaceBetween,
                AlignContent::SpaceAround => taffy::AlignContent::SpaceAround,
                AlignContent::SpaceEvenly => taffy::AlignContent::SpaceEvenly,
            }),
            size: taffy::Size {
                width: dim_to_taffy(self.width, viewport_w, viewport_h),
                height: dim_to_taffy(self.height, viewport_w, viewport_h),
            },
            min_size: taffy::Size {
                width: dim_to_taffy(self.min_width, viewport_w, viewport_h),
                height: dim_to_taffy(self.min_height, viewport_w, viewport_h),
            },
            max_size: taffy::Size {
                width: dim_to_taffy(self.max_width, viewport_w, viewport_h),
                height: dim_to_taffy(self.max_height, viewport_w, viewport_h),
            },
            padding: edges_dim_to_taffy_rect(self.padding_src, viewport_w, viewport_h),
            margin: edges_to_taffy_rect_auto(self.margin, self.margin_auto),
            gap: taffy::Size {
                width: taffy::LengthPercentage::Length(self.column_gap),
                height: taffy::LengthPercentage::Length(self.row_gap),
            },
            overflow: taffy::Point {
                x: overflow_to_taffy(self.overflow_x),
                y: overflow_to_taffy(self.overflow_y),
            },
            position: match self.position {
                CssPosition::Static | CssPosition::Relative => taffy::Position::Relative,
                CssPosition::Absolute | CssPosition::Fixed => taffy::Position::Absolute,
            },
            inset: {
                // Static elements ignore inset (top/right/bottom/left) per CSS spec
                let inset_val = |d| {
                    if self.position == CssPosition::Static {
                        taffy::LengthPercentageAuto::Auto
                    } else {
                        opt_dim_to_taffy_auto(d, viewport_w, viewport_h)
                    }
                };
                taffy::Rect {
                    left: inset_val(self.left),
                    right: inset_val(self.right),
                    top: inset_val(self.top),
                    bottom: inset_val(self.bottom),
                }
            },
            grid_template_columns: grid_track_defs_to_taffy(&self.grid_template_columns),
            grid_template_rows: grid_track_defs_to_taffy(&self.grid_template_rows),
            grid_auto_columns: grid_auto_tracks_to_taffy(&self.grid_auto_columns),
            grid_auto_rows: grid_auto_tracks_to_taffy(&self.grid_auto_rows),
            grid_auto_flow: match self.grid_auto_flow {
                GridAutoFlow::Row => taffy::GridAutoFlow::Row,
                GridAutoFlow::Column => taffy::GridAutoFlow::Column,
                GridAutoFlow::RowDense => taffy::GridAutoFlow::RowDense,
                GridAutoFlow::ColumnDense => taffy::GridAutoFlow::ColumnDense,
            },
            grid_column: taffy::Line {
                start: grid_placement_to_taffy(self.grid_column_start),
                end: grid_placement_to_taffy(self.grid_column_end),
            },
            grid_row: taffy::Line {
                start: grid_placement_to_taffy(self.grid_row_start),
                end: grid_placement_to_taffy(self.grid_row_end),
            },
            aspect_ratio: self.aspect_ratio,
            box_sizing: match self.box_sizing {
                BoxSizing::ContentBox => taffy::BoxSizing::ContentBox,
                BoxSizing::BorderBox => taffy::BoxSizing::BorderBox,
            },
            ..Default::default()
        }
    }
}

fn align_items_to_taffy(value: AlignItems) -> taffy::AlignItems {
    match value {
        AlignItems::Start => taffy::AlignItems::FlexStart,
        AlignItems::End => taffy::AlignItems::FlexEnd,
        AlignItems::Center => taffy::AlignItems::Center,
        AlignItems::Stretch => taffy::AlignItems::Stretch,
        AlignItems::Baseline => taffy::AlignItems::Baseline,
    }
}

fn align_self_to_taffy(value: AlignSelf) -> Option<taffy::AlignSelf> {
    Some(match value {
        AlignSelf::Auto => return None,
        AlignSelf::Start => taffy::AlignSelf::FlexStart,
        AlignSelf::End => taffy::AlignSelf::FlexEnd,
        AlignSelf::Center => taffy::AlignSelf::Center,
        AlignSelf::Stretch => taffy::AlignSelf::Stretch,
        AlignSelf::Baseline => taffy::AlignSelf::Baseline,
    })
}

fn justify_content_to_taffy(value: JustifyContent, display: Display) -> taffy::JustifyContent {
    match value {
        JustifyContent::Normal => {
            if display == Display::Grid {
                taffy::JustifyContent::Stretch
            } else {
                taffy::JustifyContent::FlexStart
            }
        }
        JustifyContent::Start => taffy::JustifyContent::FlexStart,
        JustifyContent::End => taffy::JustifyContent::FlexEnd,
        JustifyContent::Center => taffy::JustifyContent::Center,
        JustifyContent::Stretch => taffy::JustifyContent::Stretch,
        JustifyContent::SpaceBetween => taffy::JustifyContent::SpaceBetween,
        JustifyContent::SpaceAround => taffy::JustifyContent::SpaceAround,
        JustifyContent::SpaceEvenly => taffy::JustifyContent::SpaceEvenly,
    }
}

fn overflow_to_taffy(o: Overflow) -> taffy::Overflow {
    match o {
        Overflow::Visible => taffy::Overflow::Visible,
        Overflow::Hidden => taffy::Overflow::Hidden,
        Overflow::Scroll => taffy::Overflow::Scroll,
    }
}

fn dim_to_taffy(d: Dimension, viewport_w: f32, viewport_h: f32) -> taffy::Dimension {
    match d {
        Dimension::Auto => taffy::Dimension::Auto,
        Dimension::Px(v) => taffy::Dimension::Length(v),
        Dimension::Percent(v) => taffy::Dimension::Percent(v / 100.0),
        // Viewport units are resolved to absolute pixels against the current
        // viewport. Taffy does not natively support vh/vw, so we eagerly
        // convert to a pixel length.
        Dimension::Vh(v) => taffy::Dimension::Length(v / 100.0 * viewport_h),
        Dimension::Vw(v) => taffy::Dimension::Length(v / 100.0 * viewport_w),
        Dimension::Calc { px, vw, vh } => {
            taffy::Dimension::Length(Dimension::calc_to_px(px, vw, vh, viewport_w, viewport_h))
        }
    }
}

/// Resolve a single edge `Dimension` into a taffy `LengthPercentage`.
/// Percent stays native (taffy resolves it against the containing block);
/// vh/vw resolve against the viewport here, exactly like `dim_to_taffy`.
fn dim_to_length_percentage(
    d: Dimension,
    viewport_w: f32,
    viewport_h: f32,
) -> taffy::LengthPercentage {
    match d {
        // Padding/edge values have no `auto`; treat it as zero.
        Dimension::Auto => taffy::LengthPercentage::Length(0.0),
        Dimension::Px(v) => taffy::LengthPercentage::Length(v),
        Dimension::Percent(v) => taffy::LengthPercentage::Percent(v / 100.0),
        Dimension::Vh(v) => taffy::LengthPercentage::Length(v / 100.0 * viewport_h),
        Dimension::Vw(v) => taffy::LengthPercentage::Length(v / 100.0 * viewport_w),
        Dimension::Calc { px, vw, vh } => taffy::LengthPercentage::Length(Dimension::calc_to_px(
            px, vw, vh, viewport_w, viewport_h,
        )),
    }
}

fn edges_dim_to_taffy_rect(
    e: EdgesDim,
    viewport_w: f32,
    viewport_h: f32,
) -> taffy::Rect<taffy::LengthPercentage> {
    taffy::Rect {
        left: dim_to_length_percentage(e.left, viewport_w, viewport_h),
        right: dim_to_length_percentage(e.right, viewport_w, viewport_h),
        top: dim_to_length_percentage(e.top, viewport_w, viewport_h),
        bottom: dim_to_length_percentage(e.bottom, viewport_w, viewport_h),
    }
}

fn opt_dim_to_taffy_auto(
    d: Option<Dimension>,
    viewport_w: f32,
    viewport_h: f32,
) -> taffy::LengthPercentageAuto {
    match d {
        None | Some(Dimension::Auto) => taffy::LengthPercentageAuto::Auto,
        Some(Dimension::Px(v)) => taffy::LengthPercentageAuto::Length(v),
        Some(Dimension::Percent(v)) => taffy::LengthPercentageAuto::Percent(v / 100.0),
        Some(Dimension::Vh(v)) => taffy::LengthPercentageAuto::Length(v / 100.0 * viewport_h),
        Some(Dimension::Vw(v)) => taffy::LengthPercentageAuto::Length(v / 100.0 * viewport_w),
        Some(Dimension::Calc { px, vw, vh }) => taffy::LengthPercentageAuto::Length(
            Dimension::calc_to_px(px, vw, vh, viewport_w, viewport_h),
        ),
    }
}

fn edges_to_taffy_rect_auto(
    e: Edges,
    auto: EdgeAutoFlags,
) -> taffy::Rect<taffy::LengthPercentageAuto> {
    let value = |v, is_auto| {
        if is_auto {
            taffy::LengthPercentageAuto::Auto
        } else {
            taffy::LengthPercentageAuto::Length(v)
        }
    };
    taffy::Rect {
        left: value(e.left, auto.left),
        right: value(e.right, auto.right),
        top: value(e.top, auto.top),
        bottom: value(e.bottom, auto.bottom),
    }
}

fn grid_min_track_to_taffy(m: GridMinTrackSize) -> taffy::MinTrackSizingFunction {
    match m {
        GridMinTrackSize::Px(v) => {
            taffy::MinTrackSizingFunction::Fixed(taffy::LengthPercentage::Length(v))
        }
        GridMinTrackSize::Percent(v) => {
            taffy::MinTrackSizingFunction::Fixed(taffy::LengthPercentage::Percent(v / 100.0))
        }
        GridMinTrackSize::Auto => taffy::MinTrackSizingFunction::Auto,
        GridMinTrackSize::MinContent => taffy::MinTrackSizingFunction::MinContent,
        GridMinTrackSize::MaxContent => taffy::MinTrackSizingFunction::MaxContent,
    }
}

fn grid_max_track_to_taffy(m: GridMaxTrackSize) -> taffy::MaxTrackSizingFunction {
    match m {
        GridMaxTrackSize::Px(v) => {
            taffy::MaxTrackSizingFunction::Fixed(taffy::LengthPercentage::Length(v))
        }
        GridMaxTrackSize::Percent(v) => {
            taffy::MaxTrackSizingFunction::Fixed(taffy::LengthPercentage::Percent(v / 100.0))
        }
        GridMaxTrackSize::Auto => taffy::MaxTrackSizingFunction::Auto,
        GridMaxTrackSize::MinContent => taffy::MaxTrackSizingFunction::MinContent,
        GridMaxTrackSize::MaxContent => taffy::MaxTrackSizingFunction::MaxContent,
        GridMaxTrackSize::Fr(v) => taffy::MaxTrackSizingFunction::Fraction(v),
        GridMaxTrackSize::FitContent(v) => {
            taffy::MaxTrackSizingFunction::FitContent(taffy::LengthPercentage::Length(v))
        }
        GridMaxTrackSize::FitContentPercent(v) => {
            taffy::MaxTrackSizingFunction::FitContent(taffy::LengthPercentage::Percent(v / 100.0))
        }
    }
}

fn grid_track_size_to_taffy(t: &GridTrackSize) -> taffy::NonRepeatedTrackSizingFunction {
    taffy::geometry::MinMax {
        min: grid_min_track_to_taffy(t.min),
        max: grid_max_track_to_taffy(t.max),
    }
}

fn grid_track_defs_to_taffy(defs: &[GridTrackDef]) -> Vec<taffy::TrackSizingFunction> {
    defs.iter()
        .map(|d| match d {
            GridTrackDef::Single(t) => {
                taffy::TrackSizingFunction::Single(grid_track_size_to_taffy(t))
            }
            GridTrackDef::Repeat(count, tracks) => {
                let repetition = match count {
                    GridRepeatCount::Count(n) => taffy::GridTrackRepetition::Count(*n),
                    GridRepeatCount::AutoFill => taffy::GridTrackRepetition::AutoFill,
                    GridRepeatCount::AutoFit => taffy::GridTrackRepetition::AutoFit,
                };
                let track_list: Vec<taffy::NonRepeatedTrackSizingFunction> =
                    tracks.iter().map(grid_track_size_to_taffy).collect();
                taffy::TrackSizingFunction::Repeat(repetition, track_list)
            }
        })
        .collect()
}

fn grid_auto_tracks_to_taffy(
    tracks: &[GridTrackSize],
) -> Vec<taffy::NonRepeatedTrackSizingFunction> {
    tracks.iter().map(grid_track_size_to_taffy).collect()
}

fn grid_placement_to_taffy(p: GridPlacement) -> taffy::GridPlacement {
    match p {
        GridPlacement::Auto => taffy::GridPlacement::Auto,
        GridPlacement::Line(n) => taffy::style_helpers::line::<taffy::GridPlacement>(n),
        GridPlacement::Span(n) => taffy::GridPlacement::Span(n),
    }
}

impl ComputedStyle {
    /// Apply the inherited runtime font scale once to this element's own
    /// declared font size. Inherited font sizes are already effective values
    /// from the parent and must not be multiplied again.
    pub fn apply_font_size_scale(&mut self) {
        if self.font_size_explicit {
            self.font_size *= self.font_size_scale;
        }
        self.font_size_explicit = true;
    }

    /// Scale all dimensional properties (sizes, spacing, fonts) by a factor.
    /// Used to convert logical CSS pixels to physical pixels for HiDPI displays.
    pub fn scale_by(&mut self, s: f32) {
        // Sizes
        self.width = scale_dim(self.width, s);
        self.height = scale_dim(self.height, s);
        self.min_width = scale_dim(self.min_width, s);
        self.min_height = scale_dim(self.min_height, s);
        self.max_width = scale_dim(self.max_width, s);
        self.max_height = scale_dim(self.max_height, s);

        // Spacing. Scale both the resolved f32 mirror and the unit-preserving
        // source; scale_dim leaves vh/vw/percent untouched (they resolve against
        // the already-physical viewport in to_taffy_style), matching the f32
        // mirror which stays 0 for those units.
        self.padding = scale_edges(self.padding, s);
        self.padding_src = scale_edges_dim(self.padding_src, s);
        self.margin = scale_edges(self.margin, s);
        self.row_gap *= s;
        self.column_gap *= s;

        // Grid track pixel values
        for def in &mut self.grid_template_columns {
            scale_grid_track_def(def, s);
        }
        for def in &mut self.grid_template_rows {
            scale_grid_track_def(def, s);
        }
        for t in &mut self.grid_auto_columns {
            scale_grid_track_size(t, s);
        }
        for t in &mut self.grid_auto_rows {
            scale_grid_track_size(t, s);
        }

        self.top = self.top.map(|d| scale_dim(d, s));
        self.right = self.right.map(|d| scale_dim(d, s));
        self.bottom = self.bottom.map(|d| scale_dim(d, s));
        self.left = self.left.map(|d| scale_dim(d, s));

        // Borders
        self.border_width = scale_edges(self.border_width, s);
        self.border_radius = scale_corners(self.border_radius, s);
        self.border_radius_src = scale_corners_dim(self.border_radius_src, s);

        // Box shadow
        for shadow in &mut self.box_shadow {
            shadow.offset_x *= s;
            shadow.offset_y *= s;
            shadow.blur_radius *= s;
            shadow.spread_radius *= s;
        }

        // Outline
        self.outline_width *= s;
        self.outline_offset *= s;

        // Text
        self.font_size *= s;
        self.letter_spacing *= s;
    }
}

fn scale_dim(d: Dimension, s: f32) -> Dimension {
    match d {
        Dimension::Px(v) => Dimension::Px(v * s),
        // Only the constant px term scales; viewport fractions are
        // scale-independent (like Percent / Vw / Vh).
        Dimension::Calc { px, vw, vh } => Dimension::Calc { px: px * s, vw, vh },
        other => other, // Auto, Percent, Vw, Vh are scale-independent
    }
}

fn scale_edges(e: Edges, s: f32) -> Edges {
    Edges { top: e.top * s, right: e.right * s, bottom: e.bottom * s, left: e.left * s }
}

fn scale_edges_dim(e: EdgesDim, s: f32) -> EdgesDim {
    EdgesDim {
        top: scale_dim(e.top, s),
        right: scale_dim(e.right, s),
        bottom: scale_dim(e.bottom, s),
        left: scale_dim(e.left, s),
    }
}

fn scale_corners(c: Corners, s: f32) -> Corners {
    Corners {
        top_left: c.top_left * s,
        top_right: c.top_right * s,
        bottom_right: c.bottom_right * s,
        bottom_left: c.bottom_left * s,
    }
}

fn scale_corner_dim(c: LengthOrPercent, s: f32) -> LengthOrPercent {
    match c {
        LengthOrPercent::Px(v) => LengthOrPercent::Px(v * s),
        // Percent corners resolve against the (already-scaled) box at paint
        // time, so they are scale-independent.
        other => other,
    }
}

fn scale_corners_dim(c: CornersDim, s: f32) -> CornersDim {
    CornersDim {
        top_left: scale_corner_dim(c.top_left, s),
        top_right: scale_corner_dim(c.top_right, s),
        bottom_right: scale_corner_dim(c.bottom_right, s),
        bottom_left: scale_corner_dim(c.bottom_left, s),
    }
}

fn scale_grid_min_track(m: &mut GridMinTrackSize, s: f32) {
    if let GridMinTrackSize::Px(ref mut v) = m {
        *v *= s;
    }
}

fn scale_grid_max_track(m: &mut GridMaxTrackSize, s: f32) {
    match m {
        GridMaxTrackSize::Px(ref mut v) => *v *= s,
        GridMaxTrackSize::FitContent(ref mut v) => *v *= s,
        _ => {}
    }
}

fn scale_grid_track_size(t: &mut GridTrackSize, s: f32) {
    scale_grid_min_track(&mut t.min, s);
    scale_grid_max_track(&mut t.max, s);
}

fn scale_grid_track_def(d: &mut GridTrackDef, s: f32) {
    match d {
        GridTrackDef::Single(ref mut t) => scale_grid_track_size(t, s),
        GridTrackDef::Repeat(_, ref mut tracks) => {
            for t in tracks {
                scale_grid_track_size(t, s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vh_resolves_against_viewport_height() {
        // 50vh with a 600px viewport height should resolve to 300px.
        let taffy_dim = dim_to_taffy(Dimension::Vh(50.0), 800.0, 600.0);
        assert_eq!(taffy_dim, taffy::Dimension::Length(300.0));
    }

    #[test]
    fn vw_resolves_against_viewport_width() {
        // 25vw with an 800px viewport width should resolve to 200px.
        let taffy_dim = dim_to_taffy(Dimension::Vw(25.0), 800.0, 600.0);
        assert_eq!(taffy_dim, taffy::Dimension::Length(200.0));
    }

    #[test]
    fn vh_vw_zero_yields_zero_length() {
        assert_eq!(dim_to_taffy(Dimension::Vh(0.0), 800.0, 600.0), taffy::Dimension::Length(0.0));
        assert_eq!(dim_to_taffy(Dimension::Vw(0.0), 800.0, 600.0), taffy::Dimension::Length(0.0));
    }

    #[test]
    fn opt_dim_vh_vw_resolves() {
        assert_eq!(
            opt_dim_to_taffy_auto(Some(Dimension::Vh(100.0)), 800.0, 600.0),
            taffy::LengthPercentageAuto::Length(600.0)
        );
        assert_eq!(
            opt_dim_to_taffy_auto(Some(Dimension::Vw(10.0)), 800.0, 600.0),
            taffy::LengthPercentageAuto::Length(80.0)
        );
    }

    #[test]
    fn scale_dim_leaves_vh_vw_unchanged() {
        // Viewport units are scale-independent (like Percent).
        assert_eq!(scale_dim(Dimension::Vh(50.0), 2.0), Dimension::Vh(50.0));
        assert_eq!(scale_dim(Dimension::Vw(25.0), 1.5), Dimension::Vw(25.0));
    }

    #[test]
    fn to_taffy_style_applies_viewport_to_max_height() {
        let mut style = ComputedStyle::default();
        style.max_height = Dimension::Vh(80.0);
        let taffy_style = style.to_taffy_style(1000.0, 500.0);
        // 80vh of a 500px-tall viewport = 400px.
        assert_eq!(taffy_style.max_size.height, taffy::Dimension::Length(400.0));
    }

    #[test]
    fn to_taffy_style_resolves_calc_against_viewport() {
        // `calc(100vw - 48px)` at a 1000px-wide viewport = 952px.
        let mut style = ComputedStyle::default();
        style.max_width = Dimension::Calc { px: -48.0, vw: 100.0, vh: 0.0 };
        let taffy_style = style.to_taffy_style(1000.0, 500.0);
        assert_eq!(taffy_style.max_size.width, taffy::Dimension::Length(952.0));
        // `calc(72vh - 46px)` at a 500px-tall viewport = 360 - 46 = 314px.
        style.max_height = Dimension::Calc { px: -46.0, vw: 0.0, vh: 72.0 };
        let taffy_style = style.to_taffy_style(1000.0, 500.0);
        assert_eq!(taffy_style.max_size.height, taffy::Dimension::Length(314.0));
    }

    #[test]
    fn to_taffy_style_resolves_vh_padding_against_viewport() {
        let mut style = ComputedStyle::default();
        style.padding_src = EdgesDim { top: Dimension::Vh(12.0), ..EdgesDim::ZERO };
        let taffy_style = style.to_taffy_style(800.0, 1000.0);
        // 12vh of a 1000px-tall viewport = 120px.
        assert_eq!(taffy_style.padding.top, taffy::LengthPercentage::Length(120.0));
        assert_eq!(taffy_style.padding.left, taffy::LengthPercentage::Length(0.0));
    }

    #[test]
    fn to_taffy_style_maps_percent_padding_to_taffy_percent() {
        let mut style = ComputedStyle::default();
        style.padding_src = EdgesDim::ZERO;
        style.padding_src.left = Dimension::Percent(10.0);
        let taffy_style = style.to_taffy_style(800.0, 600.0);
        assert_eq!(taffy_style.padding.left, taffy::LengthPercentage::Percent(0.10));
    }

    #[test]
    fn scale_by_scales_px_padding_src_but_leaves_vh_for_the_viewport() {
        let mut style = ComputedStyle::default();
        style.padding_src =
            EdgesDim { top: Dimension::Vh(50.0), ..EdgesDim::from_px(Edges::all(8.0)) };
        style.scale_by(2.0);
        // px source scales with HiDPI...
        assert_eq!(style.padding_src.left, Dimension::Px(16.0));
        // ...vh stays unit-relative and resolves against the physical viewport.
        assert_eq!(style.padding_src.top, Dimension::Vh(50.0));
        let taffy_style = style.to_taffy_style(800.0, 600.0);
        assert_eq!(taffy_style.padding.top, taffy::LengthPercentage::Length(300.0));
    }

    #[test]
    fn to_taffy_style_defaults_grid_justify_items_to_stretch() {
        let style = ComputedStyle { display: Display::Grid, ..Default::default() };
        let taffy_style = style.to_taffy_style(800.0, 600.0);
        assert_eq!(taffy_style.justify_items, Some(taffy::AlignItems::Stretch));
        assert_eq!(taffy_style.justify_content, Some(taffy::JustifyContent::Stretch));
    }

    #[test]
    fn to_taffy_style_defaults_flex_justify_content_to_start() {
        let style = ComputedStyle { display: Display::Flex, ..Default::default() };
        let taffy_style = style.to_taffy_style(800.0, 600.0);
        assert_eq!(taffy_style.justify_content, Some(taffy::JustifyContent::FlexStart));
    }

    #[test]
    fn justify_content_stretch_maps_to_taffy_stretch() {
        assert_eq!(
            justify_content_to_taffy(JustifyContent::Stretch, Display::Flex),
            taffy::JustifyContent::Stretch
        );
        let style = ComputedStyle {
            display: Display::Flex,
            justify_content: JustifyContent::Stretch,
            ..Default::default()
        };
        let taffy_style = style.to_taffy_style(800.0, 600.0);
        assert_eq!(taffy_style.justify_content, Some(taffy::JustifyContent::Stretch));
    }

    #[test]
    fn to_taffy_style_maps_align_self() {
        let style = ComputedStyle { align_self: AlignSelf::Center, ..Default::default() };
        let taffy_style = style.to_taffy_style(800.0, 600.0);
        assert_eq!(taffy_style.align_self, Some(taffy::AlignSelf::Center));
    }

    #[test]
    fn text_transform_helpers_match_ascii_css_cases() {
        assert_eq!(
            apply_text_transform("settings · appearance", TextTransform::Uppercase),
            "SETTINGS · APPEARANCE"
        );
        assert_eq!(apply_text_transform("SETTINGS", TextTransform::Lowercase), "settings");
        assert_eq!(apply_text_transform("danger zone", TextTransform::Capitalize), "Danger Zone");
    }
}
