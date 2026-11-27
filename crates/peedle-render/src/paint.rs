use std::collections::HashMap;

use peedle_css::{
    BackgroundImage, BorderStyle, CalcContext, Color, ComputedStyle, FontStyle, Overflow, Position,
    TextDecorationLine, TransformOp, Visibility,
};
use peedle_html::{Document, NodeId};
use peedle_layout::{BoxKind, LayoutBoxId, LayoutTree, ReplacedKind};

use crate::canvas2d::Canvas2D;
use crate::fonts::{FontFaceKey, FontSystem};
use crate::framebuffer::{ClipRect, Framebuffer, RgbaColor};
use crate::images::ImageCache;
use crate::svg::render_svg_node_with_color;

pub fn paint_layout(
    doc: &Document,
    tree: &LayoutTree,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    canvases: &HashMap<NodeId, Canvas2D>,
    scroll_x: f32,
    scroll_y: f32,
) {
    let bg = doc
        .document_element()
        .and_then(|id| tree.box_of(id))
        .map(|b| b.style.background_color)
        .filter(|c| c.a > 0)
        .or_else(|| {
            doc.body()
                .and_then(|id| tree.box_of(id))
                .map(|b| b.style.background_color)
                .filter(|c| c.a > 0)
        })
        .unwrap_or(Color::WHITE);

    fb.clear(to_rgba(bg));

    if let Some(root) = tree.root {
        paint_box(doc, tree, root, fb, fonts, images, canvases, -scroll_x, -scroll_y);
    }
}

fn is_inline_only_container(tree: &LayoutTree, id: LayoutBoxId) -> bool {
    let b = tree.get(id);
    if b.style.background_color.a > 0
        || !matches!(b.style.background_image, BackgroundImage::None)
        || b.dims.border.horizontal() > 0.0
        || b.dims.border.vertical() > 0.0
    {
        return false;
    }
    !b.children.is_empty()
        && b.children.iter().all(|&c| {
            let cb = tree.get(c);
            cb.style.float == peedle_css::Float::None
                && cb.style.position == peedle_css::Position::Static
                && matches!(
                    cb.kind,
                    BoxKind::Inline | BoxKind::InlineBlock | BoxKind::Text(_) | BoxKind::Replaced(_)
                )
        })
}

fn to_rgba(c: Color) -> RgbaColor {
    RgbaColor::new(c.r, c.g, c.b, c.a)
}

fn resolve_radii(style: &ComputedStyle, w: f32, h: f32) -> [f32; 4] {
    let ctx = CalcContext {
        font_size: style.font_size,
        root_font_size: 16.0,
        viewport_width: w,
        viewport_height: h,
        percentage_base: w.min(h),
    };
    [
        style.border_top_left_radius.resolve_or(&ctx, 0.0).max(0.0),
        style.border_top_right_radius.resolve_or(&ctx, 0.0).max(0.0),
        style.border_bottom_right_radius.resolve_or(&ctx, 0.0).max(0.0),
        style.border_bottom_left_radius.resolve_or(&ctx, 0.0).max(0.0),
    ]
}

fn paint_box(
    doc: &Document,
    tree: &LayoutTree,
    id: LayoutBoxId,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    canvases: &HashMap<NodeId, Canvas2D>,
    ox: f32,
    oy: f32,
) {
    let b = tree.get(id);
    if b.style.visibility == Visibility::Hidden || b.style.opacity <= 0.001 {
        return;
    }

    let is_fixed = b.style.position == Position::Fixed;
    let mut dx = if is_fixed { 0.0 } else { ox };
    let mut dy = if is_fixed { 0.0 } else { oy };

    let bb = b.dims.border_box();
    if b.style.position == Position::Sticky {
        let ctx_x = CalcContext {
            font_size: b.style.font_size,
            root_font_size: 16.0,
            viewport_width: fb.width as f32,
            viewport_height: fb.height as f32,
            percentage_base: fb.width as f32,
        };
        let ctx_y = CalcContext {
            percentage_base: fb.height as f32,
            ..ctx_x
        };
        if let Some(top_limit) = b.style.top.resolve(&ctx_y) {
            let screen_y = bb.y + dy;
            if screen_y < top_limit {
                dy += top_limit - screen_y;
            }
        }
        if let Some(left_limit) = b.style.left.resolve(&ctx_x) {
            let screen_x = bb.x + dx;
            if screen_x < left_limit {
                dx += left_limit - screen_x;
            }
        }
    }

    if !b.style.transform.is_empty() {
        let ctx_w = CalcContext {
            font_size: b.style.font_size,
            root_font_size: 16.0,
            viewport_width: fb.width as f32,
            viewport_height: fb.height as f32,
            percentage_base: bb.width,
        };
        let ctx_h = CalcContext {
            percentage_base: bb.height,
            ..ctx_w
        };
        for op in &b.style.transform {
            match op {
                TransformOp::Translate(tx, ty) => {
                    dx += tx.resolve_or(&ctx_w, 0.0);
                    dy += ty.resolve_or(&ctx_h, 0.0);
                }
                TransformOp::TranslateX(tx) => {
                    dx += tx.resolve_or(&ctx_w, 0.0);
                }
                TransformOp::TranslateY(ty) => {
                    dy += ty.resolve_or(&ctx_h, 0.0);
                }
                TransformOp::Matrix(_, _, _, _, e, f) => {
                    dx += *e;
                    dy += *f;
                }
                _ => {}
            }
        }
    }

    let mut alpha = b.style.opacity.clamp(0.0, 1.0);
    for fop in &b.style.filter {
        if let peedle_css::FilterOp::Opacity(fo) = fop {
            alpha *= fo.clamp(0.0, 1.0);
        }
    }
    let rx = bb.x + dx;
    let ry = bb.y + dy;
    let radii = resolve_radii(&b.style, bb.width, bb.height);

    if !matches!(b.kind, BoxKind::Text(_)) {
        for sh in &b.style.box_shadow {
            if !sh.inset {
                fb.draw_box_shadow(
                    rx,
                    ry,
                    bb.width,
                    bb.height,
                    radii,
                    sh.offset_x,
                    sh.offset_y,
                    sh.blur_radius,
                    sh.spread_radius,
                    to_rgba(sh.color).with_alpha(alpha),
                    false,
                );
            }
        }

        if let Some(ref mask_u) = b.style.mask_image {
            if b.style.background_color.a > 0 {
                if let Some(mimg) = images.get(mask_u) {
                    if mimg.width > 0 && mimg.height > 0 && bb.width > 0.0 && bb.height > 0.0 {
                        let sx = bb.width / mimg.width as f32;
                        let sy = bb.height / mimg.height as f32;
                        let s = sx.min(sy);
                        let tw = mimg.width as f32 * s;
                        let th = mimg.height as f32 * s;
                        let tx = rx + (bb.width - tw) * 0.5;
                        let ty = ry + (bb.height - th) * 0.5;
                        fb.blit_tinted_mask_rgba(
                            mimg.width,
                            mimg.height,
                            &mimg.rgba,
                            tx,
                            ty,
                            tw,
                            th,
                            to_rgba(b.style.background_color).with_alpha(alpha),
                        );
                    }
                }
            }
        } else if b.style.background_color.a > 0 {
            let (bg_x, bg_y, bg_w, bg_h) = match b.style.background_clip {
                peedle_css::BackgroundClip::PaddingBox => {
                    let pb = b.dims.padding_box();
                    (pb.x + dx, pb.y + dy, pb.width, pb.height)
                }
                peedle_css::BackgroundClip::ContentBox => {
                    let cb = b.dims.content;
                    (cb.x + dx, cb.y + dy, cb.width, cb.height)
                }
                _ => (rx, ry, bb.width, bb.height),
            };
            fb.fill_rounded_rect(
                bg_x,
                bg_y,
                bg_w,
                bg_h,
                radii,
                to_rgba(b.style.background_color).with_alpha(alpha),
            );
        }

        match &b.style.background_image {
            BackgroundImage::LinearGradient {
                angle_deg,
                repeating,
                stops,
            } => {
                let norm = normalize_stops(stops, alpha);
                fb.fill_linear_gradient(rx, ry, bb.width, bb.height, radii, *angle_deg, &norm, *repeating);
            }
            BackgroundImage::RadialGradient { repeating, stops, .. } => {
                let norm = normalize_stops(stops, alpha);
                fb.fill_radial_gradient(rx, ry, bb.width, bb.height, radii, &norm, *repeating);
            }
            BackgroundImage::ConicGradient {
                from_deg,
                repeating,
                stops,
                ..
            } => {
                let norm = normalize_stops(stops, alpha);
                fb.fill_conic_gradient(
                    rx,
                    ry,
                    bb.width,
                    bb.height,
                    radii,
                    *from_deg,
                    &norm,
                    *repeating,
                );
            }
            BackgroundImage::Url(u) => {
                if let Some(img) = images.get(u) {
                    if img.width > 0 && img.height > 0 && bb.width > 0.0 && bb.height > 0.0 {
                        fb.push_clip(ClipRect {
                            x: rx.floor() as i32,
                            y: ry.floor() as i32,
                            w: bb.width.ceil() as i32,
                            h: bb.height.ceil() as i32,
                        });
                        let (tw, th) = match &b.style.background_size {
                            peedle_css::BackgroundSize::Cover => {
                                let sx = bb.width / img.width as f32;
                                let sy = bb.height / img.height as f32;
                                let s = sx.max(sy);
                                (img.width as f32 * s, img.height as f32 * s)
                            }
                            peedle_css::BackgroundSize::Contain => {
                                let sx = bb.width / img.width as f32;
                                let sy = bb.height / img.height as f32;
                                let s = sx.min(sy);
                                (img.width as f32 * s, img.height as f32 * s)
                            }
                            peedle_css::BackgroundSize::Explicit(bw, bh) => {
                                let ctx_w = CalcContext {
                                    font_size: b.style.font_size,
                                    root_font_size: 16.0,
                                    viewport_width: fb.width as f32,
                                    viewport_height: fb.height as f32,
                                    percentage_base: bb.width,
                                };
                                let ctx_h = CalcContext {
                                    percentage_base: bb.height,
                                    ..ctx_w
                                };
                                let aspect = img.width as f32 / img.height.max(1) as f32;
                                match (bw.resolve(&ctx_w), bh.resolve(&ctx_h)) {
                                    (Some(w), Some(h)) => (w.max(1.0), h.max(1.0)),
                                    (Some(w), None) => (w.max(1.0), (w / aspect).max(1.0)),
                                    (None, Some(h)) => ((h * aspect).max(1.0), h.max(1.0)),
                                    (None, None) => (img.width as f32, img.height as f32),
                                }
                            }
                            _ => (img.width as f32, img.height as f32),
                        };
                        let (rep_x, rep_y) = match b.style.background_repeat {
                            peedle_css::BackgroundRepeat::NoRepeat => (false, false),
                            peedle_css::BackgroundRepeat::RepeatX => (true, false),
                            peedle_css::BackgroundRepeat::RepeatY => (false, true),
                            _ => (true, true),
                        };
                        let pb = b.dims.padding_box();
                        let (area_x, area_y, area_w, area_h) = if b.style.background_attachment_fixed {
                            (0.0f32, 0.0f32, fb.width as f32, fb.height as f32)
                        } else {
                            (pb.x + dx, pb.y + dy, pb.width, pb.height)
                        };
                        let ctx_x = CalcContext {
                            font_size: b.style.font_size,
                            root_font_size: 16.0,
                            viewport_width: fb.width as f32,
                            viewport_height: fb.height as f32,
                            percentage_base: (area_w - tw).max(0.0),
                        };
                        let ctx_y = CalcContext {
                            percentage_base: (area_h - th).max(0.0),
                            ..ctx_x
                        };
                        let bg_ox = area_x + b.style.background_position_x.resolve_or(&ctx_x, 0.0);
                        let bg_oy = area_y + b.style.background_position_y.resolve_or(&ctx_y, 0.0);

                        if img.width == 1 && img.height == 1 && rep_x && rep_y {
                            let c = RgbaColor::new(img.rgba[0], img.rgba[1], img.rgba[2], img.rgba[3])
                                .with_alpha(alpha);
                            fb.fill_rounded_rect(rx, ry, bb.width, bb.height, radii, c);
                        } else {
                            let (c_start, c_end) = if rep_x && tw >= 1.0 {
                                let s = ((rx - bg_ox) / tw).floor() as i32;
                                let e = ((rx + bb.width - bg_ox) / tw).ceil() as i32;
                                (s, e.min(s + 2048))
                            } else {
                                (0, 1)
                            };
                            let (r_start, r_end) = if rep_y && th >= 1.0 {
                                let s = ((ry - bg_oy) / th).floor() as i32;
                                let e = ((ry + bb.height - bg_oy) / th).ceil() as i32;
                                (s, e.min(s + 2048))
                            } else {
                                (0, 1)
                            };
                            for row in r_start..r_end {
                                for col in c_start..c_end {
                                    let tx = bg_ox + col as f32 * tw;
                                    let ty = bg_oy + row as f32 * th;
                                    if tx + tw <= rx || tx >= rx + bb.width || ty + th <= ry || ty >= ry + bb.height {
                                        continue;
                                    }
                                    fb.blit_rgba(
                                        img.width,
                                        img.height,
                                        &img.rgba,
                                        tx,
                                        ty,
                                        tw,
                                        th,
                                        alpha,
                                    );
                                }
                            }
                        }
                        fb.pop_clip();
                    }
                }
            }
            _ => {}
        }

        let pb_in = b.dims.padding_box();
        let in_radii = [
            (radii[0] - b.dims.border.left.max(b.dims.border.top)).max(0.0),
            (radii[1] - b.dims.border.right.max(b.dims.border.top)).max(0.0),
            (radii[2] - b.dims.border.right.max(b.dims.border.bottom)).max(0.0),
            (radii[3] - b.dims.border.left.max(b.dims.border.bottom)).max(0.0),
        ];
        for sh in &b.style.box_shadow {
            if sh.inset {
                fb.draw_box_shadow(
                    pb_in.x + dx,
                    pb_in.y + dy,
                    pb_in.width,
                    pb_in.height,
                    in_radii,
                    sh.offset_x,
                    sh.offset_y,
                    sh.blur_radius,
                    sh.spread_radius,
                    to_rgba(sh.color).with_alpha(alpha),
                    true,
                );
            }
        }

        let widths = [
            b.dims.border.top,
            b.dims.border.right,
            b.dims.border.bottom,
            b.dims.border.left,
        ];
        if widths.iter().any(|&w| w > 0.0) {
            let colors = [
                to_rgba(b.style.border_top_color).with_alpha(alpha),
                to_rgba(b.style.border_right_color).with_alpha(alpha),
                to_rgba(b.style.border_bottom_color).with_alpha(alpha),
                to_rgba(b.style.border_left_color).with_alpha(alpha),
            ];
            let style_code = |bs: BorderStyle| -> u8 {
                match bs {
                    BorderStyle::Dashed => 1,
                    BorderStyle::Dotted => 2,
                    BorderStyle::Double => 3,
                    _ => 0,
                }
            };
            let bstyles = [
                style_code(b.style.border_top_style),
                style_code(b.style.border_right_style),
                style_code(b.style.border_bottom_style),
                style_code(b.style.border_left_style),
            ];
            fb.stroke_styled_borders(rx, ry, bb.width, bb.height, radii, widths, colors, bstyles);
        }

        if b.style.outline_width > 0.0
            && !matches!(b.style.outline_style, BorderStyle::None | BorderStyle::Hidden)
        {
            let ow = b.style.outline_width;
            let off = b.style.outline_offset;
            let expand = off + ow;
            let oc = to_rgba(b.style.outline_color).with_alpha(alpha);
            let out_radii = [
                (radii[0] + expand).max(0.0),
                (radii[1] + expand).max(0.0),
                (radii[2] + expand).max(0.0),
                (radii[3] + expand).max(0.0),
            ];
            fb.stroke_rounded_rect(
                rx - expand,
                ry - expand,
                bb.width + expand * 2.0,
                bb.height + expand * 2.0,
                out_radii,
                [ow; 4],
                [oc; 4],
            );
        }

        if let Some(ref marker) = b.list_marker {
            let key = fonts.select_face(
                &b.style.font_family,
                b.style.font_weight,
                b.style.font_style == FontStyle::Italic,
            );
            let mw = fonts.measure_text(marker, key, b.style.font_size);
            let asc = fonts.line_ascent(key, b.style.font_size);
            fonts.draw_text(
                fb,
                marker,
                b.dims.content.x + dx - mw - 4.0,
                b.dims.content.y + dy + asc,
                key,
                b.style.font_size,
                to_rgba(b.style.color).with_alpha(alpha),
            );
        }
    }

    match &b.kind {
        BoxKind::Text(_) => {
            let key = fonts.select_face(
                &b.style.font_family,
                b.style.font_weight,
                b.style.font_style == FontStyle::Italic,
            );
            let color = to_rgba(b.style.color).with_alpha(alpha);
            for frag in &b.text_fragments {
                let fx = frag.x + dx;
                let fy = frag.baseline + dy;
                for ts in &b.style.text_shadow {
                    fonts.draw_text(
                        fb,
                        &frag.text,
                        fx + ts.offset_x,
                        fy + ts.offset_y,
                        key,
                        b.style.font_size,
                        to_rgba(ts.color).with_alpha(alpha),
                    );
                }
                fonts.draw_text(fb, &frag.text, fx, fy, key, b.style.font_size, color);
                match b.style.text_decoration_line {
                    TextDecorationLine::Underline => {
                        let dec_c = b
                            .style
                            .text_decoration_color
                            .map(to_rgba)
                            .unwrap_or(color);
                        fb.fill_rect(fx, fy + 2.0, frag.width, 1.0, dec_c);
                    }
                    TextDecorationLine::LineThrough => {
                        let dec_c = b
                            .style
                            .text_decoration_color
                            .map(to_rgba)
                            .unwrap_or(color);
                        fb.fill_rect(fx, fy - b.style.font_size * 0.3, frag.width, 1.0, dec_c);
                    }
                    TextDecorationLine::Overline => {
                        let dec_c = b
                            .style
                            .text_decoration_color
                            .map(to_rgba)
                            .unwrap_or(color);
                        fb.fill_rect(fx, frag.y + dy, frag.width, 1.0, dec_c);
                    }
                    TextDecorationLine::None => {}
                }
            }
        }
        BoxKind::Replaced(rep) => {
            let cx = b.dims.content.x + dx;
            let cy = b.dims.content.y + dy;
            let cw = b.dims.content.width;
            let ch = b.dims.content.height;
            paint_replaced(doc, b.node, rep, &b.style, fb, fonts, images, canvases, cx, cy, cw, ch, alpha);
        }
        _ => {}
    }

    let clips = !matches!(b.style.overflow_x, Overflow::Visible)
        || !matches!(b.style.overflow_y, Overflow::Visible);
    if clips {
        let pb = b.dims.padding_box();
        fb.push_clip(ClipRect {
            x: (pb.x + dx).floor() as i32,
            y: (pb.y + dy).floor() as i32,
            w: pb.width.ceil() as i32,
            h: pb.height.ceil() as i32,
        });
    }

    let mut ordered_children = b.children.clone();
    ordered_children.sort_by_key(|&cid| {
        let cb = tree.get(cid);
        let z = cb.style.z_index.unwrap_or(0);
        let positioned = cb.style.position != peedle_css::Position::Static;
        if positioned && z < 0 {
            (0i32, z)
        } else if positioned {
            (4i32, z)
        } else if cb.style.float != peedle_css::Float::None {
            (2i32, 0)
        } else if matches!(cb.kind, BoxKind::Inline | BoxKind::InlineBlock | BoxKind::Text(_) | BoxKind::Replaced(_))
            || is_inline_only_container(tree, cid)
        {
            (3i32, 0)
        } else {
            (1i32, 0)
        }
    });

    let (scroll_dx, scroll_dy) = b
        .node
        .and_then(|nid| doc.element(nid))
        .map(|el| (el.scroll_left, el.scroll_top))
        .unwrap_or((0.0, 0.0));

    for cid in ordered_children {
        paint_box(
            doc,
            tree,
            cid,
            fb,
            fonts,
            images,
            canvases,
            dx - scroll_dx,
            dy - scroll_dy,
        );
    }

    if clips {
        fb.pop_clip();
    }
}

fn paint_replaced(
    doc: &Document,
    node: Option<NodeId>,
    rep: &ReplacedKind,
    style: &ComputedStyle,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    canvases: &HashMap<NodeId, Canvas2D>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    alpha: f32,
) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    match rep {
        ReplacedKind::Image { url, .. } => {
            if let Some(img) = images.get(url) {
                let radii = resolve_radii(style, w, h);
                let iw = img.width.max(1) as f32;
                let ih = img.height.max(1) as f32;
                let (dw, dh) = match style.object_fit {
                    peedle_css::ObjectFit::Cover => {
                        let s = (w / iw).max(h / ih);
                        (iw * s, ih * s)
                    }
                    peedle_css::ObjectFit::Contain => {
                        let s = (w / iw).min(h / ih);
                        (iw * s, ih * s)
                    }
                    peedle_css::ObjectFit::ScaleDown => {
                        let s = (w / iw).min(h / ih).min(1.0);
                        (iw * s, ih * s)
                    }
                    peedle_css::ObjectFit::None => (iw, ih),
                    peedle_css::ObjectFit::Fill => (w, h),
                };
                let dx = x + (w - dw) * 0.5;
                let dy = y + (h - dh) * 0.5;
                if radii.iter().any(|&r| r > 0.01) || (dw - w).abs() > 0.5 || (dh - h).abs() > 0.5 {
                    fb.blit_rgba_rounded(img.width, img.height, &img.rgba, dx, dy, dw, dh, x, y, w, h, radii, alpha);
                } else {
                    fb.blit_rgba(img.width, img.height, &img.rgba, x, y, w, h, alpha);
                }
            } else {
                fb.fill_rect(x, y, w, h, RgbaColor::new(235, 238, 242, 255).with_alpha(alpha));
                fb.stroke_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    [0.0; 4],
                    [1.0; 4],
                    [RgbaColor::new(190, 196, 204, 255); 4],
                );
            }
        }
        ReplacedKind::Canvas { .. } => {
            if let Some(nid) = node {
                if let Some(cv) = canvases.get(&nid) {
                    fb.blit_rgba(cv.fb.width, cv.fb.height, &cv.fb.data, x, y, w, h, alpha);
                }
            }
        }
        ReplacedKind::Svg { .. } => {
            if let Some(nid) = node {
                render_svg_node_with_color(
                    doc,
                    nid,
                    fb,
                    fonts,
                    x,
                    y,
                    w,
                    h,
                    to_rgba(style.color).with_alpha(alpha),
                );
            }
        }
        ReplacedKind::Input {
            input_type,
            value,
            placeholder,
            checked,
        } => match input_type.as_str() {
            "hidden" => {}
            "checkbox" => {
                let bg = if *checked {
                    RgbaColor::new(37, 99, 235, 255)
                } else {
                    RgbaColor::WHITE
                };
                fb.fill_rounded_rect(x, y, w, h, [3.0; 4], bg);
                fb.stroke_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    [3.0; 4],
                    [1.0; 4],
                    [RgbaColor::new(148, 163, 184, 255); 4],
                );
                if *checked {
                    fb.draw_line(x + 3.0, y + h * 0.55, x + w * 0.42, y + h - 3.5, 2.0, RgbaColor::WHITE);
                    fb.draw_line(x + w * 0.42, y + h - 3.5, x + w - 3.0, y + 3.5, 2.0, RgbaColor::WHITE);
                }
            }
            "radio" => {
                let r = w.min(h) * 0.5;
                fb.fill_rounded_rect(x, y, w, h, [r; 4], RgbaColor::WHITE);
                fb.stroke_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    [r; 4],
                    [1.5; 4],
                    [RgbaColor::new(100, 116, 139, 255); 4],
                );
                if *checked {
                    let ir = r * 0.5;
                    fb.fill_rounded_rect(
                        x + ir,
                        y + ir,
                        w - ir * 2.0,
                        h - ir * 2.0,
                        [ir; 4],
                        RgbaColor::new(37, 99, 235, 255),
                    );
                }
            }
            "submit" | "button" | "reset" => {
                let has_author_chrome = style.background_color.a > 0
                    || !matches!(style.background_image, BackgroundImage::None)
                    || style.effective_border_top_width() > 0.0
                    || style.effective_border_right_width() > 0.0
                    || style.effective_border_bottom_width() > 0.0
                    || style.effective_border_left_width() > 0.0;
                if !has_author_chrome {
                    fb.fill_rounded_rect(x, y, w, h, [4.0; 4], RgbaColor::new(241, 245, 249, 255));
                    fb.stroke_rounded_rect(
                        x,
                        y,
                        w,
                        h,
                        [4.0; 4],
                        [1.0; 4],
                        [RgbaColor::new(148, 163, 184, 255); 4],
                    );
                }
                let label = if value.is_empty() { "Submit" } else { value.as_str() };
                let fs = style.font_size.clamp(10.0, 24.0);
                let key = fonts.select_face(
                    &style.font_family,
                    style.font_weight,
                    style.font_style == FontStyle::Italic,
                );
                let tw = fonts.measure_text(label, key, fs);
                let tc = if has_author_chrome {
                    to_rgba(style.color).with_alpha(alpha)
                } else {
                    RgbaColor::new(15, 23, 42, 255).with_alpha(alpha)
                };
                fonts.draw_text(
                    fb,
                    label,
                    x + (w - tw) * 0.5,
                    y + (h + fs * 0.72) * 0.5,
                    key,
                    fs,
                    tc,
                );
            }
            _ => {
                let has_author_chrome = style.background_color.a > 0
                    || !matches!(style.background_image, BackgroundImage::None)
                    || style.effective_border_top_width() > 0.0
                    || style.effective_border_right_width() > 0.0
                    || style.effective_border_bottom_width() > 0.0
                    || style.effective_border_left_width() > 0.0;
                let focused = node
                    .and_then(|nid| doc.element(nid))
                    .map_or(false, |e| e.focused);
                if !has_author_chrome {
                    let border_c = if focused {
                        RgbaColor::new(37, 99, 235, 255)
                    } else {
                        RgbaColor::new(180, 188, 198, 255)
                    };
                    fb.fill_rounded_rect(x, y, w, h, [4.0; 4], RgbaColor::WHITE);
                    fb.stroke_rounded_rect(
                        x,
                        y,
                        w,
                        h,
                        [4.0; 4],
                        [if focused { 2.0 } else { 1.0 }; 4],
                        [border_c; 4],
                    );
                }
                let fs = style.font_size.clamp(10.0, 24.0);
                let key = fonts.select_face(
                    &style.font_family,
                    style.font_weight,
                    style.font_style == FontStyle::Italic,
                );
                let pad_x = if has_author_chrome { 0.0 } else { 8.0 };
                let (shown, tc) = if !value.is_empty() {
                    let s = if input_type == "password" {
                        "\u{2022}".repeat(value.chars().count())
                    } else {
                        value.clone()
                    };
                    (s, to_rgba(style.color).with_alpha(alpha))
                } else {
                    (placeholder.clone(), RgbaColor::new(148, 163, 184, 255).with_alpha(alpha))
                };
                let end_x = fonts.draw_text(
                    fb,
                    &shown,
                    x + pad_x,
                    y + (h + fs * 0.72) * 0.5,
                    key,
                    fs,
                    tc,
                );
                if focused {
                    let caret_x = if value.is_empty() { x + pad_x } else { end_x + 1.0 };
                    fb.fill_rect(
                        caret_x,
                        y + (h - fs).max(4.0) * 0.5,
                        1.5,
                        fs.min(h - 4.0).max(8.0),
                        to_rgba(style.color).with_alpha(alpha),
                    );
                }
            }
        },
        ReplacedKind::Textarea { value, .. } => {
            let has_author_chrome = style.background_color.a > 0
                || !matches!(style.background_image, BackgroundImage::None)
                || style.effective_border_top_width() > 0.0
                || style.effective_border_right_width() > 0.0
                || style.effective_border_bottom_width() > 0.0
                || style.effective_border_left_width() > 0.0;
            if !has_author_chrome {
                fb.fill_rounded_rect(x, y, w, h, [4.0; 4], RgbaColor::WHITE);
                fb.stroke_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    [4.0; 4],
                    [1.0; 4],
                    [RgbaColor::new(180, 188, 198, 255); 4],
                );
            }
            let fs = style.font_size.clamp(10.0, 22.0);
            let pad_x = if has_author_chrome { 0.0 } else { 6.0 };
            for (idx, line) in value.lines().enumerate() {
                let ly = y + fs + 4.0 + idx as f32 * (fs * 1.3);
                if ly < y + h {
                    fonts.draw_text(
                        fb,
                        line,
                        x + pad_x,
                        ly,
                        FontFaceKey::SansRegular,
                        fs,
                        to_rgba(style.color).with_alpha(alpha),
                    );
                }
            }
        }
        ReplacedKind::Select { label } => {
            let has_author_chrome = style.background_color.a > 0
                || !matches!(style.background_image, BackgroundImage::None)
                || style.effective_border_top_width() > 0.0
                || style.effective_border_right_width() > 0.0
                || style.effective_border_bottom_width() > 0.0
                || style.effective_border_left_width() > 0.0;
            if !has_author_chrome {
                fb.fill_rounded_rect(x, y, w, h, [4.0; 4], RgbaColor::new(248, 250, 252, 255));
                fb.stroke_rounded_rect(
                    x,
                    y,
                    w,
                    h,
                    [4.0; 4],
                    [1.0; 4],
                    [RgbaColor::new(180, 188, 198, 255); 4],
                );
            }
            let fs = style.font_size.clamp(10.0, 20.0);
            let pad_x = if has_author_chrome { 2.0 } else { 8.0 };
            fonts.draw_text(
                fb,
                label,
                x + pad_x,
                y + (h + fs * 0.72) * 0.5,
                FontFaceKey::SansRegular,
                fs,
                to_rgba(style.color).with_alpha(alpha),
            );
            fonts.draw_text(
                fb,
                "v",
                x + w - 14.0,
                y + (h + 10.0) * 0.5,
                FontFaceKey::SansRegular,
                11.0,
                RgbaColor::new(100, 116, 139, 255),
            );
        }
        ReplacedKind::Hr => {
            fb.fill_rect(x, y, w, h.max(1.0), RgbaColor::new(203, 213, 225, 255));
        }
        ReplacedKind::Br => {}
    }
}

fn normalize_stops(stops: &[peedle_css::GradientStop], alpha: f32) -> Vec<(RgbaColor, f32)> {
    let n = stops.len();
    if n == 0 {
        return Vec::new();
    }
    stops
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let pos = s.position.unwrap_or_else(|| {
                if n == 1 {
                    0.0
                } else {
                    i as f32 / (n - 1) as f32
                }
            });
            (to_rgba(s.color).with_alpha(alpha), pos)
        })
        .collect()
}
