use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use brotli_decompressor::Decompressor as BrotliDecoder;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use peedle_url::percent::percent_decode;
use peedle_url::Url;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::cookies::CookieJar;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Patch,
    Options,
    Custom(String),
}

impl HttpMethod {
    pub fn as_str(&self) -> &str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Head => "HEAD",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Options => "OPTIONS",
            HttpMethod::Custom(s) => s.as_str(),
        }
    }

    pub fn from_str_method(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "GET" => HttpMethod::Get,
            "HEAD" => HttpMethod::Head,
            "POST" => HttpMethod::Post,
            "PUT" => HttpMethod::Put,
            "DELETE" => HttpMethod::Delete,
            "PATCH" => HttpMethod::Patch,
            "OPTIONS" => HttpMethod::Options,
            other => HttpMethod::Custom(other.to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub max_redirects: usize,
}

impl HttpRequest {
    pub fn get(url: Url) -> Self {
        Self {
            method: HttpMethod::Get,
            url,
            headers: Vec::new(),
            body: None,
            max_redirects: 10,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub url: Url,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn content_type(&self) -> &str {
        self.header("content-type").unwrap_or("application/octet-stream")
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Debug)]
pub enum HttpError {
    Io(std::io::Error),
    Tls(String),
    Protocol(String),
    TooManyRedirects,
    UnsupportedScheme(String),
    InvalidUrl(String),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Io(e) => write!(f, "io error: {}", e),
            HttpError::Tls(s) => write!(f, "tls error: {}", s),
            HttpError::Protocol(s) => write!(f, "protocol error: {}", s),
            HttpError::TooManyRedirects => write!(f, "too many redirects"),
            HttpError::UnsupportedScheme(s) => write!(f, "unsupported scheme: {}", s),
            HttpError::InvalidUrl(s) => write!(f, "invalid url: {}", s),
        }
    }
}

impl std::error::Error for HttpError {}

impl From<std::io::Error> for HttpError {
    fn from(e: std::io::Error) -> Self {
        HttpError::Io(e)
    }
}

enum Transport {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Transport::Plain(s) => s.read(buf),
            Transport::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Transport::Plain(s) => s.write(buf),
            Transport::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Transport::Plain(s) => s.flush(),
            Transport::Tls(s) => s.flush(),
        }
    }
}

#[derive(Clone)]
pub struct HttpClient {
    tls_config: Arc<ClientConfig>,
    pub cookies: Arc<Mutex<CookieJar>>,
    pub user_agent: String,
    pub timeout: Duration,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    pub fn new() -> Self {
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        Self {
            tls_config: Arc::new(config),
            cookies: Arc::new(Mutex::new(CookieJar::new())),
            user_agent: "Mozilla/5.0 (X11; Linux x86_64) Peedle/0.1".to_string(),
            timeout: Duration::from_secs(20),
        }
    }

    pub fn get(&self, url_str: &str) -> Result<HttpResponse, HttpError> {
        let url = Url::parse(url_str).map_err(|e| HttpError::InvalidUrl(e.to_string()))?;
        self.send(HttpRequest::get(url))
    }

    pub fn send(&self, mut req: HttpRequest) -> Result<HttpResponse, HttpError> {
        let mut redirects = 0usize;
        loop {
            match req.url.scheme.as_str() {
                "data" => return parse_data_url(&req.url),
                "file" => return read_file_url(&req.url),
                "about" => {
                    return Ok(HttpResponse {
                        url: req.url,
                        status: 200,
                        status_text: "OK".to_string(),
                        headers: vec![("content-type".to_string(), "text/html; charset=utf-8".to_string())],
                        body: Vec::new(),
                    });
                }
                "http" | "https" => {}
                other => return Err(HttpError::UnsupportedScheme(other.to_string())),
            }

            let resp = self.send_once(&req)?;
            if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                if let Some(loc) = resp.header("location") {
                    if redirects >= req.max_redirects {
                        return Err(HttpError::TooManyRedirects);
                    }
                    let next_url = req
                        .url
                        .join(loc)
                        .map_err(|e| HttpError::InvalidUrl(e.to_string()))?;
                    if resp.status == 303
                        || (matches!(resp.status, 301 | 302) && req.method == HttpMethod::Post)
                    {
                        req.method = HttpMethod::Get;
                        req.body = None;
                    }
                    req.url = next_url;
                    redirects += 1;
                    continue;
                }
            }
            return Ok(resp);
        }
    }

    fn send_once(&self, req: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let host = req.url.hostname();
        if host.is_empty() {
            return Err(HttpError::InvalidUrl("missing host".to_string()));
        }
        let port = req.url.port_or_default().unwrap_or(80);
        let addr = format!("{}:{}", host, port);
        let tcp = TcpStream::connect(&addr)?;
        tcp.set_read_timeout(Some(self.timeout))?;
        tcp.set_write_timeout(Some(self.timeout))?;

        let mut transport = if req.url.scheme == "https" {
            let server_name = ServerName::try_from(host.clone())
                .map_err(|e| HttpError::Tls(e.to_string()))?;
            let conn = ClientConnection::new(self.tls_config.clone(), server_name)
                .map_err(|e| HttpError::Tls(e.to_string()))?;
            Transport::Tls(Box::new(StreamOwned::new(conn, tcp)))
        } else {
            Transport::Plain(tcp)
        };

        let target = req.url.request_target();
        let host_header = req.url.host_str();

        let mut wire = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            req.method.as_str(),
            target,
            host_header
        );

        let has_ua = req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("user-agent"));
        if !has_ua {
            wire.push_str(&format!("User-Agent: {}\r\n", self.user_agent));
        }
        let has_ae = req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept-encoding"));
        if !has_ae {
            wire.push_str("Accept-Encoding: gzip, deflate, br\r\n");
        }
        let has_accept = req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept"));
        if !has_accept {
            wire.push_str("Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\n");
        }

        let cookie_header = self
            .cookies
            .lock()
            .unwrap()
            .cookie_header_for(&req.url, true);
        if !cookie_header.is_empty()
            && !req.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("cookie"))
        {
            wire.push_str(&format!("Cookie: {}\r\n", cookie_header));
        }

        for (k, v) in &req.headers {
            if !k.eq_ignore_ascii_case("host") && !k.eq_ignore_ascii_case("connection") {
                wire.push_str(&format!("{}: {}\r\n", k, v));
            }
        }
        if let Some(ref body) = req.body {
            if !req
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            {
                wire.push_str(&format!("Content-Length: {}\r\n", body.len()));
            }
        }
        wire.push_str("\r\n");

        transport.write_all(wire.as_bytes())?;
        if let Some(ref body) = req.body {
            transport.write_all(body)?;
        }
        transport.flush()?;

        let mut reader = BufReader::new(transport);
        let mut status_line = String::new();
        reader.read_line(&mut status_line)?;
        let status_trimmed = status_line.trim_end_matches(['\r', '\n']);
        let mut parts = status_trimmed.splitn(3, ' ');
        let _version = parts
            .next()
            .ok_or_else(|| HttpError::Protocol("missing http version".to_string()))?;
        let status: u16 = parts
            .next()
            .ok_or_else(|| HttpError::Protocol("missing status code".to_string()))?
            .parse()
            .map_err(|_| HttpError::Protocol("invalid status code".to_string()))?;
        let status_text = parts.next().unwrap_or("").to_string();

        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(colon) = trimmed.find(':') {
                let name = trimmed[..colon].trim().to_ascii_lowercase();
                let value = trimmed[colon + 1..].trim().to_string();
                if name == "set-cookie" {
                    self.cookies
                        .lock()
                        .unwrap()
                        .set_from_header(&req.url, &value);
                }
                headers.push((name, value));
            }
        }

        let is_chunked = headers.iter().any(|(k, v)| {
            k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked")
        });
        let content_length = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .and_then(|(_, v)| v.parse::<usize>().ok());

        let raw_body = if req.method == HttpMethod::Head || status == 204 || status == 304 {
            Vec::new()
        } else if is_chunked {
            read_chunked(&mut reader)?
        } else if let Some(len) = content_length {
            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf)?;
            buf
        } else {
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            buf
        };

        let content_encoding = headers
            .iter()
            .find(|(k, _)| k == "content-encoding")
            .map(|(_, v)| v.to_ascii_lowercase());

        let body = match content_encoding.as_deref() {
            Some("gzip") | Some("x-gzip") => {
                let mut dec = GzDecoder::new(&raw_body[..]);
                let mut out = Vec::new();
                dec.read_to_end(&mut out).unwrap_or(0);
                if out.is_empty() && !raw_body.is_empty() {
                    raw_body
                } else {
                    out
                }
            }
            Some("deflate") => {
                let mut out = Vec::new();
                let ok = ZlibDecoder::new(&raw_body[..]).read_to_end(&mut out).is_ok();
                if !ok {
                    out.clear();
                    let _ = DeflateDecoder::new(&raw_body[..]).read_to_end(&mut out);
                }
                if out.is_empty() && !raw_body.is_empty() {
                    raw_body
                } else {
                    out
                }
            }
            Some("br") => {
                let mut dec = BrotliDecoder::new(&raw_body[..], 4096);
                let mut out = Vec::new();
                dec.read_to_end(&mut out).unwrap_or(0);
                if out.is_empty() && !raw_body.is_empty() {
                    raw_body
                } else {
                    out
                }
            }
            _ => raw_body,
        };

        Ok(HttpResponse {
            url: req.url.clone(),
            status,
            status_text,
            headers,
            body,
        })
    }
}

fn read_chunked<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, HttpError> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        let n = reader.read_line(&mut size_line)?;
        if n == 0 {
            break;
        }
        let hex = size_line
            .trim()
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        if hex.is_empty() {
            continue;
        }
        let chunk_len = usize::from_str_radix(hex, 16)
            .map_err(|_| HttpError::Protocol("invalid chunk size".to_string()))?;
        if chunk_len == 0 {
            break;
        }
        let mut chunk = vec![0u8; chunk_len];
        reader.read_exact(&mut chunk)?;
        out.extend_from_slice(&chunk);
        let mut crlf = [0u8; 2];
        let _ = reader.read_exact(&mut crlf);
    }
    Ok(out)
}

pub fn parse_data_url(url: &Url) -> Result<HttpResponse, HttpError> {
    let href = url.href();
    let without_frag = href.split('#').next().unwrap_or(&href);
    let fallback = url.pathname();
    let raw = without_frag
        .strip_prefix("data:")
        .or_else(|| without_frag.strip_prefix("DATA:"))
        .unwrap_or(&fallback);
    let Some(comma) = raw.find(',') else {
        return Err(HttpError::InvalidUrl("data url missing comma".to_string()));
    };
    let meta = raw[..comma].trim();
    let payload = &raw[comma + 1..];
    let lower_meta = meta.to_ascii_lowercase();
    let is_base64 = lower_meta.ends_with(";base64") || lower_meta == "base64";
    let mime = if is_base64 {
        let trim_len = if lower_meta.ends_with(";base64") {
            meta.len() - 7
        } else {
            0
        };
        meta[..trim_len].trim()
    } else {
        meta
    };
    let content_type = if mime.is_empty() {
        "text/plain;charset=US-ASCII".to_string()
    } else {
        mime.to_string()
    };
    let decoded_pct = percent_decode(payload.as_bytes());
    let body = if is_base64 {
        let mut cleaned: Vec<u8> = decoded_pct
            .into_iter()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        while cleaned.len() % 4 != 0 {
            cleaned.push(b'=');
        }
        base64::engine::general_purpose::STANDARD
            .decode(&cleaned)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(cleaned.trim_ascii_end()))
            .unwrap_or_else(|_| percent_decode(payload.as_bytes()))
    } else {
        decoded_pct
    };
    Ok(HttpResponse {
        url: url.clone(),
        status: 200,
        status_text: "OK".to_string(),
        headers: vec![("content-type".to_string(), content_type)],
        body,
    })
}

fn read_file_url(url: &Url) -> Result<HttpResponse, HttpError> {
    let path_str = String::from_utf8_lossy(&percent_decode(url.pathname().as_bytes())).into_owned();
    let bytes = std::fs::read(&path_str)?;
    let ext = std::path::Path::new(&path_str)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    Ok(HttpResponse {
        url: url.clone(),
        status: 200,
        status_text: "OK".to_string(),
        headers: vec![("content-type".to_string(), mime.to_string())],
        body: bytes,
    })
}
