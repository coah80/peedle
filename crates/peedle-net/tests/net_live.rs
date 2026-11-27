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
}

#[test]
fn fetch_https_example_com() {
    let client = HttpClient::new();
    let resp = client.get("https://example.com").unwrap();
    assert_eq!(resp.status, 200);
    let body = resp.text();
    assert!(body.contains("Example Domain"), "unexpected body: {}", &body[..body.len().min(200)]);
}
