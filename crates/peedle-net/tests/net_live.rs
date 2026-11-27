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

#[test]
fn http11_keep_alive_connection_reuse_and_gzip() {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        for idx in 0..2 {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            assert!(n > 0, "expected request {}", idx);
            if idx == 0 {
                let body = b"first-keepalive";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    body.len()
                );
                stream.write_all(resp.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
                stream.flush().unwrap();
            } else {
                let mut enc = GzEncoder::new(Vec::new(), Compression::default());
                enc.write_all(b"second-gzipped-reused").unwrap();
                let gz = enc.finish().unwrap();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    gz.len()
                );
                stream.write_all(resp.as_bytes()).unwrap();
                stream.write_all(&gz).unwrap();
                stream.flush().unwrap();
            }
        }
    });

    let client = HttpClient::new();
    let r1 = client.get(&format!("http://127.0.0.1:{}/one", port)).unwrap();
    let r2 = client.get(&format!("http://127.0.0.1:{}/two", port)).unwrap();
    server.join().unwrap();
    assert_eq!(r1.text(), "first-keepalive");
    assert_eq!(r2.text(), "second-gzipped-reused");
}

#[test]
fn websocket_rfc6455_handshake_text_binary_close() {
    use base64::Engine;
    use peedle_net::{WebSocketClient, WsMessage};
    use peedle_url::Url;
    use sha1::{Digest, Sha1};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap();
        let req = String::from_utf8_lossy(&buf[..n]);
        let key = req
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                if k.trim().eq_ignore_ascii_case("sec-websocket-key") {
                    Some(v.trim().to_string())
                } else {
                    None
                }
            })
            .unwrap();
        let mut hasher = Sha1::new();
        hasher.update(key.as_bytes());
        hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
        let accept = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
        let resp = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            accept
        );
        stream.write_all(resp.as_bytes()).unwrap();
        stream.flush().unwrap();

        let read_client_frame = |s: &mut std::net::TcpStream| -> (u8, Vec<u8>) {
            let mut h = [0u8; 2];
            s.read_exact(&mut h).unwrap();
            let op = h[0] & 0x0F;
            let len = (h[1] & 0x7F) as usize;
            let mut mask = [0u8; 4];
            s.read_exact(&mut mask).unwrap();
            let mut payload = vec![0u8; len];
            s.read_exact(&mut payload).unwrap();
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
            (op, payload)
        };

        let (op1, p1) = read_client_frame(&mut stream);
        assert_eq!(op1, 0x1);
        assert_eq!(String::from_utf8(p1).unwrap(), "ping-peedle");
        let reply1 = b"pong-peedle";
        stream.write_all(&[0x81, reply1.len() as u8]).unwrap();
        stream.write_all(reply1).unwrap();

        let (op2, p2) = read_client_frame(&mut stream);
        assert_eq!(op2, 0x2);
        assert_eq!(p2, vec![1, 2, 3, 4]);
        stream.write_all(&[0x82, 4, 4, 3, 2, 1]).unwrap();

        let (op3, _) = read_client_frame(&mut stream);
        assert_eq!(op3, 0x8);
        stream.write_all(&[0x88, 2, 0x03, 0xE8]).unwrap();
        stream.flush().unwrap();
    });

    let url = Url::parse(&format!("ws://127.0.0.1:{}/chat", port)).unwrap();
    let mut ws = WebSocketClient::connect(&url).unwrap();
    ws.send_text("ping-peedle").unwrap();
    assert_eq!(ws.read_message().unwrap(), WsMessage::Text("pong-peedle".to_string()));
    ws.send_binary(&[1, 2, 3, 4]).unwrap();
    assert_eq!(ws.read_message().unwrap(), WsMessage::Binary(vec![4, 3, 2, 1]));
    ws.close().unwrap();
    assert_eq!(ws.read_message().unwrap(), WsMessage::Close(1000, String::new()));
    server.join().unwrap();
}

#[test]
fn http11_keep_alive_chunked_reuse() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_srv = accepts.clone();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        accepts_srv.fetch_add(1, Ordering::SeqCst);
        for body in ["first-chunked-html", "body { color: red; }"] {
            let mut req_buf = [0u8; 2048];
            let n = stream.read(&mut req_buf).unwrap();
            assert!(n > 0);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).unwrap();
            stream.flush().unwrap();
        }
    });

    let client = HttpClient::new();
    let r1 = client.get(&format!("http://127.0.0.1:{}/index.html", port)).unwrap();
    let r2 = client.get(&format!("http://127.0.0.1:{}/styles.css", port)).unwrap();
    assert_eq!(r1.status, 200);
    assert_eq!(r1.text(), "first-chunked-html");
    assert_eq!(r2.status, 200);
    assert_eq!(r2.text(), "body { color: red; }");
    assert_eq!(accepts.load(Ordering::SeqCst), 1);
    server.join().unwrap();
}
