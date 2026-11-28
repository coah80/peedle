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
    let mut eval_code: Option<String> = None;
    let mut screenshot_url: Option<String> = None;
    let mut screenshot_out: Option<String> = None;
    let mut dump_dom_url: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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
            _ => i += 1,
        }
    }

    if let (Some(url), Some(out)) = (screenshot_url, screenshot_out) {
        let mut page = Page::new(width, height);
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
