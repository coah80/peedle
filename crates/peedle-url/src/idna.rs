use unicode_normalization::UnicodeNormalization;

const BASE: u32 = 36;
const TMIN: u32 = 1;
const TMAX: u32 = 26;
const SKEW: u32 = 38;
const DAMP: u32 = 700;
const INITIAL_BIAS: u32 = 72;
const INITIAL_N: u32 = 128;

fn adapt(mut delta: u32, num_points: u32, first_time: bool) -> u32 {
    delta = if first_time { delta / DAMP } else { delta >> 1 };
    delta += delta / num_points;
    let mut k = 0;
    while delta > ((BASE - TMIN) * TMAX) / 2 {
        delta /= BASE - TMIN;
        k += BASE;
    }
    k + (((BASE - TMIN + 1) * delta) / (delta + SKEW))
}

fn encode_digit(d: u32) -> char {
    if d < 26 {
        (b'a' + d as u8) as char
    } else {
        (b'0' + (d - 26) as u8) as char
    }
}

pub fn punycode_encode(input: &str) -> Result<String, ()> {
    let chars: Vec<char> = input.chars().collect();
    let mut output = String::new();
    for &c in &chars {
        if c.is_ascii() {
            output.push(c);
        }
    }
    let basic_len = output.len() as u32;
    let mut handled = basic_len;
    if basic_len > 0 {
        output.push('-');
    }

    let mut n = INITIAL_N;
    let mut delta: u32 = 0;
    let mut bias = INITIAL_BIAS;
    let total = chars.len() as u32;

    while handled < total {
        let m = chars
            .iter()
            .map(|&c| c as u32)
            .filter(|&cp| cp >= n)
            .min()
            .ok_or(())?;
        delta = delta
            .checked_add((m - n).checked_mul(handled + 1).ok_or(())?)
            .ok_or(())?;
        n = m;
        for &c in &chars {
            let cp = c as u32;
            if cp < n {
                delta = delta.checked_add(1).ok_or(())?;
            } else if cp == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = if k <= bias {
                        TMIN
                    } else if k >= bias + TMAX {
                        TMAX
                    } else {
                        k - bias
                    };
                    if q < t {
                        break;
                    }
                    output.push(encode_digit(t + ((q - t) % (BASE - t))));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                output.push(encode_digit(q));
                bias = adapt(delta, handled + 1, handled == basic_len);
                delta = 0;
                handled += 1;
            }
        }
        delta += 1;
        n += 1;
    }
    Ok(output)
}

pub fn punycode_decode(input: &str) -> Result<String, ()> {
    let mut output: Vec<char> = Vec::new();
    let bytes = input.as_bytes();
    let delim = bytes.iter().rposition(|&b| b == b'-');
    let mut idx = 0usize;
    if let Some(d) = delim {
        for &b in &bytes[..d] {
            if !b.is_ascii() {
                return Err(());
            }
            output.push(b as char);
        }
        idx = d + 1;
    }

    let mut n = INITIAL_N;
    let mut i: u32 = 0;
    let mut bias = INITIAL_BIAS;

    while idx < bytes.len() {
        let old_i = i;
        let mut w: u32 = 1;
        let mut k = BASE;
        loop {
            if idx >= bytes.len() {
                return Err(());
            }
            let b = bytes[idx];
            idx += 1;
            let digit = match b {
                b'a'..=b'z' => (b - b'a') as u32,
                b'A'..=b'Z' => (b - b'A') as u32,
                b'0'..=b'9' => (b - b'0') as u32 + 26,
                _ => return Err(()),
            };
            i = i.checked_add(digit.checked_mul(w).ok_or(())?).ok_or(())?;
            let t = if k <= bias {
                TMIN
            } else if k >= bias + TMAX {
                TMAX
            } else {
                k - bias
            };
            if digit < t {
                break;
            }
            w = w.checked_mul(BASE - t).ok_or(())?;
            k += BASE;
        }
        let len = output.len() as u32 + 1;
        bias = adapt(i - old_i, len, old_i == 0);
        n = n.checked_add(i / len).ok_or(())?;
        i %= len;
        let ch = char::from_u32(n).ok_or(())?;
        output.insert(i as usize, ch);
        i += 1;
    }
    Ok(output.into_iter().collect())
}

fn is_uts46_ignored(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{1806}'
            | '\u{180B}'..='\u{180D}'
            | '\u{200B}'
            | '\u{2060}'
            | '\u{2064}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
    )
}

fn is_uts46_disallowed(c: char) -> bool {
    let cp = c as u32;
    matches!(
        cp,
        0x0000..=0x001F
            | 0x007F..=0x009F
            | 0x00A0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028..=0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFDD0..=0xFDEF
            | 0xFFFC..=0xFFFF
    ) || (cp & 0xFFFF) >= 0xFFFE
}

fn map_uts46_char(c: char, out: &mut String) -> Result<(), ()> {
    if is_uts46_ignored(c) {
        return Ok(());
    }
    if is_uts46_disallowed(c) {
        return Err(());
    }
    match c {
        '\u{3002}' | '\u{FF0E}' | '\u{FF61}' => out.push('.'),
        '\u{FF01}'..='\u{FF5E}' => {
            let mapped = char::from_u32(c as u32 - 0xFEE0).ok_or(())?;
            out.push(mapped.to_ascii_lowercase());
        }
        '\u{00DF}' => out.push('\u{00DF}'),
        _ => {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
        }
    }
    Ok(())
}

pub fn domain_to_ascii(domain: &str, be_strict: bool) -> Result<String, ()> {
    if domain.is_empty() {
        return Ok(String::new());
    }
    let mut mapped = String::with_capacity(domain.len());
    for c in domain.chars() {
        map_uts46_char(c, &mut mapped)?;
    }
    let normalized: String = mapped.nfkc().collect();
    if normalized.chars().any(is_uts46_disallowed) {
        return Err(());
    }

    let mut out_labels = Vec::new();
    for label in normalized.split('.') {
        if label.is_empty() {
            out_labels.push(String::new());
            continue;
        }
        let lower: String = label.to_lowercase();
        let ascii_label = if lower.is_ascii() {
            if lower.starts_with("xn--") {
                let decoded = punycode_decode(&lower[4..])?;
                if decoded.chars().any(is_uts46_disallowed) {
                    return Err(());
                }
                let reencoded = punycode_encode(&decoded)?;
                if reencoded != &lower[4..] && be_strict {
                    return Err(());
                }
            }
            lower
        } else {
            let encoded = punycode_encode(&lower)?;
            format!("xn--{}", encoded)
        };
        if be_strict && ascii_label.len() > 63 {
            return Err(());
        }
        out_labels.push(ascii_label);
    }
    let joined = out_labels.join(".");
    if be_strict && joined.len() > 253 {
        return Err(());
    }
    Ok(joined)
}
