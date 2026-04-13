//! Hand rolled SVG path mini language parser.
//!
//! Supports the full SVG 1.1 path grammar for straight and curved primitives:
//! `M`, `L`, `H`, `V`, `C`, `S`, `Q`, `T`, `A`, `Z` in both absolute and
//! relative forms. Numbers accept exponent notation. Arc commands are
//! decomposed into cubic beziers using the w3c implementation notes on
//! elliptical arc to curve conversion, so downstream consumers only ever
//! deal with `MoveTo`, `LineTo`, `CubicTo`, `QuadTo`, and `Close`.
//!
//! Malformed input returns `SvgPathError` with the byte offset where the
//! parser gave up. The caller is expected to log the error and fall back to
//! rendering nothing for that element.

use super::types::PathCommand;

/// Reasons a path string can fail to parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SvgPathError {
    /// The first non whitespace character was not a move to.
    MustStartWithMoveTo { offset: usize },
    /// Unknown command letter.
    UnknownCommand { offset: usize, byte: u8 },
    /// Expected a number and got something else.
    ExpectedNumber { offset: usize },
    /// Arc flag must be exactly `0` or `1`.
    InvalidFlag { offset: usize },
    /// Trailing garbage after the last command.
    TrailingGarbage { offset: usize },
}

/// Parse a path data string into a vector of absolute commands.
pub fn parse_svg_path(input: &str) -> Result<Vec<PathCommand>, SvgPathError> {
    let mut parser = Parser::new(input);
    let mut out = Vec::new();
    parser.parse_commands(&mut out)?;
    parser.skip_ws();
    if parser.cursor < parser.bytes.len() {
        return Err(SvgPathError::TrailingGarbage { offset: parser.cursor });
    }
    Ok(out)
}

struct Parser<'a> {
    bytes: &'a [u8],
    cursor: usize,
    // Pen position, always in absolute coordinates.
    px: f32,
    py: f32,
    // Start of the current subpath (target of `Z`).
    start_x: f32,
    start_y: f32,
    // Control point reflection state for `S` and `T`.
    last_cubic_ctrl: Option<(f32, f32)>,
    last_quad_ctrl: Option<(f32, f32)>,
    // Last issued command letter, used for implicit repeats.
    last_cmd: Option<u8>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            cursor: 0,
            px: 0.0,
            py: 0.0,
            start_x: 0.0,
            start_y: 0.0,
            last_cubic_ctrl: None,
            last_quad_ctrl: None,
            last_cmd: None,
        }
    }

    fn parse_commands(&mut self, out: &mut Vec<PathCommand>) -> Result<(), SvgPathError> {
        self.skip_ws();
        if self.cursor >= self.bytes.len() {
            return Ok(());
        }

        // Path must begin with an M or m.
        let first = self.bytes[self.cursor];
        if first != b'M' && first != b'm' {
            return Err(SvgPathError::MustStartWithMoveTo { offset: self.cursor });
        }

        while self.cursor < self.bytes.len() {
            self.skip_ws();
            if self.cursor >= self.bytes.len() {
                break;
            }
            let byte = self.bytes[self.cursor];
            if byte.is_ascii_alphabetic() {
                self.cursor += 1;
                self.last_cmd = Some(byte);
                self.dispatch_command(byte, out)?;
            } else if is_number_start(byte) {
                // Implicit repeat: reuse the last command letter with the
                // convention that an `M` repeats as `L` (or `l` if the
                // original was lowercase).
                let last =
                    self.last_cmd.ok_or(SvgPathError::ExpectedNumber { offset: self.cursor })?;
                let implicit = match last {
                    b'M' => b'L',
                    b'm' => b'l',
                    other => other,
                };
                self.dispatch_command(implicit, out)?;
            } else {
                return Err(SvgPathError::ExpectedNumber { offset: self.cursor });
            }
        }

        Ok(())
    }

    fn dispatch_command(
        &mut self,
        byte: u8,
        out: &mut Vec<PathCommand>,
    ) -> Result<(), SvgPathError> {
        match byte {
            b'M' | b'm' => self.cmd_move_to(byte == b'm', out)?,
            b'L' | b'l' => self.cmd_line_to(byte == b'l', out)?,
            b'H' | b'h' => self.cmd_horizontal(byte == b'h', out)?,
            b'V' | b'v' => self.cmd_vertical(byte == b'v', out)?,
            b'C' | b'c' => self.cmd_cubic(byte == b'c', out)?,
            b'S' | b's' => self.cmd_smooth_cubic(byte == b's', out)?,
            b'Q' | b'q' => self.cmd_quad(byte == b'q', out)?,
            b'T' | b't' => self.cmd_smooth_quad(byte == b't', out)?,
            b'A' | b'a' => self.cmd_arc(byte == b'a', out)?,
            b'Z' | b'z' => {
                out.push(PathCommand::Close);
                self.px = self.start_x;
                self.py = self.start_y;
                self.last_cubic_ctrl = None;
                self.last_quad_ctrl = None;
            }
            other => return Err(SvgPathError::UnknownCommand { offset: self.cursor, byte: other }),
        }
        Ok(())
    }

    fn cmd_move_to(
        &mut self,
        relative: bool,
        out: &mut Vec<PathCommand>,
    ) -> Result<(), SvgPathError> {
        // Read the initial coordinate pair.
        let (x, y) = self.read_coord_pair(relative)?;
        self.px = x;
        self.py = y;
        self.start_x = x;
        self.start_y = y;
        out.push(PathCommand::MoveTo { x, y });
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;

        // Additional coordinate pairs after the first are implicit line tos.
        while self.peek_is_number() {
            let (x, y) = self.read_coord_pair(relative)?;
            out.push(PathCommand::LineTo { x, y });
            self.px = x;
            self.py = y;
        }
        Ok(())
    }

    fn cmd_line_to(
        &mut self,
        relative: bool,
        out: &mut Vec<PathCommand>,
    ) -> Result<(), SvgPathError> {
        let (x, y) = self.read_coord_pair(relative)?;
        self.px = x;
        self.py = y;
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
        out.push(PathCommand::LineTo { x, y });
        while self.peek_is_number() {
            let (x, y) = self.read_coord_pair(relative)?;
            self.px = x;
            self.py = y;
            out.push(PathCommand::LineTo { x, y });
        }
        Ok(())
    }

    fn cmd_horizontal(
        &mut self,
        relative: bool,
        out: &mut Vec<PathCommand>,
    ) -> Result<(), SvgPathError> {
        let mut x = self.read_number()?;
        if relative {
            x += self.px;
        }
        let y = self.py;
        self.px = x;
        out.push(PathCommand::LineTo { x, y });
        while self.peek_is_number() {
            let mut x = self.read_number()?;
            if relative {
                x += self.px;
            }
            self.px = x;
            out.push(PathCommand::LineTo { x, y: self.py });
        }
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn cmd_vertical(
        &mut self,
        relative: bool,
        out: &mut Vec<PathCommand>,
    ) -> Result<(), SvgPathError> {
        let mut y = self.read_number()?;
        if relative {
            y += self.py;
        }
        let x = self.px;
        self.py = y;
        out.push(PathCommand::LineTo { x, y });
        while self.peek_is_number() {
            let mut y = self.read_number()?;
            if relative {
                y += self.py;
            }
            self.py = y;
            out.push(PathCommand::LineTo { x: self.px, y });
        }
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn cmd_cubic(
        &mut self,
        relative: bool,
        out: &mut Vec<PathCommand>,
    ) -> Result<(), SvgPathError> {
        loop {
            let (x1, y1) = self.read_coord_pair(relative)?;
            let (x2, y2) = self.read_coord_pair(relative)?;
            let (x, y) = self.read_coord_pair(relative)?;
            out.push(PathCommand::CubicTo { x1, y1, x2, y2, x, y });
            self.last_cubic_ctrl = Some((x2, y2));
            self.last_quad_ctrl = None;
            self.px = x;
            self.py = y;
            if !self.peek_is_number() {
                break;
            }
        }
        Ok(())
    }

    fn cmd_smooth_cubic(
        &mut self,
        relative: bool,
        out: &mut Vec<PathCommand>,
    ) -> Result<(), SvgPathError> {
        loop {
            let (rx1, ry1) = match self.last_cubic_ctrl {
                // Reflection of the previous control point about the pen.
                Some((cx, cy)) => (2.0 * self.px - cx, 2.0 * self.py - cy),
                None => (self.px, self.py),
            };
            let (x2, y2) = self.read_coord_pair(relative)?;
            let (x, y) = self.read_coord_pair(relative)?;
            out.push(PathCommand::CubicTo { x1: rx1, y1: ry1, x2, y2, x, y });
            self.last_cubic_ctrl = Some((x2, y2));
            self.last_quad_ctrl = None;
            self.px = x;
            self.py = y;
            if !self.peek_is_number() {
                break;
            }
        }
        Ok(())
    }

    fn cmd_quad(&mut self, relative: bool, out: &mut Vec<PathCommand>) -> Result<(), SvgPathError> {
        loop {
            let (x1, y1) = self.read_coord_pair(relative)?;
            let (x, y) = self.read_coord_pair(relative)?;
            out.push(PathCommand::QuadTo { x1, y1, x, y });
            self.last_quad_ctrl = Some((x1, y1));
            self.last_cubic_ctrl = None;
            self.px = x;
            self.py = y;
            if !self.peek_is_number() {
                break;
            }
        }
        Ok(())
    }

    fn cmd_smooth_quad(
        &mut self,
        relative: bool,
        out: &mut Vec<PathCommand>,
    ) -> Result<(), SvgPathError> {
        loop {
            let (x1, y1) = match self.last_quad_ctrl {
                Some((cx, cy)) => (2.0 * self.px - cx, 2.0 * self.py - cy),
                None => (self.px, self.py),
            };
            let (x, y) = self.read_coord_pair(relative)?;
            out.push(PathCommand::QuadTo { x1, y1, x, y });
            self.last_quad_ctrl = Some((x1, y1));
            self.last_cubic_ctrl = None;
            self.px = x;
            self.py = y;
            if !self.peek_is_number() {
                break;
            }
        }
        Ok(())
    }

    fn cmd_arc(&mut self, relative: bool, out: &mut Vec<PathCommand>) -> Result<(), SvgPathError> {
        loop {
            let rx = self.read_number()?;
            self.skip_ws_or_comma();
            let ry = self.read_number()?;
            self.skip_ws_or_comma();
            let x_axis_rot_deg = self.read_number()?;
            self.skip_ws_or_comma();
            let large_arc = self.read_flag()?;
            self.skip_ws_or_comma();
            let sweep = self.read_flag()?;
            self.skip_ws_or_comma();
            let mut x = self.read_number()?;
            self.skip_ws_or_comma();
            let mut y = self.read_number()?;
            if relative {
                x += self.px;
                y += self.py;
            }

            arc_to_cubics(self.px, self.py, rx, ry, x_axis_rot_deg, large_arc, sweep, x, y, out);
            self.px = x;
            self.py = y;
            self.last_cubic_ctrl = None;
            self.last_quad_ctrl = None;
            if !self.peek_is_number() {
                break;
            }
        }
        Ok(())
    }

    // --- Lexer helpers ------------------------------------------------------

    fn skip_ws(&mut self) {
        while self.cursor < self.bytes.len() {
            let b = self.bytes[self.cursor];
            if is_wsp(b) {
                self.cursor += 1;
            } else {
                break;
            }
        }
    }

    fn skip_ws_or_comma(&mut self) {
        self.skip_ws();
        if self.cursor < self.bytes.len() && self.bytes[self.cursor] == b',' {
            self.cursor += 1;
            self.skip_ws();
        }
    }

    fn peek_is_number(&mut self) -> bool {
        self.skip_ws_or_comma();
        if self.cursor >= self.bytes.len() {
            return false;
        }
        is_number_start(self.bytes[self.cursor])
    }

    fn read_coord_pair(&mut self, relative: bool) -> Result<(f32, f32), SvgPathError> {
        self.skip_ws_or_comma();
        let mut x = self.read_number()?;
        self.skip_ws_or_comma();
        let mut y = self.read_number()?;
        if relative {
            x += self.px;
            y += self.py;
        }
        Ok((x, y))
    }

    fn read_number(&mut self) -> Result<f32, SvgPathError> {
        self.skip_ws();
        let start = self.cursor;
        if self.cursor >= self.bytes.len() {
            return Err(SvgPathError::ExpectedNumber { offset: start });
        }

        // Optional sign.
        if matches!(self.bytes[self.cursor], b'+' | b'-') {
            self.cursor += 1;
        }

        let mut has_digit = false;
        while self.cursor < self.bytes.len() && self.bytes[self.cursor].is_ascii_digit() {
            self.cursor += 1;
            has_digit = true;
        }

        if self.cursor < self.bytes.len() && self.bytes[self.cursor] == b'.' {
            self.cursor += 1;
            while self.cursor < self.bytes.len() && self.bytes[self.cursor].is_ascii_digit() {
                self.cursor += 1;
                has_digit = true;
            }
        }

        if !has_digit {
            return Err(SvgPathError::ExpectedNumber { offset: start });
        }

        // Optional exponent.
        if self.cursor < self.bytes.len() && matches!(self.bytes[self.cursor], b'e' | b'E') {
            self.cursor += 1;
            if self.cursor < self.bytes.len() && matches!(self.bytes[self.cursor], b'+' | b'-') {
                self.cursor += 1;
            }
            let exp_start = self.cursor;
            while self.cursor < self.bytes.len() && self.bytes[self.cursor].is_ascii_digit() {
                self.cursor += 1;
            }
            if self.cursor == exp_start {
                return Err(SvgPathError::ExpectedNumber { offset: start });
            }
        }

        let slice = &self.bytes[start..self.cursor];
        let s = std::str::from_utf8(slice)
            .map_err(|_| SvgPathError::ExpectedNumber { offset: start })?;
        s.parse::<f32>().map_err(|_| SvgPathError::ExpectedNumber { offset: start })
    }

    fn read_flag(&mut self) -> Result<bool, SvgPathError> {
        self.skip_ws();
        if self.cursor >= self.bytes.len() {
            return Err(SvgPathError::InvalidFlag { offset: self.cursor });
        }
        let b = self.bytes[self.cursor];
        let result = match b {
            b'0' => false,
            b'1' => true,
            _ => return Err(SvgPathError::InvalidFlag { offset: self.cursor }),
        };
        self.cursor += 1;
        Ok(result)
    }
}

fn is_wsp(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

fn is_number_start(b: u8) -> bool {
    b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'+'
}

/// Approximate an elliptical arc with cubic beziers.
///
/// Implements the `endpoint to center` conversion from the SVG 1.1 Appendix
/// F.6.5 plus the standard arc to cubic split that uses 4 segments of at
/// most pi/2 each. Degenerate cases (same start and end point, zero radius)
/// fall back to a simple line to.
#[allow(clippy::too_many_arguments)]
fn arc_to_cubics(
    x1: f32,
    y1: f32,
    mut rx: f32,
    mut ry: f32,
    x_axis_rot_deg: f32,
    large_arc: bool,
    sweep: bool,
    x2: f32,
    y2: f32,
    out: &mut Vec<PathCommand>,
) {
    if (x1 - x2).abs() < 1e-6 && (y1 - y2).abs() < 1e-6 {
        return;
    }
    rx = rx.abs();
    ry = ry.abs();
    if rx < 1e-6 || ry < 1e-6 {
        out.push(PathCommand::LineTo { x: x2, y: y2 });
        return;
    }

    let phi = x_axis_rot_deg.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();

    // Step 1: compute (x1', y1') in a rotated frame.
    let dx = (x1 - x2) * 0.5;
    let dy = (y1 - y2) * 0.5;
    let x1p = cos_phi * dx + sin_phi * dy;
    let y1p = -sin_phi * dx + cos_phi * dy;

    // Step 2: correct out of range radii.
    let mut rx_sq = rx * rx;
    let mut ry_sq = ry * ry;
    let x1p_sq = x1p * x1p;
    let y1p_sq = y1p * y1p;
    let radii_check = x1p_sq / rx_sq + y1p_sq / ry_sq;
    if radii_check > 1.0 {
        let scale = radii_check.sqrt();
        rx *= scale;
        ry *= scale;
        rx_sq = rx * rx;
        ry_sq = ry * ry;
    }

    // Step 3: compute the center (cx', cy') in the rotated frame.
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let numerator = (rx_sq * ry_sq - rx_sq * y1p_sq - ry_sq * x1p_sq).max(0.0);
    let denominator = rx_sq * y1p_sq + ry_sq * x1p_sq;
    let coef =
        if denominator.abs() < 1e-12 { 0.0 } else { sign * (numerator / denominator).sqrt() };
    let cxp = coef * (rx * y1p) / ry;
    let cyp = coef * -(ry * x1p) / rx;

    // Step 4: compute the center (cx, cy) in the original frame.
    let cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) * 0.5;
    let cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) * 0.5;

    // Step 5: compute theta1 and delta theta.
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = angle_between(1.0, 0.0, ux, uy);
    let mut delta_theta = angle_between(ux, uy, vx, vy);
    if !sweep && delta_theta > 0.0 {
        delta_theta -= std::f32::consts::TAU;
    } else if sweep && delta_theta < 0.0 {
        delta_theta += std::f32::consts::TAU;
    }

    // Number of segments to keep each bezier below pi/2.
    let segments = ((delta_theta.abs() / (std::f32::consts::FRAC_PI_2)).ceil() as i32).max(1);
    let delta = delta_theta / segments as f32;
    let t = (4.0 / 3.0) * (delta * 0.25).tan();

    let mut start_angle = theta1;
    let _ = (x1, y1);

    for _ in 0..segments {
        let end_angle = start_angle + delta;
        let (sin_start, cos_start) = start_angle.sin_cos();
        let (sin_end, cos_end) = end_angle.sin_cos();

        // Unit circle control points before ellipse transform.
        let e_x1 = cos_start - t * sin_start;
        let e_y1 = sin_start + t * cos_start;
        let e_x2 = cos_end + t * sin_end;
        let e_y2 = sin_end - t * cos_end;
        let end_ux = cos_end;
        let end_uy = sin_end;

        // Apply ellipse radii and rotation, then translate to center.
        let (c1x, c1y) = ellipse_to_world(e_x1, e_y1, rx, ry, sin_phi, cos_phi, cx, cy);
        let (c2x, c2y) = ellipse_to_world(e_x2, e_y2, rx, ry, sin_phi, cos_phi, cx, cy);
        let (ex, ey) = ellipse_to_world(end_ux, end_uy, rx, ry, sin_phi, cos_phi, cx, cy);

        out.push(PathCommand::CubicTo { x1: c1x, y1: c1y, x2: c2x, y2: c2y, x: ex, y: ey });

        start_angle = end_angle;
    }
}

fn angle_between(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    let dot = ux * vx + uy * vy;
    let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
    let mut c = dot / len;
    c = c.clamp(-1.0, 1.0);
    let sign = if ux * vy - uy * vx < 0.0 { -1.0 } else { 1.0 };
    sign * c.acos()
}

#[allow(clippy::too_many_arguments)]
fn ellipse_to_world(
    ux: f32,
    uy: f32,
    rx: f32,
    ry: f32,
    sin_phi: f32,
    cos_phi: f32,
    cx: f32,
    cy: f32,
) -> (f32, f32) {
    let px = rx * ux;
    let py = ry * uy;
    (cos_phi * px - sin_phi * py + cx, sin_phi * px + cos_phi * py + cy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approx(a: f32, b: f32, eps: f32) {
        assert!((a - b).abs() <= eps, "expected ~{}, got {}", b, a);
    }

    #[test]
    fn empty_input_is_ok() {
        assert_eq!(parse_svg_path("").unwrap(), vec![]);
    }

    #[test]
    fn whitespace_only_is_ok() {
        assert_eq!(parse_svg_path("   \t\n").unwrap(), vec![]);
    }

    #[test]
    fn must_start_with_move_to() {
        let err = parse_svg_path("L 10 10").unwrap_err();
        assert!(matches!(err, SvgPathError::MustStartWithMoveTo { .. }));
    }

    #[test]
    fn basic_move_to_line_to() {
        let cmds = parse_svg_path("M 10 20 L 30 40").unwrap();
        assert_eq!(cmds[0], PathCommand::MoveTo { x: 10.0, y: 20.0 });
        assert_eq!(cmds[1], PathCommand::LineTo { x: 30.0, y: 40.0 });
    }

    #[test]
    fn relative_move_to_adds_to_pen() {
        let cmds = parse_svg_path("M 10 10 m 5 5").unwrap();
        assert_eq!(cmds[0], PathCommand::MoveTo { x: 10.0, y: 10.0 });
        assert_eq!(cmds[1], PathCommand::MoveTo { x: 15.0, y: 15.0 });
    }

    #[test]
    fn implicit_line_to_after_move_to() {
        // After an M, extra coordinate pairs become implicit L commands.
        let cmds = parse_svg_path("M 0 0 10 10 20 20").unwrap();
        assert_eq!(cmds[0], PathCommand::MoveTo { x: 0.0, y: 0.0 });
        assert_eq!(cmds[1], PathCommand::LineTo { x: 10.0, y: 10.0 });
        assert_eq!(cmds[2], PathCommand::LineTo { x: 20.0, y: 20.0 });
    }

    #[test]
    fn implicit_line_to_after_relative_move_to() {
        // Relative variant: m 0 0 10 10 = M 0 0 l 10 10.
        let cmds = parse_svg_path("m 0 0 10 10").unwrap();
        assert_eq!(cmds[0], PathCommand::MoveTo { x: 0.0, y: 0.0 });
        assert_eq!(cmds[1], PathCommand::LineTo { x: 10.0, y: 10.0 });
    }

    #[test]
    fn relative_line_to() {
        let cmds = parse_svg_path("M 10 10 l 5 5").unwrap();
        assert_eq!(cmds[1], PathCommand::LineTo { x: 15.0, y: 15.0 });
    }

    #[test]
    fn horizontal_absolute_and_relative() {
        let cmds = parse_svg_path("M 10 10 H 50 h 10").unwrap();
        assert_eq!(cmds[1], PathCommand::LineTo { x: 50.0, y: 10.0 });
        assert_eq!(cmds[2], PathCommand::LineTo { x: 60.0, y: 10.0 });
    }

    #[test]
    fn vertical_absolute_and_relative() {
        let cmds = parse_svg_path("M 10 10 V 50 v 10").unwrap();
        assert_eq!(cmds[1], PathCommand::LineTo { x: 10.0, y: 50.0 });
        assert_eq!(cmds[2], PathCommand::LineTo { x: 10.0, y: 60.0 });
    }

    #[test]
    fn cubic_absolute() {
        let cmds = parse_svg_path("M 0 0 C 10 20 30 40 50 60").unwrap();
        assert_eq!(
            cmds[1],
            PathCommand::CubicTo { x1: 10.0, y1: 20.0, x2: 30.0, y2: 40.0, x: 50.0, y: 60.0 }
        );
    }

    #[test]
    fn cubic_relative() {
        let cmds = parse_svg_path("M 10 10 c 1 2 3 4 5 6").unwrap();
        assert_eq!(
            cmds[1],
            PathCommand::CubicTo { x1: 11.0, y1: 12.0, x2: 13.0, y2: 14.0, x: 15.0, y: 16.0 }
        );
    }

    #[test]
    fn smooth_cubic_reflects_previous_control() {
        // S without a preceding C reflects from the pen.
        let cmds = parse_svg_path("M 0 0 S 10 0 20 0").unwrap();
        let got = cmds[1];
        if let PathCommand::CubicTo { x1, y1, .. } = got {
            assert_approx(x1, 0.0, 1e-4);
            assert_approx(y1, 0.0, 1e-4);
        } else {
            panic!("expected cubic, got {:?}", got);
        }

        // After a C the S reflects the last control point about the pen.
        let cmds = parse_svg_path("M 0 0 C 0 10 10 10 10 0 S 20 -10 20 0").unwrap();
        if let PathCommand::CubicTo { x1, y1, .. } = cmds[2] {
            // Previous cubic ended at (10,0) with control (10,10).
            // Reflection: 2*10 - 10 = 10, 2*0 - 10 = -10.
            assert_approx(x1, 10.0, 1e-4);
            assert_approx(y1, -10.0, 1e-4);
        } else {
            panic!("expected cubic");
        }
    }

    #[test]
    fn quadratic_absolute_and_relative() {
        let cmds = parse_svg_path("M 0 0 Q 10 20 30 40 q 1 2 3 4").unwrap();
        assert_eq!(cmds[1], PathCommand::QuadTo { x1: 10.0, y1: 20.0, x: 30.0, y: 40.0 });
        assert_eq!(cmds[2], PathCommand::QuadTo { x1: 31.0, y1: 42.0, x: 33.0, y: 44.0 });
    }

    #[test]
    fn smooth_quadratic_reflects_previous_control() {
        let cmds = parse_svg_path("M 0 0 Q 10 10 20 0 T 40 0").unwrap();
        if let PathCommand::QuadTo { x1, y1, .. } = cmds[2] {
            // Reflection of (10, 10) about (20, 0) is (30, -10).
            assert_approx(x1, 30.0, 1e-4);
            assert_approx(y1, -10.0, 1e-4);
        } else {
            panic!("expected quad");
        }
    }

    #[test]
    fn arc_degenerate_same_points_is_noop() {
        let cmds = parse_svg_path("M 10 10 A 5 5 0 0 0 10 10").unwrap();
        // Only the move to should be present; arc is a no op.
        assert_eq!(cmds.len(), 1);
    }

    #[test]
    fn arc_zero_radius_is_line() {
        let cmds = parse_svg_path("M 0 0 A 0 5 0 0 0 10 0").unwrap();
        assert!(matches!(cmds[1], PathCommand::LineTo { x: 10.0, y: 0.0 }));
    }

    #[test]
    fn arc_produces_cubic_segments() {
        // A half circle arc should give two or three cubics.
        let cmds = parse_svg_path("M 0 0 A 10 10 0 0 1 20 0").unwrap();
        assert!(cmds.iter().skip(1).all(|c| matches!(c, PathCommand::CubicTo { .. })));
        assert!(cmds.len() >= 2);
    }

    #[test]
    fn close_returns_pen_to_subpath_start() {
        let cmds = parse_svg_path("M 10 10 L 20 10 L 20 20 Z l 0 5").unwrap();
        // The `Z` should return the pen to (10,10), so the relative `l 0 5`
        // ends at (10, 15).
        let last = *cmds.last().unwrap();
        assert_eq!(last, PathCommand::LineTo { x: 10.0, y: 15.0 });
    }

    #[test]
    fn exponent_numbers() {
        let cmds = parse_svg_path("M 1e3 -1.5e-2").unwrap();
        assert_eq!(cmds[0], PathCommand::MoveTo { x: 1000.0, y: -0.015 });
    }

    #[test]
    fn comma_and_whitespace_separation() {
        let a = parse_svg_path("M10,10 L20,20").unwrap();
        let b = parse_svg_path("M 10 10 L 20 20").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn negative_coords_without_separator() {
        // `M-1-1` is valid since the minus sign starts a new number.
        let cmds = parse_svg_path("M-1-1").unwrap();
        assert_eq!(cmds[0], PathCommand::MoveTo { x: -1.0, y: -1.0 });
    }

    #[test]
    fn malformed_unknown_command_rejected() {
        let err = parse_svg_path("M 0 0 X 1 1").unwrap_err();
        assert!(matches!(err, SvgPathError::UnknownCommand { .. }));
    }

    #[test]
    fn malformed_non_number_rejected() {
        let err = parse_svg_path("M a b").unwrap_err();
        assert!(matches!(err, SvgPathError::ExpectedNumber { .. }));
    }

    #[test]
    fn trailing_garbage_rejected() {
        let err = parse_svg_path("M 0 0 Z extra").unwrap_err();
        // The parser sees `e` as the start of an unknown command letter.
        assert!(
            matches!(
                err,
                SvgPathError::UnknownCommand { .. } | SvgPathError::TrailingGarbage { .. }
            ),
            "got {:?}",
            err
        );
    }

    #[test]
    fn multi_cubic_implicit_repeat() {
        let cmds = parse_svg_path("M 0 0 C 1 1 2 2 3 3 4 4 5 5 6 6").unwrap();
        assert!(matches!(cmds[1], PathCommand::CubicTo { .. }));
        assert!(matches!(cmds[2], PathCommand::CubicTo { .. }));
    }
}
