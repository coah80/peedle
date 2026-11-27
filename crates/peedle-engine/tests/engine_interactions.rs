use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use peedle_engine::Page;

#[test]
fn details_summary_toggle_and_form_get_submission() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let first_line = req.lines().next().unwrap_or("");
            let body = if first_line.contains("/search?q=rust+browser") {
                "<!DOCTYPE html><html><head><title>Results</title></head><body><h1 id='ok'>Found: rust browser</h1></body></html>"
            } else {
                r#"<!DOCTYPE html><html><head><title>Form Page</title><style>
                    body { margin: 0; }
                    summary { display: block; height: 30px; }
                    #hidden-body { height: 40px; }
                    input { display: block; width: 200px; height: 30px; }
                    button { display: block; width: 100px; height: 30px; }
                </style></head><body>
                    <details id='det'><summary id='sum'>More</summary><p id='hidden-body'>Secret</p></details>
                    <form action='/search' method='get'>
                        <input id='q' name='q' value='rust browser'>
                        <button id='go' type='submit'>Go</button>
                    </form>
                </body></html>"#
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    let mut page = Page::new(800.0, 600.0);
    page.navigate(&format!("http://127.0.0.1:{}/", port)).unwrap();

    let is_open_before = page.eval_js("document.getElementById('det').hasAttribute('open')").unwrap();
    assert_eq!(is_open_before, "false");
    let hidden_before = page
        .eval_js("getComputedStyle(document.getElementById('hidden-body')).display")
        .unwrap();
    assert_eq!(hidden_before, "none");

    page.on_click(20.0, 15.0);
    let is_open_after = page.eval_js("document.getElementById('det').hasAttribute('open')").unwrap();
    assert_eq!(is_open_after, "true");
    let hidden_after = page
        .eval_js("getComputedStyle(document.getElementById('hidden-body')).display")
        .unwrap();
    assert_eq!(hidden_after, "block");

    let btn_rect = {
        let st = page.js_env.dom.borrow();
        let bid = st.doc.get_element_by_id("go").unwrap();
        st.layout.bounding_rect(bid).unwrap()
    };
    let nav = page.on_click(btn_rect.x + 10.0, btn_rect.y + 10.0);
    server.join().unwrap();
    assert!(nav.is_some());
    assert_eq!(page.title(), "Results");
    let h1 = page.eval_js("document.getElementById('ok').textContent").unwrap();
    assert_eq!(h1, "Found: rust browser");
}
