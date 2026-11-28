use image::{ImageBuffer, Rgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RgbaColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl RgbaColor {
    pub const TRANSPARENT: Self = Self { r: 0, g: 0, b: 0, a: 0 };
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0, a: 255 };
    pub const WHITE: Self = Self { r: 255, g: 255, b: 255, a: 255 };

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        let a = ((self.a as f32) * alpha.clamp(0.0, 1.0)).round() as u8;
        Self { a, ..self }
    }

    pub fn to_xrgb_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ClipRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl ClipRect {
    pub fn intersect(self, other: Self) -> Self {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.w).min(other.x + other.w);
        let y2 = (self.y + self.h).min(other.y + other.h);
        Self {
            x: x1,
            y: y1,
            w: (x2 - x1).max(0),
            h: (y2 - y1).max(0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Affine2D {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Affine2D {
    fn default() -> Self {
        Self::identity()
    }
}

impl Affine2D {
    pub const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn translate(tx: f32, ty: f32) -> Self {
        Self {
            e: tx,
            f: ty,
            ..Self::identity()
        }
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            d: sy,
            ..Self::identity()
        }
    }

    pub fn rotate(rad: f32) -> Self {
        let (sin, cos) = rad.sin_cos();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn mul(self, rhs: Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }

    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

#[derive(Clone, Debug)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    clip_stack: Vec<ClipRect>,
    rounded_clips: Vec<(f32, f32, f32, f32, [f32; 4])>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width as usize).saturating_mul(height as usize).saturating_mul(4);
        Self {
            width,
            height,
            data: vec![0u8; size],
            clip_stack: vec![ClipRect {
                x: 0,
                y: 0,
                w: width as i32,
                h: height as i32,
            }],
            rounded_clips: Vec::new(),
        }
    }

    pub fn clear(&mut self, color: RgbaColor) {
        let len = self.data.len();
        if len < 4 {
            return;
        }
        self.data[0] = color.r;
        self.data[1] = color.g;
        self.data[2] = color.b;
        self.data[3] = color.a;
        let mut filled = 4usize;
        while filled < len {
            let copy_len = filled.min(len - filled);
            self.data.copy_within(0..copy_len, filled);
            filled += copy_len;
        }
    }

    pub fn resize_or_clear(&mut self, width: u32, height: u32, color: RgbaColor) {
        let needed = (width as usize).saturating_mul(height as usize).saturating_mul(4);
        if self.width != width || self.height != height || self.data.len() != needed {
            self.width = width;
            self.height = height;
            self.data.resize(needed, 0);
        }
        self.clip_stack.clear();
        self.rounded_clips.clear();
        self.clip_stack.push(ClipRect {
            x: 0,
            y: 0,
            w: width as i32,
            h: height as i32,
        });
        self.clear(color);
    }

    pub fn draw_line_aa(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, thickness: f32, color: RgbaColor) {
        if color.a == 0 || thickness <= 0.0 {
            return;
        }
        let half = thickness * 0.5;
        let pad = half + 1.5;
        let clip = self.current_clip();
        let min_x = (x0.min(x1) - pad).floor() as i32;
        let max_x = (x0.max(x1) + pad).ceil() as i32;
        let min_y = (y0.min(y1) - pad).floor() as i32;
        let max_y = (y0.max(y1) + pad).ceil() as i32;
        let ix0 = min_x.max(clip.x);
        let iy0 = min_y.max(clip.y);
        let ix1 = max_x.min(clip.x + clip.w);
        let iy1 = max_y.min(clip.y + clip.h);
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len_sq = dx * dx + dy * dy;
        for py in iy0..iy1 {
            let cy = py as f32 + 0.5;
            for px in ix0..ix1 {
                let cx = px as f32 + 0.5;
                let dist = if len_sq < 1e-6 {
                    (cx - x0).hypot(cy - y0)
                } else {
                    let t = (((cx - x0) * dx + (cy - y0) * dy) / len_sq).clamp(0.0, 1.0);
                    let nx = x0 + t * dx;
                    let ny = y0 + t * dy;
                    (cx - nx).hypot(cy - ny)
                };
                let cov = (half + 0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(px, py, color.with_alpha(cov));
                }
            }
        }
    }

    pub fn draw_circle_fill(&mut self, cx: f32, cy: f32, r: f32, color: RgbaColor) {
        if color.a == 0 || r <= 0.0 {
            return;
        }
        let clip = self.current_clip();
        let x0 = ((cx - r - 1.5).floor() as i32).max(clip.x);
        let y0 = ((cy - r - 1.5).floor() as i32).max(clip.y);
        let x1 = ((cx + r + 1.5).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((cy + r + 1.5).ceil() as i32).min(clip.y + clip.h);
        for py in y0..y1 {
            let dy = py as f32 + 0.5 - cy;
            for px in x0..x1 {
                let dx = px as f32 + 0.5 - cx;
                let cov = (r + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(px, py, color.with_alpha(cov));
                }
            }
        }
    }

    pub fn draw_circle_stroke(&mut self, cx: f32, cy: f32, r: f32, thickness: f32, color: RgbaColor) {
        if color.a == 0 || r <= 0.0 || thickness <= 0.0 {
            return;
        }
        let half = thickness * 0.5;
        let outer = r + half + 1.5;
        let clip = self.current_clip();
        let x0 = ((cx - outer).floor() as i32).max(clip.x);
        let y0 = ((cy - outer).floor() as i32).max(clip.y);
        let x1 = ((cx + outer).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((cy + outer).ceil() as i32).min(clip.y + clip.h);
        for py in y0..y1 {
            let dy = py as f32 + 0.5 - cy;
            for px in x0..x1 {
                let dx = px as f32 + 0.5 - cx;
                let d = (dx.hypot(dy) - r).abs();
                let cov = (half + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(px, py, color.with_alpha(cov));
                }
            }
        }
    }

    pub fn draw_arc_stroke(
        &mut self,
        cx: f32,
        cy: f32,
        r: f32,
        start_rad: f32,
        sweep_rad: f32,
        thickness: f32,
        color: RgbaColor,
    ) {
        if color.a == 0 || r <= 0.0 || thickness <= 0.0 || sweep_rad.abs() < 1e-4 {
            return;
        }
        let steps = ((r * sweep_rad.abs()).ceil() as usize).clamp(8, 64);
        let mut prev_x = cx + r * start_rad.cos();
        let mut prev_y = cy + r * start_rad.sin();
        for i in 1..=steps {
            let a = start_rad + sweep_rad * (i as f32 / steps as f32);
            let nx = cx + r * a.cos();
            let ny = cy + r * a.sin();
            self.draw_line_aa(prev_x, prev_y, nx, ny, thickness, color);
            prev_x = nx;
            prev_y = ny;
        }
    }

    pub fn draw_triangle_fill(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: RgbaColor,
    ) {
        self.fill_polygon(&[vec![(x0, y0), (x1, y1), (x2, y2)]], color, false);
    }

    pub fn current_clip(&self) -> ClipRect {
        *self.clip_stack.last().unwrap()
    }

    pub fn push_clip(&mut self, rect: ClipRect) {
        let merged = self.current_clip().intersect(rect);
        self.clip_stack.push(merged);
    }

    pub fn pop_clip(&mut self) {
        if self.clip_stack.len() > 1 {
            self.clip_stack.pop();
        }
    }

    pub fn push_rounded_clip(&mut self, x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) {
        let max_r = (w * 0.5).min(h * 0.5).max(0.0);
        let clamped = [
            radii[0].clamp(0.0, max_r),
            radii[1].clamp(0.0, max_r),
            radii[2].clamp(0.0, max_r),
            radii[3].clamp(0.0, max_r),
        ];
        self.rounded_clips.push((x, y, w, h, clamped));
    }

    pub fn pop_rounded_clip(&mut self) {
        self.rounded_clips.pop();
    }

    fn rounded_clips_safe_inner(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> (i32, i32, i32, i32) {
        let mut sx0 = x0;
        let mut sy0 = y0;
        let mut sx1 = x1;
        let mut sy1 = y1;
        for &(rx, ry, rw, rh, radii) in &self.rounded_clips {
            let max_r = radii[0].max(radii[1]).max(radii[2]).max(radii[3]);
            sx0 = sx0.max((rx + max_r + 1.0).ceil() as i32);
            sy0 = sy0.max((ry + max_r + 1.0).ceil() as i32);
            sx1 = sx1.min((rx + rw - max_r - 1.0).floor() as i32);
            sy1 = sy1.min((ry + rh - max_r - 1.0).floor() as i32);
        }
        if sx0 > sx1 {
            sx0 = x1;
            sx1 = x1;
        }
        if sy0 > sy1 {
            sy0 = y1;
            sy1 = y1;
        }
        (sx0, sy0, sx1, sy1)
    }

    #[inline]
    pub fn blend_pixel(&mut self, x: i32, y: i32, mut color: RgbaColor) {
        if color.a == 0 {
            return;
        }
        let clip = self.current_clip();
        if x < clip.x || y < clip.y || x >= clip.x + clip.w || y >= clip.y + clip.h {
            return;
        }
        if !self.rounded_clips.is_empty() {
            let cx = x as f32 + 0.5;
            let cy = y as f32 + 0.5;
            let mut cov = 1.0f32;
            for &(rx, ry, rw, rh, radii) in &self.rounded_clips {
                let max_r = radii[0].max(radii[1]).max(radii[2]).max(radii[3]);
                if cx >= rx + max_r + 0.5
                    && cx <= rx + rw - max_r - 0.5
                    && cy >= ry + max_r + 0.5
                    && cy <= ry + rh - max_r - 0.5
                {
                    continue;
                }
                let d = rounded_rect_sdf(cx, cy, rx, ry, rw, rh, radii);
                let c = (0.5 - d).clamp(0.0, 1.0);
                if c <= 0.0 {
                    return;
                }
                cov *= c;
            }
            if cov < 1.0 {
                color = color.with_alpha(cov);
                if color.a == 0 {
                    return;
                }
            }
        }
        let idx = ((y as u32 * self.width + x as u32) * 4) as usize;
        if idx + 3 >= self.data.len() {
            return;
        }
        if color.a == 255 {
            self.data[idx] = color.r;
            self.data[idx + 1] = color.g;
            self.data[idx + 2] = color.b;
            self.data[idx + 3] = 255;
            return;
        }
        let sa = color.a as u32;
        let inv = 255 - sa;
        let dr = self.data[idx] as u32;
        let dg = self.data[idx + 1] as u32;
        let db = self.data[idx + 2] as u32;
        let da = self.data[idx + 3] as u32;
        self.data[idx] = ((color.r as u32 * sa + dr * inv + 127) / 255) as u8;
        self.data[idx + 1] = ((color.g as u32 * sa + dg * inv + 127) / 255) as u8;
        self.data[idx + 2] = ((color.b as u32 * sa + db * inv + 127) / 255) as u8;
        self.data[idx + 3] = (sa + (da * inv + 127) / 255).min(255) as u8;
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: RgbaColor) {
        if color.a == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let clip = self.current_clip();
        let x0 = (x.floor() as i32).max(clip.x);
        let y0 = (y.floor() as i32).max(clip.y);
        let x1 = ((x + w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((y + h).ceil() as i32).min(clip.y + clip.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        if color.a == 255 && self.rounded_clips.is_empty() {
            let row_w = (x1 - x0) as usize;
            let row_bytes = row_w * 4;
            let px_val = [color.r, color.g, color.b, 255];
            let first_off = (y0 as usize * self.width as usize + x0 as usize) * 4;
            let first_row = &mut self.data[first_off..first_off + row_bytes];
            for chunk in first_row.chunks_exact_mut(4) {
                chunk.copy_from_slice(&px_val);
            }
            let stride = self.width as usize * 4;
            for py in (y0 + 1)..y1 {
                let dst_off = (py as usize * self.width as usize + x0 as usize) * 4;
                self.data.copy_within(first_off..first_off + row_bytes, dst_off);
                let _ = stride;
            }
        } else {
            for py in y0..y1 {
                for px in x0..x1 {
                    self.blend_pixel(px, py, color);
                }
            }
        }
    }

    pub fn fill_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
        color: RgbaColor,
    ) {
        if color.a == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        if radii.iter().all(|&r| r <= 0.01) {
            self.fill_rect(x, y, w, h, color);
            return;
        }
        let clip = self.current_clip();
        let x0 = (x.floor() as i32).max(clip.x);
        let y0 = (y.floor() as i32).max(clip.y);
        let x1 = ((x + w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((y + h).ceil() as i32).min(clip.y + clip.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let max_r = (w * 0.5).min(h * 0.5);
        let r = [
            radii[0].clamp(0.0, max_r),
            radii[1].clamp(0.0, max_r),
            radii[2].clamp(0.0, max_r),
            radii[3].clamp(0.0, max_r),
        ];
        if self.rounded_clips.is_empty() && color.a == 255 {
            let top_r = r[0].max(r[1]);
            let bot_r = r[2].max(r[3]);
            let mid_y0 = ((y + top_r).ceil() as i32).clamp(y0, y1);
            let mid_y1 = ((y + h - bot_r).floor() as i32).clamp(mid_y0, y1);
            if mid_y0 < mid_y1 {
                let row_px = (x1 - x0) as usize;
                let mut row_buf = vec![0u8; row_px * 4];
                for chunk in row_buf.chunks_exact_mut(4) {
                    chunk[0] = color.r;
                    chunk[1] = color.g;
                    chunk[2] = color.b;
                    chunk[3] = 255;
                }
                let stride = self.width as usize * 4;
                for py in mid_y0..mid_y1 {
                    let dst_off = py as usize * stride + x0 as usize * 4;
                    if dst_off + row_buf.len() <= self.data.len() {
                        self.data[dst_off..dst_off + row_buf.len()].copy_from_slice(&row_buf);
                    }
                }
            }
            for py in (y0..mid_y0).chain(mid_y1..y1) {
                let cy = py as f32 + 0.5;
                let is_top = py < mid_y0;
                let left_r = if is_top { r[0] } else { r[3] };
                let right_r = if is_top { r[1] } else { r[2] };
                let inner_x0 = ((x + left_r).ceil() as i32).clamp(x0, x1);
                let inner_x1 = ((x + w - right_r).floor() as i32).clamp(inner_x0, x1);
                for px in x0..inner_x0 {
                    let cx = px as f32 + 0.5;
                    let cov = (0.5 - rounded_rect_sdf(cx, cy, x, y, w, h, r)).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.blend_pixel(px, py, color.with_alpha(cov));
                    }
                }
                if inner_x0 < inner_x1 {
                    let row = (py as u32 * self.width) as usize;
                    for px in inner_x0..inner_x1 {
                        let idx = (row + px as usize) * 4;
                        self.data[idx] = color.r;
                        self.data[idx + 1] = color.g;
                        self.data[idx + 2] = color.b;
                        self.data[idx + 3] = 255;
                    }
                }
                for px in inner_x1..x1 {
                    let cx = px as f32 + 0.5;
                    let cov = (0.5 - rounded_rect_sdf(cx, cy, x, y, w, h, r)).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.blend_pixel(px, py, color.with_alpha(cov));
                    }
                }
            }
            return;
        }
        for py in y0..y1 {
            let cy = py as f32 + 0.5;
            for px in x0..x1 {
                let cx = px as f32 + 0.5;
                let dist = rounded_rect_sdf(cx, cy, x, y, w, h, r);
                let cov = (0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(px, py, color.with_alpha(cov));
                }
            }
        }
    }

    pub fn stroke_styled_borders(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
        widths: [f32; 4],
        colors: [RgbaColor; 4],
        styles: [u8; 4],
    ) {
        if styles.iter().all(|&s| s == 0) {
            self.stroke_rounded_rect(x, y, w, h, radii, widths, colors);
            return;
        }
        let draw_side = |fb: &mut Framebuffer,
                         sx: f32,
                         sy: f32,
                         len: f32,
                         thick: f32,
                         horiz: bool,
                         color: RgbaColor,
                         st: u8| {
            if thick <= 0.0 || len <= 0.0 || color.a == 0 {
                return;
            }
            match st {
                1 => {
                    let dash = (thick * 3.0).max(4.0);
                    let gap = (thick * 2.0).max(3.0);
                    let mut pos = 0.0;
                    while pos < len {
                        let seg = dash.min(len - pos);
                        if horiz {
                            fb.fill_rect(sx + pos, sy, seg, thick, color);
                        } else {
                            fb.fill_rect(sx, sy + pos, thick, seg, color);
                        }
                        pos += dash + gap;
                    }
                }
                2 => {
                    let dot = thick.max(1.5);
                    let step = (dot * 2.0).max(3.0);
                    let mut pos = 0.0;
                    while pos < len {
                        let seg = dot.min(len - pos);
                        if horiz {
                            fb.fill_rect(sx + pos, sy, seg, thick, color);
                        } else {
                            fb.fill_rect(sx, sy + pos, thick, seg, color);
                        }
                        pos += step;
                    }
                }
                3 => {
                    let band = (thick / 3.0).max(1.0);
                    if horiz {
                        fb.fill_rect(sx, sy, len, band, color);
                        fb.fill_rect(sx, sy + thick - band, len, band, color);
                    } else {
                        fb.fill_rect(sx, sy, band, len, color);
                        fb.fill_rect(sx + thick - band, sy, band, len, color);
                    }
                }
                _ => {
                    if horiz {
                        fb.fill_rect(sx, sy, len, thick, color);
                    } else {
                        fb.fill_rect(sx, sy, thick, len, color);
                    }
                }
            }
        };
        draw_side(self, x, y, w, widths[0], true, colors[0], styles[0]);
        draw_side(self, x + w - widths[1], y, h, widths[1], false, colors[1], styles[1]);
        draw_side(self, x, y + h - widths[2], w, widths[2], true, colors[2], styles[2]);
        draw_side(self, x, y, h, widths[3], false, colors[3], styles[3]);
    }

    pub fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
        widths: [f32; 4],
        colors: [RgbaColor; 4],
    ) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let uniform = widths[0] == widths[1]
            && widths[1] == widths[2]
            && widths[2] == widths[3]
            && colors[0] == colors[1]
            && colors[1] == colors[2]
            && colors[2] == colors[3];

        if uniform && widths[0] > 0.0 && radii.iter().any(|&r| r > 0.01) {
            let bw = widths[0];
            let clip = self.current_clip();
            let x0 = (x.floor() as i32).max(clip.x);
            let y0 = (y.floor() as i32).max(clip.y);
            let x1 = ((x + w).ceil() as i32).min(clip.x + clip.w);
            let y1 = ((y + h).ceil() as i32).min(clip.y + clip.h);
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            let max_r = (w * 0.5).min(h * 0.5);
            let ro = [
                radii[0].clamp(0.0, max_r),
                radii[1].clamp(0.0, max_r),
                radii[2].clamp(0.0, max_r),
                radii[3].clamp(0.0, max_r),
            ];
            let ri = [
                (ro[0] - bw).max(0.0),
                (ro[1] - bw).max(0.0),
                (ro[2] - bw).max(0.0),
                (ro[3] - bw).max(0.0),
            ];
            let iw = (w - bw * 2.0).max(0.0);
            let ih = (h - bw * 2.0).max(0.0);
            let max_ro = ro[0].max(ro[1]).max(ro[2]).max(ro[3]);
            let pad_skip = bw + max_ro + 2.0;
            let skip_x0 = ((x + pad_skip).ceil() as i32).clamp(x0, x1);
            let skip_x1 = ((x + w - pad_skip).floor() as i32).clamp(skip_x0, x1);
            let skip_y0 = ((y + pad_skip).ceil() as i32).clamp(y0, y1);
            let skip_y1 = ((y + h - pad_skip).floor() as i32).clamp(skip_y0, y1);
            for py in y0..y1 {
                let cy = py as f32 + 0.5;
                let skip_mid = py >= skip_y0 && py < skip_y1 && skip_x0 < skip_x1;
                let ranges: [(i32, i32); 2] = if skip_mid {
                    [(x0, skip_x0), (skip_x1, x1)]
                } else {
                    [(x0, x1), (0, 0)]
                };
                for &(rx0, rx1) in &ranges {
                    for px in rx0..rx1 {
                        let cx = px as f32 + 0.5;
                        let d_out = rounded_rect_sdf(cx, cy, x, y, w, h, ro);
                        if d_out > 0.5 {
                            continue;
                        }
                        let d_in = if iw > 0.0 && ih > 0.0 {
                            rounded_rect_sdf(cx, cy, x + bw, y + bw, iw, ih, ri)
                        } else {
                            1.0
                        };
                        let cov_out = (0.5 - d_out).clamp(0.0, 1.0);
                        let cov_in = (0.5 - d_in).clamp(0.0, 1.0);
                        let cov = (cov_out - cov_in).clamp(0.0, 1.0);
                        if cov > 0.0 {
                            self.blend_pixel(px, py, colors[0].with_alpha(cov));
                        }
                    }
                }
            }
            return;
        }

        let [bt, br, bb, bl] = widths;
        let clip = self.current_clip();
        let x0 = (x.floor() as i32).max(clip.x);
        let y0 = (y.floor() as i32).max(clip.y);
        let x1 = ((x + w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((y + h).ceil() as i32).min(clip.y + clip.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }

        if radii.iter().any(|&r| r > 0.01) {
            let max_r = (w * 0.5).min(h * 0.5);
            let ro = [
                radii[0].clamp(0.0, max_r),
                radii[1].clamp(0.0, max_r),
                radii[2].clamp(0.0, max_r),
                radii[3].clamp(0.0, max_r),
            ];
            let iw = (w - bl - br).max(0.0);
            let ih = (h - bt - bb).max(0.0);
            let max_ri = (iw * 0.5).min(ih * 0.5);
            let ri = [
                (ro[0] - bt.max(bl)).clamp(0.0, max_ri),
                (ro[1] - bt.max(br)).clamp(0.0, max_ri),
                (ro[2] - bb.max(br)).clamp(0.0, max_ri),
                (ro[3] - bb.max(bl)).clamp(0.0, max_ri),
            ];
            let max_bw = bt.max(br).max(bb).max(bl);
            let max_ro = ro[0].max(ro[1]).max(ro[2]).max(ro[3]);
            let pad_skip = max_bw + max_ro + 2.0;
            let skip_x0 = ((x + pad_skip).ceil() as i32).clamp(x0, x1);
            let skip_x1 = ((x + w - pad_skip).floor() as i32).clamp(skip_x0, x1);
            let skip_y0 = ((y + pad_skip).ceil() as i32).clamp(y0, y1);
            let skip_y1 = ((y + h - pad_skip).floor() as i32).clamp(skip_y0, y1);
            let mid_x = x + w * 0.5;
            let mid_y = y + h * 0.5;
            let inv_w = 1.0 / w.max(1.0);
            let inv_h = 1.0 / h.max(1.0);
            for py in y0..y1 {
                let cy = py as f32 + 0.5;
                let skip_mid = py >= skip_y0 && py < skip_y1 && skip_x0 < skip_x1;
                let ranges: [(i32, i32); 2] = if skip_mid {
                    [(x0, skip_x0), (skip_x1, x1)]
                } else {
                    [(x0, x1), (0, 0)]
                };
                for &(rx0, rx1) in &ranges {
                    for px in rx0..rx1 {
                        let cx = px as f32 + 0.5;
                        let d_out = rounded_rect_sdf(cx, cy, x, y, w, h, ro);
                        if d_out > 0.5 {
                            continue;
                        }
                        let d_in = if iw > 0.0 && ih > 0.0 {
                            rounded_rect_sdf(cx, cy, x + bl, y + bt, iw, ih, ri)
                        } else {
                            1.0
                        };
                        let cov_out = (0.5 - d_out).clamp(0.0, 1.0);
                        let cov_in = (0.5 - d_in).clamp(0.0, 1.0);
                        let cov = (cov_out - cov_in).clamp(0.0, 1.0);
                        if cov <= 0.0 {
                            continue;
                        }
                        let nx = (cx - mid_x) * inv_w;
                        let ny = (cy - mid_y) * inv_h;
                        let side = if ny.abs() >= nx.abs() {
                            if ny < 0.0 { 0 } else { 2 }
                        } else if nx > 0.0 {
                            1
                        } else {
                            3
                        };
                        if widths[side] > 0.0 && colors[side].a > 0 {
                            self.blend_pixel(px, py, colors[side].with_alpha(cov));
                        }
                    }
                }
            }
            return;
        }

        let inner_l = x + bl;
        let inner_r = x + w - br;
        let inner_t = y + bt;
        let inner_b = y + h - bb;

        for py in y0..y1 {
            let cy = py as f32 + 0.5;
            let in_top = bt > 0.0 && cy < inner_t;
            let in_bot = bb > 0.0 && cy >= inner_b;
            for px in x0..x1 {
                let cx = px as f32 + 0.5;
                let in_left = bl > 0.0 && cx < inner_l;
                let in_right = br > 0.0 && cx >= inner_r;
                if !in_top && !in_bot && !in_left && !in_right {
                    continue;
                }
                let mut best_side: Option<usize> = None;
                let mut best_ratio = f32::INFINITY;
                if in_top {
                    let r = (cy - y) / bt;
                    if r < best_ratio {
                        best_ratio = r;
                        best_side = Some(0);
                    }
                }
                if in_right {
                    let r = (x + w - cx) / br;
                    if r < best_ratio {
                        best_ratio = r;
                        best_side = Some(1);
                    }
                }
                if in_bot {
                    let r = (y + h - cy) / bb;
                    if r < best_ratio {
                        best_ratio = r;
                        best_side = Some(2);
                    }
                }
                if in_left {
                    let r = (cx - x) / bl;
                    if r < best_ratio {
                        best_side = Some(3);
                    }
                }
                if let Some(side) = best_side {
                    if colors[side].a > 0 {
                        self.blend_pixel(px, py, colors[side]);
                    }
                }
            }
        }
    }

    pub fn fill_linear_gradient(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
        angle_deg: f32,
        stops: &[(RgbaColor, f32)],
        repeating: bool,
    ) {
        if w <= 0.0 || h <= 0.0 || stops.is_empty() {
            return;
        }
        let clip = self.current_clip();
        let x0 = (x.floor() as i32).max(clip.x);
        let y0 = (y.floor() as i32).max(clip.y);
        let x1 = ((x + w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((y + h).ceil() as i32).min(clip.y + clip.h);
        let rad = (angle_deg - 90.0).to_radians();
        let dx = rad.cos();
        let dy = rad.sin();
        let cx0 = x + w * 0.5;
        let cy0 = y + h * 0.5;
        let half_len = ((w * dx.abs()) + (h * dy.abs())) * 0.5;
        let len = (half_len * 2.0).max(1.0);
        let max_r = (w * 0.5).min(h * 0.5);
        let r = [
            radii[0].clamp(0.0, max_r),
            radii[1].clamp(0.0, max_r),
            radii[2].clamp(0.0, max_r),
            radii[3].clamp(0.0, max_r),
        ];
        let has_radius = r.iter().any(|&v| v > 0.01);

        for py in y0..y1 {
            let cy = py as f32 + 0.5;
            for px in x0..x1 {
                let cx = px as f32 + 0.5;
                let cov = if has_radius {
                    (0.5 - rounded_rect_sdf(cx, cy, x, y, w, h, r)).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if cov <= 0.0 {
                    continue;
                }
                let proj = (cx - cx0) * dx + (cy - cy0) * dy;
                let mut t = (proj + half_len) / len;
                if repeating {
                    let span = (stops.last().unwrap().1 - stops[0].1).max(0.0001);
                    t = stops[0].1 + (t - stops[0].1).rem_euclid(span);
                }
                let c = sample_gradient(stops, t);
                self.blend_pixel(px, py, c.with_alpha(cov));
            }
        }
    }

    pub fn fill_radial_gradient(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
        stops: &[(RgbaColor, f32)],
        repeating: bool,
    ) {
        if w <= 0.0 || h <= 0.0 || stops.is_empty() {
            return;
        }
        let clip = self.current_clip();
        let x0 = (x.floor() as i32).max(clip.x);
        let y0 = (y.floor() as i32).max(clip.y);
        let x1 = ((x + w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((y + h).ceil() as i32).min(clip.y + clip.h);
        let cx0 = x + w * 0.5;
        let cy0 = y + h * 0.5;
        let rx = (w * 0.5).max(1.0);
        let ry = (h * 0.5).max(1.0);
        let max_r = rx.min(ry);
        let r = [
            radii[0].clamp(0.0, max_r),
            radii[1].clamp(0.0, max_r),
            radii[2].clamp(0.0, max_r),
            radii[3].clamp(0.0, max_r),
        ];
        let has_radius = r.iter().any(|&v| v > 0.01);

        for py in y0..y1 {
            let cy = py as f32 + 0.5;
            for px in x0..x1 {
                let cx = px as f32 + 0.5;
                let cov = if has_radius {
                    (0.5 - rounded_rect_sdf(cx, cy, x, y, w, h, r)).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if cov <= 0.0 {
                    continue;
                }
                let nx = (cx - cx0) / rx;
                let ny = (cy - cy0) / ry;
                let mut t = (nx * nx + ny * ny).sqrt();
                if repeating {
                    let span = (stops.last().unwrap().1 - stops[0].1).max(0.0001);
                    t = stops[0].1 + (t - stops[0].1).rem_euclid(span);
                }
                let c = sample_gradient(stops, t);
                self.blend_pixel(px, py, c.with_alpha(cov));
            }
        }
    }

    pub fn fill_conic_gradient(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
        from_deg: f32,
        stops: &[(RgbaColor, f32)],
        repeating: bool,
    ) {
        if w <= 0.0 || h <= 0.0 || stops.is_empty() {
            return;
        }
        let clip = self.current_clip();
        let x0 = (x.floor() as i32).max(clip.x);
        let y0 = (y.floor() as i32).max(clip.y);
        let x1 = ((x + w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((y + h).ceil() as i32).min(clip.y + clip.h);
        let cx0 = x + w * 0.5;
        let cy0 = y + h * 0.5;
        let from_rad = from_deg.to_radians();
        let tau = std::f32::consts::TAU;
        for py in y0..y1 {
            let cy = py as f32 + 0.5;
            for px in x0..x1 {
                let cx = px as f32 + 0.5;
                let dist = rounded_rect_sdf(cx, cy, x, y, w, h, radii);
                let cov = (0.5 - dist).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let angle = (cx - cx0).atan2(-(cy - cy0)) - from_rad;
                let mut t = angle.rem_euclid(tau) / tau;
                if repeating && stops.len() >= 2 {
                    let span = (stops.last().unwrap().1 - stops[0].1).max(0.0001);
                    t = stops[0].1 + (t - stops[0].1).rem_euclid(span);
                }
                let c = sample_gradient(stops, t);
                self.blend_pixel(px, py, c.with_alpha(cov));
            }
        }
    }

    pub fn draw_box_shadow(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radii: [f32; 4],
        offset_x: f32,
        offset_y: f32,
        blur: f32,
        spread: f32,
        color: RgbaColor,
        inset: bool,
    ) {
        if color.a == 0 {
            return;
        }
        let clip = self.current_clip();
        let pad = blur.max(0.0);
        let sigma = (blur * 0.5).max(0.5);
        if inset {
            let x0 = (x.floor() as i32).max(clip.x);
            let y0 = (y.floor() as i32).max(clip.y);
            let x1 = ((x + w).ceil() as i32).min(clip.x + clip.w);
            let y1 = ((y + h).ceil() as i32).min(clip.y + clip.h);
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            let ix = x + offset_x + spread;
            let iy = y + offset_y + spread;
            let iw = (w - spread * 2.0).max(0.0);
            let ih = (h - spread * 2.0).max(0.0);
            let max_r = (iw * 0.5).min(ih * 0.5);
            let ir = [
                (radii[0] - spread).clamp(0.0, max_r),
                (radii[1] - spread).clamp(0.0, max_r),
                (radii[2] - spread).clamp(0.0, max_r),
                (radii[3] - spread).clamp(0.0, max_r),
            ];
            for py in y0..y1 {
                let cy = py as f32 + 0.5;
                for px in x0..x1 {
                    let cx = px as f32 + 0.5;
                    let outer_d = rounded_rect_sdf(cx, cy, x, y, w, h, radii);
                    let outer_cov = (0.5 - outer_d).clamp(0.0, 1.0);
                    if outer_cov <= 0.0 {
                        continue;
                    }
                    let inner_d = if iw <= 0.0 || ih <= 0.0 {
                        pad + 1.0
                    } else {
                        rounded_rect_sdf(cx, cy, ix, iy, iw, ih, ir)
                    };
                    let sh_alpha = if blur <= 0.1 {
                        (inner_d + 0.5).clamp(0.0, 1.0)
                    } else {
                        let t = ((inner_d + pad) / (2.0 * sigma)).clamp(0.0, 1.0);
                        t * t * (3.0 - 2.0 * t)
                    };
                    let a = sh_alpha * outer_cov;
                    if a > 0.0 {
                        self.blend_pixel(px, py, color.with_alpha(a));
                    }
                }
            }
            return;
        }
        let sx = x + offset_x - spread;
        let sy = y + offset_y - spread;
        let sw = (w + spread * 2.0).max(0.0);
        let sh = (h + spread * 2.0).max(0.0);
        let x0 = ((sx - pad).floor() as i32).max(clip.x);
        let y0 = ((sy - pad).floor() as i32).max(clip.y);
        let x1 = ((sx + sw + pad).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((sy + sh + pad).ceil() as i32).min(clip.y + clip.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let max_r = (sw * 0.5).min(sh * 0.5);
        let sr = [
            (radii[0] + spread).clamp(0.0, max_r),
            (radii[1] + spread).clamp(0.0, max_r),
            (radii[2] + spread).clamp(0.0, max_r),
            (radii[3] + spread).clamp(0.0, max_r),
        ];
        let box_max_r = radii[0].max(radii[1]).max(radii[2]).max(radii[3]);
        let skip_x0 = ((x + box_max_r + 1.0).ceil() as i32).clamp(x0, x1);
        let skip_x1 = ((x + w - box_max_r - 1.0).floor() as i32).clamp(skip_x0, x1);
        let skip_y0 = ((y + box_max_r + 1.0).ceil() as i32).clamp(y0, y1);
        let skip_y1 = ((y + h - box_max_r - 1.0).floor() as i32).clamp(skip_y0, y1);
        for py in y0..y1 {
            let cy = py as f32 + 0.5;
            let skip_mid = py >= skip_y0 && py < skip_y1 && skip_x0 < skip_x1;
            let ranges: [(i32, i32); 2] = if skip_mid {
                [(x0, skip_x0), (skip_x1, x1)]
            } else {
                [(x0, x1), (0, 0)]
            };
            for &(rx0, rx1) in &ranges {
                for px in rx0..rx1 {
                    let cx = px as f32 + 0.5;
                    let box_d = rounded_rect_sdf(cx, cy, x, y, w, h, radii);
                    let out_cov = (box_d + 0.5).clamp(0.0, 1.0);
                    if out_cov <= 0.0 {
                        continue;
                    }
                    let dist = rounded_rect_sdf(cx, cy, sx, sy, sw, sh, sr);
                    let alpha = if blur <= 0.1 {
                        (0.5 - dist).clamp(0.0, 1.0)
                    } else {
                        let t = ((pad - dist) / (2.0 * sigma)).clamp(0.0, 1.0);
                        t * t * (3.0 - 2.0 * t)
                    };
                    let a = alpha * out_cov;
                    if a > 0.0 {
                        self.blend_pixel(px, py, color.with_alpha(a));
                    }
                }
            }
        }
    }

    pub fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: RgbaColor) {
        if color.a == 0 || width <= 0.0 {
            return;
        }
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.001 {
            let r = width * 0.5;
            self.fill_rounded_rect(x0 - r, y0 - r, width, width, [r; 4], color);
            return;
        }
        let half = width * 0.5;
        let clip = self.current_clip();
        let min_x = ((x0.min(x1) - half - 1.0).floor() as i32).max(clip.x);
        let min_y = ((y0.min(y1) - half - 1.0).floor() as i32).max(clip.y);
        let max_x = ((x0.max(x1) + half + 1.0).ceil() as i32).min(clip.x + clip.w);
        let max_y = ((y0.max(y1) + half + 1.0).ceil() as i32).min(clip.y + clip.h);
        let inv_len_sq = 1.0 / (len * len);
        for py in min_y..max_y {
            let cy = py as f32 + 0.5;
            for px in min_x..max_x {
                let cx = px as f32 + 0.5;
                let t = (((cx - x0) * dx + (cy - y0) * dy) * inv_len_sq).clamp(0.0, 1.0);
                let proj_x = x0 + t * dx;
                let proj_y = y0 + t * dy;
                let dist = ((cx - proj_x).powi(2) + (cy - proj_y).powi(2)).sqrt() - half;
                let cov = (0.5 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend_pixel(px, py, color.with_alpha(cov));
                }
            }
        }
    }

    pub fn fill_polygon(&mut self, contours: &[Vec<(f32, f32)>], color: RgbaColor, even_odd: bool) {
        if color.a == 0 || contours.is_empty() {
            return;
        }
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for c in contours {
            for &(_, y) in c {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
        if !min_y.is_finite() || !max_y.is_finite() {
            return;
        }
        let clip = self.current_clip();
        let y0 = (min_y.floor() as i32).max(clip.y);
        let y1 = (max_y.ceil() as i32).min(clip.y + clip.h);

        let mut intersections: Vec<(f32, i32)> = Vec::new();
        for py in y0..y1 {
            let scan_y = py as f32 + 0.5;
            intersections.clear();
            for poly in contours {
                if poly.len() < 2 {
                    continue;
                }
                for i in 0..poly.len() {
                    let (x1, y1p) = poly[i];
                    let (x2, y2p) = poly[(i + 1) % poly.len()];
                    if (y1p <= scan_y && y2p > scan_y) || (y2p <= scan_y && y1p > scan_y) {
                        let t = (scan_y - y1p) / (y2p - y1p);
                        let ix = x1 + t * (x2 - x1);
                        let dir = if y2p > y1p { 1 } else { -1 };
                        intersections.push((ix, dir));
                    }
                }
            }
            if intersections.is_empty() {
                continue;
            }
            intersections.sort_by(|a, b| a.0.total_cmp(&b.0));
            if even_odd {
                let mut i = 0;
                while i + 1 < intersections.len() {
                    let sx = (intersections[i].0.floor() as i32).max(clip.x);
                    let ex = (intersections[i + 1].0.ceil() as i32).min(clip.x + clip.w);
                    for px in sx..ex {
                        self.blend_pixel(px, py, color);
                    }
                    i += 2;
                }
            } else {
                let mut winding = 0i32;
                let mut span_start = 0.0f32;
                for &(ix, dir) in &intersections {
                    let prev = winding;
                    winding += dir;
                    if prev == 0 && winding != 0 {
                        span_start = ix;
                    } else if prev != 0 && winding == 0 {
                        let sx = (span_start.floor() as i32).max(clip.x);
                        let ex = (ix.ceil() as i32).min(clip.x + clip.w);
                        for px in sx..ex {
                            self.blend_pixel(px, py, color);
                        }
                    }
                }
            }
        }
    }

    pub fn blit_rgba(
        &mut self,
        src_w: u32,
        src_h: u32,
        src: &[u8],
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
        opacity: f32,
    ) {
        if src_w == 0 || src_h == 0 || dst_w <= 0.0 || dst_h <= 0.0 || opacity <= 0.0 {
            return;
        }
        let clip = self.current_clip();
        let x0 = (dst_x.floor() as i32).max(clip.x);
        let y0 = (dst_y.floor() as i32).max(clip.y);
        let x1 = ((dst_x + dst_w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((dst_y + dst_h).ceil() as i32).min(clip.y + clip.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let is_scaled = (dst_w - src_w as f32).abs() > 0.5 || (dst_h - src_h as f32).abs() > 0.5;
        let max_sx = src_w.saturating_sub(1);
        let max_sy = src_h.saturating_sub(1);
        let src_stride = src_w as usize * 4;
        let dst_stride = self.width as usize * 4;
        let (rc_x0, rc_y0, rc_x1, rc_y1) = self.rounded_clips_safe_inner(x0, y0, x1, y1);
        let no_rc = self.rounded_clips.is_empty();
        let opaque_alpha = opacity >= 0.999;
        if is_scaled && src_w > 1 && src_h > 1 {
            let max_fx_fp = (max_sx as i64) << 16;
            let fx_step = ((src_w as f64 / dst_w as f64) * 65536.0) as i64;
            let fx_base = (((x0 as f64 + 0.5 - dst_x as f64) / dst_w as f64 * src_w as f64 - 0.5)
                * 65536.0) as i64;
            for py in y0..y1 {
                let row_rc_safe = no_rc || (py >= rc_y0 && py < rc_y1);
                let fy = (((py as f32 + 0.5 - dst_y) / dst_h) * src_h as f32 - 0.5)
                    .clamp(0.0, max_sy as f32);
                let sy0 = fy as u32;
                let sy1 = (sy0 + 1).min(max_sy);
                let wy = ((fy - sy0 as f32) * 256.0) as u32;
                let iwy = 256 - wy;
                let row0 = sy0 as usize * src_stride;
                let row1 = sy1 as usize * src_stride;
                let dst_row = py as usize * dst_stride;
                let mut fx_cur = fx_base;
                for px in x0..x1 {
                    let fxc = fx_cur.clamp(0, max_fx_fp);
                    fx_cur += fx_step;
                    let sx0 = (fxc >> 16) as u32;
                    let sx1 = (sx0 + 1).min(max_sx);
                    let wx = ((fxc >> 8) & 0xff) as u32;
                    let iwx = 256 - wx;
                    let i00 = row0 + sx0 as usize * 4;
                    let i10 = row0 + sx1 as usize * 4;
                    let i01 = row1 + sx0 as usize * 4;
                    let i11 = row1 + sx1 as usize * 4;
                    if i11 + 3 >= src.len() {
                        continue;
                    }
                    let a00 = src[i00 + 3];
                    let a10 = src[i10 + 3];
                    let a01 = src[i01 + 3];
                    let a11 = src[i11 + 3];
                    if (a00 | a10 | a01 | a11) == 0 {
                        continue;
                    }
                    let w00 = iwx * iwy;
                    let w10 = wx * iwy;
                    let w01 = iwx * wy;
                    let w11 = wx * wy;
                    let r = (src[i00] as u32 * w00
                        + src[i10] as u32 * w10
                        + src[i01] as u32 * w01
                        + src[i11] as u32 * w11
                        + 32768)
                        >> 16;
                    let g = (src[i00 + 1] as u32 * w00
                        + src[i10 + 1] as u32 * w10
                        + src[i01 + 1] as u32 * w01
                        + src[i11 + 1] as u32 * w11
                        + 32768)
                        >> 16;
                    let b = (src[i00 + 2] as u32 * w00
                        + src[i10 + 2] as u32 * w10
                        + src[i01 + 2] as u32 * w01
                        + src[i11 + 2] as u32 * w11
                        + 32768)
                        >> 16;
                    let rc_safe = no_rc || (row_rc_safe && px >= rc_x0 && px < rc_x1);
                    if opaque_alpha && rc_safe && (a00 & a10 & a01 & a11) == 255 {
                        let didx = dst_row + px as usize * 4;
                        self.data[didx] = r as u8;
                        self.data[didx + 1] = g as u8;
                        self.data[didx + 2] = b as u8;
                        self.data[didx + 3] = 255;
                    } else {
                        let a = (a00 as u32 * w00
                            + a10 as u32 * w10
                            + a01 as u32 * w01
                            + a11 as u32 * w11
                            + 32768)
                            >> 16;
                        if a == 0 {
                            continue;
                        }
                        let c = RgbaColor::new(r as u8, g as u8, b as u8, a as u8)
                            .with_alpha(opacity);
                        self.blend_pixel(px, py, c);
                    }
                }
            }
        } else {
            for py in y0..y1 {
                let row_rc_safe = no_rc || (py >= rc_y0 && py < rc_y1);
                let v = ((py as f32 + 0.5 - dst_y) / dst_h).clamp(0.0, 0.9999);
                let sy = ((v * src_h as f32) as u32).min(max_sy);
                let row0 = sy as usize * src_stride;
                let dst_row = py as usize * dst_stride;
                for px in x0..x1 {
                    let u = ((px as f32 + 0.5 - dst_x) / dst_w).clamp(0.0, 0.9999);
                    let sx = ((u * src_w as f32) as u32).min(max_sx);
                    let sidx = row0 + sx as usize * 4;
                    if sidx + 3 < src.len() {
                        let sa = src[sidx + 3];
                        if sa == 0 {
                            continue;
                        }
                        let rc_safe = no_rc || (row_rc_safe && px >= rc_x0 && px < rc_x1);
                        if opaque_alpha && rc_safe && sa == 255 {
                            let didx = dst_row + px as usize * 4;
                            self.data[didx] = src[sidx];
                            self.data[didx + 1] = src[sidx + 1];
                            self.data[didx + 2] = src[sidx + 2];
                            self.data[didx + 3] = 255;
                        } else {
                            let c = RgbaColor::new(src[sidx], src[sidx + 1], src[sidx + 2], sa)
                                .with_alpha(opacity);
                            self.blend_pixel(px, py, c);
                        }
                    }
                }
            }
        }
    }

    pub fn blit_rgba_opaque(
        &mut self,
        src_w: u32,
        src_h: u32,
        src: &[u8],
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
    ) {
        if src_w == 0 || src_h == 0 || dst_w <= 0.0 || dst_h <= 0.0 {
            return;
        }
        if !self.rounded_clips.is_empty() {
            self.blit_rgba(src_w, src_h, src, dst_x, dst_y, dst_w, dst_h, 1.0);
            return;
        }
        let clip = self.current_clip();
        let x0 = (dst_x.floor() as i32).max(clip.x).max(0);
        let y0 = (dst_y.floor() as i32).max(clip.y).max(0);
        let x1 = ((dst_x + dst_w).ceil() as i32)
            .min(clip.x + clip.w)
            .min(self.width as i32);
        let y1 = ((dst_y + dst_h).ceil() as i32)
            .min(clip.y + clip.h)
            .min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let exact_1to1 = (dst_w - src_w as f32).abs() < 0.01
            && (dst_h - src_h as f32).abs() < 0.01
            && (dst_x - dst_x.round()).abs() < 0.01
            && (dst_y - dst_y.round()).abs() < 0.01;
        if exact_1to1 {
            let dx0 = dst_x.round() as i32;
            let dy0 = dst_y.round() as i32;
            let src_stride = src_w as usize * 4;
            let dst_stride = self.width as usize * 4;
            let sx_start = (x0 - dx0).max(0) as usize;
            let span_px = ((x1 - x0) as usize).min((src_w as usize).saturating_sub(sx_start));
            let row_bytes = span_px * 4;
            for py in y0..y1 {
                let sy = py - dy0;
                if sy < 0 || sy >= src_h as i32 {
                    continue;
                }
                let s_off = sy as usize * src_stride + sx_start * 4;
                let d_off = py as usize * dst_stride + x0 as usize * 4;
                if s_off + row_bytes <= src.len() && d_off + row_bytes <= self.data.len() {
                    self.data[d_off..d_off + row_bytes]
                        .copy_from_slice(&src[s_off..s_off + row_bytes]);
                }
            }
            return;
        }
        let span_w = (x1 - x0) as usize;
        let mut sx_table = Vec::with_capacity(span_w);
        for px in x0..x1 {
            let u = ((px as f32 + 0.5 - dst_x) / dst_w).clamp(0.0, 0.9999);
            let sx = ((u * src_w as f32) as u32).min(src_w - 1) as usize;
            sx_table.push(sx * 4);
        }
        let dst_stride = self.width as usize * 4;
        let src_stride = src_w as usize * 4;
        for py in y0..y1 {
            let v = ((py as f32 + 0.5 - dst_y) / dst_h).clamp(0.0, 0.9999);
            let sy = ((v * src_h as f32) as u32).min(src_h - 1) as usize;
            let src_row = sy * src_stride;
            let dst_row = py as usize * dst_stride + x0 as usize * 4;
            for (i, &sx_off) in sx_table.iter().enumerate() {
                let sidx = src_row + sx_off;
                let didx = dst_row + i * 4;
                if sidx + 3 < src.len() && didx + 3 < self.data.len() {
                    self.data[didx] = src[sidx];
                    self.data[didx + 1] = src[sidx + 1];
                    self.data[didx + 2] = src[sidx + 2];
                    self.data[didx + 3] = 255;
                }
            }
        }
    }

    pub fn blit_rgba_rounded(
        &mut self,
        src_w: u32,
        src_h: u32,
        src: &[u8],
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
        clip_x: f32,
        clip_y: f32,
        clip_w: f32,
        clip_h: f32,
        radii: [f32; 4],
        opacity: f32,
    ) {
        if src_w == 0 || src_h == 0 || dst_w <= 0.0 || dst_h <= 0.0 || clip_w <= 0.0 || clip_h <= 0.0 || opacity <= 0.0 {
            return;
        }
        let clip = self.current_clip();
        let x0 = (clip_x.max(dst_x).floor() as i32).max(clip.x);
        let y0 = (clip_y.max(dst_y).floor() as i32).max(clip.y);
        let x1 = ((clip_x + clip_w).min(dst_x + dst_w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((clip_y + clip_h).min(dst_y + dst_h).ceil() as i32).min(clip.y + clip.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let max_r = (clip_w * 0.5).min(clip_h * 0.5);
        let r = [
            radii[0].clamp(0.0, max_r),
            radii[1].clamp(0.0, max_r),
            radii[2].clamp(0.0, max_r),
            radii[3].clamp(0.0, max_r),
        ];
        let has_radius = r.iter().any(|&v| v > 0.01);
        let max_corner = r[0].max(r[1]).max(r[2]).max(r[3]);
        let inner_x0 = ((clip_x + max_corner + 1.0).ceil() as i32).clamp(x0, x1);
        let inner_x1 = ((clip_x + clip_w - max_corner - 1.0).floor() as i32).clamp(inner_x0, x1);
        let inner_y0 = ((clip_y + max_corner + 1.0).ceil() as i32).clamp(y0, y1);
        let inner_y1 = ((clip_y + clip_h - max_corner - 1.0).floor() as i32).clamp(inner_y0, y1);
        let is_scaled = (dst_w - src_w as f32).abs() > 0.5 || (dst_h - src_h as f32).abs() > 0.5;
        let max_sx = src_w.saturating_sub(1);
        let max_sy = src_h.saturating_sub(1);
        let src_stride = src_w as usize * 4;
        let dst_stride = self.width as usize * 4;
        let (rc_x0, rc_y0, rc_x1, rc_y1) = self.rounded_clips_safe_inner(x0, y0, x1, y1);
        let no_rc = self.rounded_clips.is_empty();
        let opaque_alpha = opacity >= 0.999;
        let max_fx_fp = (max_sx as i64) << 16;
        let fx_step = ((src_w as f64 / dst_w as f64) * 65536.0) as i64;
        let fx_base = (((x0 as f64 + 0.5 - dst_x as f64) / dst_w as f64 * src_w as f64 - 0.5)
            * 65536.0) as i64;
        let use_bilinear = is_scaled && src_w > 1 && src_h > 1;
        let fast_x0 = if has_radius {
            if no_rc { inner_x0 } else { inner_x0.max(rc_x0) }
        } else if no_rc {
            x0
        } else {
            rc_x0
        }
        .clamp(x0, x1);
        let fast_x1 = if has_radius {
            if no_rc { inner_x1 } else { inner_x1.min(rc_x1) }
        } else if no_rc {
            x1
        } else {
            rc_x1
        }
        .clamp(fast_x0, x1);
        for py in y0..y1 {
            let cy = py as f32 + 0.5;
            let row_interior = !has_radius || (py >= inner_y0 && py < inner_y1);
            let row_rc_safe = no_rc || (py >= rc_y0 && py < rc_y1);
            let fy = if use_bilinear {
                (((cy - dst_y) / dst_h) * src_h as f32 - 0.5).clamp(0.0, max_sy as f32)
            } else {
                0.0
            };
            let sy0 = if use_bilinear {
                fy as u32
            } else {
                ((((cy - dst_y) / dst_h).clamp(0.0, 0.9999) * src_h as f32) as u32).min(max_sy)
            };
            let sy1 = (sy0 + 1).min(max_sy);
            let wy = ((fy - sy0 as f32) * 256.0) as u32;
            let iwy = 256 - wy;
            let row0 = sy0 as usize * src_stride;
            let row1 = sy1 as usize * src_stride;
            let dst_row = py as usize * dst_stride;
            let mut fx_cur = fx_base;
            let row_fast_span = use_bilinear
                && opaque_alpha
                && row_interior
                && row_rc_safe
                && fast_x0 < fast_x1
                && row1 + src_stride <= src.len();
            let mut px = x0;
            while px < x1 {
                if row_fast_span && px == fast_x0 {
                    let s0 = &src[row0..row0 + src_stride];
                    let s1 = &src[row1..row1 + src_stride];
                    let dst_slice =
                        &mut self.data[dst_row + fast_x0 as usize * 4..dst_row + fast_x1 as usize * 4];
                    for dst_px in dst_slice.chunks_exact_mut(4) {
                        let fxc = fx_cur.clamp(0, max_fx_fp);
                        fx_cur += fx_step;
                        let sx0 = (fxc >> 16) as usize;
                        let sx1 = (sx0 + 1).min(max_sx as usize);
                        let wx = ((fxc >> 8) & 0xff) as u32;
                        let iwx = 256 - wx;
                        let o0 = sx0 * 4;
                        let o1 = sx1 * 4;
                        let a00 = s0[o0 + 3];
                        let a10 = s0[o1 + 3];
                        let a01 = s1[o0 + 3];
                        let a11 = s1[o1 + 3];
                        if (a00 | a10 | a01 | a11) == 0 {
                            continue;
                        }
                        let w00 = iwx * iwy;
                        let w10 = wx * iwy;
                        let w01 = iwx * wy;
                        let w11 = wx * wy;
                        let r_c = (s0[o0] as u32 * w00
                            + s0[o1] as u32 * w10
                            + s1[o0] as u32 * w01
                            + s1[o1] as u32 * w11
                            + 32768)
                            >> 16;
                        let g_c = (s0[o0 + 1] as u32 * w00
                            + s0[o1 + 1] as u32 * w10
                            + s1[o0 + 1] as u32 * w01
                            + s1[o1 + 1] as u32 * w11
                            + 32768)
                            >> 16;
                        let b_c = (s0[o0 + 2] as u32 * w00
                            + s0[o1 + 2] as u32 * w10
                            + s1[o0 + 2] as u32 * w01
                            + s1[o1 + 2] as u32 * w11
                            + 32768)
                            >> 16;
                        if (a00 & a10 & a01 & a11) == 255 {
                            dst_px[0] = r_c as u8;
                            dst_px[1] = g_c as u8;
                            dst_px[2] = b_c as u8;
                            dst_px[3] = 255;
                        } else {
                            let sa = (a00 as u32 * w00
                                + a10 as u32 * w10
                                + a01 as u32 * w01
                                + a11 as u32 * w11
                                + 32768)
                                >> 16;
                            if sa > 0 {
                                let inv = 255 - sa;
                                dst_px[0] = ((r_c * sa + dst_px[0] as u32 * inv + 127) / 255) as u8;
                                dst_px[1] = ((g_c * sa + dst_px[1] as u32 * inv + 127) / 255) as u8;
                                dst_px[2] = ((b_c * sa + dst_px[2] as u32 * inv + 127) / 255) as u8;
                                dst_px[3] = (sa + (dst_px[3] as u32 * inv + 127) / 255).min(255) as u8;
                            }
                        }
                    }
                    px = fast_x1;
                    continue;
                }
                let fxc = fx_cur.clamp(0, max_fx_fp);
                fx_cur += fx_step;
                let is_inside = !has_radius || (row_interior && px >= inner_x0 && px < inner_x1);
                let cov = if is_inside {
                    1.0
                } else {
                    let cx = px as f32 + 0.5;
                    (0.5 - rounded_rect_sdf(cx, cy, clip_x, clip_y, clip_w, clip_h, r))
                        .clamp(0.0, 1.0)
                };
                if cov <= 0.0 {
                    px += 1;
                    continue;
                }
                let rc_safe = no_rc || (row_rc_safe && px >= rc_x0 && px < rc_x1);
                if use_bilinear {
                    let sx0 = (fxc >> 16) as u32;
                    let sx1 = (sx0 + 1).min(max_sx);
                    let wx = ((fxc >> 8) & 0xff) as u32;
                    let iwx = 256 - wx;
                    let i00 = row0 + sx0 as usize * 4;
                    let i10 = row0 + sx1 as usize * 4;
                    let i01 = row1 + sx0 as usize * 4;
                    let i11 = row1 + sx1 as usize * 4;
                    if i11 + 3 >= src.len() {
                        px += 1;
                        continue;
                    }
                    let a00 = src[i00 + 3];
                    let a10 = src[i10 + 3];
                    let a01 = src[i01 + 3];
                    let a11 = src[i11 + 3];
                    if (a00 | a10 | a01 | a11) == 0 {
                        px += 1;
                        continue;
                    }
                    let w00 = iwx * iwy;
                    let w10 = wx * iwy;
                    let w01 = iwx * wy;
                    let w11 = wx * wy;
                    let r_c = (src[i00] as u32 * w00
                        + src[i10] as u32 * w10
                        + src[i01] as u32 * w01
                        + src[i11] as u32 * w11
                        + 32768)
                        >> 16;
                    let g_c = (src[i00 + 1] as u32 * w00
                        + src[i10 + 1] as u32 * w10
                        + src[i01 + 1] as u32 * w01
                        + src[i11 + 1] as u32 * w11
                        + 32768)
                        >> 16;
                    let b_c = (src[i00 + 2] as u32 * w00
                        + src[i10 + 2] as u32 * w10
                        + src[i01 + 2] as u32 * w01
                        + src[i11 + 2] as u32 * w11
                        + 32768)
                        >> 16;
                    if is_inside && opaque_alpha && rc_safe && (a00 & a10 & a01 & a11) == 255 {
                        let didx = dst_row + px as usize * 4;
                        self.data[didx] = r_c as u8;
                        self.data[didx + 1] = g_c as u8;
                        self.data[didx + 2] = b_c as u8;
                        self.data[didx + 3] = 255;
                    } else {
                        let a = (a00 as u32 * w00
                            + a10 as u32 * w10
                            + a01 as u32 * w01
                            + a11 as u32 * w11
                            + 32768)
                            >> 16;
                        if a == 0 {
                            px += 1;
                            continue;
                        }
                        let c = RgbaColor::new(r_c as u8, g_c as u8, b_c as u8, a as u8)
                            .with_alpha(opacity * cov);
                        self.blend_pixel(px, py, c);
                    }
                } else {
                    let cx = px as f32 + 0.5;
                    let u = ((cx - dst_x) / dst_w).clamp(0.0, 0.9999);
                    let sx = ((u * src_w as f32) as u32).min(max_sx);
                    let sidx = row0 + sx as usize * 4;
                    if sidx + 3 < src.len() {
                        let sa = src[sidx + 3];
                        if sa == 0 {
                            px += 1;
                            continue;
                        }
                        if is_inside && opaque_alpha && rc_safe && sa == 255 {
                            let didx = dst_row + px as usize * 4;
                            self.data[didx] = src[sidx];
                            self.data[didx + 1] = src[sidx + 1];
                            self.data[didx + 2] = src[sidx + 2];
                            self.data[didx + 3] = 255;
                        } else {
                            let c = RgbaColor::new(src[sidx], src[sidx + 1], src[sidx + 2], sa)
                                .with_alpha(opacity * cov);
                            self.blend_pixel(px, py, c);
                        }
                    }
                }
                px += 1;
            }
        }
    }

    pub fn blit_tinted_mask_rgba(
        &mut self,
        src_w: u32,
        src_h: u32,
        src: &[u8],
        dst_x: f32,
        dst_y: f32,
        dst_w: f32,
        dst_h: f32,
        color: RgbaColor,
    ) {
        if src_w == 0 || src_h == 0 || dst_w <= 0.0 || dst_h <= 0.0 || color.a == 0 {
            return;
        }
        let clip = self.current_clip();
        let x0 = (dst_x.floor() as i32).max(clip.x);
        let y0 = (dst_y.floor() as i32).max(clip.y);
        let x1 = ((dst_x + dst_w).ceil() as i32).min(clip.x + clip.w);
        let y1 = ((dst_y + dst_h).ceil() as i32).min(clip.y + clip.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for py in y0..y1 {
            let v = ((py as f32 + 0.5 - dst_y) / dst_h).clamp(0.0, 0.9999);
            let sy = ((v * src_h as f32) as u32).min(src_h - 1);
            for px in x0..x1 {
                let u = ((px as f32 + 0.5 - dst_x) / dst_w).clamp(0.0, 0.9999);
                let sx = ((u * src_w as f32) as u32).min(src_w - 1);
                let sidx = ((sy * src_w + sx) * 4) as usize;
                if sidx + 3 < src.len() {
                    let mask_a = src[sidx + 3];
                    if mask_a == 0 {
                        continue;
                    }
                    let a = ((color.a as u32 * mask_a as u32 + 127) / 255) as u8;
                    self.blend_pixel(px, py, RgbaColor::new(color.r, color.g, color.b, a));
                }
            }
        }
    }

    pub fn blit_alpha_mask(
        &mut self,
        dst_x: i32,
        dst_y: i32,
        mask_w: usize,
        mask_h: usize,
        mask: &[u8],
        color: RgbaColor,
    ) {
        if color.a == 0 || mask_w == 0 || mask_h == 0 {
            return;
        }
        let clip = self.current_clip();
        let x0 = dst_x.max(clip.x);
        let y0 = dst_y.max(clip.y);
        let x1 = (dst_x + mask_w as i32).min(clip.x + clip.w).min(self.width as i32);
        let y1 = (dst_y + mask_h as i32).min(clip.y + clip.h).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        if self.rounded_clips.is_empty() {
            let ca = color.a as u32;
            let cr = color.r as u32;
            let cg = color.g as u32;
            let cb = color.b as u32;
            let stride = self.width as usize * 4;
            for py in y0..y1 {
                let my = (py - dst_y) as usize;
                let mask_row = my * mask_w;
                let dst_row = py as usize * stride;
                for px in x0..x1 {
                    let mx = (px - dst_x) as usize;
                    let cov = mask[mask_row + mx] as u32;
                    if cov == 0 {
                        continue;
                    }
                    let sa = (ca * cov + 127) / 255;
                    if sa == 0 {
                        continue;
                    }
                    let idx = dst_row + px as usize * 4;
                    if sa == 255 {
                        self.data[idx] = color.r;
                        self.data[idx + 1] = color.g;
                        self.data[idx + 2] = color.b;
                        self.data[idx + 3] = 255;
                    } else {
                        let inv = 255 - sa;
                        let dr = self.data[idx] as u32;
                        let dg = self.data[idx + 1] as u32;
                        let db = self.data[idx + 2] as u32;
                        let da = self.data[idx + 3] as u32;
                        self.data[idx] = ((cr * sa + dr * inv + 127) / 255) as u8;
                        self.data[idx + 1] = ((cg * sa + dg * inv + 127) / 255) as u8;
                        self.data[idx + 2] = ((cb * sa + db * inv + 127) / 255) as u8;
                        self.data[idx + 3] = (sa + (da * inv + 127) / 255).min(255) as u8;
                    }
                }
            }
            return;
        }
        for py in y0..y1 {
            let my = (py - dst_y) as usize;
            for px in x0..x1 {
                let mx = (px - dst_x) as usize;
                let cov = mask[my * mask_w + mx];
                if cov == 0 {
                    continue;
                }
                let a = ((color.a as u32 * cov as u32 + 127) / 255) as u8;
                self.blend_pixel(px, py, RgbaColor::new(color.r, color.g, color.b, a));
            }
        }
    }

    pub fn to_xrgb_slice(&self, out: &mut [u32]) {
        let count = (self.width as usize * self.height as usize).min(out.len());
        for (dst, px) in out[..count].iter_mut().zip(self.data.chunks_exact(4)) {
            *dst = ((px[0] as u32) << 16) | ((px[1] as u32) << 8) | (px[2] as u32);
        }
    }

    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(self.width, self.height, self.data.clone())
                .ok_or_else(|| "framebuffer size mismatch".to_string())?;
        img.save(path).map_err(|e| e.to_string())
    }

    pub fn blit_rgba_affine(
        &mut self,
        src_w: u32,
        src_h: u32,
        src: &[u8],
        pivot_src_x: f32,
        pivot_src_y: f32,
        pivot_dst_x: f32,
        pivot_dst_y: f32,
        mat: [f32; 4],
        alpha: f32,
    ) {
        if src_w == 0 || src_h == 0 || alpha <= 0.001 {
            return;
        }
        let [a, b, c, d] = mat;
        let det = a * d - b * c;
        if det.abs() < 1e-6 {
            return;
        }
        let inv_det = 1.0 / det;
        let ia = d * inv_det;
        let ib = -b * inv_det;
        let ic = -c * inv_det;
        let id = a * inv_det;

        let sw = src_w as f32;
        let sh = src_h as f32;
        let corners = [
            (-pivot_src_x, -pivot_src_y),
            (sw - pivot_src_x, -pivot_src_y),
            (-pivot_src_x, sh - pivot_src_y),
            (sw - pivot_src_x, sh - pivot_src_y),
        ];
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for (cx, cy) in corners {
            let tx = pivot_dst_x + a * cx + c * cy;
            let ty = pivot_dst_y + b * cx + d * cy;
            min_x = min_x.min(tx);
            min_y = min_y.min(ty);
            max_x = max_x.max(tx);
            max_y = max_y.max(ty);
        }
        let clip = self.current_clip();
        let x0 = (min_x.floor() as i32).max(clip.x);
        let y0 = (min_y.floor() as i32).max(clip.y);
        let x1 = (max_x.ceil() as i32).min(clip.x + clip.w).min(self.width as i32);
        let y1 = (max_y.ceil() as i32).min(clip.y + clip.h).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }

        for py in y0..y1 {
            let ry = py as f32 + 0.5 - pivot_dst_y;
            for px in x0..x1 {
                let rx = px as f32 + 0.5 - pivot_dst_x;
                let sx = ia * rx + ic * ry + pivot_src_x - 0.5;
                let sy = ib * rx + id * ry + pivot_src_y - 0.5;
                if sx < -0.5 || sy < -0.5 || sx > sw - 0.5 || sy > sh - 0.5 {
                    continue;
                }
                let x_lo = sx.floor() as i32;
                let y_lo = sy.floor() as i32;
                let fx = sx - x_lo as f32;
                let fy = sy - y_lo as f32;
                let sample = |ix: i32, iy: i32| -> [f32; 4] {
                    if ix < 0 || iy < 0 || ix >= src_w as i32 || iy >= src_h as i32 {
                        return [0.0; 4];
                    }
                    let idx = ((iy as u32 * src_w + ix as u32) * 4) as usize;
                    if idx + 3 >= src.len() {
                        return [0.0; 4];
                    }
                    let sa = src[idx + 3] as f32;
                    [
                        src[idx] as f32 * sa,
                        src[idx + 1] as f32 * sa,
                        src[idx + 2] as f32 * sa,
                        sa,
                    ]
                };
                let c00 = sample(x_lo, y_lo);
                let c10 = sample(x_lo + 1, y_lo);
                let c01 = sample(x_lo, y_lo + 1);
                let c11 = sample(x_lo + 1, y_lo + 1);
                let w00 = (1.0 - fx) * (1.0 - fy);
                let w10 = fx * (1.0 - fy);
                let w01 = (1.0 - fx) * fy;
                let w11 = fx * fy;
                let pa = c00[3] * w00 + c10[3] * w10 + c01[3] * w01 + c11[3] * w11;
                if pa <= 0.5 {
                    continue;
                }
                let pr = (c00[0] * w00 + c10[0] * w10 + c01[0] * w01 + c11[0] * w11) / pa;
                let pg = (c00[1] * w00 + c10[1] * w10 + c01[1] * w01 + c11[1] * w11) / pa;
                let pb = (c00[2] * w00 + c10[2] * w10 + c01[2] * w01 + c11[2] * w11) / pa;
                let fa = (pa * alpha).clamp(0.0, 255.0).round() as u8;
                if fa == 0 {
                    continue;
                }
                self.blend_pixel(
                    px,
                    py,
                    RgbaColor::new(
                        pr.clamp(0.0, 255.0).round() as u8,
                        pg.clamp(0.0, 255.0).round() as u8,
                        pb.clamp(0.0, 255.0).round() as u8,
                        fa,
                    ),
                );
            }
        }
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, String> {
        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(self.width, self.height, self.data.clone())
                .ok_or_else(|| "framebuffer size mismatch".to_string())?;
        let mut cursor = std::io::Cursor::new(Vec::new());
        img.write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(cursor.into_inner())
    }
}

fn rounded_rect_sdf(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> f32 {
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let dx = px - cx;
    let dy = py - cy;
    let radius = if dx >= 0.0 {
        if dy >= 0.0 { r[2] } else { r[1] }
    } else if dy >= 0.0 {
        r[3]
    } else {
        r[0]
    };
    let qx = dx.abs() - (w * 0.5 - radius);
    let qy = dy.abs() - (h * 0.5 - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - radius
}

fn sample_gradient(stops: &[(RgbaColor, f32)], t: f32) -> RgbaColor {
    if stops.len() == 1 || t <= stops[0].1 {
        return stops[0].0;
    }
    if t >= stops.last().unwrap().1 {
        return stops.last().unwrap().0;
    }
    for win in stops.windows(2) {
        let (c0, p0) = win[0];
        let (c1, p1) = win[1];
        if t >= p0 && t <= p1 {
            let span = (p1 - p0).max(0.0001);
            let f = ((t - p0) / span).clamp(0.0, 1.0);
            return RgbaColor::new(
                (c0.r as f32 + (c1.r as f32 - c0.r as f32) * f).round() as u8,
                (c0.g as f32 + (c1.g as f32 - c0.g as f32) * f).round() as u8,
                (c0.b as f32 + (c1.b as f32 - c0.b as f32) * f).round() as u8,
                (c0.a as f32 + (c1.a as f32 - c0.a as f32) * f).round() as u8,
            );
        }
    }
    stops.last().unwrap().0
}
