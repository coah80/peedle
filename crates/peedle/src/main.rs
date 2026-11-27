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
        if let Err(e) = page.save_screenshot(Path::new(&out)) {
            eprintln!("screenshot error: {}", e);
            std::process::exit(1);
        }
        println!(
            "Rendered {} ({}x{}, {} boxes, {} net reqs) -> {}",
            page.url,
            width as u32,
            height as u32,
            page.layout.boxes.len(),
            page.network_log.len(),
            out
        );
        return;
    }

    if let Some(url) = dump_dom_url {
        let mut page = Page::new(width, height);
        let _ = page.navigate(&url);
        println!("{}", page.document.outer_html(page.document.root));
        return;
    }

    eprintln!("usage: peedle --headless --screenshot <url> <out.png> [--width W --height H]");
    std::process::exit(2);
}
