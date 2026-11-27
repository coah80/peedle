use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use base64::Engine;
use peedle_url::Url;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use sha1::{Digest, Sha1};

use crate::http::HttpError;

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close(u16, String),
}

#[derive(Clone, Debug)]
pub struct WsFrame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

enum WsStream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for WsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            WsStream::Plain(s) => s.read(buf),
            WsStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for WsStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            WsStream::Plain(s) => s.write(buf),
            WsStream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            WsStream::Plain(s) => s.flush(),
            WsStream::Tls(s) => s.flush(),
        }
    }
}

pub struct WebSocketClient {
    reader: BufReader<WsStream>,
}

impl WebSocketClient {
    pub fn connect(url: &Url) -> Result<Self, HttpError> {
        let host = url.hostname();
        let port = url.port_or_default().unwrap_or(80);
        let tcp = TcpStream::connect(format!("{}:{}", host, port))?;

        let mut stream = if url.scheme == "wss" {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let cfg = Arc::new(
                ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            );
            let sn = ServerName::try_from(host.clone()).map_err(|e| HttpError::Tls(e.to_string()))?;
            let conn = ClientConnection::new(cfg, sn).map_err(|e| HttpError::Tls(e.to_string()))?;
            WsStream::Tls(Box::new(StreamOwned::new(conn, tcp)))
        } else {
            WsStream::Plain(tcp)
        };

        let mut nonce = [0u8; 16];
        let _ = getrandom::fill(&mut nonce);
        let key = base64::engine::general_purpose::STANDARD.encode(nonce);

        let mut hasher = Sha1::new();
        hasher.update(key.as_bytes());
        hasher.update(WS_GUID.as_bytes());
        let expected_accept = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());

        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
            url.request_target(),
            url.host_str(),
            key
        );
        stream.write_all(req.as_bytes())?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status)?;
        if !status.contains(" 101 ") {
            return Err(HttpError::Protocol(format!("websocket upgrade failed: {}", status.trim())));
        }
        let mut accept_ok = false;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            if n == 0 || line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(colon) = line.find(':') {
                let k = line[..colon].trim();
                let v = line[colon + 1..].trim();
                if k.eq_ignore_ascii_case("sec-websocket-accept") && v == expected_accept {
                    accept_ok = true;
                }
            }
        }
        if !accept_ok {
            return Err(HttpError::Protocol("invalid sec-websocket-accept".to_string()));
        }
        Ok(Self { reader })
    }

    pub fn send_text(&mut self, text: &str) -> Result<(), HttpError> {
        self.send_frame(0x1, text.as_bytes())
    }

    pub fn send_binary(&mut self, data: &[u8]) -> Result<(), HttpError> {
        self.send_frame(0x2, data)
    }

    pub fn close(&mut self) -> Result<(), HttpError> {
        self.send_frame(0x8, &1000u16.to_be_bytes())
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), HttpError> {
        let mut header = Vec::with_capacity(14);
        header.push(0x80 | (opcode & 0x0F));
        let len = payload.len();
        if len <= 125 {
            header.push(0x80 | len as u8);
        } else if len <= 65535 {
            header.push(0x80 | 126);
            header.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            header.push(0x80 | 127);
            header.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mut mask = [0u8; 4];
        let _ = getrandom::fill(&mut mask);
        header.extend_from_slice(&mask);
        let mut masked = Vec::with_capacity(len);
        for (i, &b) in payload.iter().enumerate() {
            masked.push(b ^ mask[i % 4]);
        }
        let inner = self.reader.get_mut();
        inner.write_all(&header)?;
        inner.write_all(&masked)?;
        inner.flush()?;
        Ok(())
    }

    pub fn read_message(&mut self) -> Result<WsMessage, HttpError> {
        let mut head = [0u8; 2];
        self.reader.read_exact(&mut head)?;
        let opcode = head[0] & 0x0F;
        let masked = (head[1] & 0x80) != 0;
        let len_code = head[1] & 0x7F;
        let payload_len = if len_code <= 125 {
            len_code as usize
        } else if len_code == 126 {
            let mut b = [0u8; 2];
            self.reader.read_exact(&mut b)?;
            u16::from_be_bytes(b) as usize
        } else {
            let mut b = [0u8; 8];
            self.reader.read_exact(&mut b)?;
            u64::from_be_bytes(b) as usize
        };
        let mask = if masked {
            let mut m = [0u8; 4];
            self.reader.read_exact(&mut m)?;
            Some(m)
        } else {
            None
        };
        let mut payload = vec![0u8; payload_len];
        self.reader.read_exact(&mut payload)?;
        if let Some(m) = mask {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= m[i % 4];
            }
        }
        match opcode {
            0x1 => Ok(WsMessage::Text(String::from_utf8_lossy(&payload).into_owned())),
            0x2 => Ok(WsMessage::Binary(payload)),
            0x8 => {
                let code = if payload.len() >= 2 {
                    u16::from_be_bytes([payload[0], payload[1]])
                } else {
                    1005
                };
                let reason = if payload.len() > 2 {
                    String::from_utf8_lossy(&payload[2..]).into_owned()
                } else {
                    String::new()
                };
                Ok(WsMessage::Close(code, reason))
            }
            0x9 => Ok(WsMessage::Ping(payload)),
            0xA => Ok(WsMessage::Pong(payload)),
            other => Err(HttpError::Protocol(format!("unknown ws opcode {}", other))),
        }
    }
}
