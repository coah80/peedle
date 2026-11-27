use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use peedle_net::HttpClient;

#[test]
fn data_url_and_cookies() {
    let client = HttpClient::new();
    let r1 = client.get("data:text/plain;base64,SGVsbG8gUGVlZGxlIQ==").unwrap();
    assert_eq!(r1.status, 200);
    assert_eq!(r1.text(), "Hello Peedle!");

    let r2 = client.get("data:text/html,<h1>Hi%20There</h1>").unwrap();
    assert_eq!(r2.content_type(), "text/html");
    assert_eq!(r2.text(), "<h1>Hi There</h1>");

    let r3 = client
        .get("data:text/javascript;base64,%5a%44%4d%67%50%53%41%6e%64%47%68%79%5a%57%55%6e%4f%77%3D%3D")
        .unwrap();
    assert_eq!(r3.text(), "d3 = 'three';");

    let r4 = client
        .get("data:text/javascript;base64,%20ZD%20Qg%0D%0APS%20An%20Zm91cic%0D%0A%207%20")
        .unwrap();
    assert_eq!(r4.text(), "d4 = 'four';");
}

#[test]
fn http11_chunked_redirect_and_cookies() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            if req.starts_with("GET /start ") {
                let resp = "HTTP/1.1 302 Found\r\nLocation: /final\r\nSet-Cookie: sid=abc123; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(resp.as_bytes());
            } else {
                let has_cookie = req.contains("Cookie: sid=abc123");
                let body = if has_cookie { "chunked-ok!" } else { "no-cookie" };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        }
    });

    let client = HttpClient::new();
    let resp = client.get(&format!("http://127.0.0.1:{}/start", port)).unwrap();
    server.join().unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.url.pathname(), "/final");
    assert_eq!(resp.text(), "chunked-ok!");
}

#[test]
fn fetch_https_example_com() {
    let client = HttpClient::new();
    let resp = client.get("https://example.com").unwrap();
    assert_eq!(resp.status, 200);
    let body = resp.text();
    assert!(body.contains("Example Domain"), "unexpected body: {}", &body[..body.len().min(200)]);
}
