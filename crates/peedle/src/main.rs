mod app;
mod chrome;

use std::path::Path;

use peedle_engine::Page;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--headless") {
        run_headless(&args);
        return;
    }
    let url = args
        .first()
        .cloned()
        .unwrap_or_else(|| "peedle://newtab".to_string());
    if let Err(e) = app::BrowserApp::run(url) {
        eprintln!("peedle error: {}", e);
        std::process::exit(1);
    }
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
