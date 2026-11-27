#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EncodeSet {
    C0Control,
    Fragment,
    Query,
    SpecialQuery,
    Path,
    UserInfo,
    Component,
    FormUrlEncoded,
}

fn is_c0_or_non_ascii(b: u8) -> bool {
    b <= 0x1F || b > 0x7E
}

pub fn should_percent_encode(b: u8, set: EncodeSet) -> bool {
    match set {
        EncodeSet::C0Control => is_c0_or_non_ascii(b),
        EncodeSet::Fragment => is_c0_or_non_ascii(b) || matches!(b, b' ' | b'"' | b'<' | b'>' | b'`'),
        EncodeSet::Query => is_c0_or_non_ascii(b) || matches!(b, b' ' | b'"' | b'#' | b'<' | b'>'),
        EncodeSet::SpecialQuery => {
            is_c0_or_non_ascii(b) || matches!(b, b' ' | b'"' | b'#' | b'<' | b'>' | b'\'')
        }
        EncodeSet::Path => {
            is_c0_or_non_ascii(b)
                || matches!(b, b' ' | b'"' | b'#' | b'<' | b'>' | b'?' | b'`' | b'{' | b'}')
        }
        EncodeSet::UserInfo => {
            should_percent_encode(b, EncodeSet::Path)
                || matches!(b, b'/' | b':' | b';' | b'=' | b'@' | b'[' | b'\\' | b']' | b'^' | b'|')
        }
        EncodeSet::Component => {
            should_percent_encode(b, EncodeSet::UserInfo)
                || matches!(b, b'$' | b'%' | b'&' | b'+' | b',')
        }
        EncodeSet::FormUrlEncoded => {
            !(b.is_ascii_alphanumeric() || matches!(b, b'*' | b'-' | b'.' | b'_'))
        }
    }
}

pub fn percent_encode_byte(b: u8, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(HEX[(b >> 4) as usize] as char);
    out.push(HEX[(b & 0x0F) as usize] as char);
}

pub fn utf8_percent_encode(input: &str, set: EncodeSet) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if set == EncodeSet::FormUrlEncoded && b == b' ' {
            out.push('+');
        } else if should_percent_encode(b, set) {
            percent_encode_byte(b, &mut out);
        } else {
            out.push(b as char);
        }
    }
    out
}

pub fn utf8_percent_encode_char(c: char, set: EncodeSet, out: &mut String) {
    let mut buf = [0u8; 4];
    let s = c.encode_utf8(&mut buf);
    for &b in s.as_bytes() {
        if should_percent_encode(b, set) {
            percent_encode_byte(b, out);
        } else {
            out.push(b as char);
        }
    }
}

pub fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            if let (Some(hi), Some(lo)) = (hex_val(input[i + 1]), hex_val(input[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

pub fn percent_decode_str(input: &str) -> String {
    String::from_utf8_lossy(&percent_decode(input.as_bytes())).into_owned()
}

pub fn form_urlencoded_decode(input: &str) -> String {
    let replaced: Vec<u8> = input
        .as_bytes()
        .iter()
        .map(|&b| if b == b'+' { b' ' } else { b })
        .collect();
    String::from_utf8_lossy(&percent_decode(&replaced)).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
