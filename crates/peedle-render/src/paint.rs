use std::collections::HashMap;

use peedle_css::{
    BackgroundImage, CalcContext, Color, ComputedStyle, FontStyle, Overflow, TextDecorationLine,
    Visibility,
};
use peedle_html::{Document, NodeId};
use peedle_layout::{BoxKind, LayoutBoxId, LayoutTree, ReplacedKind};

use crate::canvas2d::Canvas2D;
use crate::fonts::{FontFaceKey, FontSystem};
use crate::framebuffer::{ClipRect, Framebuffer, RgbaColor};
use crate::images::ImageCache;
use crate::svg::render_svg_node;

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

    let is_fixed = b.style.position == peedle_css::Position::Fixed;
    let dx = if is_fixed { 0.0 } else { ox };
    let dy = if is_fixed { 0.0 } else { oy };

    let alpha = b.style.opacity.clamp(0.0, 1.0);
    let bb = b.dims.border_box();
    let rx = bb.x + dx;
    let ry = bb.y + dy;
    let radii = resolve_radii(&b.style, bb.width, bb.height);

    if !matches!(b.kind, BoxKind::Text(_)) {
        for sh in &b.style.box_shadow {
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
                sh.inset,
            );
        }

        if b.style.background_color.a > 0 {
            fb.fill_rounded_rect(
                rx,
                ry,
                bb.width,
                bb.height,
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
            BackgroundImage::Url(u) => {
                if let Some(img) = images.get(u) {
                    if img.width > 0 && img.height > 0 && bb.width > 0.0 && bb.height > 0.0 {
                        fb.push_clip(ClipRect {
                            x: rx.floor() as i32,
                            y: ry.floor() as i32,
                            w: bb.width.ceil() as i32,
                            h: bb.height.ceil() as i32,
                        });
                        let (tw, th) = match b.style.background_size {
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
                            _ => (img.width as f32, img.height as f32),
                        };
                        let (rep_x, rep_y) = match b.style.background_repeat {
                            peedle_css::BackgroundRepeat::NoRepeat => (false, false),
                            peedle_css::BackgroundRepeat::RepeatX => (true, false),
                            peedle_css::BackgroundRepeat::RepeatY => (false, true),
                            _ => (true, true),
                        };
                        let max_cols = if rep_x && tw >= 1.0 {
                            ((bb.width / tw).ceil() as usize).min(2048)
                        } else {
                            1
                        };
                        let max_rows = if rep_y && th >= 1.0 {
                            ((bb.height / th).ceil() as usize).min(2048)
                        } else {
                            1
                        };
                        if img.width == 1 && img.height == 1 && rep_x && rep_y {
                            let c = RgbaColor::new(img.rgba[0], img.rgba[1], img.rgba[2], img.rgba[3])
                                .with_alpha(alpha);
                            fb.fill_rounded_rect(rx, ry, bb.width, bb.height, radii, c);
                        } else {
                            for row in 0..max_rows {
                                for col in 0..max_cols {
                                    fb.blit_rgba(
                                        img.width,
                                        img.height,
                                        &img.rgba,
                                        rx + col as f32 * tw,
                                        ry + row as f32 * th,
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
            fb.stroke_rounded_rect(rx, ry, bb.width, bb.height, radii, widths, colors);
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
    ordered_children.sort_by_key(|&cid| tree.get(cid).style.z_index.unwrap_or(0));

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
    match rep {
        ReplacedKind::Image { url, .. } => {
            if let Some(img) = images.get(url) {
                fb.blit_rgba(img.width, img.height, &img.rgba, x, y, w, h, alpha);
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
                render_svg_node(doc, nid, fb, fonts, x, y, w, h);
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
                let label = if value.is_empty() { "Submit" } else { value.as_str() };
                let tw = fonts.measure_text(label, FontFaceKey::SansRegular, 13.0);
                fonts.draw_text(
                    fb,
                    label,
                    x + (w - tw) * 0.5,
                    y + h * 0.65,
                    FontFaceKey::SansRegular,
                    13.0,
                    RgbaColor::new(15, 23, 42, 255),
                );
            }
            _ => {
                let focused = node
                    .and_then(|nid| doc.element(nid))
                    .map_or(false, |e| e.focused);
                let border_c = if focused {
                    RgbaColor::new(37, 99, 235, 255)
                } else {
                    RgbaColor::new(180, 188, 198, 255)
                };
                fb.fill_rounded_rect(x, y, w, h, [4.0; 4], RgbaColor::WHITE);
                fb.stroke_rounded_rect(x, y, w, h, [4.0; 4], [if focused { 2.0 } else { 1.0 }; 4], [border_c; 4]);
                let (shown, tc) = if !value.is_empty() {
                    let s = if input_type == "password" {
                        "\u{2022}".repeat(value.chars().count())
                    } else {
                        value.clone()
                    };
                    (s, to_rgba(style.color))
                } else {
                    (placeholder.clone(), RgbaColor::new(148, 163, 184, 255))
                };
                let end_x = fonts.draw_text(
                    fb,
                    &shown,
                    x + 8.0,
                    y + h * 0.66,
                    FontFaceKey::SansRegular,
                    13.0,
                    tc,
                );
                if focused {
                    let caret_x = if value.is_empty() { x + 8.0 } else { end_x + 1.0 };
                    fb.fill_rect(caret_x, y + 5.0, 1.5, (h - 10.0).max(8.0), RgbaColor::BLACK);
                }
            }
        },
        ReplacedKind::Textarea { value, .. } => {
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
            for (idx, line) in value.lines().enumerate() {
                let ly = y + 18.0 + idx as f32 * 18.0;
                if ly < y + h {
                    fonts.draw_text(fb, line, x + 6.0, ly, FontFaceKey::SansRegular, 13.0, RgbaColor::BLACK);
                }
            }
        }
        ReplacedKind::Select { label } => {
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
            fonts.draw_text(
                fb,
                label,
                x + 8.0,
                y + h * 0.66,
                FontFaceKey::SansRegular,
                13.0,
                RgbaColor::BLACK,
            );
            fonts.draw_text(
                fb,
                "v",
                x + w - 16.0,
                y + h * 0.64,
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
