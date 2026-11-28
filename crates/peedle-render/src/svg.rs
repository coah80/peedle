use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use peedle_css::{serialize_tokens, Token};
use peedle_html::{Document, NodeId};

use crate::canvas2d::{Canvas2D, CanvasTextAlign, CanvasTextBaseline};
use crate::fonts::FontSystem;
use crate::framebuffer::{Affine2D, Framebuffer, RgbaColor};

thread_local! {
    static SVG_RASTER_CACHE: RefCell<HashMap<u64, (u32, u32, Vec<u8>)>> =
        RefCell::new(HashMap::new());
}

fn hash_svg_subtree(
    doc: &Document,
    nid: NodeId,
    custom_props: &HashMap<String, Vec<Token>>,
    h: &mut DefaultHasher,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    if let Some(el) = doc.element(nid) {
        el.tag_name.hash(h);
        for attr in &el.attributes {
            attr.name.hash(h);
            attr.value.hash(h);
            if attr.value.contains("var(") {
                expand_css_vars(&attr.value, custom_props).hash(h);
            }
        }
        if el.tag_name.eq_ignore_ascii_case("text") {
            doc.text_content(nid).hash(h);
        }
        if el.tag_name.eq_ignore_ascii_case("use") {
            let href = el.get_attr("href").or_else(|| el.get_attr("xlink:href"));
            if let Some(target) = href
                .and_then(|s| s.strip_prefix('#'))
                .and_then(|id| doc.get_element_by_id(id))
            {
                hash_svg_subtree(doc, target, custom_props, h, depth + 1);
            }
        }
        for cid in doc.element_children(nid) {
            hash_svg_subtree(doc, cid, custom_props, h, depth + 1);
        }
    }
}

struct SvgContext<'a> {
    doc: &'a Document,
    fonts: &'a FontSystem,
    vb_w: f32,
    vb_h: f32,
    base_font_size: f32,
    custom_props: &'a HashMap<String, Vec<Token>>,
}

#[derive(Clone)]
enum SvgPaint {
    Solid(RgbaColor),
    Gradient { id: String, opacity: f32 },
}

impl SvgPaint {
    fn with_alpha(self, alpha: f32) -> Self {
        match self {
            SvgPaint::Solid(c) => SvgPaint::Solid(c.with_alpha(alpha)),
            SvgPaint::Gradient { id, opacity } => SvgPaint::Gradient {
                id,
                opacity: opacity * alpha,
            },
        }
    }
}

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
    let empty = HashMap::new();
    render_svg_node_with_style(
        doc,
        svg_id,
        fb,
        fonts,
        dst_x,
        dst_y,
        dst_w,
        dst_h,
        RgbaColor::BLACK,
        16.0,
        &empty,
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
    let empty = HashMap::new();
    render_svg_node_with_style(
        doc,
        svg_id,
        fb,
        fonts,
        dst_x,
        dst_y,
        dst_w,
        dst_h,
        current_color,
        16.0,
        &empty,
    );
}

pub fn render_svg_node_with_style(
    doc: &Document,
    svg_id: NodeId,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    dst_x: f32,
    dst_y: f32,
    dst_w: f32,
    dst_h: f32,
    current_color: RgbaColor,
    base_font_size: f32,
    custom_props: &HashMap<String, Vec<Token>>,
) {
    if dst_w <= 0.0 || dst_h <= 0.0 {
        return;
    }
    let w_px = dst_w.ceil().max(1.0) as u32;
    let h_px = dst_h.ceil().max(1.0) as u32;
    let cache_eligible = w_px <= 1024 && h_px <= 1024;
    let cache_key = if cache_eligible {
        let mut h = DefaultHasher::new();
        w_px.hash(&mut h);
        h_px.hash(&mut h);
        current_color.r.hash(&mut h);
        current_color.g.hash(&mut h);
        current_color.b.hash(&mut h);
        current_color.a.hash(&mut h);
        base_font_size.to_bits().hash(&mut h);
        hash_svg_subtree(doc, svg_id, custom_props, &mut h, 0);
        let key = h.finish();
        let hit = SVG_RASTER_CACHE.with(|cache| {
            let c = cache.borrow();
            if let Some((cw, ch, data)) = c.get(&key) {
                fb.blit_rgba(*cw, *ch, data, dst_x, dst_y, dst_w, dst_h, 1.0);
                true
            } else {
                false
            }
        });
        if hit {
            return;
        }
        Some(key)
    } else {
        None
    };
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

    let ctx = SvgContext {
        doc,
        fonts,
        vb_w: vb_w.max(1.0),
        vb_h: vb_h.max(1.0),
        base_font_size: base_font_size.max(1.0),
        custom_props,
    };

    let inherited_fill = doc
        .element(svg_id)
        .and_then(|e| e.get_attr("fill"))
        .and_then(|v| parse_svg_paint(v, current_color, custom_props))
        .unwrap_or(Some(SvgPaint::Solid(current_color)));
    let inherited_stroke = doc
        .element(svg_id)
        .and_then(|e| e.get_attr("stroke"))
        .and_then(|v| parse_svg_paint(v, current_color, custom_props))
        .unwrap_or(None);
    let inherited_stroke_w = doc
        .element(svg_id)
        .and_then(|e| e.get_attr("stroke-width"))
        .and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok())
        .unwrap_or(1.0);

    render_svg_children(
        &ctx,
        svg_id,
        &mut canvas,
        &inherited_fill,
        &inherited_stroke,
        inherited_stroke_w,
        current_color,
        0,
    );
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
    if let Some(key) = cache_key {
        SVG_RASTER_CACHE.with(|cache| {
            let mut c = cache.borrow_mut();
            if c.len() >= 192 {
                c.clear();
            }
            c.insert(key, (canvas.fb.width, canvas.fb.height, canvas.fb.data));
        });
    }
}

fn render_svg_children(
    ctx: &SvgContext<'_>,
    parent: NodeId,
    canvas: &mut Canvas2D,
    parent_fill: &Option<SvgPaint>,
    parent_stroke: &Option<SvgPaint>,
    parent_stroke_w: f32,
    current_color: RgbaColor,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    for child in ctx.doc.element_children(parent) {
        render_svg_element(
            ctx,
            child,
            canvas,
            parent_fill,
            parent_stroke,
            parent_stroke_w,
            current_color,
            depth,
            false,
            false,
        );
    }
}

fn render_svg_element(
    ctx: &SvgContext<'_>,
    child: NodeId,
    canvas: &mut Canvas2D,
    parent_fill: &Option<SvgPaint>,
    parent_stroke: &Option<SvgPaint>,
    parent_stroke_w: f32,
    current_color: RgbaColor,
    depth: usize,
    from_use: bool,
    ignore_mask: bool,
) {
    if depth > 16 {
        return;
    }
    let Some(el) = ctx.doc.element(child) else {
        return;
    };
    let tag_lower = el.tag_name.to_ascii_lowercase();
    if matches!(
        tag_lower.as_str(),
        "defs" | "title" | "desc" | "style" | "mask" | "clippath" | "lineargradient" | "radialgradient" | "filter" | "metadata"
    ) || (!from_use && tag_lower == "symbol")
    {
        return;
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

    if !ignore_mask {
        if let Some(mask_id) = svg_attr("mask").and_then(extract_url_fragment) {
            if let Some(mask_node) = ctx.doc.get_element_by_id(&mask_id) {
                let mut elem_canvas = Canvas2D::new(canvas.fb.width, canvas.fb.height);
                elem_canvas.state = canvas.state.clone();
                render_svg_element(
                    ctx,
                    child,
                    &mut elem_canvas,
                    parent_fill,
                    parent_stroke,
                    parent_stroke_w,
                    current_color,
                    depth + 1,
                    from_use,
                    true,
                );
                let mut mask_canvas = Canvas2D::new(canvas.fb.width, canvas.fb.height);
                mask_canvas.state.transform = canvas.state.transform;
                let mask_default_fill = Some(SvgPaint::Solid(RgbaColor::WHITE));
                render_svg_children(
                    ctx,
                    mask_node,
                    &mut mask_canvas,
                    &mask_default_fill,
                    &None,
                    1.0,
                    RgbaColor::WHITE,
                    depth + 1,
                );
                let w = canvas.fb.width as i32;
                let h = canvas.fb.height as i32;
                for py in 0..h {
                    for px in 0..w {
                        let idx = ((py as u32 * canvas.fb.width + px as u32) * 4) as usize;
                        let ea = elem_canvas.fb.data[idx + 3];
                        if ea == 0 {
                            continue;
                        }
                        let mr = mask_canvas.fb.data[idx] as f32;
                        let mg = mask_canvas.fb.data[idx + 1] as f32;
                        let mb = mask_canvas.fb.data[idx + 2] as f32;
                        let ma = mask_canvas.fb.data[idx + 3] as f32 / 255.0;
                        let lum = ((0.2126 * mr + 0.7152 * mg + 0.0722 * mb) / 255.0) * ma;
                        let out_a = ((ea as f32) * lum).round().clamp(0.0, 255.0) as u8;
                        if out_a == 0 {
                            continue;
                        }
                        let c = RgbaColor::new(
                            elem_canvas.fb.data[idx],
                            elem_canvas.fb.data[idx + 1],
                            elem_canvas.fb.data[idx + 2],
                            out_a,
                        );
                        canvas.fb.blend_pixel(px, py, c);
                    }
                }
                return;
            }
        }
    }

    canvas.save();
    if let Some(tf) = el.get_attr("transform") {
        apply_svg_transform(canvas, tf);
    }

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
        .and_then(|v| {
            let expanded = expand_css_vars(v, ctx.custom_props);
            parse_svg_color_with_current(&expanded, current_color)
        })
        .flatten()
        .unwrap_or(current_color);

    let fill = match svg_attr("fill") {
        Some(v) => parse_svg_paint(v, effective_current, ctx.custom_props)
            .unwrap_or_else(|| parent_fill.clone()),
        None => parent_fill.clone(),
    }
    .map(|p| p.with_alpha(fill_op));

    let stroke = match svg_attr("stroke") {
        Some(v) => parse_svg_paint(v, effective_current, ctx.custom_props)
            .unwrap_or_else(|| parent_stroke.clone()),
        None => parent_stroke.clone(),
    }
    .map(|p| p.with_alpha(stroke_op));

    let stroke_w = svg_attr("stroke-width")
        .and_then(|s| s.trim_end_matches("px").parse::<f32>().ok())
        .unwrap_or(parent_stroke_w);
    let even_odd = svg_attr("fill-rule")
        .map_or(false, |v| v.eq_ignore_ascii_case("evenodd"));
    canvas.state.line_width = stroke_w;

    match tag_lower.as_str() {
        "g" | "a" | "svg" | "symbol" => {
            render_svg_children(
                ctx,
                child,
                canvas,
                &fill,
                &stroke,
                stroke_w,
                effective_current,
                depth + 1,
            );
        }
        "use" => {
            let ux = attr_len(el.get_attr("x"), ctx.vb_w, 0.0);
            let uy = attr_len(el.get_attr("y"), ctx.vb_h, 0.0);
            if ux != 0.0 || uy != 0.0 {
                canvas.translate(ux, uy);
            }
            let href = el.get_attr("href").or_else(|| el.get_attr("xlink:href"));
            if let Some(target_id) = href
                .and_then(|h| h.strip_prefix('#'))
                .and_then(|id| ctx.doc.get_element_by_id(id))
            {
                render_svg_element(
                    ctx,
                    target_id,
                    canvas,
                    &fill,
                    &stroke,
                    stroke_w,
                    effective_current,
                    depth + 1,
                    true,
                    false,
                );
            }
        }
        "rect" => {
            let x = attr_len(el.get_attr("x"), ctx.vb_w, 0.0);
            let y = attr_len(el.get_attr("y"), ctx.vb_h, 0.0);
            let w = attr_len(el.get_attr("width"), ctx.vb_w, 0.0);
            let h = attr_len(el.get_attr("height"), ctx.vb_h, 0.0);
            let rx_opt = el
                .get_attr("rx")
                .and_then(|s| parse_len_opt(s, ctx.vb_w));
            let ry_opt = el
                .get_attr("ry")
                .and_then(|s| parse_len_opt(s, ctx.vb_h));
            let rx = rx_opt.or(ry_opt).unwrap_or(0.0).clamp(0.0, w * 0.5);
            let ry = ry_opt.or(rx_opt).unwrap_or(0.0).clamp(0.0, h * 0.5);
            canvas.begin_path();
            if rx > 0.01 || ry > 0.01 {
                trace_rounded_rect(canvas, x, y, w, h, rx, ry);
            } else {
                canvas.rect(x, y, w, h);
            }
            paint_path(ctx, canvas, &fill, &stroke, even_odd);
        }
        "circle" => {
            let cx = attr_len(el.get_attr("cx"), ctx.vb_w, 0.0);
            let cy = attr_len(el.get_attr("cy"), ctx.vb_h, 0.0);
            let r = attr_len(el.get_attr("r"), ctx.vb_w.min(ctx.vb_h), 0.0);
            canvas.begin_path();
            canvas.arc(cx, cy, r, 0.0, std::f32::consts::TAU, false);
            canvas.close_path();
            paint_path(ctx, canvas, &fill, &stroke, even_odd);
        }
        "ellipse" => {
            let cx = attr_len(el.get_attr("cx"), ctx.vb_w, 0.0);
            let cy = attr_len(el.get_attr("cy"), ctx.vb_h, 0.0);
            let rx = attr_len(el.get_attr("rx"), ctx.vb_w, 0.0);
            let ry = attr_len(el.get_attr("ry"), ctx.vb_h, 0.0);
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
            paint_path(ctx, canvas, &fill, &stroke, even_odd);
        }
        "line" => {
            let x1 = attr_len(el.get_attr("x1"), ctx.vb_w, 0.0);
            let y1 = attr_len(el.get_attr("y1"), ctx.vb_h, 0.0);
            let x2 = attr_len(el.get_attr("x2"), ctx.vb_w, 0.0);
            let y2 = attr_len(el.get_attr("y2"), ctx.vb_h, 0.0);
            canvas.begin_path();
            canvas.move_to(x1, y1);
            canvas.line_to(x2, y2);
            paint_path(ctx, canvas, &None, &stroke, false);
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
                if tag_lower == "polygon" {
                    canvas.close_path();
                }
                paint_path(ctx, canvas, &fill, &stroke, even_odd);
            }
        }
        "path" => {
            if let Some(d) = el.get_attr("d") {
                trace_svg_path(canvas, d);
                paint_path(ctx, canvas, &fill, &stroke, even_odd);
            }
        }
        "text" => {
            let x = attr_len(el.get_attr("x"), ctx.vb_w, 0.0)
                + attr_len(el.get_attr("dx"), ctx.vb_w, 0.0);
            let y = attr_len(el.get_attr("y"), ctx.vb_h, 12.0)
                + attr_len(el.get_attr("dy"), ctx.vb_h, 0.0);
            let fs_user = svg_attr("font-size")
                .and_then(|s| parse_svg_font_size(s, ctx.base_font_size))
                .unwrap_or(ctx.base_font_size);
            let tf = canvas.state.transform;
            let tf_scale = tf.a.hypot(tf.b).max(0.01);
            canvas.state.font_size = fs_user * tf_scale;
            canvas.state.text_align = match svg_attr("text-anchor")
                .map(|s| s.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("middle") => CanvasTextAlign::Center,
                Some("end") => CanvasTextAlign::Right,
                _ => CanvasTextAlign::Left,
            };
            canvas.state.text_baseline = match svg_attr("dominant-baseline")
                .map(|s| s.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("central") | Some("middle") => CanvasTextBaseline::Middle,
                Some("hanging") | Some("text-top") | Some("text-before-edge") => {
                    CanvasTextBaseline::Top
                }
                Some("text-bottom") | Some("text-after-edge") => CanvasTextBaseline::Bottom,
                _ => CanvasTextBaseline::Alphabetic,
            };
            if let Some(SvgPaint::Solid(c)) = fill {
                canvas.state.fill_style = c;
                let txt = ctx.doc.text_content(child);
                canvas.fill_text(ctx.fonts, txt.trim(), x, y);
            }
        }
        _ => {}
    }
    canvas.restore();
}

fn paint_path(
    ctx: &SvgContext<'_>,
    canvas: &mut Canvas2D,
    fill: &Option<SvgPaint>,
    stroke: &Option<SvgPaint>,
    even_odd: bool,
) {
    match fill {
        Some(SvgPaint::Solid(c)) => {
            canvas.state.fill_style = *c;
            canvas.fill(even_odd);
        }
        Some(SvgPaint::Gradient { id, opacity }) => {
            paint_gradient_fill(ctx, canvas, id, *opacity, even_odd);
        }
        None => {}
    }
    match stroke {
        Some(SvgPaint::Solid(c)) => {
            canvas.state.stroke_style = *c;
            canvas.stroke();
        }
        Some(SvgPaint::Gradient { id, opacity }) => {
            if let Some(c) = fallback_gradient_color(ctx, id) {
                canvas.state.stroke_style = c.with_alpha(*opacity);
                canvas.stroke();
            }
        }
        None => {}
    }
}

fn fallback_gradient_color(ctx: &SvgContext<'_>, id: &str) -> Option<RgbaColor> {
    let grad_node = ctx.doc.get_element_by_id(id)?;
    let stops = collect_gradient_stops(ctx, grad_node, 0);
    stops.first().map(|s| s.1)
}

fn collect_gradient_stops(
    ctx: &SvgContext<'_>,
    grad_node: NodeId,
    depth: usize,
) -> Vec<(f32, RgbaColor)> {
    if depth > 4 {
        return Vec::new();
    }
    let mut stops = Vec::new();
    for cid in ctx.doc.element_children(grad_node) {
        let Some(el) = ctx.doc.element(cid) else {
            continue;
        };
        if !el.tag_name.eq_ignore_ascii_case("stop") {
            continue;
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
        let offset = el
            .get_attr("offset")
            .and_then(|raw| {
                let s = raw.trim();
                if let Some(p) = s.strip_suffix('%') {
                    p.trim().parse::<f32>().ok().map(|v| (v / 100.0).clamp(0.0, 1.0))
                } else {
                    s.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0))
                }
            })
            .unwrap_or(0.0);
        let color_str = style_prop("stop-color")
            .or_else(|| el.get_attr("stop-color"))
            .unwrap_or("#000000");
        let expanded = expand_css_vars(color_str, ctx.custom_props);
        let base_c = parse_svg_color_with_current(&expanded, RgbaColor::BLACK)
            .flatten()
            .unwrap_or(RgbaColor::BLACK);
        let stop_op = style_prop("stop-opacity")
            .or_else(|| el.get_attr("stop-opacity"))
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        stops.push((offset, base_c.with_alpha(stop_op)));
    }
    if stops.is_empty() {
        if let Some(el) = ctx.doc.element(grad_node) {
            let href = el.get_attr("href").or_else(|| el.get_attr("xlink:href"));
            if let Some(ref_node) = href
                .and_then(|h| h.strip_prefix('#'))
                .and_then(|rid| ctx.doc.get_element_by_id(rid))
            {
                return collect_gradient_stops(ctx, ref_node, depth + 1);
            }
        }
    }
    stops
}

fn sample_stops(stops: &[(f32, RgbaColor)], t: f32) -> RgbaColor {
    if stops.is_empty() {
        return RgbaColor::TRANSPARENT;
    }
    if stops.len() == 1 || t <= stops[0].0 {
        return stops[0].1;
    }
    let last = stops.len() - 1;
    if t >= stops[last].0 {
        return stops[last].1;
    }
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let span = (t1 - t0).max(1e-6);
            let f = ((t - t0) / span).clamp(0.0, 1.0);
            let lerp = |a: u8, b: u8| -> u8 {
                (a as f32 + (b as f32 - a as f32) * f).round().clamp(0.0, 255.0) as u8
            };
            return RgbaColor::new(
                lerp(c0.r, c1.r),
                lerp(c0.g, c1.g),
                lerp(c0.b, c1.b),
                lerp(c0.a, c1.a),
            );
        }
    }
    stops[last].1
}

fn paint_gradient_fill(
    ctx: &SvgContext<'_>,
    canvas: &mut Canvas2D,
    id: &str,
    opacity: f32,
    even_odd: bool,
) {
    let Some(grad_node) = ctx.doc.get_element_by_id(id) else {
        return;
    };
    let Some(grad_el) = ctx.doc.element(grad_node) else {
        return;
    };
    let stops = collect_gradient_stops(ctx, grad_node, 0);
    if stops.is_empty() {
        return;
    }
    let contours = canvas.path_contours();
    if contours.is_empty() {
        return;
    }
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for poly in &contours {
        for &(px, py) in poly {
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
    }
    let x0 = (min_x.floor() as i32).max(0);
    let y0 = (min_y.floor() as i32).max(0);
    let x1 = (max_x.ceil() as i32).min(canvas.fb.width as i32);
    let y1 = (max_y.ceil() as i32).min(canvas.fb.height as i32);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let bw = (x1 - x0) as u32;
    let bh = (y1 - y0) as u32;
    let shifted: Vec<Vec<(f32, f32)>> = contours
        .iter()
        .map(|poly| poly.iter().map(|&(px, py)| (px - x0 as f32, py - y0 as f32)).collect())
        .collect();
    let mut mask = Framebuffer::new(bw, bh);
    mask.fill_polygon(&shifted, RgbaColor::WHITE, even_odd);

    let units_user = grad_el
        .get_attr("gradientUnits")
        .or_else(|| grad_el.get_attr("gradientunits"))
        .map_or(false, |u| u.eq_ignore_ascii_case("userspaceonuse"));
    let mut gtf = canvas.state.transform;
    if let Some(gt) = grad_el
        .get_attr("gradientTransform")
        .or_else(|| grad_el.get_attr("gradienttransform"))
    {
        let mut tmp = Canvas2D::new(1, 1);
        tmp.state.transform = gtf;
        apply_svg_transform(&mut tmp, gt);
        gtf = tmp.state.transform;
    }

    let is_radial = grad_el.tag_name.eq_ignore_ascii_case("radialgradient");
    let total_alpha = (opacity * canvas.state.global_alpha).clamp(0.0, 1.0);
    let bbox_w = (max_x - min_x).max(1.0);
    let bbox_h = (max_y - min_y).max(1.0);

    if is_radial {
        let parse_coord = |attr: &str, default_pct: f32, ref_len: f32| -> f32 {
            match grad_el.get_attr(attr) {
                Some(s) => {
                    let t = s.trim();
                    if let Some(p) = t.strip_suffix('%') {
                        p.trim().parse::<f32>().unwrap_or(default_pct * 100.0) / 100.0 * ref_len
                    } else if units_user {
                        t.trim_end_matches("px").parse::<f32>().unwrap_or(default_pct * ref_len)
                    } else {
                        t.parse::<f32>().unwrap_or(default_pct) * ref_len
                    }
                }
                None => default_pct * ref_len,
            }
        };
        let (cx, cy, r) = if units_user {
            let ux = parse_coord("cx", 0.5, ctx.vb_w);
            let uy = parse_coord("cy", 0.5, ctx.vb_h);
            let ur = parse_coord("r", 0.5, ctx.vb_w.min(ctx.vb_h));
            let (pcx, pcy) = gtf.apply(ux, uy);
            let (prx, pry) = gtf.apply(ux + ur, uy);
            (pcx, pcy, (prx - pcx).hypot(pry - pcy).max(1e-4))
        } else {
            let cx = min_x + parse_coord("cx", 0.5, bbox_w);
            let cy = min_y + parse_coord("cy", 0.5, bbox_h);
            let r = parse_coord("r", 0.5, bbox_w.max(bbox_h)).max(1e-4);
            (cx, cy, r)
        };
        for ly in 0..bh {
            let py = y0 + ly as i32;
            let dy = py as f32 + 0.5 - cy;
            for lx in 0..bw {
                let a = mask.data[((ly * bw + lx) * 4 + 3) as usize];
                if a == 0 {
                    continue;
                }
                let px = x0 + lx as i32;
                let dx = px as f32 + 0.5 - cx;
                let t = (dx.hypot(dy) / r).clamp(0.0, 1.0);
                let c = sample_stops(&stops, t).with_alpha((a as f32 / 255.0) * total_alpha);
                canvas.fb.blend_pixel(px, py, c);
            }
        }
    } else {
        let parse_coord = |attr: &str, default_frac: f32, ref_len: f32| -> f32 {
            match grad_el.get_attr(attr) {
                Some(s) => {
                    let t = s.trim();
                    if let Some(p) = t.strip_suffix('%') {
                        p.trim().parse::<f32>().unwrap_or(default_frac * 100.0) / 100.0 * ref_len
                    } else if units_user {
                        t.trim_end_matches("px").parse::<f32>().unwrap_or(default_frac * ref_len)
                    } else {
                        t.parse::<f32>().unwrap_or(default_frac) * ref_len
                    }
                }
                None => default_frac * ref_len,
            }
        };
        let (p1x, p1y, p2x, p2y) = if units_user {
            let x1u = parse_coord("x1", 0.0, ctx.vb_w);
            let y1u = parse_coord("y1", 0.0, ctx.vb_h);
            let x2u = parse_coord("x2", 1.0, ctx.vb_w);
            let y2u = parse_coord("y2", 0.0, ctx.vb_h);
            let (ax, ay) = gtf.apply(x1u, y1u);
            let (bx, by) = gtf.apply(x2u, y2u);
            (ax, ay, bx, by)
        } else {
            let ax = min_x + parse_coord("x1", 0.0, bbox_w);
            let ay = min_y + parse_coord("y1", 0.0, bbox_h);
            let bx = min_x + parse_coord("x2", 1.0, bbox_w);
            let by = min_y + parse_coord("y2", 0.0, bbox_h);
            (ax, ay, bx, by)
        };
        let vx = p2x - p1x;
        let vy = p2y - p1y;
        let len_sq = (vx * vx + vy * vy).max(1e-6);
        for ly in 0..bh {
            let py = y0 + ly as i32;
            let dy = py as f32 + 0.5 - p1y;
            for lx in 0..bw {
                let a = mask.data[((ly * bw + lx) * 4 + 3) as usize];
                if a == 0 {
                    continue;
                }
                let px = x0 + lx as i32;
                let dx = px as f32 + 0.5 - p1x;
                let t = ((dx * vx + dy * vy) / len_sq).clamp(0.0, 1.0);
                let c = sample_stops(&stops, t).with_alpha((a as f32 / 255.0) * total_alpha);
                canvas.fb.blend_pixel(px, py, c);
            }
        }
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
    let mut tokens = tokenize_path_data(d);
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
                let Some(rx) = next_num(&tokens, &mut i) else { break };
                let Some(ry) = next_num(&tokens, &mut i) else { break };
                let Some(rot) = next_num(&tokens, &mut i) else { break };
                let Some(la) = next_flag(&mut tokens, &mut i) else { break };
                let Some(sw) = next_flag(&mut tokens, &mut i) else { break };
                let Some(x) = next_num(&tokens, &mut i) else { break };
                let Some(y) = next_num(&tokens, &mut i) else { break };
                let (nx, ny) = if cmd == 'a' { (cx + x, cy + y) } else { (x, y) };
                trace_svg_arc(canvas, cx, cy, rx, ry, rot, la, sw, nx, ny);
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

fn next_flag(tokens: &mut [String], i: &mut usize) -> Option<bool> {
    let tok = tokens.get_mut(*i)?;
    let b = *tok.as_bytes().first()?;
    if b != b'0' && b != b'1' {
        return None;
    }
    let flag = b == b'1';
    if tok.len() == 1 {
        *i += 1;
    } else {
        *tok = tok[1..].to_string();
    }
    Some(flag)
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

fn parse_len_opt(raw: &str, percent_base: f32) -> Option<f32> {
    let s = raw.trim();
    if let Some(p) = s.strip_suffix('%') {
        p.trim().parse::<f32>().ok().map(|v| v * percent_base / 100.0)
    } else {
        s.trim_end_matches("px").parse::<f32>().ok()
    }
}

fn attr_len(v: Option<&str>, percent_base: f32, default: f32) -> f32 {
    v.and_then(|s| parse_len_opt(s, percent_base)).unwrap_or(default)
}

fn parse_svg_font_size(raw: &str, base_fs: f32) -> Option<f32> {
    let s = raw.trim();
    if let Some(em) = s.strip_suffix("rem") {
        em.trim().parse::<f32>().ok().map(|v| v * 16.0)
    } else if let Some(em) = s.strip_suffix("em") {
        em.trim().parse::<f32>().ok().map(|v| v * base_fs)
    } else if let Some(p) = s.strip_suffix('%') {
        p.trim().parse::<f32>().ok().map(|v| v * base_fs / 100.0)
    } else {
        s.trim_end_matches("px").parse::<f32>().ok()
    }
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
                if args.len() >= 3 {
                    canvas.translate(args[1], args[2]);
                    canvas.rotate(args[0].to_radians());
                    canvas.translate(-args[1], -args[2]);
                } else {
                    canvas.rotate(args[0].to_radians());
                }
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

fn expand_css_vars(raw: &str, custom_props: &HashMap<String, Vec<Token>>) -> String {
    let mut cur = raw.trim().to_string();
    for _ in 0..6 {
        let Some(pos) = cur.find("var(") else {
            break;
        };
        let after = &cur[pos + 4..];
        let mut depth = 1i32;
        let mut close_rel = None;
        for (idx, ch) in after.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close_rel = Some(idx);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(cidx) = close_rel else {
            break;
        };
        let inner = &after[..cidx];
        let (var_name, fallback) = match inner.find(',') {
            Some(comma) => (inner[..comma].trim(), Some(inner[comma + 1..].trim())),
            None => (inner.trim(), None),
        };
        let resolved = custom_props
            .get(var_name)
            .map(|toks| serialize_tokens(toks).trim().to_string())
            .filter(|v| !v.is_empty());
        let replacement = resolved
            .as_deref()
            .or(fallback)
            .unwrap_or("");
        let mut next = String::with_capacity(cur.len());
        next.push_str(&cur[..pos]);
        next.push_str(replacement);
        next.push_str(&after[cidx + 1..]);
        cur = next;
    }
    cur
}

fn extract_url_fragment(raw: &str) -> Option<String> {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    let start = lower.find("url(")?;
    let rest = &s[start + 4..];
    let end = rest.find(')')?;
    let inner = rest[..end].trim().trim_matches(|c| c == '"' || c == '\'');
    inner.strip_prefix('#').map(|id| id.to_string())
}

fn parse_svg_paint(
    raw: &str,
    current: RgbaColor,
    custom_props: &HashMap<String, Vec<Token>>,
) -> Option<Option<SvgPaint>> {
    let expanded = expand_css_vars(raw, custom_props);
    let s = expanded.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(id) = extract_url_fragment(s) {
        return Some(Some(SvgPaint::Gradient { id, opacity: 1.0 }));
    }
    parse_svg_color_with_current(s, current).map(|opt| opt.map(SvgPaint::Solid))
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
