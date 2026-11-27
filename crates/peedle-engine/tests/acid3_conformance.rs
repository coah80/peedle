use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use peedle_engine::Page;

fn mime_for_acid_file(name: &str) -> &'static str {
    match name {
        "acid3.html" | "empty.html" | "reference.html" | "support-b.png" => "text/html; charset=utf-8",
        "empty.css" => "text/html; charset=utf-8",
        "empty.txt" => "text/plain; charset=utf-8",
        "empty.png" | "test.html" | "support-c.png" => "image/png",
        "empty.xml" => "application/xml; charset=utf-8",
        "svg.xml" => "image/svg+xml; charset=utf-8",
        "xhtml.1" | "xhtml.2" | "xhtml.3" => "application/xhtml+xml; charset=utf-8",
        "font.ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn spawn_acid_server(dir: PathBuf) -> (String, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();

    thread::spawn(move || {
        while !stop_flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let raw_path = req
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let clean = raw_path.split('?').next().unwrap_or("/").trim_start_matches('/');
                    let file_path = dir.join(clean);
                    if !clean.is_empty() && !clean.contains("..") && file_path.is_file() {
                        if let Ok(bytes) = std::fs::read(&file_path) {
                            let ct = mime_for_acid_file(clean);
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                ct,
                                bytes.len()
                            );
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(&bytes);
                            let _ = stream.flush();
                            continue;
                        }
                    }
                    let resp = "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found";
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://{}/acid3.html", addr), stop)
}

#[test]
fn acid3_conformance() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/acid")
        .canonicalize()
        .unwrap();
    let (url, stop) = spawn_acid_server(dir);

    let mut page = Page::new(800.0, 600.0);
    page.navigate(&url).unwrap();

    for _ in 0..600 {
        let idx: usize = page
            .eval_js("index")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if idx >= 100 {
            break;
        }
        page.js_env.advance_time_ms(15);
    }
    page.recompute_if_needed();

    let score: usize = page
        .eval_js("score")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let idx: usize = page
        .eval_js("index")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let log = page.eval_js("log").unwrap_or_default();
    stop.store(true, Ordering::Relaxed);

    let out_png = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../screenshots/acid3_100.png");
    let _ = page.save_screenshot(&out_png);

    eprintln!("=== ACID3 RESULT: {}/100 (index={}) ===", score, idx);
    eprintln!("{}", log);
    assert_eq!(score, 100, "Acid3 score must be 100/100 (index={})\n{}", idx, log);
}
