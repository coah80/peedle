use winit::window::Icon;

pub fn build_window_icon() -> Option<Icon> {
    let size = 64usize;
    let rgba = render_icon_rgba(size);
    Icon::from_rgba(rgba, size as u32, size as u32).ok()
}

pub fn render_icon_rgba(size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size * size * 4];
    let inv = 1.0 / size as f32;
    let aa = 1.4 * inv;
    for py in 0..size {
        let ny = (py as f32 + 0.5) * inv * 2.0 - 1.0;
        for px in 0..size {
            let nx = (px as f32 + 0.5) * inv * 2.0 - 1.0;
            let d_bg = sdf_squircle(nx, ny, 0.88, 0.38);
            let cov = clamp01(0.5 - d_bg / aa);
            if cov <= 0.0 {
                continue;
            }
            let grad_t = clamp01((ny + 0.88) / 1.76);
            let mut r = mix(15.0, 28.0, grad_t);
            let mut g = mix(23.0, 40.0, grad_t);
            let mut b = mix(42.0, 62.0, grad_t);

            let glow = (-((nx + 0.15).powi(2) + (ny + 0.35).powi(2)) * 3.2).exp();
            r = mix(r, 56.0, glow * 0.22);
            g = mix(g, 189.0, glow * 0.26);
            b = mix(b, 248.0, glow * 0.32);

            let rim = clamp01(1.0 - (d_bg + 0.014).abs() / (aa * 1.4));
            r = mix(r, 148.0, rim * 0.28);
            g = mix(g, 185.0, rim * 0.34);
            b = mix(b, 225.0, rim * 0.42);

            let ring_d = (nx.hypot(ny) - 0.56).abs() - 0.034;
            let ring_cov = clamp01(0.5 - ring_d / aa);
            if ring_cov > 0.0 {
                r = mix(r, 56.0, ring_cov * 0.32);
                g = mix(g, 189.0, ring_cov * 0.32);
                b = mix(b, 248.0, ring_cov * 0.32);
            }

            let stem = sdf_rrect(nx, ny, -0.18, 0.02, 0.095, 0.38, 0.045);
            let bowl_outer = sdf_rrect(nx, ny, 0.06, -0.11, 0.27, 0.25, 0.22);
            let bowl_inner = sdf_rrect(nx, ny, 0.07, -0.11, 0.12, 0.11, 0.095);
            let bowl = bowl_outer.max(-bowl_inner);
            let p_cov = clamp01(0.5 - stem.min(bowl) / aa);
            if p_cov > 0.0 {
                let pt = clamp01((nx + ny + 0.8) / 1.6);
                r = mix(r, mix(240.0, 56.0, pt), p_cov);
                g = mix(g, mix(249.0, 189.0, pt), p_cov);
                b = mix(b, mix(255.0, 248.0, pt), p_cov);
            }

            let dot_d = (nx - 0.26).hypot(ny - 0.27) - 0.085;
            let dot_cov = clamp01(0.5 - dot_d / aa);
            if dot_cov > 0.0 {
                r = mix(r, 74.0, dot_cov);
                g = mix(g, 222.0, dot_cov);
                b = mix(b, 128.0, dot_cov);
            }

            let idx = (py * size + px) * 4;
            buf[idx] = r.clamp(0.0, 255.0) as u8;
            buf[idx + 1] = g.clamp(0.0, 255.0) as u8;
            buf[idx + 2] = b.clamp(0.0, 255.0) as u8;
            buf[idx + 3] = (cov * 255.0).clamp(0.0, 255.0) as u8;
        }
    }
    buf
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn sdf_squircle(x: f32, y: f32, half: f32, r: f32) -> f32 {
    let ax = x.abs() - (half - r);
    let ay = y.abs() - (half - r);
    let n = 2.6f32;
    let ox = ax.max(0.0);
    let oy = ay.max(0.0);
    let outside = if ox > 0.0 || oy > 0.0 {
        (ox.powf(n) + oy.powf(n)).powf(1.0 / n)
    } else {
        0.0
    };
    let inside = ax.max(ay).min(0.0);
    outside + inside - r
}

fn sdf_rrect(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let dx = (x - cx).abs() - (hw - r);
    let dy = (y - cy).abs() - (hh - r);
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r
}
