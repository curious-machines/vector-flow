use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use vector_flow_core::types::{PathData, PathVerb, Point};

// ---------------------------------------------------------------------------
// Cache: SVG path string → parsed PathData
// ---------------------------------------------------------------------------

pub struct SvgPathCache {
    entries: Mutex<HashMap<String, Arc<PathData>>>,
}

impl SvgPathCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_or_parse(&self, data: &str) -> Arc<PathData> {
        let mut cache = self.entries.lock();
        if let Some(cached) = cache.get(data) {
            return Arc::clone(cached);
        }
        let parsed = Arc::new(parse_svg_path(data));
        cache.insert(data.to_string(), Arc::clone(&parsed));
        parsed
    }
}

// ---------------------------------------------------------------------------
// SVG path parser
// ---------------------------------------------------------------------------

/// Validate an SVG path string and return a human-readable error, or None if valid.
pub fn validate_svg_path(d: &str) -> Option<String> {
    let trimmed = d.trim();
    if trimmed.is_empty() {
        return None;
    }

    let tokens = tokenize(trimmed);
    if tokens.is_empty() {
        return Some("No valid path commands or coordinates found".into());
    }

    // Check that first token is a command
    let first_is_move = matches!(tokens.first(), Some(Token::Command(b'M' | b'm')));
    if !first_is_move {
        match tokens.first() {
            Some(Token::Number(_)) => {
                return Some("Path must start with M or m command".into());
            }
            Some(Token::Command(c)) => {
                return Some(format!(
                    "Path must start with M or m, found '{}'",
                    *c as char
                ));
            }
            None => unreachable!(),
        }
    }

    // Check for sufficient coordinates per command
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Command(cmd) => {
                let needed = coords_needed(*cmd);
                i += 1;
                let mut available = 0;
                while i < tokens.len() && matches!(tokens[i], Token::Number(_)) {
                    available += 1;
                    i += 1;
                }
                if *cmd != b'Z' && *cmd != b'z' && available < needed {
                    return Some(format!(
                        "'{}' needs {} coordinate(s), found {}",
                        *cmd as char, needed, available
                    ));
                }
            }
            Token::Number(_) => {
                i += 1; // stray number, not an error by itself
            }
        }
    }

    None
}

fn coords_needed(cmd: u8) -> usize {
    match cmd.to_ascii_uppercase() {
        b'M' | b'L' | b'T' => 2,
        b'H' | b'V' => 1,
        b'C' => 6,
        b'S' | b'Q' => 4,
        b'A' => 7,
        b'Z' => 0,
        _ => 0,
    }
}

pub fn parse_svg_path(d: &str) -> PathData {
    let tokens = tokenize(d);
    let mut verbs = Vec::new();
    let mut cur = Point { x: 0.0, y: 0.0 };
    let mut start = cur; // subpath start for Z
    let mut last_ctrl: Option<Point> = None; // for S/T smooth commands
    let mut last_cmd = b'\0';

    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Command(cmd) => {
                last_cmd = *cmd;
                i += 1;
                i = execute_command(
                    last_cmd,
                    &tokens,
                    i,
                    &mut cur,
                    &mut start,
                    &mut last_ctrl,
                    &mut verbs,
                );
            }
            Token::Number(_) => {
                // Implicit repeat of last command (M becomes L after first pair)
                let repeat_cmd = if last_cmd == b'M' {
                    b'L'
                } else if last_cmd == b'm' {
                    b'l'
                } else {
                    last_cmd
                };
                if repeat_cmd == b'\0' {
                    i += 1; // skip stray number
                    continue;
                }
                i = execute_command(
                    repeat_cmd,
                    &tokens,
                    i,
                    &mut cur,
                    &mut start,
                    &mut last_ctrl,
                    &mut verbs,
                );
            }
        }
    }

    let closed = verbs.iter().any(|v| matches!(v, PathVerb::Close));
    PathData { verbs, closed }
}

fn execute_command(
    cmd: u8,
    tokens: &[Token],
    i: usize,
    cur: &mut Point,
    start: &mut Point,
    last_ctrl: &mut Option<Point>,
    verbs: &mut Vec<PathVerb>,
) -> usize {
    let rel = cmd.is_ascii_lowercase();
    match cmd.to_ascii_uppercase() {
        b'M' => {
            let (x, y, next) = read2(tokens, i, rel, *cur);
            *cur = Point { x, y };
            *start = *cur;
            verbs.push(PathVerb::MoveTo(*cur));
            *last_ctrl = None;
            next
        }
        b'L' => {
            let (x, y, next) = read2(tokens, i, rel, *cur);
            *cur = Point { x, y };
            verbs.push(PathVerb::LineTo(*cur));
            *last_ctrl = None;
            next
        }
        b'H' => {
            let (val, next) = read1(tokens, i);
            let x = if rel { cur.x + val } else { val };
            *cur = Point { x, y: cur.y };
            verbs.push(PathVerb::LineTo(*cur));
            *last_ctrl = None;
            next
        }
        b'V' => {
            let (val, next) = read1(tokens, i);
            let y = if rel { cur.y + val } else { val };
            *cur = Point { x: cur.x, y };
            verbs.push(PathVerb::LineTo(*cur));
            *last_ctrl = None;
            next
        }
        b'C' => {
            let (x1, y1, next1) = read2(tokens, i, rel, *cur);
            let (x2, y2, next2) = read2(tokens, next1, rel, *cur);
            let (x, y, next3) = read2(tokens, next2, rel, *cur);
            let ctrl2 = Point { x: x2, y: y2 };
            *cur = Point { x, y };
            verbs.push(PathVerb::CubicTo {
                ctrl1: Point { x: x1, y: y1 },
                ctrl2,
                to: *cur,
            });
            *last_ctrl = Some(ctrl2);
            next3
        }
        b'S' => {
            // Smooth cubic: ctrl1 is reflection of previous ctrl2
            let ctrl1 = match *last_ctrl {
                Some(lc) => Point {
                    x: 2.0 * cur.x - lc.x,
                    y: 2.0 * cur.y - lc.y,
                },
                None => *cur,
            };
            let (x2, y2, next1) = read2(tokens, i, rel, *cur);
            let (x, y, next2) = read2(tokens, next1, rel, *cur);
            let ctrl2 = Point { x: x2, y: y2 };
            *cur = Point { x, y };
            verbs.push(PathVerb::CubicTo {
                ctrl1,
                ctrl2,
                to: *cur,
            });
            *last_ctrl = Some(ctrl2);
            next2
        }
        b'Q' => {
            let (cx, cy, next1) = read2(tokens, i, rel, *cur);
            let (x, y, next2) = read2(tokens, next1, rel, *cur);
            let ctrl = Point { x: cx, y: cy };
            *cur = Point { x, y };
            verbs.push(PathVerb::QuadTo { ctrl, to: *cur });
            *last_ctrl = Some(ctrl);
            next2
        }
        b'T' => {
            // Smooth quad: ctrl is reflection of previous ctrl
            let ctrl = match *last_ctrl {
                Some(lc) => Point {
                    x: 2.0 * cur.x - lc.x,
                    y: 2.0 * cur.y - lc.y,
                },
                None => *cur,
            };
            let (x, y, next) = read2(tokens, i, rel, *cur);
            *cur = Point { x, y };
            verbs.push(PathVerb::QuadTo { ctrl, to: *cur });
            *last_ctrl = Some(ctrl);
            next
        }
        b'A' => {
            let (rx, next1) = read1(tokens, i);
            let (ry, next2) = read1(tokens, next1);
            let (x_rot, next3) = read1(tokens, next2);
            let (large_arc_f, next4) = read1(tokens, next3);
            let (sweep_f, next5) = read1(tokens, next4);
            let (x, y, next6) = read2(tokens, next5, rel, *cur);
            let to = Point { x, y };
            arc_to_cubics(
                *cur,
                rx,
                ry,
                x_rot,
                large_arc_f != 0.0,
                sweep_f != 0.0,
                to,
                verbs,
            );
            *cur = to;
            *last_ctrl = None;
            next6
        }
        b'Z' => {
            verbs.push(PathVerb::Close);
            *cur = *start;
            *last_ctrl = None;
            i
        }
        _ => i, // unknown command, skip
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Token {
    Command(u8),
    Number(f32),
}

fn tokenize(d: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = d.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        match b {
            b' ' | b'\t' | b'\n' | b'\r' | b',' => {
                i += 1;
            }
            b'M' | b'm' | b'L' | b'l' | b'H' | b'h' | b'V' | b'v' | b'C' | b'c' | b'S'
            | b's' | b'Q' | b'q' | b'T' | b't' | b'A' | b'a' | b'Z' | b'z' => {
                tokens.push(Token::Command(b));
                i += 1;
            }
            b'0'..=b'9' | b'.' | b'-' | b'+' => {
                let start = i;
                // Handle optional sign
                if b == b'-' || b == b'+' {
                    i += 1;
                }
                let mut has_dot = false;
                while i < len {
                    match bytes[i] {
                        b'0'..=b'9' => i += 1,
                        b'.' if !has_dot => {
                            has_dot = true;
                            i += 1;
                        }
                        b'e' | b'E' => {
                            i += 1;
                            if i < len && (bytes[i] == b'-' || bytes[i] == b'+') {
                                i += 1;
                            }
                            while i < len && bytes[i].is_ascii_digit() {
                                i += 1;
                            }
                            break;
                        }
                        _ => break,
                    }
                }
                if let Ok(val) = d[start..i].parse::<f32>() {
                    tokens.push(Token::Number(val));
                }
            }
            _ => {
                i += 1; // skip unknown
            }
        }
    }

    tokens
}

// ---------------------------------------------------------------------------
// Helpers for reading numbers from token stream
// ---------------------------------------------------------------------------

fn read1(tokens: &[Token], i: usize) -> (f32, usize) {
    if i < tokens.len() {
        if let Token::Number(v) = tokens[i] {
            return (v, i + 1);
        }
    }
    (0.0, i)
}

fn read2(tokens: &[Token], i: usize, rel: bool, cur: Point) -> (f32, f32, usize) {
    let (x, next1) = read1(tokens, i);
    let (y, next2) = read1(tokens, next1);
    if rel {
        (cur.x + x, cur.y + y, next2)
    } else {
        (x, y, next2)
    }
}

// ---------------------------------------------------------------------------
// Arc to cubic Bezier conversion
// ---------------------------------------------------------------------------

fn arc_to_cubics(
    from: Point,
    rx: f32,
    ry: f32,
    x_rotation_deg: f32,
    large_arc: bool,
    sweep: bool,
    to: Point,
    verbs: &mut Vec<PathVerb>,
) {
    // Degenerate cases
    if (from.x - to.x).abs() < 1e-10 && (from.y - to.y).abs() < 1e-10 {
        return;
    }
    let mut rx = rx.abs();
    let mut ry = ry.abs();
    if rx < 1e-10 || ry < 1e-10 {
        verbs.push(PathVerb::LineTo(to));
        return;
    }

    let phi = x_rotation_deg.to_radians();
    let cos_phi = phi.cos();
    let sin_phi = phi.sin();

    // Step 1: Transform to unit-circle space
    let dx = (from.x - to.x) / 2.0;
    let dy = (from.y - to.y) / 2.0;
    let x1p = cos_phi * dx + sin_phi * dy;
    let y1p = -sin_phi * dx + cos_phi * dy;

    // Step 2: Correct radii if needed
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let sqrt_l = lambda.sqrt();
        rx *= sqrt_l;
        ry *= sqrt_l;
    }

    // Step 3: Compute center (parameterization)
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let x1p2 = x1p * x1p;
    let y1p2 = y1p * y1p;
    let num = (rx2 * ry2 - rx2 * y1p2 - ry2 * x1p2).max(0.0);
    let den = rx2 * y1p2 + ry2 * x1p2;
    let sq = if den > 1e-10 { (num / den).sqrt() } else { 0.0 };
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let cxp = sign * sq * rx * y1p / ry;
    let cyp = sign * sq * -ry * x1p / rx;

    let cx = cos_phi * cxp - sin_phi * cyp + (from.x + to.x) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (from.y + to.y) / 2.0;

    // Step 4: Compute angles
    let theta1 = angle_between(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dtheta = angle_between(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && dtheta > 0.0 {
        dtheta -= std::f32::consts::TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += std::f32::consts::TAU;
    }

    // Step 5: Split into segments of at most π/2
    let n_segs = (dtheta.abs() / (std::f32::consts::FRAC_PI_2 + 0.001)).ceil() as usize;
    let n_segs = n_segs.max(1);
    let seg_angle = dtheta / n_segs as f32;

    for seg in 0..n_segs {
        let t1 = theta1 + seg as f32 * seg_angle;
        let t2 = t1 + seg_angle;
        arc_segment_to_cubic(cx, cy, rx, ry, cos_phi, sin_phi, t1, t2, verbs);
    }
}

fn arc_segment_to_cubic(
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    cos_phi: f32,
    sin_phi: f32,
    t1: f32,
    t2: f32,
    verbs: &mut Vec<PathVerb>,
) {
    let alpha = (t2 - t1).sin() / (1.0 + (t2 - t1).cos()) * 4.0 / 3.0;

    let cos1 = t1.cos();
    let sin1 = t1.sin();
    let cos2 = t2.cos();
    let sin2 = t2.sin();

    let ex1 = rx * cos1;
    let ey1 = ry * sin1;
    let ex2 = rx * cos2;
    let ey2 = ry * sin2;

    let dx1 = -rx * sin1;
    let dy1 = ry * cos1;
    let dx2 = -rx * sin2;
    let dy2 = ry * cos2;

    let cp1x = ex1 + alpha * dx1;
    let cp1y = ey1 + alpha * dy1;
    let cp2x = ex2 - alpha * dx2;
    let cp2y = ey2 - alpha * dy2;

    // Rotate back
    let ctrl1 = Point {
        x: cos_phi * cp1x - sin_phi * cp1y + cx,
        y: sin_phi * cp1x + cos_phi * cp1y + cy,
    };
    let ctrl2 = Point {
        x: cos_phi * cp2x - sin_phi * cp2y + cx,
        y: sin_phi * cp2x + cos_phi * cp2y + cy,
    };
    let to = Point {
        x: cos_phi * ex2 - sin_phi * ey2 + cx,
        y: sin_phi * ex2 + cos_phi * ey2 + cy,
    };

    verbs.push(PathVerb::CubicTo { ctrl1, ctrl2, to });
}

fn angle_between(ux: f32, uy: f32, vx: f32, vy: f32) -> f32 {
    let n = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
    if n < 1e-10 {
        return 0.0;
    }
    let cos_a = ((ux * vx + uy * vy) / n).clamp(-1.0, 1.0);
    let angle = cos_a.acos();
    if ux * vy - uy * vx < 0.0 {
        -angle
    } else {
        angle
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_move_line_close() {
        let path = parse_svg_path("M 10 20 L 30 40 Z");
        assert_eq!(path.verbs.len(), 3);
        assert_eq!(path.verbs[0], PathVerb::MoveTo(Point { x: 10.0, y: 20.0 }));
        assert_eq!(path.verbs[1], PathVerb::LineTo(Point { x: 30.0, y: 40.0 }));
        assert_eq!(path.verbs[2], PathVerb::Close);
        assert!(path.closed);
    }

    #[test]
    fn parse_relative_commands() {
        let path = parse_svg_path("m 10 20 l 5 5 z");
        assert_eq!(path.verbs[0], PathVerb::MoveTo(Point { x: 10.0, y: 20.0 }));
        assert_eq!(path.verbs[1], PathVerb::LineTo(Point { x: 15.0, y: 25.0 }));
    }

    #[test]
    fn parse_horizontal_vertical() {
        let path = parse_svg_path("M 0 0 H 50 V 30");
        assert_eq!(path.verbs[1], PathVerb::LineTo(Point { x: 50.0, y: 0.0 }));
        assert_eq!(path.verbs[2], PathVerb::LineTo(Point { x: 50.0, y: 30.0 }));
    }

    #[test]
    fn parse_cubic_bezier() {
        let path = parse_svg_path("M 0 0 C 10 20 30 40 50 60");
        assert_eq!(path.verbs.len(), 2);
        assert_eq!(
            path.verbs[1],
            PathVerb::CubicTo {
                ctrl1: Point { x: 10.0, y: 20.0 },
                ctrl2: Point { x: 30.0, y: 40.0 },
                to: Point { x: 50.0, y: 60.0 },
            }
        );
    }

    #[test]
    fn parse_smooth_cubic() {
        let path = parse_svg_path("M 0 0 C 10 20 30 40 50 50 S 70 80 90 90");
        assert_eq!(path.verbs.len(), 3);
        // Smooth ctrl1 should be reflection of (30,40) around (50,50) = (70,60)
        if let PathVerb::CubicTo { ctrl1, .. } = path.verbs[2] {
            assert!((ctrl1.x - 70.0).abs() < 1e-3);
            assert!((ctrl1.y - 60.0).abs() < 1e-3);
        } else {
            panic!("expected CubicTo");
        }
    }

    #[test]
    fn parse_quadratic_bezier() {
        let path = parse_svg_path("M 0 0 Q 10 20 30 40");
        assert_eq!(path.verbs.len(), 2);
        assert_eq!(
            path.verbs[1],
            PathVerb::QuadTo {
                ctrl: Point { x: 10.0, y: 20.0 },
                to: Point { x: 30.0, y: 40.0 },
            }
        );
    }

    #[test]
    fn parse_smooth_quad() {
        let path = parse_svg_path("M 0 0 Q 10 20 30 30 T 50 50");
        assert_eq!(path.verbs.len(), 3);
        // Smooth ctrl should be reflection of (10,20) around (30,30) = (50,40)
        if let PathVerb::QuadTo { ctrl, .. } = path.verbs[2] {
            assert!((ctrl.x - 50.0).abs() < 1e-3);
            assert!((ctrl.y - 40.0).abs() < 1e-3);
        } else {
            panic!("expected QuadTo");
        }
    }

    #[test]
    fn parse_arc() {
        let path = parse_svg_path("M 10 80 A 45 45 0 0 0 125 125");
        assert!(path.verbs.len() >= 2); // MoveTo + at least one CubicTo
        // Arc should end at (125, 125)
        let last = path.verbs.last().unwrap();
        if let PathVerb::CubicTo { to, .. } = last {
            assert!((to.x - 125.0).abs() < 0.5);
            assert!((to.y - 125.0).abs() < 0.5);
        } else {
            panic!("expected CubicTo from arc");
        }
    }

    #[test]
    fn parse_implicit_lineto_after_move() {
        // After M, implicit coordinates become L
        let path = parse_svg_path("M 0 0 10 10 20 20");
        assert_eq!(path.verbs.len(), 3);
        assert_eq!(path.verbs[0], PathVerb::MoveTo(Point { x: 0.0, y: 0.0 }));
        assert_eq!(path.verbs[1], PathVerb::LineTo(Point { x: 10.0, y: 10.0 }));
        assert_eq!(path.verbs[2], PathVerb::LineTo(Point { x: 20.0, y: 20.0 }));
    }

    #[test]
    fn parse_no_spaces() {
        let path = parse_svg_path("M0,0L100,0L100,100Z");
        assert_eq!(path.verbs.len(), 4);
    }

    #[test]
    fn parse_negative_numbers_no_separator() {
        let path = parse_svg_path("M10-20L30-40");
        assert_eq!(path.verbs[0], PathVerb::MoveTo(Point { x: 10.0, y: -20.0 }));
        assert_eq!(path.verbs[1], PathVerb::LineTo(Point { x: 30.0, y: -40.0 }));
    }

    #[test]
    fn parse_empty_string() {
        let path = parse_svg_path("");
        assert!(path.verbs.is_empty());
        assert!(!path.closed);
    }

    #[test]
    fn parse_complex_svg_star() {
        let path = parse_svg_path(
            "M 50 0 L 61 35 L 98 35 L 68 57 L 79 91 L 50 70 L 21 91 L 32 57 L 2 35 L 39 35 Z",
        );
        assert_eq!(path.verbs.len(), 11); // 1 MoveTo + 9 LineTo + Close
        assert!(path.closed);
    }

    #[test]
    fn cache_returns_same_arc() {
        let cache = SvgPathCache::new();
        let a = cache.get_or_parse("M 0 0 L 10 10");
        let b = cache.get_or_parse("M 0 0 L 10 10");
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn cache_different_for_different_input() {
        let cache = SvgPathCache::new();
        let a = cache.get_or_parse("M 0 0 L 10 10");
        let b = cache.get_or_parse("M 0 0 L 20 20");
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn parse_relative_horizontal_vertical() {
        let path = parse_svg_path("M 10 10 h 20 v 30");
        assert_eq!(path.verbs[1], PathVerb::LineTo(Point { x: 30.0, y: 10.0 }));
        assert_eq!(path.verbs[2], PathVerb::LineTo(Point { x: 30.0, y: 40.0 }));
    }

    #[test]
    fn parse_multiple_subpaths() {
        let path = parse_svg_path("M 0 0 L 10 10 Z M 20 20 L 30 30 Z");
        assert_eq!(path.verbs.len(), 6);
        assert_eq!(path.verbs[3], PathVerb::MoveTo(Point { x: 20.0, y: 20.0 }));
    }

    #[test]
    fn parse_scientific_notation() {
        let path = parse_svg_path("M 1e1 2e1 L 1.5e2 3E1");
        assert_eq!(path.verbs[0], PathVerb::MoveTo(Point { x: 10.0, y: 20.0 }));
        assert_eq!(path.verbs[1], PathVerb::LineTo(Point { x: 150.0, y: 30.0 }));
    }
}
