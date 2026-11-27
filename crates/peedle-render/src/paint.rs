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
    paint_layout_scaled(doc, tree, fb, fonts, images, canvases, scroll_x, scroll_y, 1.0);
}

pub fn paint_layout_scaled(
    doc: &Document,
    tree: &LayoutTree,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    canvases: &HashMap<NodeId, Canvas2D>,
    scroll_x: f32,
    scroll_y: f32,
    scale: f32,
) {
    let s = scale.max(0.25);
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
        paint_box(doc, tree, root, fb, fonts, images, canvases, -scroll_x, -scroll_y, s);
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

fn scale_arr4(v: [f32; 4], s: f32) -> [f32; 4] {
    [v[0] * s, v[1] * s, v[2] * s, v[3] * s]
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
    s: f32,
) {
    let b = tree.get(id);
    if b.style.visibility == Visibility::Hidden || b.style.opacity <= 0.001 || b.style.clip_hidden {
        return;
    }

    let vp_w = fb.width as f32 / s;
    let vp_h = fb.height as f32 / s;

    let is_fixed = b.style.position == Position::Fixed;
    let mut dx = if is_fixed { 0.0 } else { ox };
    let mut dy = if is_fixed { 0.0 } else { oy };

    let bb = b.dims.border_box();
    if b.style.position == Position::Sticky {
        let ctx_x = CalcContext {
            font_size: b.style.font_size,
            root_font_size: 16.0,
            viewport_width: vp_w,
            viewport_height: vp_h,
            percentage_base: vp_w,
        };
        let ctx_y = CalcContext {
            percentage_base: vp_h,
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
            viewport_width: vp_w,
            viewport_height: vp_h,
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
    let sradii = scale_arr4(radii, s);

    let box_in_view = ry + bb.height >= -96.0 && ry <= vp_h + 96.0;

    if !matches!(b.kind, BoxKind::Text(_)) && box_in_view {
        for sh in &b.style.box_shadow {
            if !sh.inset {
                fb.draw_box_shadow(
                    rx * s,
                    ry * s,
                    bb.width * s,
                    bb.height * s,
                    sradii,
                    sh.offset_x * s,
                    sh.offset_y * s,
                    sh.blur_radius * s,
                    sh.spread_radius * s,
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
                        let ms = sx.min(sy);
                        let tw = mimg.width as f32 * ms;
                        let th = mimg.height as f32 * ms;
                        let tx = rx + (bb.width - tw) * 0.5;
                        let ty = ry + (bb.height - th) * 0.5;
                        fb.blit_tinted_mask_rgba(
                            mimg.width,
                            mimg.height,
                            &mimg.rgba,
                            tx * s,
                            ty * s,
                            tw * s,
                            th * s,
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
                bg_x * s,
                bg_y * s,
                bg_w * s,
                bg_h * s,
                sradii,
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
                fb.fill_linear_gradient(
                    rx * s,
                    ry * s,
                    bb.width * s,
                    bb.height * s,
                    sradii,
                    *angle_deg,
                    &norm,
                    *repeating,
                );
            }
            BackgroundImage::RadialGradient { repeating, stops, .. } => {
                let norm = normalize_stops(stops, alpha);
                fb.fill_radial_gradient(
                    rx * s,
                    ry * s,
                    bb.width * s,
                    bb.height * s,
                    sradii,
                    &norm,
                    *repeating,
                );
            }
            BackgroundImage::ConicGradient {
                from_deg,
                repeating,
                stops,
                ..
            } => {
                let norm = normalize_stops(stops, alpha);
                fb.fill_conic_gradient(
                    rx * s,
                    ry * s,
                    bb.width * s,
                    bb.height * s,
                    sradii,
                    *from_deg,
                    &norm,
                    *repeating,
                );
            }
            BackgroundImage::Url(u) => {
                if let Some(img) = images.get(u) {
                    if img.width > 0 && img.height > 0 && bb.width > 0.0 && bb.height > 0.0 {
                        fb.push_clip(ClipRect {
                            x: (rx * s).floor() as i32,
                            y: (ry * s).floor() as i32,
                            w: (bb.width * s).ceil() as i32,
                            h: (bb.height * s).ceil() as i32,
                        });
                        let (tw, th) = match &b.style.background_size {
                            peedle_css::BackgroundSize::Cover => {
                                let sx = bb.width / img.width as f32;
                                let sy = bb.height / img.height as f32;
                                let bs = sx.max(sy);
                                (img.width as f32 * bs, img.height as f32 * bs)
                            }
                            peedle_css::BackgroundSize::Contain => {
                                let sx = bb.width / img.width as f32;
                                let sy = bb.height / img.height as f32;
                                let bs = sx.min(sy);
                                (img.width as f32 * bs, img.height as f32 * bs)
                            }
                            peedle_css::BackgroundSize::Explicit(bw, bh) => {
                                let ctx_w = CalcContext {
                                    font_size: b.style.font_size,
                                    root_font_size: 16.0,
                                    viewport_width: vp_w,
                                    viewport_height: vp_h,
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
                            (0.0f32, 0.0f32, vp_w, vp_h)
                        } else {
                            (pb.x + dx, pb.y + dy, pb.width, pb.height)
                        };
                        let ctx_x = CalcContext {
                            font_size: b.style.font_size,
                            root_font_size: 16.0,
                            viewport_width: vp_w,
                            viewport_height: vp_h,
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
                            fb.fill_rounded_rect(rx * s, ry * s, bb.width * s, bb.height * s, sradii, c);
                        } else {
                            let (c_start, c_end) = if rep_x && tw >= 1.0 {
                                let cs = ((rx - bg_ox) / tw).floor() as i32;
                                let ce = ((rx + bb.width - bg_ox) / tw).ceil() as i32;
                                (cs, ce.min(cs + 2048))
                            } else {
                                (0, 1)
                            };
                            let (r_start, r_end) = if rep_y && th >= 1.0 {
                                let rs = ((ry - bg_oy) / th).floor() as i32;
                                let re = ((ry + bb.height - bg_oy) / th).ceil() as i32;
                                (rs, re.min(rs + 2048))
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
                                        tx * s,
                                        ty * s,
                                        tw * s,
                                        th * s,
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
                    (pb_in.x + dx) * s,
                    (pb_in.y + dy) * s,
                    pb_in.width * s,
                    pb_in.height * s,
                    scale_arr4(in_radii, s),
                    sh.offset_x * s,
                    sh.offset_y * s,
                    sh.blur_radius * s,
                    sh.spread_radius * s,
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
            fb.stroke_styled_borders(
                rx * s,
                ry * s,
                bb.width * s,
                bb.height * s,
                sradii,
                scale_arr4(widths, s),
                colors,
                bstyles,
            );
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
                (rx - expand) * s,
                (ry - expand) * s,
                (bb.width + expand * 2.0) * s,
                (bb.height + expand * 2.0) * s,
                scale_arr4(out_radii, s),
                [ow * s; 4],
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
                (b.dims.content.x + dx - mw - 4.0) * s,
                (b.dims.content.y + dy + asc) * s,
                key,
                b.style.font_size * s,
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
                let top_y = frag.y + dy;
                if top_y + frag.height < -8.0 || top_y > vp_h + 8.0 {
                    continue;
                }
                let fx = frag.x + dx;
                let fy = frag.baseline + dy;
                for ts in &b.style.text_shadow {
                    fonts.draw_text(
                        fb,
                        &frag.text,
                        (fx + ts.offset_x) * s,
                        (fy + ts.offset_y) * s,
                        key,
                        b.style.font_size * s,
                        to_rgba(ts.color).with_alpha(alpha),
                    );
                }
                fonts.draw_text(fb, &frag.text, fx * s, fy * s, key, b.style.font_size * s, color);
                match b.style.text_decoration_line {
                    TextDecorationLine::Underline => {
                        let dec_c = b
                            .style
                            .text_decoration_color
                            .map(to_rgba)
                            .unwrap_or(color);
                        fb.fill_rect(fx * s, (fy + 2.0) * s, frag.width * s, 1.0 * s, dec_c);
                    }
                    TextDecorationLine::LineThrough => {
                        let dec_c = b
                            .style
                            .text_decoration_color
                            .map(to_rgba)
                            .unwrap_or(color);
                        fb.fill_rect(
                            fx * s,
                            (fy - b.style.font_size * 0.3) * s,
                            frag.width * s,
                            1.0 * s,
                            dec_c,
                        );
                    }
                    TextDecorationLine::Overline => {
                        let dec_c = b
                            .style
                            .text_decoration_color
                            .map(to_rgba)
                            .unwrap_or(color);
                        fb.fill_rect(fx * s, (frag.y + dy) * s, frag.width * s, 1.0 * s, dec_c);
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
            if cy + ch >= -8.0 && cy <= vp_h + 8.0 {
                paint_replaced(
                    doc,
                    b.node,
                    rep,
                    &b.style,
                    fb,
                    fonts,
                    images,
                    canvases,
                    cx,
                    cy,
                    cw,
                    ch,
                    alpha,
                    s,
                );
            }
        }
        _ => {}
    }

    let clips = !matches!(b.style.overflow_x, Overflow::Visible)
        || !matches!(b.style.overflow_y, Overflow::Visible);
    let mut rounded_clipped = false;
    if clips {
        let pb = b.dims.padding_box();
        let clip_top = pb.y + dy;
        if clip_top + pb.height < -4.0 || clip_top > vp_h + 4.0 {
            return;
        }
        fb.push_clip(ClipRect {
            x: ((pb.x + dx) * s).floor() as i32,
            y: (clip_top * s).floor() as i32,
            w: (pb.width * s).ceil() as i32,
            h: (pb.height * s).ceil() as i32,
        });
        if sradii.iter().any(|&r| r > 0.5) && pb.width > 0.0 && pb.height > 0.0 {
            fb.push_rounded_clip(
                (pb.x + dx) * s,
                clip_top * s,
                pb.width * s,
                pb.height * s,
                sradii,
            );
            rounded_clipped = true;
        }
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
        } else if matches!(
            cb.kind,
            BoxKind::Inline | BoxKind::InlineBlock | BoxKind::Text(_) | BoxKind::Replaced(_)
        ) || is_inline_only_container(tree, cid)
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
            s,
        );
    }

    if rounded_clipped {
        fb.pop_rounded_clip();
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
    s: f32,
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
                        let sc = (w / iw).max(h / ih);
                        (iw * sc, ih * sc)
                    }
                    peedle_css::ObjectFit::Contain => {
                        let sc = (w / iw).min(h / ih);
                        (iw * sc, ih * sc)
                    }
                    peedle_css::ObjectFit::ScaleDown => {
                        let sc = (w / iw).min(h / ih).min(1.0);
                        (iw * sc, ih * sc)
                    }
                    peedle_css::ObjectFit::None => (iw, ih),
                    peedle_css::ObjectFit::Fill => (w, h),
                };
                let dx = x + (w - dw) * 0.5;
                let dy = y + (h - dh) * 0.5;
                if radii.iter().any(|&r| r > 0.01) || (dw - w).abs() > 0.5 || (dh - h).abs() > 0.5 {
                    fb.blit_rgba_rounded(
                        img.width,
                        img.height,
                        &img.rgba,
                        dx * s,
                        dy * s,
                        dw * s,
                        dh * s,
                        x * s,
                        y * s,
                        w * s,
                        h * s,
                        scale_arr4(radii, s),
                        alpha,
                    );
                } else {
                    fb.blit_rgba(img.width, img.height, &img.rgba, x * s, y * s, w * s, h * s, alpha);
                }
            } else {
                fb.fill_rect(
                    x * s,
                    y * s,
                    w * s,
                    h * s,
                    RgbaColor::new(235, 238, 242, 255).with_alpha(alpha),
                );
                fb.stroke_rounded_rect(
                    x * s,
                    y * s,
                    w * s,
                    h * s,
                    [0.0; 4],
                    [1.0 * s; 4],
                    [RgbaColor::new(190, 196, 204, 255); 4],
                );
            }
        }
        ReplacedKind::Canvas { .. } => {
            if let Some(nid) = node {
                if let Some(cv) = canvases.get(&nid) {
                    fb.blit_rgba(cv.fb.width, cv.fb.height, &cv.fb.data, x * s, y * s, w * s, h * s, alpha);
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
                    x * s,
                    y * s,
                    w * s,
                    h * s,
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
                fb.fill_rounded_rect(x * s, y * s, w * s, h * s, [3.0 * s; 4], bg);
                fb.stroke_rounded_rect(
                    x * s,
                    y * s,
                    w * s,
                    h * s,
                    [3.0 * s; 4],
                    [1.0 * s; 4],
                    [RgbaColor::new(148, 163, 184, 255); 4],
                );
                if *checked {
                    fb.draw_line(
                        (x + 3.0) * s,
                        (y + h * 0.55) * s,
                        (x + w * 0.42) * s,
                        (y + h - 3.5) * s,
                        2.0 * s,
                        RgbaColor::WHITE,
                    );
                    fb.draw_line(
                        (x + w * 0.42) * s,
                        (y + h - 3.5) * s,
                        (x + w - 3.0) * s,
                        (y + 3.5) * s,
                        2.0 * s,
                        RgbaColor::WHITE,
                    );
                }
            }
            "radio" => {
                let r = w.min(h) * 0.5;
                fb.fill_rounded_rect(x * s, y * s, w * s, h * s, [r * s; 4], RgbaColor::WHITE);
                fb.stroke_rounded_rect(
                    x * s,
                    y * s,
                    w * s,
                    h * s,
                    [r * s; 4],
                    [1.5 * s; 4],
                    [RgbaColor::new(100, 116, 139, 255); 4],
                );
                if *checked {
                    let ir = r * 0.5;
                    fb.fill_rounded_rect(
                        (x + ir) * s,
                        (y + ir) * s,
                        (w - ir * 2.0) * s,
                        (h - ir * 2.0) * s,
                        [ir * s; 4],
                        RgbaColor::new(37, 99, 235, 255),
                    );
                }
            }
            "submit" | "button" | "reset" => {
                if !value.is_empty() {
                    let fs = style.font_size.clamp(10.0, 24.0);
                    let key = fonts.select_face(
                        &style.font_family,
                        style.font_weight,
                        style.font_style == FontStyle::Italic,
                    );
                    let tw = fonts.measure_text(value, key, fs);
                    let tc = to_rgba(style.color).with_alpha(alpha);
                    fonts.draw_text(
                        fb,
                        value,
                        (x + (w - tw) * 0.5) * s,
                        (y + (h + fs * 0.72) * 0.5) * s,
                        key,
                        fs * s,
                        tc,
                    );
                }
            }
            _ => {
                let focused = node
                    .and_then(|nid| doc.element(nid))
                    .map_or(false, |e| e.focused);
                let fs = style.font_size.clamp(10.0, 24.0);
                let key = fonts.select_face(
                    &style.font_family,
                    style.font_weight,
                    style.font_style == FontStyle::Italic,
                );
                let (shown, tc) = if !value.is_empty() {
                    let sv = if input_type == "password" {
                        "\u{2022}".repeat(value.chars().count())
                    } else {
                        value.clone()
                    };
                    (sv, to_rgba(style.color).with_alpha(alpha))
                } else {
                    (placeholder.clone(), RgbaColor::new(148, 163, 184, 255).with_alpha(alpha))
                };
                let end_x_phys = fonts.draw_text(
                    fb,
                    &shown,
                    x * s,
                    (y + (h + fs * 0.72) * 0.5) * s,
                    key,
                    fs * s,
                    tc,
                );
                if focused {
                    let caret_x = if value.is_empty() {
                        x * s
                    } else {
                        end_x_phys + 1.0 * s
                    };
                    fb.fill_rect(
                        caret_x,
                        (y + (h - fs).max(2.0) * 0.5) * s,
                        1.5 * s,
                        fs.min(h.max(10.0)).max(8.0) * s,
                        to_rgba(style.color).with_alpha(alpha),
                    );
                }
            }
        },
        ReplacedKind::Textarea { value, .. } => {
            let fs = style.font_size.clamp(10.0, 22.0);
            for (idx, line) in value.lines().enumerate() {
                let ly = y + fs + 2.0 + idx as f32 * (fs * 1.3);
                if ly < y + h {
                    fonts.draw_text(
                        fb,
                        line,
                        x * s,
                        ly * s,
                        FontFaceKey::SansRegular,
                        fs * s,
                        to_rgba(style.color).with_alpha(alpha),
                    );
                }
            }
        }
        ReplacedKind::Select { label } => {
            let fs = style.font_size.clamp(10.0, 20.0);
            fonts.draw_text(
                fb,
                label,
                (x + 2.0) * s,
                (y + (h + fs * 0.72) * 0.5) * s,
                FontFaceKey::SansRegular,
                fs * s,
                to_rgba(style.color).with_alpha(alpha),
            );
            let ax = (x + w - 10.0) * s;
            let ay = (y + h * 0.44) * s;
            fb.draw_line_aa(
                ax - 3.5 * s,
                ay,
                ax,
                ay + 3.5 * s,
                1.4 * s,
                RgbaColor::new(100, 116, 139, 255),
            );
            fb.draw_line_aa(
                ax,
                ay + 3.5 * s,
                ax + 3.5 * s,
                ay,
                1.4 * s,
                RgbaColor::new(100, 116, 139, 255),
            );
        }
        ReplacedKind::Hr => {
            fb.fill_rect(x * s, y * s, w * s, h.max(1.0) * s, RgbaColor::new(203, 213, 225, 255));
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
