use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::idna::domain_to_ascii;
use crate::params::UrlSearchParams;
use crate::percent::{
    percent_decode_str, utf8_percent_encode, utf8_percent_encode_char, EncodeSet,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlError {
    InvalidScheme,
    MissingHost,
    InvalidHost,
    InvalidPort,
    InvalidIpv4,
    InvalidIpv6,
    RelativeWithoutBase,
    InvalidSpecialRelative,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for UrlError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
    Domain(String),
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
    Opaque(String),
    Empty,
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Domain(d) => f.write_str(d),
            Host::Ipv4(ip) => write!(f, "{}", ip),
            Host::Ipv6(ip) => write!(f, "[{}]", format_ipv6(*ip)),
            Host::Opaque(s) => f.write_str(s),
            Host::Empty => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlPath {
    Opaque(String),
    Segments(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub username: String,
    pub password: String,
    pub host: Option<Host>,
    pub port: Option<u16>,
    pub path: UrlPath,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

pub fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "ftp" => Some(21),
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

pub fn is_special_scheme(scheme: &str) -> bool {
    matches!(scheme, "ftp" | "file" | "http" | "https" | "ws" | "wss")
}

fn is_c0_or_space(c: char) -> bool {
    c <= '\u{0020}'
}

fn is_ascii_tab_or_newline(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r')
}

fn is_windows_drive_letter(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && (b[1] == b':' || b[1] == b'|')
}

fn is_normalized_windows_drive_letter(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn starts_with_windows_drive_letter(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 2 {
        return false;
    }
    if !b[0].is_ascii_alphabetic() || (b[1] != b':' && b[1] != b'|') {
        return false;
    }
    if b.len() == 2 {
        return true;
    }
    matches!(b[2], b'/' | b'\\' | b'?' | b'#')
}

fn is_single_dot_segment(s: &str) -> bool {
    s == "." || s.eq_ignore_ascii_case("%2e")
}

fn is_double_dot_segment(s: &str) -> bool {
    s == ".."
        || s.eq_ignore_ascii_case(".%2e")
        || s.eq_ignore_ascii_case("%2e.")
        || s.eq_ignore_ascii_case("%2e%2e")
}

fn is_forbidden_host_char(c: char) -> bool {
    matches!(
        c,
        '\0' | '\t' | '\n' | '\r' | ' ' | '#' | '/' | ':' | '<' | '>' | '?' | '@' | '[' | '\\' | ']' | '^' | '|'
    )
}

fn is_forbidden_domain_char(c: char) -> bool {
    is_forbidden_host_char(c) || c <= '\u{001F}' || c == '%' || c == '\u{007F}'
}

fn format_ipv6(ip: Ipv6Addr) -> String {
    let segs = ip.segments();
    let mut best_start = None;
    let mut best_len = 0usize;
    let mut cur_start = None;
    let mut cur_len = 0usize;
    for (i, &s) in segs.iter().enumerate() {
        if s == 0 {
            if cur_start.is_none() {
                cur_start = Some(i);
                cur_len = 1;
            } else {
                cur_len += 1;
            }
            if cur_len > best_len && cur_len > 1 {
                best_start = cur_start;
                best_len = cur_len;
            }
        } else {
            cur_start = None;
            cur_len = 0;
        }
    }
    let mut out = String::new();
    let mut i = 0;
    while i < 8 {
        if best_start == Some(i) {
            out.push_str("::");
            i += best_len;
            continue;
        }
        if i > 0 && best_start.map_or(true, |st| i != st + best_len) {
            out.push(':');
        }
        out.push_str(&format!("{:x}", segs[i]));
        i += 1;
    }
    out
}

fn parse_ipv4_number(part: &str) -> Result<u64, UrlError> {
    if part.is_empty() {
        return Err(UrlError::InvalidIpv4);
    }
    let (radix, digits) = if part.len() >= 2 && (part.starts_with("0x") || part.starts_with("0X")) {
        (16, &part[2..])
    } else if part.len() >= 2 && part.starts_with('0') {
        (8, &part[1..])
    } else {
        (10, part)
    };
    if digits.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(digits, radix).map_err(|_| UrlError::InvalidIpv4)
}

fn ends_in_number(input: &str) -> bool {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") {
        if parts.len() == 1 {
            return false;
        }
        parts.pop();
    }
    let Some(&last) = parts.last() else {
        return false;
    };
    if !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    if last.len() >= 2
        && (last.starts_with("0x") || last.starts_with("0X"))
        && last[2..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return true;
    }
    parse_ipv4_number(last).is_ok()
}

fn parse_ipv4(input: &str) -> Result<Ipv4Addr, UrlError> {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") {
        if parts.len() > 1 {
            parts.pop();
        }
    }
    if parts.len() > 4 {
        return Err(UrlError::InvalidIpv4);
    }
    let mut numbers = Vec::with_capacity(parts.len());
    for part in parts {
        let n = parse_ipv4_number(part)?;
        numbers.push(n);
    }
    for &n in &numbers[..numbers.len() - 1] {
        if n > 255 {
            return Err(UrlError::InvalidIpv4);
        }
    }
    let last = *numbers.last().unwrap();
    let max_last = 256u64.pow((5 - numbers.len()) as u32);
    if last >= max_last {
        return Err(UrlError::InvalidIpv4);
    }
    let mut ipv4 = last;
    for (idx, &n) in numbers[..numbers.len() - 1].iter().enumerate() {
        ipv4 += n * 256u64.pow((3 - idx) as u32);
    }
    Ok(Ipv4Addr::from(ipv4 as u32))
}

fn parse_ipv6(input: &str) -> Result<Ipv6Addr, UrlError> {
    let bytes = input.as_bytes();
    let mut address = [0u16; 8];
    let mut piece_index = 0usize;
    let mut compress: Option<usize> = None;
    let mut i = 0usize;

    if i < bytes.len() && bytes[i] == b':' {
        if i + 1 >= bytes.len() || bytes[i + 1] != b':' {
            return Err(UrlError::InvalidIpv6);
        }
        i += 2;
        piece_index += 1;
        compress = Some(piece_index);
    }

    while i < bytes.len() {
        if piece_index == 8 {
            return Err(UrlError::InvalidIpv6);
        }
        if bytes[i] == b':' {
            if compress.is_some() {
                return Err(UrlError::InvalidIpv6);
            }
            i += 1;
            piece_index += 1;
            compress = Some(piece_index);
            continue;
        }
        let mut value: u16 = 0;
        let mut length = 0usize;
        while length < 4 && i < bytes.len() && bytes[i].is_ascii_hexdigit() {
            let d = match bytes[i] {
                b'0'..=b'9' => (bytes[i] - b'0') as u16,
                b'a'..=b'f' => (bytes[i] - b'a' + 10) as u16,
                b'A'..=b'F' => (bytes[i] - b'A' + 10) as u16,
                _ => unreachable!(),
            };
            value = value * 16 + d;
            i += 1;
            length += 1;
        }
        if i < bytes.len() && bytes[i] == b'.' {
            if length == 0 {
                return Err(UrlError::InvalidIpv6);
            }
            i -= length;
            if piece_index > 6 {
                return Err(UrlError::InvalidIpv6);
            }
            let mut numbers_seen = 0usize;
            while i < bytes.len() {
                let mut ipv4_piece: Option<u32> = None;
                if numbers_seen > 0 {
                    if bytes[i] == b'.' && numbers_seen < 4 {
                        i += 1;
                    } else {
                        return Err(UrlError::InvalidIpv6);
                    }
                }
                if i >= bytes.len() || !bytes[i].is_ascii_digit() {
                    return Err(UrlError::InvalidIpv6);
                }
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    let number = (bytes[i] - b'0') as u32;
                    ipv4_piece = match ipv4_piece {
                        None => Some(number),
                        Some(0) => return Err(UrlError::InvalidIpv6),
                        Some(v) => Some(v * 10 + number),
                    };
                    if ipv4_piece.unwrap() > 255 {
                        return Err(UrlError::InvalidIpv6);
                    }
                    i += 1;
                }
                address[piece_index] = address[piece_index] * 256 + ipv4_piece.unwrap() as u16;
                numbers_seen += 1;
                if numbers_seen == 2 || numbers_seen == 4 {
                    piece_index += 1;
                }
            }
            if numbers_seen != 4 {
                return Err(UrlError::InvalidIpv6);
            }
            break;
        } else if i < bytes.len() && bytes[i] == b':' {
            i += 1;
            if i == bytes.len() {
                return Err(UrlError::InvalidIpv6);
            }
        } else if i < bytes.len() {
            return Err(UrlError::InvalidIpv6);
        }
        address[piece_index] = value;
        piece_index += 1;
    }

    if let Some(mut c) = compress {
        let mut swaps = piece_index - c;
        piece_index = 7;
        while piece_index != 0 && swaps > 0 {
            address.swap(piece_index, c + swaps - 1);
            piece_index -= 1;
            swaps -= 1;
        }
        let _ = &mut c;
    } else if piece_index != 8 {
        return Err(UrlError::InvalidIpv6);
    }

    Ok(Ipv6Addr::from(address))
}

fn parse_opaque_host(input: &str) -> Result<Host, UrlError> {
    if input.chars().any(is_forbidden_host_char) {
        return Err(UrlError::InvalidHost);
    }
    Ok(Host::Opaque(utf8_percent_encode(input, EncodeSet::C0Control)))
}

pub fn parse_host(input: &str, is_opaque: bool) -> Result<Host, UrlError> {
    if input.starts_with('[') {
        if !input.ends_with(']') {
            return Err(UrlError::InvalidIpv6);
        }
        let ip = parse_ipv6(&input[1..input.len() - 1])?;
        return Ok(Host::Ipv6(ip));
    }
    if is_opaque {
        return parse_opaque_host(input);
    }
    let decoded = percent_decode_str(input);
    let ascii_domain = domain_to_ascii(&decoded, false).map_err(|_| UrlError::InvalidHost)?;
    if ascii_domain.is_empty() {
        return Err(UrlError::InvalidHost);
    }
    if ascii_domain.chars().any(is_forbidden_domain_char) {
        return Err(UrlError::InvalidHost);
    }
    if ends_in_number(&ascii_domain) {
        let ip = parse_ipv4(&ascii_domain)?;
        return Ok(Host::Ipv4(ip));
    }
    Ok(Host::Domain(ascii_domain))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    SchemeStart,
    Scheme,
    NoScheme,
    SpecialRelativeOrAuthority,
    PathOrAuthority,
    Relative,
    RelativeSlash,
    SpecialAuthoritySlashes,
    SpecialAuthorityIgnoreSlashes,
    Authority,
    Host,
    Hostname,
    Port,
    File,
    FileSlash,
    FileHost,
    PathStart,
    Path,
    OpaquePath,
    Query,
    Fragment,
}

impl Url {
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        Self::parse_with_base(input, None)
    }

    pub fn parse_with_base(input: &str, base: Option<&Url>) -> Result<Self, UrlError> {
        let mut url = Url {
            scheme: String::new(),
            username: String::new(),
            password: String::new(),
            host: None,
            port: None,
            path: UrlPath::Segments(Vec::new()),
            query: None,
            fragment: None,
        };
        url.basic_parse_into(input, base, None)?;
        Ok(url)
    }

    pub(crate) fn includes_credentials(&self) -> bool {
        !self.username.is_empty() || !self.password.is_empty()
    }

    pub(crate) fn has_empty_or_null_host(&self) -> bool {
        matches!(self.host, None | Some(Host::Empty))
            || matches!(&self.host, Some(Host::Opaque(s) | Host::Domain(s)) if s.is_empty())
    }

    pub(crate) fn cannot_have_username_password_or_port(&self) -> bool {
        self.has_empty_or_null_host() || self.scheme == "file"
    }

    pub(crate) fn basic_parse_into(
        &mut self,
        input: &str,
        base: Option<&Url>,
        state_override: Option<State>,
    ) -> Result<(), UrlError> {
        let trimmed = if state_override.is_none() {
            input.trim_matches(is_c0_or_space)
        } else {
            input
        };
        let cleaned: String = trimmed
            .chars()
            .filter(|&c| !is_ascii_tab_or_newline(c))
            .collect();
        let chars: Vec<char> = cleaned.chars().collect();
        let len = chars.len();

        let url = self;
        let mut state = state_override.unwrap_or(State::SchemeStart);
        let mut buffer = String::new();
        let mut at_sign_seen = false;
        let mut inside_brackets = false;
        let mut password_token_seen = false;
        let mut idx: isize = 0;

        loop {
            let prev_idx = idx;
            let prev_state = state;
            let c: Option<char> = if idx >= 0 && (idx as usize) < len {
                Some(chars[idx as usize])
            } else {
                None
            };

            match state {
                State::SchemeStart => {
                    if let Some(ch) = c {
                        if ch.is_ascii_alphabetic() {
                            buffer.push(ch.to_ascii_lowercase());
                            state = State::Scheme;
                        } else if state_override.is_none() {
                            state = State::NoScheme;
                            idx -= 1;
                        } else {
                            return Err(UrlError::InvalidScheme);
                        }
                    } else if state_override.is_none() {
                        state = State::NoScheme;
                        idx -= 1;
                    } else {
                        return Err(UrlError::InvalidScheme);
                    }
                }
                State::Scheme => {
                    if let Some(ch) = c {
                        if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.') {
                            buffer.push(ch.to_ascii_lowercase());
                        } else if ch == ':' {
                            if state_override.is_some() {
                                let was_special = is_special_scheme(&url.scheme);
                                let now_special = is_special_scheme(&buffer);
                                if was_special != now_special {
                                    return Ok(());
                                }
                                if (url.includes_credentials() || url.port.is_some())
                                    && buffer == "file"
                                {
                                    return Ok(());
                                }
                                if url.scheme == "file" && url.has_empty_or_null_host() {
                                    return Ok(());
                                }
                            }
                            url.scheme = std::mem::take(&mut buffer);
                            if state_override.is_some() {
                                if let Some(p) = url.port {
                                    if default_port(&url.scheme) == Some(p) {
                                        url.port = None;
                                    }
                                }
                                return Ok(());
                            }
                            if url.scheme == "file" {
                                state = State::File;
                            } else if is_special_scheme(&url.scheme) {
                                if let Some(b) = base {
                                    if b.scheme == url.scheme {
                                        state = State::SpecialRelativeOrAuthority;
                                    } else {
                                        state = State::SpecialAuthoritySlashes;
                                    }
                                } else {
                                    state = State::SpecialAuthoritySlashes;
                                }
                            } else if idx as usize + 1 < len && chars[idx as usize + 1] == '/' {
                                state = State::PathOrAuthority;
                                idx += 1;
                            } else {
                                url.path = UrlPath::Opaque(String::new());
                                state = State::OpaquePath;
                            }
                        } else if state_override.is_none() {
                            buffer.clear();
                            state = State::NoScheme;
                            idx = -1;
                        } else {
                            return Err(UrlError::InvalidScheme);
                        }
                    } else if state_override.is_none() {
                        buffer.clear();
                        state = State::NoScheme;
                        idx = -1;
                    } else {
                        return Err(UrlError::InvalidScheme);
                    }
                }
                State::NoScheme => {
                    let Some(b) = base else {
                        return Err(UrlError::RelativeWithoutBase);
                    };
                    if matches!(b.path, UrlPath::Opaque(_)) {
                        if c != Some('#') {
                            return Err(UrlError::RelativeWithoutBase);
                        }
                        url.scheme = b.scheme.clone();
                        url.path = b.path.clone();
                        url.query = b.query.clone();
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    } else if b.scheme != "file" {
                        state = State::Relative;
                        idx -= 1;
                    } else {
                        state = State::File;
                        idx -= 1;
                    }
                }
                State::SpecialRelativeOrAuthority => {
                    if c == Some('/') && idx as usize + 1 < len && chars[idx as usize + 1] == '/' {
                        state = State::SpecialAuthorityIgnoreSlashes;
                        idx += 1;
                    } else {
                        state = State::Relative;
                        idx -= 1;
                    }
                }
                State::PathOrAuthority => {
                    if c == Some('/') {
                        state = State::Authority;
                    } else {
                        state = State::Path;
                        idx -= 1;
                    }
                }
                State::Relative => {
                    let b = base.ok_or(UrlError::RelativeWithoutBase)?;
                    url.scheme = b.scheme.clone();
                    if c == Some('/') {
                        state = State::RelativeSlash;
                    } else if is_special_scheme(&url.scheme) && c == Some('\\') {
                        state = State::RelativeSlash;
                    } else {
                        url.username = b.username.clone();
                        url.password = b.password.clone();
                        url.host = b.host.clone();
                        url.port = b.port;
                        url.path = b.path.clone();
                        url.query = b.query.clone();
                        if c == Some('?') {
                            url.query = Some(String::new());
                            state = State::Query;
                        } else if c == Some('#') {
                            url.fragment = Some(String::new());
                            state = State::Fragment;
                        } else if c.is_some() {
                            url.query = None;
                            url.shorten_path();
                            state = State::Path;
                            idx -= 1;
                        }
                    }
                }
                State::RelativeSlash => {
                    let b = base.ok_or(UrlError::RelativeWithoutBase)?;
                    if is_special_scheme(&url.scheme) && matches!(c, Some('/') | Some('\\')) {
                        state = State::SpecialAuthorityIgnoreSlashes;
                    } else if c == Some('/') {
                        state = State::Authority;
                    } else {
                        url.username = b.username.clone();
                        url.password = b.password.clone();
                        url.host = b.host.clone();
                        url.port = b.port;
                        state = State::Path;
                        idx -= 1;
                    }
                }
                State::SpecialAuthoritySlashes => {
                    if c == Some('/') && idx as usize + 1 < len && chars[idx as usize + 1] == '/' {
                        state = State::SpecialAuthorityIgnoreSlashes;
                        idx += 1;
                    } else {
                        state = State::SpecialAuthorityIgnoreSlashes;
                        idx -= 1;
                    }
                }
                State::SpecialAuthorityIgnoreSlashes => {
                    if !matches!(c, Some('/') | Some('\\')) {
                        state = State::Authority;
                        idx -= 1;
                    }
                }
                State::Authority => {
                    if c == Some('@') {
                        if at_sign_seen {
                            buffer.insert_str(0, "%40");
                        }
                        at_sign_seen = true;
                        for cp in buffer.chars() {
                            if cp == ':' && !password_token_seen {
                                password_token_seen = true;
                                continue;
                            }
                            if password_token_seen {
                                utf8_percent_encode_char(cp, EncodeSet::UserInfo, &mut url.password);
                            } else {
                                utf8_percent_encode_char(cp, EncodeSet::UserInfo, &mut url.username);
                            }
                        }
                        buffer.clear();
                    } else if matches!(c, None | Some('/') | Some('?') | Some('#'))
                        || (is_special_scheme(&url.scheme) && c == Some('\\'))
                    {
                        if at_sign_seen && buffer.is_empty() {
                            return Err(UrlError::MissingHost);
                        }
                        let backtrack = buffer.chars().count() as isize + 1;
                        idx -= backtrack;
                        buffer.clear();
                        state = State::Host;
                    } else if let Some(ch) = c {
                        buffer.push(ch);
                    }
                }
                State::Host | State::Hostname => {
                    if state_override.is_some() && url.scheme == "file" {
                        idx -= 1;
                        state = State::FileHost;
                    } else if c == Some(':') && !inside_brackets {
                        if buffer.is_empty() {
                            return Err(UrlError::MissingHost);
                        }
                        if state_override == Some(State::Hostname) {
                            return Err(UrlError::InvalidHost);
                        }
                        let host = parse_host(&buffer, !is_special_scheme(&url.scheme))?;
                        url.host = Some(host);
                        buffer.clear();
                        state = State::Port;
                    } else if matches!(c, None | Some('/') | Some('?') | Some('#'))
                        || (is_special_scheme(&url.scheme) && c == Some('\\'))
                    {
                        idx -= 1;
                        if is_special_scheme(&url.scheme) && buffer.is_empty() {
                            return Err(UrlError::MissingHost);
                        }
                        if state_override.is_some()
                            && buffer.is_empty()
                            && (url.includes_credentials() || url.port.is_some())
                        {
                            return Ok(());
                        }
                        let host = parse_host(&buffer, !is_special_scheme(&url.scheme))?;
                        url.host = Some(host);
                        buffer.clear();
                        state = State::PathStart;
                        if state_override.is_some() {
                            return Ok(());
                        }
                    } else if let Some(ch) = c {
                        if ch == '[' {
                            inside_brackets = true;
                        } else if ch == ']' {
                            inside_brackets = false;
                        }
                        buffer.push(ch);
                    }
                }
                State::Port => {
                    if let Some(ch) = c.filter(|ch| ch.is_ascii_digit()) {
                        buffer.push(ch);
                    } else if matches!(c, None | Some('/') | Some('?') | Some('#'))
                        || (is_special_scheme(&url.scheme) && c == Some('\\'))
                        || state_override.is_some()
                    {
                        if !buffer.is_empty() {
                            let port = buffer.parse::<u16>().map_err(|_| UrlError::InvalidPort)?;
                            url.port = if default_port(&url.scheme) == Some(port) {
                                None
                            } else {
                                Some(port)
                            };
                            buffer.clear();
                            if state_override.is_some() {
                                return Ok(());
                            }
                        }
                        if state_override.is_some() {
                            return Ok(());
                        }
                        state = State::PathStart;
                        idx -= 1;
                    } else {
                        return Err(UrlError::InvalidPort);
                    }
                }
                State::File => {
                    url.scheme = "file".to_string();
                    url.host = Some(Host::Empty);
                    if matches!(c, Some('/') | Some('\\')) {
                        state = State::FileSlash;
                    } else if let Some(b) = base.filter(|b| b.scheme == "file") {
                        url.host = b.host.clone();
                        url.path = b.path.clone();
                        url.query = b.query.clone();
                        if c == Some('?') {
                            url.query = Some(String::new());
                            state = State::Query;
                        } else if c == Some('#') {
                            url.fragment = Some(String::new());
                            state = State::Fragment;
                        } else if c.is_some() {
                            url.query = None;
                            let rem: String = chars[idx as usize..].iter().collect();
                            if !starts_with_windows_drive_letter(&rem) {
                                url.shorten_path();
                            } else {
                                url.path = UrlPath::Segments(Vec::new());
                            }
                            state = State::Path;
                            idx -= 1;
                        }
                    } else {
                        state = State::Path;
                        idx -= 1;
                    }
                }
                State::FileSlash => {
                    if matches!(c, Some('/') | Some('\\')) {
                        state = State::FileHost;
                    } else {
                        if let Some(b) = base.filter(|b| b.scheme == "file") {
                            url.host = b.host.clone();
                            let rem: String = if idx >= 0 && (idx as usize) < len {
                                chars[idx as usize..].iter().collect()
                            } else {
                                String::new()
                            };
                            if !starts_with_windows_drive_letter(&rem) {
                                if let UrlPath::Segments(segs) = &b.path {
                                    if let Some(first) = segs.first() {
                                        if is_normalized_windows_drive_letter(first) {
                                            url.push_segment(first.clone());
                                        }
                                    }
                                }
                            }
                        }
                        state = State::Path;
                        idx -= 1;
                    }
                }
                State::FileHost => {
                    if matches!(c, None | Some('/') | Some('\\') | Some('?') | Some('#')) {
                        idx -= 1;
                        if state_override.is_none() && is_windows_drive_letter(&buffer) {
                            state = State::Path;
                        } else if buffer.is_empty() {
                            url.host = Some(Host::Empty);
                            if state_override.is_some() {
                                return Ok(());
                            }
                            state = State::PathStart;
                        } else {
                            let mut host = parse_host(&buffer, false)?;
                            if host == Host::Domain("localhost".to_string()) {
                                host = Host::Empty;
                            }
                            url.host = Some(host);
                            if state_override.is_some() {
                                return Ok(());
                            }
                            buffer.clear();
                            state = State::PathStart;
                        }
                    } else if let Some(ch) = c {
                        buffer.push(ch);
                    }
                }
                State::PathStart => {
                    if is_special_scheme(&url.scheme) {
                        state = State::Path;
                        if !matches!(c, Some('/') | Some('\\')) {
                            idx -= 1;
                        }
                    } else if state_override.is_none() && c == Some('?') {
                        url.query = Some(String::new());
                        state = State::Query;
                    } else if state_override.is_none() && c == Some('#') {
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    } else if c.is_some() {
                        state = State::Path;
                        if c != Some('/') {
                            idx -= 1;
                        }
                    } else if state_override.is_some() && url.host.is_none() {
                        url.push_segment(String::new());
                    }
                }
                State::Path => {
                    if matches!(c, None | Some('/'))
                        || (is_special_scheme(&url.scheme) && c == Some('\\'))
                        || (state_override.is_none() && matches!(c, Some('?') | Some('#')))
                    {
                        if is_double_dot_segment(&buffer) {
                            url.shorten_path();
                            if c != Some('/') && !(is_special_scheme(&url.scheme) && c == Some('\\')) {
                                url.push_segment(String::new());
                            }
                        } else if is_single_dot_segment(&buffer) {
                            if c != Some('/') && !(is_special_scheme(&url.scheme) && c == Some('\\')) {
                                url.push_segment(String::new());
                            }
                        } else {
                            if url.scheme == "file"
                                && url.segments_is_empty()
                                && is_windows_drive_letter(&buffer)
                            {
                                let b = buffer.as_bytes();
                                buffer = format!("{}:", b[0] as char);
                            }
                            url.push_segment(std::mem::take(&mut buffer));
                        }
                        buffer.clear();
                        if state_override.is_none() && c == Some('?') {
                            url.query = Some(String::new());
                            state = State::Query;
                        } else if state_override.is_none() && c == Some('#') {
                            url.fragment = Some(String::new());
                            state = State::Fragment;
                        }
                    } else if let Some(ch) = c {
                        utf8_percent_encode_char(ch, EncodeSet::Path, &mut buffer);
                    }
                }
                State::OpaquePath => {
                    if c == Some('?') {
                        if let UrlPath::Opaque(ref mut s) = url.path {
                            if s.ends_with(' ') {
                                s.pop();
                                s.push_str("%20");
                            }
                        }
                        url.query = Some(String::new());
                        state = State::Query;
                    } else if c == Some('#') {
                        if let UrlPath::Opaque(ref mut s) = url.path {
                            if s.ends_with(' ') {
                                s.pop();
                                s.push_str("%20");
                            }
                        }
                        url.fragment = Some(String::new());
                        state = State::Fragment;
                    } else if let Some(ch) = c {
                        if let UrlPath::Opaque(ref mut s) = url.path {
                            utf8_percent_encode_char(ch, EncodeSet::C0Control, s);
                        }
                    }
                }
                State::Query => {
                    if c.is_none() || (state_override.is_none() && c == Some('#')) {
                        let set = if is_special_scheme(&url.scheme) {
                            EncodeSet::SpecialQuery
                        } else {
                            EncodeSet::Query
                        };
                        let encoded = utf8_percent_encode(&buffer, set);
                        if let Some(ref mut q) = url.query {
                            q.push_str(&encoded);
                        }
                        buffer.clear();
                        if state_override.is_none() && c == Some('#') {
                            url.fragment = Some(String::new());
                            state = State::Fragment;
                        }
                    } else if let Some(ch) = c {
                        buffer.push(ch);
                    }
                }
                State::Fragment => {
                    if let Some(ch) = c {
                        if let Some(ref mut f) = url.fragment {
                            utf8_percent_encode_char(ch, EncodeSet::Fragment, f);
                        }
                    }
                }
            }

            if c.is_none() && idx == prev_idx && state == prev_state {
                break;
            }
            if c.is_none() && idx >= len as isize && state == prev_state {
                break;
            }
            idx += 1;
        }

        Ok(())
    }

    fn shorten_path(&mut self) {
        if let UrlPath::Segments(ref mut segs) = self.path {
            if self.scheme == "file"
                && segs.len() == 1
                && is_normalized_windows_drive_letter(&segs[0])
            {
                return;
            }
            segs.pop();
        }
    }

    fn push_segment(&mut self, seg: String) {
        if let UrlPath::Segments(ref mut segs) = self.path {
            segs.push(seg);
        }
    }

    fn segments_is_empty(&self) -> bool {
        matches!(&self.path, UrlPath::Segments(segs) if segs.is_empty())
    }

    pub fn join(&self, relative: &str) -> Result<Self, UrlError> {
        Self::parse_with_base(relative, Some(self))
    }

    pub fn href(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.scheme);
        out.push(':');
        if let Some(ref host) = self.host {
            out.push_str("//");
            if !self.username.is_empty() || !self.password.is_empty() {
                out.push_str(&self.username);
                if !self.password.is_empty() {
                    out.push(':');
                    out.push_str(&self.password);
                }
                out.push('@');
            }
            out.push_str(&host.to_string());
            if let Some(port) = self.port {
                out.push(':');
                out.push_str(&port.to_string());
            }
        } else if let UrlPath::Segments(segs) = &self.path {
            if segs.len() > 1 && segs[0].is_empty() {
                out.push_str("/.");
            }
        }
        out.push_str(&self.pathname());
        if let Some(ref q) = self.query {
            out.push('?');
            out.push_str(q);
        }
        if let Some(ref f) = self.fragment {
            out.push('#');
            out.push_str(f);
        }
        out
    }

    pub fn protocol(&self) -> String {
        format!("{}:", self.scheme)
    }

    pub fn hostname(&self) -> String {
        self.host.as_ref().map_or(String::new(), |h| h.to_string())
    }

    pub fn host_str(&self) -> String {
        let h = self.hostname();
        match self.port {
            Some(p) if !h.is_empty() => format!("{}:{}", h, p),
            _ => h,
        }
    }

    pub fn port_str(&self) -> String {
        self.port.map_or(String::new(), |p| p.to_string())
    }

    pub fn port_or_default(&self) -> Option<u16> {
        self.port.or_else(|| default_port(&self.scheme))
    }

    pub fn pathname(&self) -> String {
        match &self.path {
            UrlPath::Opaque(s) => s.clone(),
            UrlPath::Segments(segs) => {
                if segs.is_empty() {
                    String::new()
                } else {
                    format!("/{}", segs.join("/"))
                }
            }
        }
    }

    pub fn search(&self) -> String {
        match &self.query {
            Some(q) if !q.is_empty() => format!("?{}", q),
            _ => String::new(),
        }
    }

    pub fn hash(&self) -> String {
        match &self.fragment {
            Some(f) if !f.is_empty() => format!("#{}", f),
            _ => String::new(),
        }
    }

    pub fn origin(&self) -> String {
        match self.scheme.as_str() {
            "http" | "https" | "ws" | "wss" | "ftp" => {
                format!("{}://{}", self.scheme, self.host_str())
            }
            "blob" => {
                if let UrlPath::Opaque(ref inner) = self.path {
                    if let Ok(parsed) = Url::parse(inner) {
                        if matches!(parsed.scheme.as_str(), "http" | "https") {
                            return parsed.origin();
                        }
                    }
                }
                "null".to_string()
            }
            _ => "null".to_string(),
        }
    }

    pub fn search_params(&self) -> UrlSearchParams {
        UrlSearchParams::parse(self.query.as_deref().unwrap_or(""))
    }

    pub fn request_target(&self) -> String {
        let mut target = self.pathname();
        if target.is_empty() {
            target.push('/');
        }
        if let Some(ref q) = self.query {
            target.push('?');
            target.push_str(q);
        }
        target
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.href())
    }
}
