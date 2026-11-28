use std::collections::HashMap;
use std::sync::Arc;

use peedle_css::{
    BackgroundImage, BorderStyle, CalcContext, Color, ComputedStyle, FontStyle, Overflow, Position,
    TextDecorationLine, TransformOp, Visibility,
};
use peedle_html::{Document, NodeId};
use peedle_layout::{BoxKind, LayoutBoxId, LayoutTree, ReplacedKind};
use peedle_media::VideoFrame;

use crate::canvas2d::Canvas2D;
use crate::fonts::{FontFaceKey, FontSystem};
use crate::framebuffer::{ClipRect, Framebuffer, RgbaColor};
use crate::images::ImageCache;
use crate::svg::render_svg_node_with_style;

#[derive(Clone, Debug, Default)]
pub struct VideoOverlayState {
    pub frame: Option<Arc<VideoFrame>>,
    pub poster_url: Option<String>,
    pub playing: bool,
    pub loading: bool,
    pub ended: bool,
    pub current_time: f64,
    pub duration: f64,
    pub buffered_sec: f64,
    pub volume: f32,
    pub muted: bool,
    pub playback_rate: f32,
    pub controls: bool,
    pub caption: Option<String>,
    pub captions_enabled: bool,
    pub has_captions: bool,
    pub quality_label: Option<String>,
    pub fullscreen: bool,
}

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
    let empty_media = HashMap::new();
    paint_layout_with_media(
        doc,
        tree,
        fb,
        fonts,
        images,
        canvases,
        &empty_media,
        scroll_x,
        scroll_y,
        scale,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn paint_layout_with_media(
    doc: &Document,
    tree: &LayoutTree,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    canvases: &HashMap<NodeId, Canvas2D>,
    media: &HashMap<NodeId, VideoOverlayState>,
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
        paint_box(
            doc,
            tree,
            root,
            fb,
            fonts,
            images,
            canvases,
            media,
            -scroll_x,
            -scroll_y,
            s,
        );
    }
}

pub fn paint_fullscreen_video(
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    state: &VideoOverlayState,
    scale: f32,
) {
    let s = scale.max(0.25);
    let w = fb.width as f32 / s;
    let h = fb.height as f32 / s;
    fb.clear(RgbaColor::BLACK);
    paint_video_surface(fb, fonts, images, state, 0.0, 0.0, w, h, [0.0; 4], 1.0, s, true);
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

#[allow(clippy::too_many_arguments)]
fn paint_box(
    doc: &Document,
    tree: &LayoutTree,
    id: LayoutBoxId,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    canvases: &HashMap<NodeId, Canvas2D>,
    media: &HashMap<NodeId, VideoOverlayState>,
    ox: f32,
    oy: f32,
    s: f32,
) {
    paint_box_with_opts(doc, tree, id, fb, fonts, images, canvases, media, ox, oy, s, false);
}

fn compose_affine(m1: [f32; 6], m2: [f32; 6]) -> [f32; 6] {
    [
        m1[0] * m2[0] + m1[2] * m2[1],
        m1[1] * m2[0] + m1[3] * m2[1],
        m1[0] * m2[2] + m1[2] * m2[3],
        m1[1] * m2[2] + m1[3] * m2[3],
        m1[0] * m2[4] + m1[2] * m2[5] + m1[4],
        m1[1] * m2[4] + m1[3] * m2[5] + m1[5],
    ]
}

fn collect_subtree_bounds(tree: &LayoutTree, id: LayoutBoxId, out: &mut (f32, f32, f32, f32)) {
    let b = tree.get(id);
    if b.style.visibility == Visibility::Hidden || b.style.opacity <= 0.001 || b.style.clip_hidden {
        return;
    }
    let bb = b.dims.border_box();
    if bb.width > 0.0 || bb.height > 0.0 {
        out.0 = out.0.min(bb.x);
        out.1 = out.1.min(bb.y);
        out.2 = out.2.max(bb.x + bb.width);
        out.3 = out.3.max(bb.y + bb.height);
    }
    for f in &b.text_fragments {
        out.0 = out.0.min(f.x);
        out.1 = out.1.min(f.y);
        out.2 = out.2.max(f.x + f.width);
        out.3 = out.3.max(f.y + f.height);
    }
    for &cid in &b.children {
        collect_subtree_bounds(tree, cid, out);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_box_with_opts(
    doc: &Document,
    tree: &LayoutTree,
    id: LayoutBoxId,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    canvases: &HashMap<NodeId, Canvas2D>,
    media: &HashMap<NodeId, VideoOverlayState>,
    ox: f32,
    oy: f32,
    s: f32,
    skip_root_transform: bool,
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

    if !is_fixed
        && !b.has_fixed_descendant
        && b.subtree_min_y.is_finite()
        && b.subtree_max_y.is_finite()
        && (b.subtree_max_y + dy < -96.0 || b.subtree_min_y + dy > vp_h + 96.0)
    {
        return;
    }

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

    if !skip_root_transform && !b.style.transform.is_empty() {
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
        let mut mat = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
        for op in &b.style.transform {
            let m = match op {
                TransformOp::Translate(tx, ty) => [
                    1.0,
                    0.0,
                    0.0,
                    1.0,
                    tx.resolve_or(&ctx_w, 0.0),
                    ty.resolve_or(&ctx_h, 0.0),
                ],
                TransformOp::TranslateX(tx) => [1.0, 0.0, 0.0, 1.0, tx.resolve_or(&ctx_w, 0.0), 0.0],
                TransformOp::TranslateY(ty) => [1.0, 0.0, 0.0, 1.0, 0.0, ty.resolve_or(&ctx_h, 0.0)],
                TransformOp::Scale(sx, sy) => [*sx, 0.0, 0.0, *sy, 0.0, 0.0],
                TransformOp::ScaleX(sx) => [*sx, 0.0, 0.0, 1.0, 0.0, 0.0],
                TransformOp::ScaleY(sy) => [1.0, 0.0, 0.0, *sy, 0.0, 0.0],
                TransformOp::Rotate(deg) => {
                    let rad = deg.to_radians();
                    let (sn, cs) = rad.sin_cos();
                    [cs, sn, -sn, cs, 0.0, 0.0]
                }
                TransformOp::Skew(ax, ay) => [
                    1.0,
                    ay.to_radians().tan(),
                    ax.to_radians().tan(),
                    1.0,
                    0.0,
                    0.0,
                ],
                TransformOp::SkewX(ax) => [1.0, 0.0, ax.to_radians().tan(), 1.0, 0.0, 0.0],
                TransformOp::SkewY(ay) => [1.0, ay.to_radians().tan(), 0.0, 1.0, 0.0, 0.0],
                TransformOp::Matrix(a, bm, c, d, e, f) => [*a, *bm, *c, *d, *e, *f],
            };
            mat = compose_affine(mat, m);
        }
        let is_pure_translate = (mat[0] - 1.0).abs() < 1e-4
            && mat[1].abs() < 1e-4
            && mat[2].abs() < 1e-4
            && (mat[3] - 1.0).abs() < 1e-4;
        if is_pure_translate {
            dx += mat[4];
            dy += mat[5];
        } else {
            let mut bnds = (bb.x, bb.y, bb.x + bb.width, bb.y + bb.height);
            collect_subtree_bounds(tree, id, &mut bnds);
            let pad = 6.0f32;
            let src_x = bnds.0 - pad;
            let src_y = bnds.1 - pad;
            let src_w = ((bnds.2 - bnds.0) + pad * 2.0).max(1.0);
            let src_h = ((bnds.3 - bnds.1) + pad * 2.0).max(1.0);
            let pw = (src_w * s).ceil() as u32;
            let ph = (src_h * s).ceil() as u32;
            if pw > 0 && ph > 0 && pw <= 1600 && ph <= 1600 {
                let mut off = Framebuffer::new(pw, ph);
                paint_box_with_opts(
                    doc,
                    tree,
                    id,
                    &mut off,
                    fonts,
                    images,
                    canvases,
                    media,
                    -src_x,
                    -src_y,
                    s,
                    true,
                );
                let orig_x = b.style.transform_origin_x.resolve_or(&ctx_w, bb.width * 0.5);
                let orig_y = b.style.transform_origin_y.resolve_or(&ctx_h, bb.height * 0.5);
                let pivot_src_x = (bb.x + orig_x - src_x) * s;
                let pivot_src_y = (bb.y + orig_y - src_y) * s;
                let pivot_dst_x = (bb.x + dx + orig_x + mat[4]) * s;
                let pivot_dst_y = (bb.y + dy + orig_y + mat[5]) * s;
                fb.blit_rgba_affine(
                    off.width,
                    off.height,
                    &off.data,
                    pivot_src_x,
                    pivot_src_y,
                    pivot_dst_x,
                    pivot_dst_y,
                    [mat[0], mat[1], mat[2], mat[3]],
                    1.0,
                );
                return;
            }
            dx += mat[4];
            dy += mat[5];
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
                    media,
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

    let (scroll_dx, scroll_dy) = b
        .node
        .and_then(|nid| doc.element(nid))
        .map(|el| (el.scroll_left, el.scroll_top))
        .unwrap_or((0.0, 0.0));

    let paint_order_key = |cid: LayoutBoxId| -> (i32, i32) {
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
    };

    let already_sorted = b.children.len() <= 1
        || b.children
            .windows(2)
            .all(|w| paint_order_key(w[0]) <= paint_order_key(w[1]));

    if already_sorted {
        for &cid in &b.children {
            paint_box(
                doc,
                tree,
                cid,
                fb,
                fonts,
                images,
                canvases,
                media,
                dx - scroll_dx,
                dy - scroll_dy,
                s,
            );
        }
    } else {
        let mut ordered_children = b.children.clone();
        ordered_children.sort_by_key(|&cid| paint_order_key(cid));
        for cid in ordered_children {
            paint_box(
                doc,
                tree,
                cid,
                fb,
                fonts,
                images,
                canvases,
                media,
                dx - scroll_dx,
                dy - scroll_dy,
                s,
            );
        }
    }

    if rounded_clipped {
        fb.pop_rounded_clip();
    }
    if clips {
        fb.pop_clip();
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_replaced(
    doc: &Document,
    node: Option<NodeId>,
    rep: &ReplacedKind,
    style: &ComputedStyle,
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    canvases: &HashMap<NodeId, Canvas2D>,
    media: &HashMap<NodeId, VideoOverlayState>,
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
                let dx = x + (w - dw) * style.object_position_x;
                let dy = y + (h - dh) * style.object_position_y;
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
                render_svg_node_with_style(
                    doc,
                    nid,
                    fb,
                    fonts,
                    x * s,
                    y * s,
                    w * s,
                    h * s,
                    to_rgba(style.color).with_alpha(alpha),
                    style.font_size,
                    style.custom_properties.as_ref(),
                );
            }
        }
        ReplacedKind::Video {
            poster, controls, ..
        } => {
            let radii = resolve_radii(style, w, h);
            let default_state = VideoOverlayState {
                poster_url: if poster.is_empty() {
                    None
                } else {
                    Some(poster.clone())
                },
                controls: *controls,
                volume: 1.0,
                playback_rate: 1.0,
                ..Default::default()
            };
            let st = node
                .and_then(|nid| media.get(&nid))
                .unwrap_or(&default_state);
            paint_video_surface(
                fb,
                fonts,
                images,
                st,
                x,
                y,
                w,
                h,
                radii,
                alpha,
                s,
                *controls || st.controls,
            );
        }
        ReplacedKind::Audio { controls, .. } => {
            if *controls {
                let default_state = VideoOverlayState {
                    controls: true,
                    volume: 1.0,
                    playback_rate: 1.0,
                    ..Default::default()
                };
                let st = node
                    .and_then(|nid| media.get(&nid))
                    .unwrap_or(&default_state);
                paint_audio_bar(fb, fonts, st, x, y, w, h, alpha, s);
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
        ReplacedKind::Textarea { value, placeholder, .. } => {
            let fs = style.font_size.clamp(10.0, 24.0);
            let key = fonts.select_face(
                &style.font_family,
                style.font_weight,
                style.font_style == FontStyle::Italic,
            );
            let (text_to_draw, tc) = if !value.is_empty() {
                (value.as_str(), to_rgba(style.color).with_alpha(alpha))
            } else {
                (
                    placeholder.as_str(),
                    RgbaColor::new(148, 163, 184, 255).with_alpha(alpha),
                )
            };
            let line_step = style.line_height.resolve(fs).max(fs * 1.2);
            let single_line = !text_to_draw.contains('\n');
            if single_line && !text_to_draw.is_empty() {
                let ly = if h <= line_step * 1.6 {
                    y + (h + fs * 0.72) * 0.5
                } else {
                    y + fs + 2.0
                };
                fonts.draw_text(fb, text_to_draw, x * s, ly * s, key, fs * s, tc);
            } else {
                for (idx, line) in text_to_draw.lines().enumerate() {
                    let ly = y + fs + 2.0 + idx as f32 * line_step;
                    if ly < y + h + fs {
                        fonts.draw_text(fb, line, x * s, ly * s, key, fs * s, tc);
                    }
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

fn format_media_time(secs: f64) -> String {
    let total = secs.max(0.0).floor() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_video_surface(
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    images: &ImageCache,
    st: &VideoOverlayState,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radii: [f32; 4],
    alpha: f32,
    s: f32,
    show_controls: bool,
) {
    let sradii = scale_arr4(radii, s);
    let has_radius = sradii.iter().any(|&r| r > 0.5);
    fb.fill_rounded_rect(
        x * s,
        y * s,
        w * s,
        h * s,
        sradii,
        RgbaColor::new(0, 0, 0, 255).with_alpha(alpha),
    );
    fb.push_clip(ClipRect {
        x: (x * s).floor() as i32,
        y: (y * s).floor() as i32,
        w: (w * s).ceil() as i32,
        h: (h * s).ceil() as i32,
    });
    if has_radius {
        fb.push_rounded_clip(x * s, y * s, w * s, h * s, sradii);
    }

    if let Some(ref frame) = st.frame {
        let iw = frame.width.max(1) as f32;
        let ih = frame.height.max(1) as f32;
        let sc = (w / iw).min(h / ih);
        let dw = iw * sc;
        let dh = ih * sc;
        let dx = x + (w - dw) * 0.5;
        let dy = y + (h - dh) * 0.5;
        fb.blit_rgba(
            frame.width,
            frame.height,
            &frame.rgba,
            dx * s,
            dy * s,
            dw * s,
            dh * s,
            alpha,
        );
    } else if let Some(ref poster_u) = st.poster_url {
        if let Some(img) = images.get(poster_u) {
            let iw = img.width.max(1) as f32;
            let ih = img.height.max(1) as f32;
            let sc = (w / iw).min(h / ih);
            let dw = iw * sc;
            let dh = ih * sc;
            let dx = x + (w - dw) * 0.5;
            let dy = y + (h - dh) * 0.5;
            fb.blit_rgba(
                img.width,
                img.height,
                &img.rgba,
                dx * s,
                dy * s,
                dw * s,
                dh * s,
                alpha,
            );
        }
    }

    if st.captions_enabled {
        if let Some(ref cap) = st.caption {
            let trimmed = cap.trim();
            if !trimmed.is_empty() {
                let cap_px = (15.0 * (w / 720.0).clamp(0.85, 1.35)) * s;
                let tw = fonts.measure_text(trimmed, FontFaceKey::SansRegular, cap_px);
                let pad_x = 12.0 * s;
                let pad_y = 6.0 * s;
                let box_w = (tw + pad_x * 2.0).min((w - 24.0) * s);
                let box_h = cap_px + pad_y * 2.0;
                let bx = (x + w * 0.5) * s - box_w * 0.5;
                let by = (y + h - 62.0) * s - box_h;
                fb.fill_rounded_rect(
                    bx,
                    by,
                    box_w,
                    box_h,
                    [4.0 * s; 4],
                    RgbaColor::new(8, 8, 8, 205),
                );
                let asc = fonts.line_ascent(FontFaceKey::SansRegular, cap_px);
                fonts.draw_text(
                    fb,
                    trimmed,
                    bx + pad_x,
                    by + pad_y + asc,
                    FontFaceKey::SansRegular,
                    cap_px,
                    RgbaColor::WHITE,
                );
            }
        }
    }

    if !st.playing && !st.loading && w >= 120.0 && h >= 90.0 {
        let cx = (x + w * 0.5) * s;
        let cy = (y + h * 0.5) * s;
        let pw = 68.0 * s;
        let ph = 48.0 * s;
        fb.fill_rounded_rect(
            cx - pw * 0.5,
            cy - ph * 0.5,
            pw,
            ph,
            [12.0 * s; 4],
            RgbaColor::new(15, 15, 15, 205),
        );
        let tri = vec![
            (cx - 8.0 * s, cy - 11.0 * s),
            (cx + 12.0 * s, cy),
            (cx - 8.0 * s, cy + 11.0 * s),
        ];
        fb.fill_polygon(&[tri], RgbaColor::WHITE, false);
    } else if st.loading && st.frame.is_none() && w >= 120.0 && h >= 90.0 {
        let cx = (x + w * 0.5) * s;
        let cy = (y + h * 0.5) * s;
        fb.draw_circle_stroke(cx, cy, 18.0 * s, 3.0 * s, RgbaColor::new(255, 255, 255, 175));
    }

    if show_controls && h >= 80.0 && w >= 160.0 {
        let scrim_h = 84.0f32.min(h * 0.45);
        fb.fill_linear_gradient(
            x * s,
            (y + h - scrim_h) * s,
            w * s,
            scrim_h * s,
            [0.0; 4],
            180.0,
            &[
                (RgbaColor::new(0, 0, 0, 0), 0.0),
                (RgbaColor::new(0, 0, 0, 210), 1.0),
            ],
            false,
        );

        let track_x = x + 12.0;
        let track_w = (w - 24.0).max(20.0);
        let track_y = y + h - 46.0;
        let track_h = 4.0;
        fb.fill_rounded_rect(
            track_x * s,
            track_y * s,
            track_w * s,
            track_h * s,
            [2.0 * s; 4],
            RgbaColor::new(255, 255, 255, 75),
        );
        let dur = st.duration.max(0.001);
        let buf_frac = (st.buffered_sec / dur).clamp(0.0, 1.0) as f32;
        if buf_frac > 0.0 {
            fb.fill_rounded_rect(
                track_x * s,
                track_y * s,
                (track_w * buf_frac) * s,
                track_h * s,
                [2.0 * s; 4],
                RgbaColor::new(255, 255, 255, 120),
            );
        }
        let play_frac = if st.duration > 0.0 {
            (st.current_time / dur).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let fill_w = track_w * play_frac;
        if fill_w > 0.5 {
            fb.fill_rounded_rect(
                track_x * s,
                track_y * s,
                fill_w * s,
                track_h * s,
                [2.0 * s; 4],
                RgbaColor::new(255, 0, 51, 255),
            );
        }
        fb.draw_circle_fill(
            (track_x + fill_w) * s,
            (track_y + track_h * 0.5) * s,
            6.0 * s,
            RgbaColor::new(255, 0, 51, 255),
        );

        let cy = (y + h - 21.0) * s;
        let px = (x + 24.0) * s;
        if st.playing {
            fb.fill_rounded_rect(
                px - 6.0 * s,
                cy - 7.0 * s,
                4.0 * s,
                14.0 * s,
                [1.0 * s; 4],
                RgbaColor::WHITE,
            );
            fb.fill_rounded_rect(
                px + 2.0 * s,
                cy - 7.0 * s,
                4.0 * s,
                14.0 * s,
                [1.0 * s; 4],
                RgbaColor::WHITE,
            );
        } else {
            let tri = vec![
                (px - 5.0 * s, cy - 7.5 * s),
                (px + 8.0 * s, cy),
                (px - 5.0 * s, cy + 7.5 * s),
            ];
            fb.fill_polygon(&[tri], RgbaColor::WHITE, false);
        }

        let vx = (x + 56.0) * s;
        let spk = vec![
            (vx - 6.0 * s, cy - 3.5 * s),
            (vx - 2.0 * s, cy - 3.5 * s),
            (vx + 3.0 * s, cy - 7.5 * s),
            (vx + 3.0 * s, cy + 7.5 * s),
            (vx - 2.0 * s, cy + 3.5 * s),
            (vx - 6.0 * s, cy + 3.5 * s),
        ];
        fb.fill_polygon(&[spk], RgbaColor::WHITE, false);
        if st.muted || st.volume <= 0.01 {
            fb.draw_line_aa(
                vx + 6.0 * s,
                cy - 4.0 * s,
                vx + 12.0 * s,
                cy + 4.0 * s,
                1.6 * s,
                RgbaColor::new(255, 90, 90, 255),
            );
            fb.draw_line_aa(
                vx + 12.0 * s,
                cy - 4.0 * s,
                vx + 6.0 * s,
                cy + 4.0 * s,
                1.6 * s,
                RgbaColor::new(255, 90, 90, 255),
            );
        } else {
            fb.draw_line_aa(
                vx + 6.5 * s,
                cy - 3.5 * s,
                vx + 8.0 * s,
                cy,
                1.5 * s,
                RgbaColor::WHITE,
            );
            fb.draw_line_aa(
                vx + 8.0 * s,
                cy,
                vx + 6.5 * s,
                cy + 3.5 * s,
                1.5 * s,
                RgbaColor::WHITE,
            );
        }

        let mut time_x = x + 84.0;
        if w >= 360.0 {
            let vol_x = x + 78.0;
            let vol_w = 56.0;
            let vol_y = y + h - 22.5;
            fb.fill_rounded_rect(
                vol_x * s,
                vol_y * s,
                vol_w * s,
                3.0 * s,
                [1.5 * s; 4],
                RgbaColor::new(255, 255, 255, 90),
            );
            let eff_v = if st.muted { 0.0 } else { st.volume.clamp(0.0, 1.0) };
            fb.fill_rounded_rect(
                vol_x * s,
                vol_y * s,
                (vol_w * eff_v) * s,
                3.0 * s,
                [1.5 * s; 4],
                RgbaColor::WHITE,
            );
            fb.draw_circle_fill(
                (vol_x + vol_w * eff_v) * s,
                (vol_y + 1.5) * s,
                4.5 * s,
                RgbaColor::WHITE,
            );
            time_x = x + 146.0;
        }

        let label = format!(
            "{} / {}",
            format_media_time(st.current_time),
            format_media_time(st.duration)
        );
        let t_px = 12.5 * s;
        let asc = fonts.line_ascent(FontFaceKey::SansRegular, t_px);
        fonts.draw_text(
            fb,
            &label,
            time_x * s,
            cy - t_px * 0.5 + asc,
            FontFaceKey::SansRegular,
            t_px,
            RgbaColor::new(240, 240, 240, 255),
        );

        let fs_cx = (x + w - 26.0) * s;
        let d = 6.0 * s;
        let arm = 3.5 * s;
        let stroke = 1.6 * s;
        fb.draw_line_aa(fs_cx - d, cy - d, fs_cx - d + arm, cy - d, stroke, RgbaColor::WHITE);
        fb.draw_line_aa(fs_cx - d, cy - d, fs_cx - d, cy - d + arm, stroke, RgbaColor::WHITE);
        fb.draw_line_aa(fs_cx + d, cy - d, fs_cx + d - arm, cy - d, stroke, RgbaColor::WHITE);
        fb.draw_line_aa(fs_cx + d, cy - d, fs_cx + d, cy - d + arm, stroke, RgbaColor::WHITE);
        fb.draw_line_aa(fs_cx - d, cy + d, fs_cx - d + arm, cy + d, stroke, RgbaColor::WHITE);
        fb.draw_line_aa(fs_cx - d, cy + d, fs_cx - d, cy + d - arm, stroke, RgbaColor::WHITE);
        fb.draw_line_aa(fs_cx + d, cy + d, fs_cx + d - arm, cy + d, stroke, RgbaColor::WHITE);
        fb.draw_line_aa(fs_cx + d, cy + d, fs_cx + d, cy + d - arm, stroke, RgbaColor::WHITE);

        if let Some(ref ql) = st.quality_label {
            let qx = (x + w - 92.0) * s;
            let qy = cy - 10.0 * s;
            fb.fill_rounded_rect(
                qx,
                qy,
                44.0 * s,
                20.0 * s,
                [4.0 * s; 4],
                RgbaColor::new(255, 255, 255, 36),
            );
            let q_px = 11.0 * s;
            let qtw = fonts.measure_text(ql, FontFaceKey::SansBold, q_px);
            let qasc = fonts.line_ascent(FontFaceKey::SansBold, q_px);
            fonts.draw_text(
                fb,
                ql,
                qx + (44.0 * s - qtw) * 0.5,
                qy + (20.0 * s - q_px) * 0.5 + qasc,
                FontFaceKey::SansBold,
                q_px,
                RgbaColor::WHITE,
            );
        }

        if st.has_captions && w >= 300.0 {
            let cc_x = (x + w - 130.0) * s;
            let cc_y = cy - 10.0 * s;
            let bg_c = if st.captions_enabled {
                RgbaColor::new(255, 0, 51, 210)
            } else {
                RgbaColor::new(255, 255, 255, 36)
            };
            fb.fill_rounded_rect(cc_x, cc_y, 30.0 * s, 20.0 * s, [4.0 * s; 4], bg_c);
            let cc_px = 11.0 * s;
            let ctw = fonts.measure_text("CC", FontFaceKey::SansBold, cc_px);
            let casc = fonts.line_ascent(FontFaceKey::SansBold, cc_px);
            fonts.draw_text(
                fb,
                "CC",
                cc_x + (30.0 * s - ctw) * 0.5,
                cc_y + (20.0 * s - cc_px) * 0.5 + casc,
                FontFaceKey::SansBold,
                cc_px,
                RgbaColor::WHITE,
            );
        }
    }

    if has_radius {
        fb.pop_rounded_clip();
    }
    fb.pop_clip();
}

#[allow(clippy::too_many_arguments)]
fn paint_audio_bar(
    fb: &mut Framebuffer,
    fonts: &FontSystem,
    st: &VideoOverlayState,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    alpha: f32,
    s: f32,
) {
    let r = (h * 0.5).min(22.0) * s;
    fb.fill_rounded_rect(
        x * s,
        y * s,
        w * s,
        h * s,
        [r; 4],
        RgbaColor::new(28, 31, 38, 255).with_alpha(alpha),
    );
    let cy = (y + h * 0.5) * s;
    let px = (x + 20.0) * s;
    if st.playing {
        fb.fill_rect(px - 4.0 * s, cy - 6.0 * s, 3.0 * s, 12.0 * s, RgbaColor::WHITE);
        fb.fill_rect(px + 2.0 * s, cy - 6.0 * s, 3.0 * s, 12.0 * s, RgbaColor::WHITE);
    } else {
        let tri = vec![
            (px - 4.0 * s, cy - 6.0 * s),
            (px + 6.0 * s, cy),
            (px - 4.0 * s, cy + 6.0 * s),
        ];
        fb.fill_polygon(&[tri], RgbaColor::WHITE, false);
    }
    let label = format!(
        "{} / {}",
        format_media_time(st.current_time),
        format_media_time(st.duration)
    );
    let t_px = 11.5 * s;
    let asc = fonts.line_ascent(FontFaceKey::SansRegular, t_px);
    fonts.draw_text(
        fb,
        &label,
        (x + 36.0) * s,
        cy - t_px * 0.5 + asc,
        FontFaceKey::SansRegular,
        t_px,
        RgbaColor::new(230, 235, 240, 255),
    );
    let bar_x = x + 112.0;
    let bar_w = (w - 130.0).max(20.0);
    if bar_w > 20.0 {
        fb.fill_rounded_rect(
            bar_x * s,
            (y + h * 0.5 - 2.0) * s,
            bar_w * s,
            4.0 * s,
            [2.0 * s; 4],
            RgbaColor::new(255, 255, 255, 65),
        );
        let frac = if st.duration > 0.0 {
            (st.current_time / st.duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        fb.fill_rounded_rect(
            bar_x * s,
            (y + h * 0.5 - 2.0) * s,
            (bar_w * frac) * s,
            4.0 * s,
            [2.0 * s; 4],
            RgbaColor::new(96, 165, 250, 255),
        );
    }
}
