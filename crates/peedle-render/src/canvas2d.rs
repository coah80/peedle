use crate::fonts::{FontFaceKey, FontSystem};
use crate::framebuffer::{Affine2D, Framebuffer, RgbaColor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasTextAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasTextBaseline {
    Alphabetic,
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Debug)]
pub struct CanvasState {
    pub fill_style: RgbaColor,
    pub stroke_style: RgbaColor,
    pub line_width: f32,
    pub global_alpha: f32,
    pub font_size: f32,
    pub font_key: FontFaceKey,
    pub text_align: CanvasTextAlign,
    pub text_baseline: CanvasTextBaseline,
    pub transform: Affine2D,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            fill_style: RgbaColor::BLACK,
            stroke_style: RgbaColor::BLACK,
            line_width: 1.0,
            global_alpha: 1.0,
            font_size: 10.0,
            font_key: FontFaceKey::SansRegular,
            text_align: CanvasTextAlign::Left,
            text_baseline: CanvasTextBaseline::Alphabetic,
            transform: Affine2D::identity(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Canvas2D {
    pub fb: Framebuffer,
    pub state: CanvasState,
    stack: Vec<CanvasState>,
    subpaths: Vec<Vec<(f32, f32)>>,
    current: Vec<(f32, f32)>,
}

impl Canvas2D {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            fb: Framebuffer::new(width.max(1), height.max(1)),
            state: CanvasState::default(),
            stack: Vec::new(),
            subpaths: Vec::new(),
            current: Vec::new(),
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.fb = Framebuffer::new(width.max(1), height.max(1));
        self.state = CanvasState::default();
        self.stack.clear();
        self.begin_path();
    }

    pub fn save(&mut self) {
        self.stack.push(self.state.clone());
    }

    pub fn restore(&mut self) {
        if let Some(prev) = self.stack.pop() {
            self.state = prev;
        }
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        self.state.transform = self.state.transform.mul(Affine2D::translate(x, y));
    }

    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.state.transform = self.state.transform.mul(Affine2D::scale(sx, sy));
    }

    pub fn rotate(&mut self, rad: f32) {
        self.state.transform = self.state.transform.mul(Affine2D::rotate(rad));
    }

    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.state.transform = Affine2D { a, b, c, d, e, f };
    }

    pub fn reset_transform(&mut self) {
        self.state.transform = Affine2D::identity();
    }

    pub fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let (x0, y0) = self.state.transform.apply(x, y);
        let (x1, y1) = self.state.transform.apply(x + w, y + h);
        let rx0 = (x0.min(x1).floor() as i32).max(0);
        let ry0 = (y0.min(y1).floor() as i32).max(0);
        let rx1 = (x0.max(x1).ceil() as i32).min(self.fb.width as i32);
        let ry1 = (y0.max(y1).ceil() as i32).min(self.fb.height as i32);
        for py in ry0..ry1 {
            for px in rx0..rx1 {
                let idx = ((py as u32 * self.fb.width + px as u32) * 4) as usize;
                self.fb.data[idx] = 0;
                self.fb.data[idx + 1] = 0;
                self.fb.data[idx + 2] = 0;
                self.fb.data[idx + 3] = 0;
            }
        }
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let pts = vec![
            self.state.transform.apply(x, y),
            self.state.transform.apply(x + w, y),
            self.state.transform.apply(x + w, y + h),
            self.state.transform.apply(x, y + h),
        ];
        let color = self.state.fill_style.with_alpha(self.state.global_alpha);
        self.fb.fill_polygon(&[pts], color, false);
    }

    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let p0 = self.state.transform.apply(x, y);
        let p1 = self.state.transform.apply(x + w, y);
        let p2 = self.state.transform.apply(x + w, y + h);
        let p3 = self.state.transform.apply(x, y + h);
        let color = self.state.stroke_style.with_alpha(self.state.global_alpha);
        let lw = self.state.line_width;
        self.fb.draw_line(p0.0, p0.1, p1.0, p1.1, lw, color);
        self.fb.draw_line(p1.0, p1.1, p2.0, p2.1, lw, color);
        self.fb.draw_line(p2.0, p2.1, p3.0, p3.1, lw, color);
        self.fb.draw_line(p3.0, p3.1, p0.0, p0.1, lw, color);
    }

    pub fn begin_path(&mut self) {
        self.subpaths.clear();
        self.current.clear();
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        if !self.current.is_empty() {
            self.subpaths.push(std::mem::take(&mut self.current));
        }
        self.current.push(self.state.transform.apply(x, y));
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        self.current.push(self.state.transform.apply(x, y));
    }

    pub fn close_path(&mut self) {
        if let Some(&first) = self.current.first() {
            if self.current.last() != Some(&first) {
                self.current.push(first);
            }
            self.subpaths.push(std::mem::take(&mut self.current));
        }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close_path();
    }

    pub fn round_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let r = radius.max(0.0).min(w.abs() * 0.5).min(h.abs() * 0.5);
        if r <= 0.01 {
            self.rect(x, y, w, h);
            return;
        }
        let k = 0.552_284_8 * r;
        self.move_to(x + r, y);
        self.line_to(x + w - r, y);
        self.bezier_curve_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
        self.line_to(x + w, y + h - r);
        self.bezier_curve_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
        self.line_to(x + r, y + h);
        self.bezier_curve_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
        self.line_to(x, y + r);
        self.bezier_curve_to(x, y + r - k, x + r - k, y, x + r, y);
        self.close_path();
    }

    pub fn ellipse(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        rotation: f32,
        start: f32,
        end: f32,
        ccw: bool,
    ) {
        let mut sweep = end - start;
        if ccw {
            if sweep >= 0.0 {
                sweep = (sweep % std::f32::consts::TAU) - std::f32::consts::TAU;
            }
        } else if sweep <= 0.0 {
            sweep = (sweep % std::f32::consts::TAU) + std::f32::consts::TAU;
        }
        if sweep.abs() >= std::f32::consts::TAU {
            sweep = if ccw {
                -std::f32::consts::TAU
            } else {
                std::f32::consts::TAU
            };
        }
        let cos_r = rotation.cos();
        let sin_r = rotation.sin();
        let steps = ((sweep.abs() * rx.abs().max(ry.abs())).clamp(16.0, 128.0)) as usize;
        for i in 0..=steps {
            let a = start + sweep * (i as f32 / steps as f32);
            let lx = rx * a.cos();
            let ly = ry * a.sin();
            let px = cx + lx * cos_r - ly * sin_r;
            let py = cy + lx * sin_r + ly * cos_r;
            let pt = self.state.transform.apply(px, py);
            self.current.push(pt);
        }
    }

    pub fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32, ccw: bool) {
        let mut sweep = end - start;
        if ccw {
            if sweep >= 0.0 {
                sweep = (sweep % std::f32::consts::TAU) - std::f32::consts::TAU;
            }
        } else if sweep <= 0.0 {
            sweep = (sweep % std::f32::consts::TAU) + std::f32::consts::TAU;
        }
        if sweep.abs() >= std::f32::consts::TAU {
            sweep = if ccw {
                -std::f32::consts::TAU
            } else {
                std::f32::consts::TAU
            };
        }
        let steps = ((sweep.abs() * r.abs()).clamp(12.0, 128.0)) as usize;
        for i in 0..=steps {
            let a = start + sweep * (i as f32 / steps as f32);
            let px = cx + r * a.cos();
            let py = cy + r * a.sin();
            let pt = self.state.transform.apply(px, py);
            self.current.push(pt);
        }
    }

    pub fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) {
        let p0 = self.current.last().copied().unwrap_or((0.0, 0.0));
        let p1 = self.state.transform.apply(cpx, cpy);
        let p2 = self.state.transform.apply(x, y);
        let steps = 20usize;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let inv = 1.0 - t;
            let px = inv * inv * p0.0 + 2.0 * inv * t * p1.0 + t * t * p2.0;
            let py = inv * inv * p0.1 + 2.0 * inv * t * p1.1 + t * t * p2.1;
            self.current.push((px, py));
        }
    }

    pub fn bezier_curve_to(
        &mut self,
        cp1x: f32,
        cp1y: f32,
        cp2x: f32,
        cp2y: f32,
        x: f32,
        y: f32,
    ) {
        let p0 = self.current.last().copied().unwrap_or((0.0, 0.0));
        let p1 = self.state.transform.apply(cp1x, cp1y);
        let p2 = self.state.transform.apply(cp2x, cp2y);
        let p3 = self.state.transform.apply(x, y);
        let steps = 24usize;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let inv = 1.0 - t;
            let px = inv.powi(3) * p0.0
                + 3.0 * inv.powi(2) * t * p1.0
                + 3.0 * inv * t.powi(2) * p2.0
                + t.powi(3) * p3.0;
            let py = inv.powi(3) * p0.1
                + 3.0 * inv.powi(2) * t * p1.1
                + 3.0 * inv * t.powi(2) * p2.1
                + t.powi(3) * p3.1;
            self.current.push((px, py));
        }
    }

    fn all_contours(&self) -> Vec<Vec<(f32, f32)>> {
        let mut all = self.subpaths.clone();
        if !self.current.is_empty() {
            all.push(self.current.clone());
        }
        all
    }

    pub fn fill(&mut self, even_odd: bool) {
        let contours = self.all_contours();
        let color = self.state.fill_style.with_alpha(self.state.global_alpha);
        self.fb.fill_polygon(&contours, color, even_odd);
    }

    pub fn stroke(&mut self) {
        let contours = self.all_contours();
        let color = self.state.stroke_style.with_alpha(self.state.global_alpha);
        let lw = self.state.line_width;
        for poly in contours {
            for win in poly.windows(2) {
                self.fb
                    .draw_line(win[0].0, win[0].1, win[1].0, win[1].1, lw, color);
            }
        }
    }

    pub fn fill_text(&mut self, fonts: &FontSystem, text: &str, x: f32, y: f32) {
        let (mut tx, mut ty) = self.state.transform.apply(x, y);
        let tw = fonts.measure_text(text, self.state.font_key, self.state.font_size);
        match self.state.text_align {
            CanvasTextAlign::Left => {}
            CanvasTextAlign::Center => tx -= tw * 0.5,
            CanvasTextAlign::Right => tx -= tw,
        }
        match self.state.text_baseline {
            CanvasTextBaseline::Alphabetic => {}
            CanvasTextBaseline::Top => ty += self.state.font_size * 0.8,
            CanvasTextBaseline::Middle => ty += self.state.font_size * 0.35,
            CanvasTextBaseline::Bottom => ty -= self.state.font_size * 0.2,
        }
        let color = self.state.fill_style.with_alpha(self.state.global_alpha);
        fonts.draw_text(
            &mut self.fb,
            text,
            tx,
            ty,
            self.state.font_key,
            self.state.font_size,
            color,
        );
    }

    pub fn get_image_data(&self, sx: i32, sy: i32, sw: u32, sh: u32) -> Vec<u8> {
        let mut out = vec![0u8; (sw * sh * 4) as usize];
        for y in 0..sh {
            let src_y = sy + y as i32;
            if src_y < 0 || src_y >= self.fb.height as i32 {
                continue;
            }
            for x in 0..sw {
                let src_x = sx + x as i32;
                if src_x < 0 || src_x >= self.fb.width as i32 {
                    continue;
                }
                let sidx = ((src_y as u32 * self.fb.width + src_x as u32) * 4) as usize;
                let didx = ((y * sw + x) * 4) as usize;
                out[didx..didx + 4].copy_from_slice(&self.fb.data[sidx..sidx + 4]);
            }
        }
        out
    }

    pub fn put_image_data(&mut self, data: &[u8], w: u32, h: u32, dx: i32, dy: i32) {
        for y in 0..h {
            let dst_y = dy + y as i32;
            if dst_y < 0 || dst_y >= self.fb.height as i32 {
                continue;
            }
            for x in 0..w {
                let dst_x = dx + x as i32;
                if dst_x < 0 || dst_x >= self.fb.width as i32 {
                    continue;
                }
                let sidx = ((y * w + x) * 4) as usize;
                let didx = ((dst_y as u32 * self.fb.width + dst_x as u32) * 4) as usize;
                if sidx + 4 <= data.len() && didx + 4 <= self.fb.data.len() {
                    self.fb.data[didx..didx + 4].copy_from_slice(&data[sidx..sidx + 4]);
                }
            }
        }
    }
}
