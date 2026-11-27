use crate::bigint::JsBigInt;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnvId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum JsValue {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    BigInt(JsBigInt),
    String(String),
    Symbol(u64),
    Object(ObjectId),
}

impl JsValue {
    pub fn is_undefined(&self) -> bool {
        matches!(self, JsValue::Undefined)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, JsValue::Null)
    }

    pub fn is_nullish(&self) -> bool {
        matches!(self, JsValue::Undefined | JsValue::Null)
    }

    pub fn is_object(&self) -> bool {
        matches!(self, JsValue::Object(_))
    }

    pub fn as_object(&self) -> Option<ObjectId> {
        match self {
            JsValue::Object(id) => Some(*id),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            JsValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn to_boolean(&self) -> bool {
        match self {
            JsValue::Undefined | JsValue::Null => false,
            JsValue::Boolean(b) => *b,
            JsValue::Number(n) => *n != 0.0 && !n.is_nan(),
            JsValue::BigInt(n) => !n.is_zero(),
            JsValue::String(s) => !s.is_empty(),
            JsValue::Symbol(_) | JsValue::Object(_) => true,
        }
    }

    pub fn same_value(&self, other: &JsValue) -> bool {
        match (self, other) {
            (JsValue::Undefined, JsValue::Undefined) => true,
            (JsValue::Null, JsValue::Null) => true,
            (JsValue::Boolean(a), JsValue::Boolean(b)) => a == b,
            (JsValue::Number(a), JsValue::Number(b)) => {
                if a.is_nan() && b.is_nan() {
                    true
                } else if *a == 0.0 && *b == 0.0 {
                    a.is_sign_positive() == b.is_sign_positive()
                } else {
                    a == b
                }
            }
            (JsValue::BigInt(a), JsValue::BigInt(b)) => a == b,
            (JsValue::String(a), JsValue::String(b)) => a == b,
            (JsValue::Symbol(a), JsValue::Symbol(b)) => a == b,
            (JsValue::Object(a), JsValue::Object(b)) => a == b,
            _ => false,
        }
    }

    pub fn same_value_zero(&self, other: &JsValue) -> bool {
        match (self, other) {
            (JsValue::Number(a), JsValue::Number(b)) => {
                if a.is_nan() && b.is_nan() {
                    true
                } else {
                    a == b
                }
            }
            _ => self.same_value(other),
        }
    }

    pub fn strict_eq(&self, other: &JsValue) -> bool {
        match (self, other) {
            (JsValue::Undefined, JsValue::Undefined) => true,
            (JsValue::Null, JsValue::Null) => true,
            (JsValue::Boolean(a), JsValue::Boolean(b)) => a == b,
            (JsValue::Number(a), JsValue::Number(b)) => a == b,
            (JsValue::BigInt(a), JsValue::BigInt(b)) => a == b,
            (JsValue::String(a), JsValue::String(b)) => a == b,
            (JsValue::Symbol(a), JsValue::Symbol(b)) => a == b,
            (JsValue::Object(a), JsValue::Object(b)) => a == b,
            _ => false,
        }
    }

    pub fn is_primitive(&self) -> bool {
        !matches!(self, JsValue::Object(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PropertyKey {
    String(String),
    Symbol(u64),
}

impl PropertyKey {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PropertyKey::String(s) => Some(s.as_str()),
            PropertyKey::Symbol(_) => None,
        }
    }

    pub fn to_value(&self) -> JsValue {
        match self {
            PropertyKey::String(s) => JsValue::String(s.clone()),
            PropertyKey::Symbol(id) => JsValue::Symbol(*id),
        }
    }

    pub fn array_index(&self) -> Option<u32> {
        match self {
            PropertyKey::String(s) => parse_array_index(s),
            PropertyKey::Symbol(_) => None,
        }
    }
}

impl From<&str> for PropertyKey {
    fn from(s: &str) -> Self {
        PropertyKey::String(s.to_string())
    }
}

impl From<String> for PropertyKey {
    fn from(s: String) -> Self {
        PropertyKey::String(s)
    }
}

impl fmt::Display for PropertyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyKey::String(s) => write!(f, "{}", s),
            PropertyKey::Symbol(id) => write!(f, "Symbol(#{})", id),
        }
    }
}

pub fn parse_array_index(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 10 {
        return None;
    }
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let val: u64 = s.parse().ok()?;
    if val < u32::MAX as u64 {
        Some(val as u32)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct PropertyDescriptor {
    pub value: Option<JsValue>,
    pub writable: Option<bool>,
    pub get: Option<JsValue>,
    pub set: Option<JsValue>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}

impl PropertyDescriptor {
    pub fn data(value: JsValue, writable: bool, enumerable: bool, configurable: bool) -> Self {
        PropertyDescriptor {
            value: Some(value),
            writable: Some(writable),
            get: None,
            set: None,
            enumerable: Some(enumerable),
            configurable: Some(configurable),
        }
    }

    pub fn accessor(
        get: Option<JsValue>,
        set: Option<JsValue>,
        enumerable: bool,
        configurable: bool,
    ) -> Self {
        PropertyDescriptor {
            value: None,
            writable: None,
            get,
            set,
            enumerable: Some(enumerable),
            configurable: Some(configurable),
        }
    }

    pub fn empty() -> Self {
        PropertyDescriptor {
            value: None,
            writable: None,
            get: None,
            set: None,
            enumerable: None,
            configurable: None,
        }
    }

    pub fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    pub fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    pub fn is_generic(&self) -> bool {
        !self.is_accessor() && !self.is_data()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedArrayKind {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
    BigInt64,
    BigUint64,
}

impl TypedArrayKind {
    pub fn element_size(self) -> usize {
        match self {
            TypedArrayKind::Int8 | TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped => 1,
            TypedArrayKind::Int16 | TypedArrayKind::Uint16 => 2,
            TypedArrayKind::Int32 | TypedArrayKind::Uint32 | TypedArrayKind::Float32 => 4,
            TypedArrayKind::Float64 | TypedArrayKind::BigInt64 | TypedArrayKind::BigUint64 => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TypedArrayKind::Int8 => "Int8Array",
            TypedArrayKind::Uint8 => "Uint8Array",
            TypedArrayKind::Uint8Clamped => "Uint8ClampedArray",
            TypedArrayKind::Int16 => "Int16Array",
            TypedArrayKind::Uint16 => "Uint16Array",
            TypedArrayKind::Int32 => "Int32Array",
            TypedArrayKind::Uint32 => "Uint32Array",
            TypedArrayKind::Float32 => "Float32Array",
            TypedArrayKind::Float64 => "Float64Array",
            TypedArrayKind::BigInt64 => "BigInt64Array",
            TypedArrayKind::BigUint64 => "BigUint64Array",
        }
    }

    pub fn is_bigint(self) -> bool {
        matches!(self, TypedArrayKind::BigInt64 | TypedArrayKind::BigUint64)
    }

    pub const ALL: [TypedArrayKind; 11] = [
        TypedArrayKind::Int8,
        TypedArrayKind::Uint8,
        TypedArrayKind::Uint8Clamped,
        TypedArrayKind::Int16,
        TypedArrayKind::Uint16,
        TypedArrayKind::Int32,
        TypedArrayKind::Uint32,
        TypedArrayKind::Float32,
        TypedArrayKind::Float64,
        TypedArrayKind::BigInt64,
        TypedArrayKind::BigUint64,
    ];
}

pub fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n == 0.0 {
        return "0".to_string();
    }
    if n.is_infinite() {
        return if n.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    let abs = n.abs();
    if n.fract() == 0.0 && abs < 1e21 {
        if abs < 9_007_199_254_740_992.0 {
            return format!("{}", n as i64);
        }
    }
    if abs >= 1e21 || abs < 1e-6 {
        let s = format!("{:e}", n);
        return format_js_exp(&s);
    }
    let s = format!("{}", n);
    s
}

fn format_js_exp(s: &str) -> String {
    if let Some((mantissa, exp)) = s.split_once('e') {
        if let Ok(e) = exp.parse::<i32>() {
            if e >= 0 {
                return format!("{}e+{}", mantissa, e);
            } else {
                return format!("{}e{}", mantissa, e);
            }
        }
    }
    s.to_string()
}

pub fn string_to_number(s: &str) -> f64 {
    let trimmed = trim_js_whitespace(s);
    if trimmed.is_empty() {
        return 0.0;
    }
    if trimmed == "Infinity" || trimmed == "+Infinity" {
        return f64::INFINITY;
    }
    if trimmed == "-Infinity" {
        return f64::NEG_INFINITY;
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return f64::NAN;
        }
        return parse_radix_f64(hex, 16);
    }
    if let Some(bin) = trimmed
        .strip_prefix("0b")
        .or_else(|| trimmed.strip_prefix("0B"))
    {
        if bin.is_empty() || !bin.bytes().all(|b| b == b'0' || b == b'1') {
            return f64::NAN;
        }
        return parse_radix_f64(bin, 2);
    }
    if let Some(oct) = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
    {
        if oct.is_empty() || !oct.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
            return f64::NAN;
        }
        return parse_radix_f64(oct, 8);
    }
    if trimmed.eq_ignore_ascii_case("nan")
        || trimmed.eq_ignore_ascii_case("inf")
        || trimmed.eq_ignore_ascii_case("-inf")
        || trimmed.eq_ignore_ascii_case("+inf")
        || trimmed.eq_ignore_ascii_case("infinity")
        || trimmed.eq_ignore_ascii_case("+infinity")
        || trimmed.eq_ignore_ascii_case("-infinity")
    {
        return f64::NAN;
    }
    if trimmed.contains('_') {
        return f64::NAN;
    }
    match trimmed.parse::<f64>() {
        Ok(v) => v,
        Err(_) => f64::NAN,
    }
}

fn parse_radix_f64(digits: &str, radix: u32) -> f64 {
    let mut acc = 0.0_f64;
    let r = radix as f64;
    for b in digits.bytes() {
        let d = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'z' => (b - b'a' + 10) as u32,
            b'A'..=b'Z' => (b - b'A' + 10) as u32,
            _ => return f64::NAN,
        };
        if d >= radix {
            return f64::NAN;
        }
        acc = acc * r + d as f64;
    }
    acc
}

pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

pub fn trim_js_whitespace(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

pub fn to_int32(n: f64) -> i32 {
    if !n.is_finite() || n == 0.0 {
        return 0;
    }
    let int = n.trunc().rem_euclid(4294967296.0);
    let u = int as u32;
    u as i32
}

pub fn to_uint32(n: f64) -> u32 {
    to_int32(n) as u32
}

pub fn to_integer_or_infinity(n: f64) -> f64 {
    if n.is_nan() || n == 0.0 {
        0.0
    } else if n.is_infinite() {
        n
    } else {
        n.trunc()
    }
}

pub fn to_length(n: f64) -> usize {
    let i = to_integer_or_infinity(n);
    if i <= 0.0 {
        0
    } else if i >= 9_007_199_254_740_991.0 {
        usize::MAX.min(9_007_199_254_740_991)
    } else {
        i as usize
    }
}
