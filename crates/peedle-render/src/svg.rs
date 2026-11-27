use peedle_html::{Document, NodeId};

use crate::canvas2d::Canvas2D;
use crate::fonts::FontSystem;
use crate::framebuffer::{Affine2D, Framebuffer, RgbaColor};

pub fn render_svg_node(
    doc: &Document,
    svg_id: NodeId,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    dst_x: f32,
    dst_y: f32,
    dst_w: f32,
    dst_h: f32,
) {
    if dst_w <= 0.0 || dst_h <= 0.0 {
        return;
    }
    let w_px = dst_w.ceil().max(1.0) as u32;
    let h_px = dst_h.ceil().max(1.0) as u32;
    let mut canvas = Canvas2D::new(w_px, h_px);

    let (vb_x, vb_y, vb_w, vb_h) = doc
        .element(svg_id)
        .and_then(|e| e.get_attr("viewBox").or_else(|| e.get_attr("viewbox")))
        .and_then(parse_viewbox)
        .unwrap_or((0.0, 0.0, dst_w, dst_h));

    if vb_w > 0.0 && vb_h > 0.0 {
        let sx = dst_w / vb_w;
        let sy = dst_h / vb_h;
        canvas.scale(sx, sy);
        canvas.translate(-vb_x, -vb_y);
    }

    let inherited_fill = doc
        .element(svg_id)
        .and_then(|e| e.get_attr("fill"))
        .and_then(parse_svg_color)
        .unwrap_or(Some(RgbaColor::BLACK));

    render_svg_children(doc, svg_id, &mut canvas, fonts, inherited_fill, None, 1.0);
    fb.blit_rgba(
        canvas.fb.width,
        canvas.fb.height,
        &canvas.fb.data,
        dst_x,
        dst_y,
        dst_w,
        dst_h,
        1.0,
    );
}

fn render_svg_children(
    doc: &Document,
    parent: NodeId,
    canvas: &mut Canvas2D,
    fonts: &FontSystem,
    parent_fill: Option<RgbaColor>,
    parent_stroke: Option<RgbaColor>,
    parent_stroke_w: f32,
) {
    for child in doc.element_children(parent) {
        let Some(el) = doc.element(child) else {
            continue;
        };
        if el.tag_name.eq_ignore_ascii_case("defs")
            || el.tag_name.eq_ignore_ascii_case("title")
            || el.tag_name.eq_ignore_ascii_case("desc")
            || el.tag_name.eq_ignore_ascii_case("style")
        {
            continue;
        }
        canvas.save();
        if let Some(tf) = el.get_attr("transform") {
            apply_svg_transform(canvas, tf);
        }
        if let Some(op) = el.get_attr("opacity").and_then(|s| s.parse::<f32>().ok()) {
            canvas.state.global_alpha *= op.clamp(0.0, 1.0);
        }

        let fill = match el.get_attr("fill") {
            Some(v) => parse_svg_color(v).unwrap_or(parent_fill),
            None => parent_fill,
        };
        let stroke = match el.get_attr("stroke") {
            Some(v) => parse_svg_color(v).unwrap_or(parent_stroke),
            None => parent_stroke,
        };
        let stroke_w = el
            .get_attr("stroke-width")
            .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
            .unwrap_or(parent_stroke_w);
        canvas.state.line_width = stroke_w;

        match el.tag_name.to_ascii_lowercase().as_str() {
            "g" | "a" | "svg" => {
                render_svg_children(doc, child, canvas, fonts, fill, stroke, stroke_w);
            }
            "rect" => {
                let x = attr_f32(el.get_attr("x"), 0.0);
                let y = attr_f32(el.get_attr("y"), 0.0);
                let w = attr_f32(el.get_attr("width"), 0.0);
                let h = attr_f32(el.get_attr("height"), 0.0);
                canvas.begin_path();
                canvas.rect(x, y, w, h);
                paint_path(canvas, fill, stroke);
            }
            "circle" => {
                let cx = attr_f32(el.get_attr("cx"), 0.0);
                let cy = attr_f32(el.get_attr("cy"), 0.0);
                let r = attr_f32(el.get_attr("r"), 0.0);
                canvas.begin_path();
                canvas.arc(cx, cy, r, 0.0, std::f32::consts::TAU, false);
                canvas.close_path();
                paint_path(canvas, fill, stroke);
            }
            "ellipse" => {
                let cx = attr_f32(el.get_attr("cx"), 0.0);
                let cy = attr_f32(el.get_attr("cy"), 0.0);
                let rx = attr_f32(el.get_attr("rx"), 0.0);
                let ry = attr_f32(el.get_attr("ry"), 0.0);
                canvas.begin_path();
                let steps = 48usize;
                for i in 0..=steps {
                    let a = std::f32::consts::TAU * (i as f32 / steps as f32);
                    let px = cx + rx * a.cos();
                    let py = cy + ry * a.sin();
                    if i == 0 {
                        canvas.move_to(px, py);
                    } else {
                        canvas.line_to(px, py);
                    }
                }
                canvas.close_path();
                paint_path(canvas, fill, stroke);
            }
            "line" => {
                let x1 = attr_f32(el.get_attr("x1"), 0.0);
                let y1 = attr_f32(el.get_attr("y1"), 0.0);
                let x2 = attr_f32(el.get_attr("x2"), 0.0);
                let y2 = attr_f32(el.get_attr("y2"), 0.0);
                canvas.begin_path();
                canvas.move_to(x1, y1);
                canvas.line_to(x2, y2);
                paint_path(canvas, None, stroke);
            }
            "polyline" | "polygon" => {
                if let Some(pts_str) = el.get_attr("points") {
                    let nums = parse_numbers(pts_str);
                    canvas.begin_path();
                    let mut i = 0;
                    while i + 1 < nums.len() {
                        if i == 0 {
                            canvas.move_to(nums[i], nums[i + 1]);
                        } else {
                            canvas.line_to(nums[i], nums[i + 1]);
                        }
                        i += 2;
                    }
                    if el.tag_name.eq_ignore_ascii_case("polygon") {
                        canvas.close_path();
                    }
                    paint_path(canvas, fill, stroke);
                }
            }
            "path" => {
                if let Some(d) = el.get_attr("d") {
                    trace_svg_path(canvas, d);
                    paint_path(canvas, fill, stroke);
                }
            }
            "text" => {
                let x = attr_f32(el.get_attr("x"), 0.0);
                let y = attr_f32(el.get_attr("y"), 12.0);
                let fs = attr_f32(el.get_attr("font-size"), 14.0);
                canvas.state.font_size = fs;
                if let Some(c) = fill {
                    canvas.state.fill_style = c;
                    let txt = doc.text_content(child);
                    canvas.fill_text(fonts, txt.trim(), x, y);
                }
            }
            _ => {}
        }
        canvas.restore();
    }
}

fn paint_path(canvas: &mut Canvas2D, fill: Option<RgbaColor>, stroke: Option<RgbaColor>) {
    if let Some(c) = fill {
        canvas.state.fill_style = c;
        canvas.fill(false);
    }
    if let Some(c) = stroke {
        canvas.state.stroke_style = c;
        canvas.stroke();
    }
}

fn trace_svg_path(canvas: &mut Canvas2D, d: &str) {
    canvas.begin_path();
    let tokens = tokenize_path_data(d);
    let mut i = 0usize;
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut sx = 0.0f32;
    let mut sy = 0.0f32;
    let mut cmd = '\0';

    while i < tokens.len() {
        if tokens[i].len() == 1 && tokens[i].as_bytes()[0].is_ascii_alphabetic() {
            cmd = tokens[i].as_bytes()[0] as char;
            i += 1;
            if cmd == 'Z' || cmd == 'z' {
                canvas.close_path();
                cx = sx;
                cy = sy;
                continue;
            }
        }
        match cmd {
            'M' | 'm' => {
                let (Some(x), Some(y)) = (next_num(&tokens, &mut i), next_num(&tokens, &mut i)) else {
                    break;
                };
                if cmd == 'm' {
                    cx += x;
                    cy += y;
                } else {
                    cx = x;
                    cy = y;
                }
                sx = cx;
                sy = cy;
                canvas.move_to(cx, cy);
                cmd = if cmd == 'm' { 'l' } else { 'L' };
            }
            'L' | 'l' => {
                let (Some(x), Some(y)) = (next_num(&tokens, &mut i), next_num(&tokens, &mut i)) else {
                    break;
                };
                if cmd == 'l' {
                    cx += x;
                    cy += y;
                } else {
                    cx = x;
                    cy = y;
                }
                canvas.line_to(cx, cy);
            }
            'H' | 'h' => {
                let Some(x) = next_num(&tokens, &mut i) else { break };
                cx = if cmd == 'h' { cx + x } else { x };
                canvas.line_to(cx, cy);
            }
            'V' | 'v' => {
                let Some(y) = next_num(&tokens, &mut i) else { break };
                cy = if cmd == 'v' { cy + y } else { y };
                canvas.line_to(cx, cy);
            }
            'C' | 'c' => {
                let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x), Some(y)) = (
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                ) else {
                    break;
                };
                let (ax1, ay1, ax2, ay2, ax, ay) = if cmd == 'c' {
                    (cx + x1, cy + y1, cx + x2, cy + y2, cx + x, cy + y)
                } else {
                    (x1, y1, x2, y2, x, y)
                };
                canvas.bezier_curve_to(ax1, ay1, ax2, ay2, ax, ay);
                cx = ax;
                cy = ay;
            }
            'Q' | 'q' => {
                let (Some(x1), Some(y1), Some(x), Some(y)) = (
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                ) else {
                    break;
                };
                let (ax1, ay1, ax, ay) = if cmd == 'q' {
                    (cx + x1, cy + y1, cx + x, cy + y)
                } else {
                    (x1, y1, x, y)
                };
                canvas.quadratic_curve_to(ax1, ay1, ax, ay);
                cx = ax;
                cy = ay;
            }
            'S' | 's' => {
                let (Some(x2), Some(y2), Some(x), Some(y)) = (
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                ) else {
                    break;
                };
                let (ax2, ay2, ax, ay) = if cmd == 's' {
                    (cx + x2, cy + y2, cx + x, cy + y)
                } else {
                    (x2, y2, x, y)
                };
                canvas.bezier_curve_to(cx, cy, ax2, ay2, ax, ay);
                cx = ax;
                cy = ay;
            }
            'A' | 'a' => {
                let (Some(_rx), Some(_ry), Some(_rot), Some(_la), Some(_sw), Some(x), Some(y)) = (
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                    next_num(&tokens, &mut i),
                ) else {
                    break;
                };
                if cmd == 'a' {
                    cx += x;
                    cy += y;
                } else {
                    cx = x;
                    cy = y;
                }
                canvas.line_to(cx, cy);
            }
            _ => i += 1,
        }
    }
}

fn next_num(tokens: &[String], i: &mut usize) -> Option<f32> {
    let val = tokens.get(*i)?.parse::<f32>().ok()?;
    *i += 1;
    Some(val)
}

fn tokenize_path_data(d: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = d.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() || b == b',' {
            i += 1;
        } else if b.is_ascii_alphabetic() {
            out.push((b as char).to_string());
            i += 1;
        } else if matches!(b, b'-' | b'+' | b'.' | b'0'..=b'9') {
            let start = i;
            i += 1;
            let mut saw_dot = b == b'.';
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_digit() {
                    i += 1;
                } else if c == b'.' && !saw_dot {
                    saw_dot = true;
                    i += 1;
                } else if matches!(c, b'e' | b'E') {
                    i += 1;
                    if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            out.push(d[start..i].to_string());
        } else {
            i += 1;
        }
    }
    out
}

fn parse_viewbox(s: &str) -> Option<(f32, f32, f32, f32)> {
    let n = parse_numbers(s);
    if n.len() >= 4 {
        Some((n[0], n[1], n[2], n[3]))
    } else {
        None
    }
}

fn parse_numbers(s: &str) -> Vec<f32> {
    s.split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<f32>().ok())
        .collect()
}

fn attr_f32(v: Option<&str>, default: f32) -> f32 {
    v.and_then(|s| s.trim().trim_end_matches("px").parse::<f32>().ok())
        .unwrap_or(default)
}

fn apply_svg_transform(canvas: &mut Canvas2D, tf: &str) {
    let mut rest = tf.trim();
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().to_ascii_lowercase();
        let Some(close) = rest[open + 1..].find(')') else {
            break;
        };
        let args = parse_numbers(&rest[open + 1..open + 1 + close]);
        match name.as_str() {
            "translate" if !args.is_empty() => {
                canvas.translate(args[0], *args.get(1).unwrap_or(&0.0));
            }
            "scale" if !args.is_empty() => {
                canvas.scale(args[0], *args.get(1).unwrap_or(&args[0]));
            }
            "rotate" if !args.is_empty() => {
                canvas.rotate(args[0].to_radians());
            }
            "matrix" if args.len() >= 6 => {
                let m = Affine2D {
                    a: args[0],
                    b: args[1],
                    c: args[2],
                    d: args[3],
                    e: args[4],
                    f: args[5],
                };
                canvas.state.transform = canvas.state.transform.mul(m);
            }
            _ => {}
        }
        rest = rest[open + 1 + close + 1..].trim();
    }
}

pub fn parse_svg_color(raw: &str) -> Option<Option<RgbaColor>> {
    let s = raw.trim().to_ascii_lowercase();
    if s == "none" || s == "transparent" {
        return Some(None);
    }
    if let Some(hex) = s.strip_prefix('#') {
        let parsed = match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                RgbaColor::new(r, g, b, 255)
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                RgbaColor::new(r, g, b, 255)
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                RgbaColor::new(r, g, b, a)
            }
            _ => return None,
        };
        return Some(Some(parsed));
    }
    let named = match s.as_str() {
        "black" | "currentcolor" => RgbaColor::BLACK,
        "white" => RgbaColor::WHITE,
        "red" => RgbaColor::new(255, 0, 0, 255),
        "green" => RgbaColor::new(0, 128, 0, 255),
        "blue" => RgbaColor::new(0, 0, 255, 255),
        "yellow" => RgbaColor::new(255, 255, 0, 255),
        "orange" => RgbaColor::new(255, 165, 0, 255),
        "purple" => RgbaColor::new(128, 0, 128, 255),
        "gray" | "grey" => RgbaColor::new(128, 128, 128, 255),
        "cyan" | "aqua" => RgbaColor::new(0, 255, 255, 255),
        "magenta" | "fuchsia" => RgbaColor::new(255, 0, 255, 255),
        _ => return None,
    };
    Some(Some(named))
}
