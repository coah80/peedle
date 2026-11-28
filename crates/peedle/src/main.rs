#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chrome;
mod icon;
mod profile;
mod text_edit;

use std::path::Path;

use peedle_engine::Page;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--headless") {
        run_headless(&args);
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--write-ico") {
        if let Some(out) = args.get(pos + 1) {
            if let Err(e) = write_ico_file(Path::new(out)) {
                eprintln!("icon error: {}", e);
                std::process::exit(1);
            }
            return;
        }
    }
    let urls: Vec<String> = args
        .into_iter()
        .filter(|a| !a.starts_with("-psn_") && !a.starts_with('-'))
        .collect();
    let initial_urls = if urls.is_empty() {
        vec!["peedle://newtab".to_string()]
    } else {
        urls
    };
    if let Err(e) = app::BrowserApp::run(initial_urls) {
        eprintln!("peedle error: {}", e);
        std::process::exit(1);
    }
}

fn write_ico_file(path: &Path) -> Result<(), String> {
    let sizes = [16usize, 32, 48, 64, 128, 256];
    let mut pngs: Vec<(u32, Vec<u8>)> = Vec::with_capacity(sizes.len());
    for sz in sizes {
        let rgba = icon::render_icon_rgba(sz);
        let img: image::ImageBuffer<image::Rgba<u8>, _> =
            image::ImageBuffer::from_raw(sz as u32, sz as u32, rgba)
                .ok_or_else(|| "icon buffer error".to_string())?;
        let mut cursor = std::io::Cursor::new(Vec::new());
        img.write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        pngs.push((sz as u32, cursor.into_inner()));
    }
    let count = pngs.len() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6u32 + (pngs.len() as u32) * 16;
    for (sz, data) in &pngs {
        let dim = if *sz >= 256 { 0u8 } else { *sz as u8 };
        out.push(dim);
        out.push(dim);
        out.push(0);
        out.push(0);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &pngs {
        out.extend_from_slice(data);
    }
    std::fs::write(path, out).map_err(|e| e.to_string())
}

fn run_headless(args: &[String]) {
    let mut width = 1280.0f32;
    let mut height = 800.0f32;
    let mut wait_ms = 0u64;
    let mut dark_mode = false;
    let mut eval_code: Option<String> = None;
    let mut screenshot_url: Option<String> = None;
    let mut screenshot_out: Option<String> = None;
    let mut dump_dom_url: Option<String> = None;
    let mut bench_url: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dark" => {
                dark_mode = true;
                i += 1;
            }
            "--width" => {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<f32>().ok()) {
                    width = v;
                }
                i += 2;
            }
            "--height" => {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<f32>().ok()) {
                    height = v;
                }
                i += 2;
            }
            "--wait-ms" => {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<u64>().ok()) {
                    wait_ms = v;
                }
                i += 2;
            }
            "--eval-js" => {
                eval_code = args.get(i + 1).cloned();
                i += 2;
            }
            "--screenshot" => {
                screenshot_url = args.get(i + 1).cloned();
                screenshot_out = args.get(i + 2).cloned();
                i += 3;
            }
            "--dump-dom" => {
                dump_dom_url = args.get(i + 1).cloned();
                i += 2;
            }
            "--bench" => {
                bench_url = args.get(i + 1).cloned();
                i += 2;
            }
            _ => i += 1,
        }
    }

    if let (Some(url), Some(out)) = (screenshot_url, screenshot_out) {
        let mut page = Page::new(width, height);
        if dark_mode {
            page.dark_mode = true;
            page.js_env.dom.borrow_mut().dark_mode = true;
        }
        if let Err(e) = page.navigate(&url) {
            eprintln!("navigation error: {}", e);
        }
        if wait_ms > 0 {
            let steps = (wait_ms / 25).clamp(1, 500);
            let step_ms = (wait_ms / steps).max(1);
            for _ in 0..steps {
                page.js_env.advance_time_ms(step_ms);
            }
            page.recompute_if_needed();
        }
        if let Some(ref code) = eval_code {
            match page.eval_js(code) {
                Ok(val) => println!("JS => {}", val),
                Err(err) => eprintln!("JS error: {}", err),
            }
        }
        if page.has_active_media() {
            page.wait_for_first_video_frame(6000);
        }
        if let Err(e) = page.save_screenshot(Path::new(&out)) {
            eprintln!("screenshot error: {}", e);
            std::process::exit(1);
        }
        println!(
            "Rendered {} ({}x{}, {} boxes, {} net reqs, {} logs) -> {}",
            page.url,
            width as u32,
            height as u32,
            page.layout_box_count(),
            page.network_log.len(),
            page.console_logs.len(),
            out
        );
        return;
    }

    if let Some(url) = bench_url {
        let mut page = Page::new(width, height);
        if dark_mode {
            page.dark_mode = true;
            page.js_env.dom.borrow_mut().dark_mode = true;
        }
        let t0 = std::time::Instant::now();
        let _ = page.navigate(&url);
        let nav_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut fb1 = peedle_render::Framebuffer::new(width as u32, height as u32);
        let t_first = std::time::Instant::now();
        page.render_to_framebuffer_scaled(&mut fb1, 1.0);
        let first_ms = t_first.elapsed().as_secs_f64() * 1000.0;
        let mut scroll1_total = 0.0f64;
        let mut scroll1_max = 0.0f64;
        for step in 0..20 {
            page.scroll_by(0.0, 120.0);
            let ts = std::time::Instant::now();
            fb1.clear(peedle_render::RgbaColor::WHITE);
            page.render_to_framebuffer_scaled(&mut fb1, 1.0);
            let dt = ts.elapsed().as_secs_f64() * 1000.0;
            scroll1_total += dt;
            scroll1_max = scroll1_max.max(dt);
            let _ = step;
        }
        page.scroll_by(0.0, -2400.0);
        let mut fb2 = peedle_render::Framebuffer::new((width * 2.0) as u32, (height * 2.0) as u32);
        let mut scroll2_total = 0.0f64;
        let mut scroll2_max = 0.0f64;
        for _ in 0..20 {
            page.scroll_by(0.0, 120.0);
            let ts = std::time::Instant::now();
            fb2.clear(peedle_render::RgbaColor::WHITE);
            page.render_to_framebuffer_scaled(&mut fb2, 2.0);
            let dt = ts.elapsed().as_secs_f64() * 1000.0;
            scroll2_total += dt;
            scroll2_max = scroll2_max.max(dt);
        }
        page.scroll_by(0.0, -2400.0);
        let mut hover_dirty_count = 0usize;
        let mut hover_total = 0.0f64;
        let mut hover_max = 0.0f64;
        for k in 0..60 {
            let mx = 120.0 + ((k * 37) % 900) as f32;
            let my = 60.0 + ((k * 53) % 680) as f32;
            let tm = std::time::Instant::now();
            let dirty = page.on_mouse_move(mx, my);
            if dirty {
                hover_dirty_count += 1;
                fb2.clear(peedle_render::RgbaColor::WHITE);
                page.render_to_framebuffer_scaled(&mut fb2, 2.0);
            }
            let dt = tm.elapsed().as_secs_f64() * 1000.0;
            hover_total += dt;
            hover_max = hover_max.max(dt);
        }
        let mut xrgb = vec![0u32; (fb2.width * fb2.height) as usize];
        let tx = std::time::Instant::now();
        fb2.to_xrgb_slice(&mut xrgb);
        let xrgb_ms = tx.elapsed().as_secs_f64() * 1000.0;
        println!(
            "BENCH {} ({} boxes): nav={:.1}ms first={:.1}ms scroll1x_avg={:.2}ms(max={:.2}ms) scroll2x_avg={:.2}ms(max={:.2}ms) hover60_avg={:.2}ms(max={:.2}ms, dirty={}) xrgb2x={:.2}ms",
            page.url,
            page.layout_box_count(),
            nav_ms,
            first_ms,
            scroll1_total / 20.0,
            scroll1_max,
            scroll2_total / 20.0,
            scroll2_max,
            hover_total / 60.0,
            hover_max,
            hover_dirty_count,
            xrgb_ms,
        );
        return;
    }

    if let Some(url) = dump_dom_url {
        let mut page = Page::new(width, height);
        let _ = page.navigate(&url);
        if wait_ms > 0 {
            let steps = (wait_ms / 25).clamp(1, 500);
            let step_ms = (wait_ms / steps).max(1);
            for _ in 0..steps {
                page.js_env.advance_time_ms(step_ms);
            }
            page.recompute_if_needed();
        }
        if let Some(ref code) = eval_code {
            match page.eval_js(code) {
                Ok(val) => eprintln!("JS => {}", val),
                Err(err) => eprintln!("JS error: {}", err),
            }
        }
        println!("{}", page.outer_html());
        return;
    }

    eprintln!(
        "usage: peedle --headless --screenshot <url> <out.png> [--width W --height H --wait-ms MS --eval-js CODE]"
    );
    std::process::exit(2);
}
