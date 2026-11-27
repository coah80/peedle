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
    render_svg_node_with_color(
        doc,
        svg_id,
        fb,
        fonts,
        dst_x,
        dst_y,
        dst_w,
        dst_h,
        RgbaColor::BLACK,
    );
}

pub fn render_svg_node_with_color(
    doc: &Document,
    svg_id: NodeId,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    dst_x: f32,
    dst_y: f32,
    dst_w: f32,
    dst_h: f32,
    current_color: RgbaColor,
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
        let par = doc
            .element(svg_id)
            .and_then(|e| {
                e.get_attr("preserveAspectRatio")
                    .or_else(|| e.get_attr("preserveaspectratio"))
            })
            .unwrap_or("xMidYMid meet")
            .trim()
            .to_ascii_lowercase();
        let sx = dst_w / vb_w;
        let sy = dst_h / vb_h;
        if par.starts_with("none") {
            canvas.scale(sx, sy);
            canvas.translate(-vb_x, -vb_y);
        } else {
            let scale = if par.contains("slice") {
                sx.max(sy)
            } else {
                sx.min(sy)
            };
            let tx = if par.starts_with("xmin") {
                0.0
            } else if par.starts_with("xmax") {
                dst_w - vb_w * scale
            } else {
                (dst_w - vb_w * scale) * 0.5
            };
            let ty = if par.contains("ymin") {
                0.0
            } else if par.contains("ymax") {
                dst_h - vb_h * scale
            } else {
                (dst_h - vb_h * scale) * 0.5
            };
            canvas.translate(tx, ty);
            canvas.scale(scale, scale);
            canvas.translate(-vb_x, -vb_y);
        }
    }

    let inherited_fill = doc
        .element(svg_id)
        .and_then(|e| e.get_attr("fill"))
        .and_then(|v| parse_svg_color_with_current(v, current_color))
        .unwrap_or(Some(current_color));

    render_svg_children(doc, svg_id, &mut canvas, fonts, inherited_fill, None, 1.0, current_color, 0);
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
    current_color: RgbaColor,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    for child in doc.element_children(parent) {
        render_svg_element(
            doc,
            child,
            canvas,
            fonts,
            parent_fill,
            parent_stroke,
            parent_stroke_w,
            current_color,
            depth,
            false,
        );
    }
}

fn render_svg_element(
    doc: &Document,
    child: NodeId,
    canvas: &mut Canvas2D,
    fonts: &FontSystem,
    parent_fill: Option<RgbaColor>,
    parent_stroke: Option<RgbaColor>,
    parent_stroke_w: f32,
    current_color: RgbaColor,
    depth: usize,
    from_use: bool,
) {
    if depth > 16 {
        return;
    }
    let Some(el) = doc.element(child) else {
        return;
    };
    if el.tag_name.eq_ignore_ascii_case("defs")
        || el.tag_name.eq_ignore_ascii_case("title")
        || el.tag_name.eq_ignore_ascii_case("desc")
        || el.tag_name.eq_ignore_ascii_case("style")
        || (!from_use && el.tag_name.eq_ignore_ascii_case("symbol"))
    {
        return;
    }
    canvas.save();
    if let Some(tf) = el.get_attr("transform") {
        apply_svg_transform(canvas, tf);
    }
    let style_prop = |name: &str| -> Option<&str> {
        let st = el.get_attr("style")?;
        for decl in st.split(';') {
            let (k, v) = decl.split_once(':')?;
            if k.trim().eq_ignore_ascii_case(name) {
                return Some(v.trim());
            }
        }
        None
    };
    let svg_attr = |name: &str| -> Option<&str> {
        style_prop(name).or_else(|| el.get_attr(name))
    };

    if let Some(op) = svg_attr("opacity").and_then(|s| s.parse::<f32>().ok()) {
        canvas.state.global_alpha *= op.clamp(0.0, 1.0);
    }

    let fill_op = svg_attr("fill-opacity")
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let stroke_op = svg_attr("stroke-opacity")
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);

    let effective_current = svg_attr("color")
        .and_then(|v| parse_svg_color_with_current(v, current_color))
        .flatten()
        .unwrap_or(current_color);

    let fill = match svg_attr("fill") {
        Some(v) => parse_svg_color_with_current(v, effective_current).unwrap_or(parent_fill),
        None => parent_fill,
    }
    .map(|c| c.with_alpha(fill_op));
    let stroke = match svg_attr("stroke") {
        Some(v) => parse_svg_color_with_current(v, effective_current).unwrap_or(parent_stroke),
        None => parent_stroke,
    }
    .map(|c| c.with_alpha(stroke_op));
    let stroke_w = svg_attr("stroke-width")
        .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
        .unwrap_or(parent_stroke_w);
    let even_odd = svg_attr("fill-rule")
        .map_or(false, |v| v.eq_ignore_ascii_case("evenodd"));
    canvas.state.line_width = stroke_w;

    match el.tag_name.to_ascii_lowercase().as_str() {
        "g" | "a" | "svg" | "symbol" => {
            render_svg_children(doc, child, canvas, fonts, fill, stroke, stroke_w, effective_current, depth + 1);
        }
        "use" => {
            let ux = attr_f32(el.get_attr("x"), 0.0);
            let uy = attr_f32(el.get_attr("y"), 0.0);
            if ux != 0.0 || uy != 0.0 {
                canvas.translate(ux, uy);
            }
            let href = el.get_attr("href").or_else(|| el.get_attr("xlink:href"));
            if let Some(target_id) = href.and_then(|h| h.strip_prefix('#')).and_then(|id| doc.get_element_by_id(id)) {
                render_svg_element(
                    doc,
                    target_id,
                    canvas,
                    fonts,
                    fill,
                    stroke,
                    stroke_w,
                    effective_current,
                    depth + 1,
                    true,
                );
            }
        }
        "rect" => {
            let x = attr_f32(el.get_attr("x"), 0.0);
            let y = attr_f32(el.get_attr("y"), 0.0);
            let w = attr_f32(el.get_attr("width"), 0.0);
            let h = attr_f32(el.get_attr("height"), 0.0);
            let rx_opt = el.get_attr("rx").and_then(|s| s.trim_end_matches("px").parse::<f32>().ok());
            let ry_opt = el.get_attr("ry").and_then(|s| s.trim_end_matches("px").parse::<f32>().ok());
            let rx = rx_opt.or(ry_opt).unwrap_or(0.0).clamp(0.0, w * 0.5);
            let ry = ry_opt.or(rx_opt).unwrap_or(0.0).clamp(0.0, h * 0.5);
            canvas.begin_path();
            if rx > 0.01 || ry > 0.01 {
                trace_rounded_rect(canvas, x, y, w, h, rx, ry);
            } else {
                canvas.rect(x, y, w, h);
            }
            paint_path(canvas, fill, stroke, even_odd);
        }
        "circle" => {
            let cx = attr_f32(el.get_attr("cx"), 0.0);
            let cy = attr_f32(el.get_attr("cy"), 0.0);
            let r = attr_f32(el.get_attr("r"), 0.0);
            canvas.begin_path();
            canvas.arc(cx, cy, r, 0.0, std::f32::consts::TAU, false);
            canvas.close_path();
            paint_path(canvas, fill, stroke, even_odd);
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
            paint_path(canvas, fill, stroke, even_odd);
        }
        "line" => {
            let x1 = attr_f32(el.get_attr("x1"), 0.0);
            let y1 = attr_f32(el.get_attr("y1"), 0.0);
            let x2 = attr_f32(el.get_attr("x2"), 0.0);
            let y2 = attr_f32(el.get_attr("y2"), 0.0);
            canvas.begin_path();
            canvas.move_to(x1, y1);
            canvas.line_to(x2, y2);
            paint_path(canvas, None, stroke, false);
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
                paint_path(canvas, fill, stroke, even_odd);
            }
        }
        "path" => {
            if let Some(d) = el.get_attr("d") {
                trace_svg_path(canvas, d);
                paint_path(canvas, fill, stroke, even_odd);
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

fn paint_path(canvas: &mut Canvas2D, fill: Option<RgbaColor>, stroke: Option<RgbaColor>, even_odd: bool) {
    if let Some(c) = fill {
        canvas.state.fill_style = c;
        canvas.fill(even_odd);
    }
    if let Some(c) = stroke {
        canvas.state.stroke_style = c;
        canvas.stroke();
    }
}

fn trace_rounded_rect(canvas: &mut Canvas2D, x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) {
    let k = 0.552_284_8;
    let ox = rx * k;
    let oy = ry * k;
    canvas.move_to(x + rx, y);
    canvas.line_to(x + w - rx, y);
    canvas.bezier_curve_to(x + w - rx + ox, y, x + w, y + ry - oy, x + w, y + ry);
    canvas.line_to(x + w, y + h - ry);
    canvas.bezier_curve_to(x + w, y + h - ry + oy, x + w - rx + ox, y + h, x + w - rx, y + h);
    canvas.line_to(x + rx, y + h);
    canvas.bezier_curve_to(x + rx - ox, y + h, x, y + h - ry + oy, x, y + h - ry);
    canvas.line_to(x, y + ry);
    canvas.bezier_curve_to(x, y + ry - oy, x + rx - ox, y, x + rx, y);
    canvas.close_path();
}

fn trace_svg_arc(
    canvas: &mut Canvas2D,
    x1: f32,
    y1: f32,
    mut rx: f32,
    mut ry: f32,
    angle_deg: f32,
    large_arc: bool,
    sweep: bool,
    x2: f32,
    y2: f32,
) {
    if (x1 - x2).abs() < 1e-4 && (y1 - y2).abs() < 1e-4 {
        return;
    }
    rx = rx.abs();
    ry = ry.abs();
    if rx < 1e-4 || ry < 1e-4 {
        canvas.line_to(x2, y2);
        return;
    }
    let phi = angle_deg.to_radians();
    let cos_p = phi.cos();
    let sin_p = phi.sin();
    let dx2 = (x1 - x2) * 0.5;
    let dy2 = (y1 - y2) * 0.5;
    let x1p = cos_p * dx2 + sin_p * dy2;
    let y1p = -sin_p * dx2 + cos_p * dy2;

    let mut rx_sq = rx * rx;
    let mut ry_sq = ry * ry;
    let x1p_sq = x1p * x1p;
    let y1p_sq = y1p * y1p;
    let lambda = x1p_sq / rx_sq + y1p_sq / ry_sq;
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
        rx_sq = rx * rx;
        ry_sq = ry * ry;
    }

    let num = (rx_sq * ry_sq - rx_sq * y1p_sq - ry_sq * x1p_sq).max(0.0);
    let den = (rx_sq * y1p_sq + ry_sq * x1p_sq).max(1e-9);
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let coef = sign * (num / den).sqrt();
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);

    let cx = cos_p * cxp - sin_p * cyp + (x1 + x2) * 0.5;
    let cy = sin_p * cxp + cos_p * cyp + (y1 + y2) * 0.5;

    let vec_angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt().max(1e-9);
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = vec_angle(1.0, 0.0, ux, uy);
    let mut dtheta = vec_angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= std::f32::consts::TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += std::f32::consts::TAU;
    }

    let steps = ((dtheta.abs() / (std::f32::consts::PI / 16.0)).ceil() as usize).clamp(4, 64);
    for i in 1..=steps {
        let t = theta1 + dtheta * (i as f32 / steps as f32);
        let px = rx * t.cos();
        let py = ry * t.sin();
        let gx = cos_p * px - sin_p * py + cx;
        let gy = sin_p * px + cos_p * py + cy;
        canvas.line_to(gx, gy);
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
    let mut last_c2 = (0.0f32, 0.0f32);
    let mut last_q1 = (0.0f32, 0.0f32);
    let mut prev_cmd = '\0';
    let mut cmd = '\0';

    while i < tokens.len() {
        if tokens[i].len() == 1 && tokens[i].as_bytes()[0].is_ascii_alphabetic() {
            cmd = tokens[i].as_bytes()[0] as char;
            i += 1;
            if cmd == 'Z' || cmd == 'z' {
                canvas.close_path();
                cx = sx;
                cy = sy;
                prev_cmd = cmd;
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
                prev_cmd = cmd;
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
                prev_cmd = cmd;
            }
            'H' | 'h' => {
                let Some(x) = next_num(&tokens, &mut i) else { break };
                cx = if cmd == 'h' { cx + x } else { x };
                canvas.line_to(cx, cy);
                prev_cmd = cmd;
            }
            'V' | 'v' => {
                let Some(y) = next_num(&tokens, &mut i) else { break };
                cy = if cmd == 'v' { cy + y } else { y };
                canvas.line_to(cx, cy);
                prev_cmd = cmd;
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
                last_c2 = (ax2, ay2);
                cx = ax;
                cy = ay;
                prev_cmd = cmd;
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
                last_q1 = (ax1, ay1);
                cx = ax;
                cy = ay;
                prev_cmd = cmd;
            }
            'T' | 't' => {
                let (Some(x), Some(y)) = (next_num(&tokens, &mut i), next_num(&tokens, &mut i)) else {
                    break;
                };
                let (ax1, ay1) = if matches!(prev_cmd, 'Q' | 'q' | 'T' | 't') {
                    (2.0 * cx - last_q1.0, 2.0 * cy - last_q1.1)
                } else {
                    (cx, cy)
                };
                let (ax, ay) = if cmd == 't' { (cx + x, cy + y) } else { (x, y) };
                canvas.quadratic_curve_to(ax1, ay1, ax, ay);
                last_q1 = (ax1, ay1);
                cx = ax;
                cy = ay;
                prev_cmd = cmd;
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
                let (ax1, ay1) = if matches!(prev_cmd, 'C' | 'c' | 'S' | 's') {
                    (2.0 * cx - last_c2.0, 2.0 * cy - last_c2.1)
                } else {
                    (cx, cy)
                };
                let (ax2, ay2, ax, ay) = if cmd == 's' {
                    (cx + x2, cy + y2, cx + x, cy + y)
                } else {
                    (x2, y2, x, y)
                };
                canvas.bezier_curve_to(ax1, ay1, ax2, ay2, ax, ay);
                last_c2 = (ax2, ay2);
                cx = ax;
                cy = ay;
                prev_cmd = cmd;
            }
            'A' | 'a' => {
                let (Some(rx), Some(ry), Some(rot), Some(la), Some(sw), Some(x), Some(y)) = (
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
                let (nx, ny) = if cmd == 'a' { (cx + x, cy + y) } else { (x, y) };
                trace_svg_arc(canvas, cx, cy, rx, ry, rot, la != 0.0, sw != 0.0, nx, ny);
                cx = nx;
                cy = ny;
                prev_cmd = cmd;
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
    parse_svg_color_with_current(raw, RgbaColor::BLACK)
}

pub fn parse_svg_color_with_current(raw: &str, current: RgbaColor) -> Option<Option<RgbaColor>> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("none") || s.eq_ignore_ascii_case("transparent") {
        return Some(None);
    }
    if s.eq_ignore_ascii_case("currentcolor") {
        return Some(Some(current));
    }
    peedle_css::parse_color(s).map(|c| Some(RgbaColor::new(c.r, c.g, c.b, c.a)))
}
