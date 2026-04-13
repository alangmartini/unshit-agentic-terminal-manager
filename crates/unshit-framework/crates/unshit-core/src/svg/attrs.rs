//! Presentation attribute parser for the supported SVG subset.
//!
//! Presentation attributes in SVG are not CSS. They have their own tiny
//! grammars for `fill`, `stroke`, `stroke-width`, `viewBox`, `transform`,
//! and `points`. We parse each one into a typed value and refuse to accept
//! anything outside the supported subset.
//!
//! Colors accept hex (`#rrggbb`, `#rgb`), `rgb(r, g, b)`, `rgba(r, g, b, a)`,
//! the SVG 1.1 basic color keywords, plus the `none` and `currentColor`
//! keywords for paints.

use super::types::{StrokeLineCap, StrokeLineJoin, SvgPaint, SvgTransform, ViewBox};
use crate::style::types::Color;

/// Error returned by the attribute parsers. Callers decide whether to log
/// and fall back or reject the whole element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SvgAttrError {
    InvalidColor,
    InvalidNumber,
    InvalidViewBox,
    InvalidTransform,
    InvalidPoints,
    InvalidLineCap,
    InvalidLineJoin,
}

/// Parse an f32 from an attribute value. Leading and trailing whitespace is
/// tolerated; unit suffixes (e.g. `px`) are stripped.
pub fn parse_attribute_f32(input: &str) -> Result<f32, SvgAttrError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(SvgAttrError::InvalidNumber);
    }

    // Strip a trailing unit. We accept `px`, `pt`, and plain numbers. For
    // the SVG subset user units are treated as pixels.
    let stripped =
        trimmed.strip_suffix("px").or_else(|| trimmed.strip_suffix("pt")).unwrap_or(trimmed);

    stripped.trim().parse::<f32>().map_err(|_| SvgAttrError::InvalidNumber)
}

/// Parse a paint value: a color, `none`, or `currentColor`.
pub fn parse_color_attr(input: &str) -> Result<SvgPaint, SvgAttrError> {
    let value = input.trim();
    if value.is_empty() {
        return Err(SvgAttrError::InvalidColor);
    }
    if value.eq_ignore_ascii_case("none") {
        return Ok(SvgPaint::None);
    }
    if value.eq_ignore_ascii_case("currentColor") {
        return Ok(SvgPaint::Current);
    }

    parse_color_literal(value).map(SvgPaint::Solid)
}

/// Parse a concrete color literal. Used both for paint values and for
/// future attributes that only accept a fully specified color.
pub fn parse_color_literal(input: &str) -> Result<Color, SvgAttrError> {
    let value = input.trim();

    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex).ok_or(SvgAttrError::InvalidColor);
    }

    if let Some(rest) = strip_function(value, "rgba") {
        return parse_rgb_tuple(rest, true).ok_or(SvgAttrError::InvalidColor);
    }
    if let Some(rest) = strip_function(value, "rgb") {
        return parse_rgb_tuple(rest, false).ok_or(SvgAttrError::InvalidColor);
    }

    named_color(value).ok_or(SvgAttrError::InvalidColor)
}

fn strip_function<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    let lower = input.to_ascii_lowercase();
    if !lower.starts_with(name) {
        return None;
    }
    let rest = &input[name.len()..];
    let rest = rest.trim_start();
    let inside = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inside)
}

fn parse_rgb_tuple(body: &str, expect_alpha: bool) -> Option<Color> {
    let parts: Vec<&str> = body.split(',').map(str::trim).collect();
    let expected = if expect_alpha { 4 } else { 3 };
    if parts.len() != expected {
        return None;
    }
    let r = parse_byte(parts[0])?;
    let g = parse_byte(parts[1])?;
    let b = parse_byte(parts[2])?;
    let a = if expect_alpha { parse_alpha(parts[3])? } else { 255 };
    Some(Color::rgba(r, g, b, a))
}

fn parse_byte(s: &str) -> Option<u8> {
    if let Some(pct) = s.strip_suffix('%') {
        let v: f32 = pct.parse().ok()?;
        return Some((v / 100.0 * 255.0).clamp(0.0, 255.0) as u8);
    }
    let v: f32 = s.parse().ok()?;
    Some(v.clamp(0.0, 255.0) as u8)
}

fn parse_alpha(s: &str) -> Option<u8> {
    if let Some(pct) = s.strip_suffix('%') {
        let v: f32 = pct.parse().ok()?;
        return Some((v / 100.0 * 255.0).clamp(0.0, 255.0) as u8);
    }
    let v: f32 = s.parse().ok()?;
    if (0.0..=1.0).contains(&v) {
        Some((v * 255.0).clamp(0.0, 255.0) as u8)
    } else {
        Some(v.clamp(0.0, 255.0) as u8)
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let bytes = hex.as_bytes();
    let digit = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    match bytes.len() {
        3 => {
            let r = digit(bytes[0])?;
            let g = digit(bytes[1])?;
            let b = digit(bytes[2])?;
            Some(Color::rgb((r << 4) | r, (g << 4) | g, (b << 4) | b))
        }
        4 => {
            let r = digit(bytes[0])?;
            let g = digit(bytes[1])?;
            let b = digit(bytes[2])?;
            let a = digit(bytes[3])?;
            Some(Color::rgba((r << 4) | r, (g << 4) | g, (b << 4) | b, (a << 4) | a))
        }
        6 => {
            let r = (digit(bytes[0])? << 4) | digit(bytes[1])?;
            let g = (digit(bytes[2])? << 4) | digit(bytes[3])?;
            let b = (digit(bytes[4])? << 4) | digit(bytes[5])?;
            Some(Color::rgb(r, g, b))
        }
        8 => {
            let r = (digit(bytes[0])? << 4) | digit(bytes[1])?;
            let g = (digit(bytes[2])? << 4) | digit(bytes[3])?;
            let b = (digit(bytes[4])? << 4) | digit(bytes[5])?;
            let a = (digit(bytes[6])? << 4) | digit(bytes[7])?;
            Some(Color::rgba(r, g, b, a))
        }
        _ => None,
    }
}

fn named_color(name: &str) -> Option<Color> {
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "transparent" => Color::TRANSPARENT,
        "white" => Color::WHITE,
        "black" => Color::BLACK,
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "orange" => Color::rgb(255, 165, 0),
        "purple" => Color::rgb(128, 0, 128),
        "pink" => Color::rgb(255, 192, 203),
        "lime" => Color::rgb(0, 255, 0),
        "navy" => Color::rgb(0, 0, 128),
        "teal" => Color::rgb(0, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        _ => return None,
    })
}

/// Parse the four numbers in a `viewBox` attribute.
pub fn parse_view_box(input: &str) -> Result<ViewBox, SvgAttrError> {
    let parts: Vec<f32> = split_numeric(input)
        .iter()
        .map(|s| s.parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SvgAttrError::InvalidViewBox)?;
    if parts.len() != 4 {
        return Err(SvgAttrError::InvalidViewBox);
    }
    let mut vb = ViewBox::new(parts[0], parts[1], parts[2], parts[3]);
    // Empty viewBox falls back to 1x1 to avoid divide by zero later.
    if vb.width <= 0.0 || vb.height <= 0.0 {
        log::warn!("svg viewBox has non positive size, falling back to 1x1");
        vb.width = 1.0;
        vb.height = 1.0;
    }
    Ok(vb)
}

/// Parse a `points` attribute (used by polyline and polygon).
pub fn parse_points(input: &str) -> Result<Vec<(f32, f32)>, SvgAttrError> {
    let nums: Vec<f32> = split_numeric(input)
        .iter()
        .map(|s| s.parse::<f32>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SvgAttrError::InvalidPoints)?;
    if nums.is_empty() || (nums.len() & 1) != 0 {
        return Err(SvgAttrError::InvalidPoints);
    }
    Ok(nums.chunks_exact(2).map(|c| (c[0], c[1])).collect())
}

/// Parse a `stroke-linecap` value.
pub fn parse_stroke_linecap(input: &str) -> Result<StrokeLineCap, SvgAttrError> {
    match input.trim().to_ascii_lowercase().as_str() {
        "butt" => Ok(StrokeLineCap::Butt),
        "round" => Ok(StrokeLineCap::Round),
        "square" => Ok(StrokeLineCap::Square),
        _ => Err(SvgAttrError::InvalidLineCap),
    }
}

/// Parse a `stroke-linejoin` value.
pub fn parse_stroke_linejoin(input: &str) -> Result<StrokeLineJoin, SvgAttrError> {
    match input.trim().to_ascii_lowercase().as_str() {
        "miter" => Ok(StrokeLineJoin::Miter),
        "round" => Ok(StrokeLineJoin::Round),
        "bevel" => Ok(StrokeLineJoin::Bevel),
        _ => Err(SvgAttrError::InvalidLineJoin),
    }
}

/// Parse a `transform` attribute value. Supports `translate`, `scale`,
/// `rotate`, and `matrix` chains, composed left to right.
pub fn parse_transform(input: &str) -> Result<SvgTransform, SvgAttrError> {
    let mut result = SvgTransform::IDENTITY;
    let mut rest = input.trim();

    while !rest.is_empty() {
        let open = rest.find('(').ok_or(SvgAttrError::InvalidTransform)?;
        let close = rest.find(')').ok_or(SvgAttrError::InvalidTransform)?;
        if close <= open {
            return Err(SvgAttrError::InvalidTransform);
        }
        let name = rest[..open].trim().to_ascii_lowercase();
        let body = &rest[open + 1..close];
        let nums: Vec<f32> = split_numeric(body)
            .iter()
            .map(|s| s.parse::<f32>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SvgAttrError::InvalidTransform)?;

        let step = match name.as_str() {
            "translate" => match nums.len() {
                1 => SvgTransform::translate(nums[0], 0.0),
                2 => SvgTransform::translate(nums[0], nums[1]),
                _ => return Err(SvgAttrError::InvalidTransform),
            },
            "scale" => match nums.len() {
                1 => SvgTransform::scale(nums[0], nums[0]),
                2 => SvgTransform::scale(nums[0], nums[1]),
                _ => return Err(SvgAttrError::InvalidTransform),
            },
            "rotate" => match nums.len() {
                1 => SvgTransform::rotate_deg(nums[0]),
                3 => {
                    // Rotate around a point = translate in, rotate, translate out.
                    let t1 = SvgTransform::translate(nums[1], nums[2]);
                    let r = SvgTransform::rotate_deg(nums[0]);
                    let t2 = SvgTransform::translate(-nums[1], -nums[2]);
                    t1.multiply(r).multiply(t2)
                }
                _ => return Err(SvgAttrError::InvalidTransform),
            },
            "matrix" => {
                if nums.len() != 6 {
                    return Err(SvgAttrError::InvalidTransform);
                }
                SvgTransform::new(nums[0], nums[1], nums[2], nums[3], nums[4], nums[5])
            }
            _ => return Err(SvgAttrError::InvalidTransform),
        };

        result = result.multiply(step);

        rest = rest[close + 1..].trim_start();
        // Skip optional comma between chained transforms.
        if let Some(stripped) = rest.strip_prefix(',') {
            rest = stripped.trim_start();
        }
    }

    Ok(result)
}

/// Split an input string into a list of numeric tokens using whitespace and
/// commas as delimiters. Handles signed numbers and exponents.
fn split_numeric(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_is_digit = false;
    for ch in input.chars() {
        if ch.is_whitespace() || ch == ',' {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            prev_is_digit = false;
            continue;
        }
        if (ch == '-' || ch == '+') && prev_is_digit {
            // Check if the preceding char was the exponent marker; if so
            // keep going, otherwise start a new token.
            let last = current.chars().last();
            if !matches!(last, Some('e') | Some('E')) {
                out.push(std::mem::take(&mut current));
            }
        }
        current.push(ch);
        prev_is_digit = ch.is_ascii_digit() || ch == '.' || ch == 'e' || ch == 'E';
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        assert_eq!(parse_color_literal("#fff").unwrap(), Color::rgb(255, 255, 255));
        assert_eq!(parse_color_literal("#ff0000").unwrap(), Color::rgb(255, 0, 0));
        assert_eq!(parse_color_literal("#80808080").unwrap(), Color::rgba(128, 128, 128, 128));
    }

    #[test]
    fn parses_rgb_and_rgba() {
        assert_eq!(parse_color_literal("rgb(0, 128, 255)").unwrap(), Color::rgb(0, 128, 255));
        assert_eq!(
            parse_color_literal("rgba(10, 20, 30, 0.5)").unwrap(),
            Color::rgba(10, 20, 30, 127)
        );
    }

    #[test]
    fn parses_named_colors() {
        assert_eq!(parse_color_literal("black").unwrap(), Color::BLACK);
        assert_eq!(parse_color_literal("white").unwrap(), Color::WHITE);
    }

    #[test]
    fn parses_none_and_current_color_as_paints() {
        assert_eq!(parse_color_attr("none").unwrap(), SvgPaint::None);
        assert_eq!(parse_color_attr("currentColor").unwrap(), SvgPaint::Current);
    }

    #[test]
    fn parses_view_box_with_four_numbers() {
        let vb = parse_view_box("0 0 100 50").unwrap();
        assert_eq!(vb, ViewBox::new(0.0, 0.0, 100.0, 50.0));
    }

    #[test]
    fn view_box_empty_falls_back_to_one() {
        let vb = parse_view_box("0 0 0 0").unwrap();
        assert!(vb.width > 0.0);
        assert!(vb.height > 0.0);
    }

    #[test]
    fn view_box_rejects_bad_count() {
        assert!(parse_view_box("0 0 100").is_err());
        assert!(parse_view_box("0 0 100 50 10").is_err());
    }

    #[test]
    fn parses_points_comma_and_whitespace() {
        let p = parse_points("10,20 30,40 50,60").unwrap();
        assert_eq!(p, vec![(10.0, 20.0), (30.0, 40.0), (50.0, 60.0)]);
    }

    #[test]
    fn points_rejects_odd_count() {
        assert!(parse_points("1 2 3").is_err());
    }

    #[test]
    fn parses_transform_translate() {
        let t = parse_transform("translate(10 20)").unwrap();
        assert_eq!(t, SvgTransform::translate(10.0, 20.0));
    }

    #[test]
    fn parses_transform_single_arg_translate() {
        let t = parse_transform("translate(10)").unwrap();
        assert_eq!(t, SvgTransform::translate(10.0, 0.0));
    }

    #[test]
    fn parses_transform_scale() {
        let t = parse_transform("scale(2 3)").unwrap();
        assert_eq!(t, SvgTransform::scale(2.0, 3.0));
    }

    #[test]
    fn parses_transform_single_arg_scale_is_uniform() {
        let t = parse_transform("scale(2)").unwrap();
        assert_eq!(t, SvgTransform::scale(2.0, 2.0));
    }

    #[test]
    fn parses_transform_rotate() {
        let t = parse_transform("rotate(90)").unwrap();
        // Rotating (1, 0) by 90 degrees gives (0, 1).
        let (x, y) = t.apply(1.0, 0.0);
        assert!(x.abs() < 1e-5, "expected 0, got {}", x);
        assert!((y - 1.0).abs() < 1e-5, "expected 1, got {}", y);
    }

    #[test]
    fn parses_transform_matrix() {
        let t = parse_transform("matrix(1 2 3 4 5 6)").unwrap();
        assert_eq!(t, SvgTransform::new(1.0, 2.0, 3.0, 4.0, 5.0, 6.0));
    }

    #[test]
    fn parses_transform_chain_composes_left_to_right() {
        // translate(10 20) scale(2) applied to (1, 1) should yield
        // first scale ((2, 2)) then translate ((12, 22)).
        let t = parse_transform("translate(10 20) scale(2)").unwrap();
        let (x, y) = t.apply(1.0, 1.0);
        assert!((x - 12.0).abs() < 1e-5);
        assert!((y - 22.0).abs() < 1e-5);
    }

    #[test]
    fn transform_invalid_rejected() {
        assert!(parse_transform("wobble(1)").is_err());
        assert!(parse_transform("translate(").is_err());
    }

    #[test]
    fn parses_stroke_linecap() {
        assert_eq!(parse_stroke_linecap("butt").unwrap(), StrokeLineCap::Butt);
        assert_eq!(parse_stroke_linecap("round").unwrap(), StrokeLineCap::Round);
        assert_eq!(parse_stroke_linecap("square").unwrap(), StrokeLineCap::Square);
        assert!(parse_stroke_linecap("diagonal").is_err());
    }

    #[test]
    fn parses_stroke_linejoin() {
        assert_eq!(parse_stroke_linejoin("miter").unwrap(), StrokeLineJoin::Miter);
        assert_eq!(parse_stroke_linejoin("round").unwrap(), StrokeLineJoin::Round);
        assert_eq!(parse_stroke_linejoin("bevel").unwrap(), StrokeLineJoin::Bevel);
    }

    #[test]
    fn invalid_color_returns_err() {
        assert!(parse_color_literal("#zzz").is_err());
        assert!(parse_color_literal("not-a-color").is_err());
    }

    #[test]
    fn attribute_f32_accepts_units() {
        assert!((parse_attribute_f32("12px").unwrap() - 12.0).abs() < 1e-6);
        assert!((parse_attribute_f32(" -3.5 ").unwrap() + 3.5).abs() < 1e-6);
    }
}
