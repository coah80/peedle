use crate::error::JsError;
use std::cmp::Ordering;
use std::fmt;

const MAX_BIGINT_BITS: usize = 1_048_576;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JsBigInt {
    neg: bool,
    limbs: Vec<u32>,
}

impl Default for JsBigInt {
    fn default() -> Self {
        Self::zero()
    }
}

impl JsBigInt {
    pub fn zero() -> Self {
        JsBigInt {
            neg: false,
            limbs: Vec::new(),
        }
    }

    pub fn one() -> Self {
        JsBigInt {
            neg: false,
            limbs: vec![1],
        }
    }

    pub fn from_limbs(neg: bool, mut limbs: Vec<u32>) -> Self {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        let neg = if limbs.is_empty() { false } else { neg };
        JsBigInt { neg, limbs }
    }

    pub fn from_u64(v: u64) -> Self {
        Self::from_u64_signed(false, v)
    }

    pub fn from_u64_signed(neg: bool, v: u64) -> Self {
        if v == 0 {
            return Self::zero();
        }
        let lo = v as u32;
        let hi = (v >> 32) as u32;
        if hi == 0 {
            JsBigInt {
                neg,
                limbs: vec![lo],
            }
        } else {
            JsBigInt {
                neg,
                limbs: vec![lo, hi],
            }
        }
    }

    pub fn from_i64(v: i64) -> Self {
        Self::from_u64_signed(v < 0, v.unsigned_abs())
    }

    pub fn from_u128(v: u128) -> Self {
        Self::from_u128_signed(false, v)
    }

    pub fn from_u128_signed(neg: bool, mut v: u128) -> Self {
        if v == 0 {
            return Self::zero();
        }
        let mut limbs = Vec::new();
        while v != 0 {
            limbs.push(v as u32);
            v >>= 32;
        }
        JsBigInt { neg, limbs }
    }

    pub fn from_i128(v: i128) -> Self {
        Self::from_u128_signed(v < 0, v.unsigned_abs())
    }

    pub fn from_f64_exact(n: f64) -> Option<Self> {
        if !n.is_finite() || n.fract() != 0.0 {
            return None;
        }
        if n == 0.0 {
            return Some(Self::zero());
        }
        let bits = n.to_bits();
        let neg = (bits >> 63) != 0;
        let exp = ((bits >> 52) & 0x7ff) as i32 - 1023;
        let mant = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
        if exp <= 52 {
            let val = mant >> (52 - exp);
            Some(Self::from_u64_signed(neg, val))
        } else {
            let base = Self::from_u64_signed(neg, mant);
            Some(base.shl_usize((exp - 52) as usize))
        }
    }

    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    pub fn is_neg(&self) -> bool {
        self.neg
    }

    pub fn bit_len(&self) -> usize {
        let Some(&top) = self.limbs.last() else {
            return 0;
        };
        (self.limbs.len() - 1) * 32 + (32 - top.leading_zeros() as usize)
    }

    pub fn to_u64_wrapping(&self) -> u64 {
        let lo = self.limbs.first().copied().unwrap_or(0) as u64;
        let hi = self.limbs.get(1).copied().unwrap_or(0) as u64;
        let mag = lo | (hi << 32);
        if self.neg {
            mag.wrapping_neg()
        } else {
            mag
        }
    }

    pub fn to_i64_wrapping(&self) -> i64 {
        self.to_u64_wrapping() as i64
    }

    pub fn to_i128_wrapping(&self) -> i128 {
        let w0 = self.limbs.first().copied().unwrap_or(0) as u128;
        let w1 = self.limbs.get(1).copied().unwrap_or(0) as u128;
        let w2 = self.limbs.get(2).copied().unwrap_or(0) as u128;
        let w3 = self.limbs.get(3).copied().unwrap_or(0) as u128;
        let mag = w0 | (w1 << 32) | (w2 << 64) | (w3 << 96);
        if self.neg {
            mag.wrapping_neg() as i128
        } else {
            mag as i128
        }
    }

    fn get_bit(&self, bit: usize) -> bool {
        let word = bit / 32;
        let shift = bit % 32;
        if word >= self.limbs.len() {
            false
        } else {
            ((self.limbs[word] >> shift) & 1) != 0
        }
    }

    fn any_bit_below(&self, bit: usize) -> bool {
        let full_words = bit / 32;
        let rem_bits = bit % 32;
        for w in 0..full_words.min(self.limbs.len()) {
            if self.limbs[w] != 0 {
                return true;
            }
        }
        if rem_bits > 0 && full_words < self.limbs.len() {
            let mask = (1u32 << rem_bits) - 1;
            if (self.limbs[full_words] & mask) != 0 {
                return true;
            }
        }
        false
    }

    pub fn to_f64(&self) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let bl = self.bit_len();
        if bl > 1024 {
            return if self.neg {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        if bl <= 53 {
            let lo = self.limbs.first().copied().unwrap_or(0) as u64;
            let hi = self.limbs.get(1).copied().unwrap_or(0) as u64;
            let mag = (lo | (hi << 32)) as f64;
            return if self.neg { -mag } else { mag };
        }
        let shift = bl - 53;
        let mut mant = 0u64;
        for i in 0..53 {
            if self.get_bit(shift + i) {
                mant |= 1u64 << i;
            }
        }
        let round_bit = self.get_bit(shift - 1);
        let sticky = self.any_bit_below(shift - 1);
        if round_bit && (sticky || (mant & 1) == 1) {
            mant += 1;
        }
        let mut exp = (bl - 1) as i32;
        if mant == (1u64 << 53) {
            mant >>= 1;
            exp += 1;
        }
        if exp > 1023 {
            return if self.neg {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        let sign_bit = if self.neg { 1u64 << 63 } else { 0 };
        let exp_bits = ((exp + 1023) as u64) << 52;
        let frac_bits = mant & ((1u64 << 52) - 1);
        f64::from_bits(sign_bit | exp_bits | frac_bits)
    }

    pub fn cmp_f64(&self, n: f64) -> Option<Ordering> {
        if n.is_nan() {
            return None;
        }
        if n == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if n == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        if n == 0.0 {
            return Some(self.cmp(&Self::zero()));
        }
        let trunc = n.trunc();
        let trunc_bi = Self::from_f64_exact(trunc)?;
        match self.cmp(&trunc_bi) {
            Ordering::Equal => {
                if n == trunc {
                    Some(Ordering::Equal)
                } else if n > trunc {
                    Some(Ordering::Less)
                } else {
                    Some(Ordering::Greater)
                }
            }
            other => Some(other),
        }
    }

    pub fn parse_literal(s: &str) -> Result<Self, JsError> {
        let (radix, digits) = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))
        {
            (16u32, h)
        } else if let Some(o) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
            (8u32, o)
        } else if let Some(b) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
            (2u32, b)
        } else {
            (10u32, s)
        };
        Self::parse_unsigned_radix(digits, radix)
            .map_err(|_| JsError::syntax(format!("Invalid BigInt literal: {}", s)))
    }

    pub fn parse_string(s: &str) -> Option<Self> {
        let t = crate::value::trim_js_whitespace(s);
        if t.is_empty() {
            return Some(Self::zero());
        }
        if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            return Self::parse_unsigned_radix(h, 16).ok();
        }
        if let Some(o) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
            return Self::parse_unsigned_radix(o, 8).ok();
        }
        if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
            return Self::parse_unsigned_radix(b, 2).ok();
        }
        let (neg, rest) = if let Some(r) = t.strip_prefix('-') {
            (true, r)
        } else if let Some(r) = t.strip_prefix('+') {
            (false, r)
        } else {
            (false, t)
        };
        if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut bi = Self::parse_unsigned_radix(rest, 10).ok()?;
        if neg && !bi.is_zero() {
            bi.neg = true;
        }
        Some(bi)
    }

    fn parse_unsigned_radix(digits: &str, radix: u32) -> Result<Self, ()> {
        if digits.is_empty() {
            return Err(());
        }
        let mut limbs: Vec<u32> = Vec::new();
        for ch in digits.chars() {
            let d = ch.to_digit(radix).ok_or(())?;
            let mut carry = d as u64;
            let r = radix as u64;
            for limb in &mut limbs {
                let prod = (*limb as u64) * r + carry;
                *limb = prod as u32;
                carry = prod >> 32;
            }
            if carry != 0 {
                limbs.push(carry as u32);
            }
        }
        Ok(Self::from_limbs(false, limbs))
    }

    pub fn to_string_radix(&self, radix: u32) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let mut tmp = self.limbs.clone();
        let mut chars: Vec<u8> = Vec::new();
        let r = radix as u64;
        while !tmp.is_empty() {
            let mut rem = 0u64;
            for limb in tmp.iter_mut().rev() {
                let cur = (rem << 32) | (*limb as u64);
                *limb = (cur / r) as u32;
                rem = cur % r;
            }
            while tmp.last() == Some(&0) {
                tmp.pop();
            }
            chars.push(DIGITS[rem as usize]);
        }
        if self.neg {
            chars.push(b'-');
        }
        chars.reverse();
        String::from_utf8(chars).unwrap()
    }

    pub fn neg(&self) -> Self {
        if self.is_zero() {
            Self::zero()
        } else {
            JsBigInt {
                neg: !self.neg,
                limbs: self.limbs.clone(),
            }
        }
    }

    pub fn bitnot(&self) -> Self {
        self.add(&Self::one()).neg()
    }

    pub fn add(&self, rhs: &Self) -> Self {
        if self.neg == rhs.neg {
            Self::from_limbs(self.neg, add_mag(&self.limbs, &rhs.limbs))
        } else {
            match cmp_mag(&self.limbs, &rhs.limbs) {
                Ordering::Equal => Self::zero(),
                Ordering::Greater => Self::from_limbs(self.neg, sub_mag(&self.limbs, &rhs.limbs)),
                Ordering::Less => Self::from_limbs(rhs.neg, sub_mag(&rhs.limbs, &self.limbs)),
            }
        }
    }

    pub fn sub(&self, rhs: &Self) -> Self {
        self.add(&rhs.neg())
    }

    pub fn mul(&self, rhs: &Self) -> Self {
        if self.is_zero() || rhs.is_zero() {
            return Self::zero();
        }
        let n = self.limbs.len();
        let m = rhs.limbs.len();
        let mut out = vec![0u32; n + m];
        for i in 0..n {
            let a = self.limbs[i] as u64;
            let mut carry = 0u64;
            for j in 0..m {
                let prod = a * (rhs.limbs[j] as u64) + (out[i + j] as u64) + carry;
                out[i + j] = prod as u32;
                carry = prod >> 32;
            }
            out[i + m] = carry as u32;
        }
        Self::from_limbs(self.neg ^ rhs.neg, out)
    }

    pub fn div_rem(&self, rhs: &Self) -> Result<(Self, Self), JsError> {
        if rhs.is_zero() {
            return Err(JsError::range("Division by zero"));
        }
        if self.is_zero() {
            return Ok((Self::zero(), Self::zero()));
        }
        let (q_limbs, r_limbs) = div_rem_mag(&self.limbs, &rhs.limbs);
        let q = Self::from_limbs(self.neg ^ rhs.neg, q_limbs);
        let r = Self::from_limbs(self.neg, r_limbs);
        Ok((q, r))
    }

    pub fn pow(&self, exp: &Self) -> Result<Self, JsError> {
        if exp.neg {
            return Err(JsError::range("Exponent must be positive"));
        }
        if exp.is_zero() {
            return Ok(Self::one());
        }
        if self.is_zero() {
            return Ok(Self::zero());
        }
        if self.limbs.len() == 1 && self.limbs[0] == 1 {
            let odd = (exp.limbs[0] & 1) == 1;
            return Ok(if self.neg && odd {
                Self::from_i64(-1)
            } else {
                Self::one()
            });
        }
        if exp.limbs.len() > 1 || exp.limbs[0] as usize > MAX_BIGINT_BITS {
            return Err(JsError::range("Maximum BigInt size exceeded"));
        }
        let mut e = exp.limbs[0];
        let mut base = self.clone();
        let mut res = Self::one();
        while e > 0 {
            if (e & 1) == 1 {
                if res.bit_len() + base.bit_len() > MAX_BIGINT_BITS {
                    return Err(JsError::range("Maximum BigInt size exceeded"));
                }
                res = res.mul(&base);
            }
            e >>= 1;
            if e > 0 {
                if base.bit_len() * 2 > MAX_BIGINT_BITS {
                    return Err(JsError::range("Maximum BigInt size exceeded"));
                }
                base = base.mul(&base);
            }
        }
        Ok(res)
    }

    fn shl_usize(&self, bits: usize) -> Self {
        if self.is_zero() || bits == 0 {
            return self.clone();
        }
        let word_shift = bits / 32;
        let bit_shift = bits % 32;
        let mut out = vec![0u32; self.limbs.len() + word_shift + 1];
        if bit_shift == 0 {
            for (i, &l) in self.limbs.iter().enumerate() {
                out[i + word_shift] = l;
            }
        } else {
            let mut carry = 0u32;
            for (i, &l) in self.limbs.iter().enumerate() {
                out[i + word_shift] = (l << bit_shift) | carry;
                carry = l >> (32 - bit_shift);
            }
            out[self.limbs.len() + word_shift] = carry;
        }
        Self::from_limbs(self.neg, out)
    }

    fn shr_usize(&self, bits: usize) -> Self {
        if self.is_zero() || bits == 0 {
            return self.clone();
        }
        let bl = self.bit_len();
        if bits >= bl {
            return if self.neg {
                Self::from_i64(-1)
            } else {
                Self::zero()
            };
        }
        let word_shift = bits / 32;
        let bit_shift = bits % 32;
        let mut out = Vec::with_capacity(self.limbs.len() - word_shift);
        if bit_shift == 0 {
            out.extend_from_slice(&self.limbs[word_shift..]);
        } else {
            for i in word_shift..self.limbs.len() {
                let lo = self.limbs[i] >> bit_shift;
                let hi = if i + 1 < self.limbs.len() {
                    self.limbs[i + 1] << (32 - bit_shift)
                } else {
                    0
                };
                out.push(lo | hi);
            }
        }
        if self.neg && self.any_bit_below(bits) {
            out = add_mag(&out, &[1]);
        }
        Self::from_limbs(self.neg, out)
    }

    pub fn shl(&self, rhs: &Self) -> Result<Self, JsError> {
        if rhs.neg {
            return self.shr(&rhs.neg());
        }
        if self.is_zero() || rhs.is_zero() {
            return Ok(self.clone());
        }
        if rhs.limbs.len() > 1 || rhs.limbs[0] as usize + self.bit_len() > MAX_BIGINT_BITS {
            return Err(JsError::range("Maximum BigInt size exceeded"));
        }
        Ok(self.shl_usize(rhs.limbs[0] as usize))
    }

    pub fn shr(&self, rhs: &Self) -> Result<Self, JsError> {
        if rhs.neg {
            return self.shl(&rhs.neg());
        }
        if self.is_zero() || rhs.is_zero() {
            return Ok(self.clone());
        }
        if rhs.limbs.len() > 1 || rhs.limbs[0] as usize >= self.bit_len() {
            return Ok(if self.neg {
                Self::from_i64(-1)
            } else {
                Self::zero()
            });
        }
        Ok(self.shr_usize(rhs.limbs[0] as usize))
    }

    fn to_twos_complement(&self, len: usize) -> (Vec<u32>, u32) {
        let mut words = vec![0u32; len];
        for i in 0..self.limbs.len().min(len) {
            words[i] = self.limbs[i];
        }
        if !self.neg {
            return (words, 0);
        }
        let mut carry = 1u64;
        for w in &mut words {
            let v = (!*w as u64) + carry;
            *w = v as u32;
            carry = v >> 32;
        }
        let sign = if carry != 0 { 0u32 } else { u32::MAX };
        (words, sign)
    }

    fn from_twos_complement(mut words: Vec<u32>, sign: u32) -> Self {
        if sign == 0 {
            return Self::from_limbs(false, words);
        }
        let mut carry = 1u64;
        for w in &mut words {
            let v = (!*w as u64) + carry;
            *w = v as u32;
            carry = v >> 32;
        }
        if carry != 0 {
            words.push(carry as u32);
        }
        Self::from_limbs(true, words)
    }

    fn bitwise_op(&self, rhs: &Self, f: impl Fn(u32, u32) -> u32) -> Self {
        let len = self.limbs.len().max(rhs.limbs.len());
        let (a, sa) = self.to_twos_complement(len);
        let (b, sb) = rhs.to_twos_complement(len);
        let out: Vec<u32> = a.into_iter().zip(b).map(|(x, y)| f(x, y)).collect();
        let s = f(sa, sb);
        Self::from_twos_complement(out, s)
    }

    pub fn bitand(&self, rhs: &Self) -> Self {
        self.bitwise_op(rhs, |a, b| a & b)
    }

    pub fn bitor(&self, rhs: &Self) -> Self {
        self.bitwise_op(rhs, |a, b| a | b)
    }

    pub fn bitxor(&self, rhs: &Self) -> Self {
        self.bitwise_op(rhs, |a, b| a ^ b)
    }

    pub fn as_uint_n(&self, bits: usize) -> Result<Self, JsError> {
        if bits == 0 || self.is_zero() {
            return Ok(Self::zero());
        }
        if !self.neg && self.bit_len() <= bits {
            return Ok(self.clone());
        }
        if bits > MAX_BIGINT_BITS {
            return Err(JsError::range("Maximum BigInt size exceeded"));
        }
        let word_len = bits.div_ceil(32);
        let (mut words, _) = self.to_twos_complement(word_len);
        let rem = bits % 32;
        if rem != 0 {
            words[word_len - 1] &= (1u32 << rem) - 1;
        }
        Ok(Self::from_limbs(false, words))
    }

    pub fn as_int_n(&self, bits: usize) -> Result<Self, JsError> {
        if bits == 0 || self.is_zero() {
            return Ok(Self::zero());
        }
        let bl = self.bit_len();
        if bl < bits {
            return Ok(self.clone());
        }
        if self.neg && bl == bits && !self.any_bit_below(bits - 1) {
            return Ok(self.clone());
        }
        if bits > MAX_BIGINT_BITS {
            return Err(JsError::range("Maximum BigInt size exceeded"));
        }
        let u = self.as_uint_n(bits)?;
        if !u.get_bit(bits - 1) {
            return Ok(u);
        }
        let word_len = bits.div_ceil(32);
        let mut words = vec![0u32; word_len];
        for i in 0..u.limbs.len().min(word_len) {
            words[i] = u.limbs[i];
        }
        let rem = bits % 32;
        if rem != 0 {
            words[word_len - 1] |= !((1u32 << rem) - 1);
        }
        Ok(Self::from_twos_complement(words, u32::MAX))
    }
}

impl Ord for JsBigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.neg, other.neg) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => cmp_mag(&self.limbs, &other.limbs),
            (true, true) => cmp_mag(&other.limbs, &self.limbs),
        }
    }
}

impl PartialOrd for JsBigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for JsBigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_radix(10))
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    let mut al = a.len();
    while al > 0 && a[al - 1] == 0 {
        al -= 1;
    }
    let mut bl = b.len();
    while bl > 0 && b[bl - 1] == 0 {
        bl -= 1;
    }
    match al.cmp(&bl) {
        Ordering::Equal => {
            for i in (0..al).rev() {
                match a[i].cmp(&b[i]) {
                    Ordering::Equal => continue,
                    other => return other,
                }
            }
            Ordering::Equal
        }
        other => other,
    }
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let n = a.len().max(b.len());
    let mut out = Vec::with_capacity(n + 1);
    let mut carry = 0u64;
    for i in 0..n {
        let av = a.get(i).copied().unwrap_or(0) as u64;
        let bv = b.get(i).copied().unwrap_or(0) as u64;
        let sum = av + bv + carry;
        out.push(sum as u32);
        carry = sum >> 32;
    }
    if carry != 0 {
        out.push(carry as u32);
    }
    out
}

fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let av = a[i] as i64;
        let bv = b.get(i).copied().unwrap_or(0) as i64;
        let diff = av - bv - borrow;
        if diff < 0 {
            out.push((diff + (1i64 << 32)) as u32);
            borrow = 1;
        } else {
            out.push(diff as u32);
            borrow = 0;
        }
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    out
}

fn div_rem_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    match cmp_mag(a, b) {
        Ordering::Less => return (Vec::new(), a.to_vec()),
        Ordering::Equal => return (vec![1], Vec::new()),
        Ordering::Greater => {}
    }
    if b.len() == 1 {
        let d = b[0] as u64;
        let mut q = vec![0u32; a.len()];
        let mut rem = 0u64;
        for i in (0..a.len()).rev() {
            let cur = (rem << 32) | (a[i] as u64);
            q[i] = (cur / d) as u32;
            rem = cur % d;
        }
        while q.last() == Some(&0) {
            q.pop();
        }
        let r = if rem == 0 {
            Vec::new()
        } else {
            vec![rem as u32]
        };
        return (q, r);
    }
    let total_bits = (a.len() - 1) * 32 + (32 - a[a.len() - 1].leading_zeros() as usize);
    let mut q = vec![0u32; a.len()];
    let mut rem: Vec<u32> = Vec::new();
    for bit_idx in (0..total_bits).rev() {
        let mut carry = ((a[bit_idx / 32] >> (bit_idx % 32)) & 1) as u64;
        for w in &mut rem {
            let next = ((*w as u64) << 1) | carry;
            *w = next as u32;
            carry = next >> 32;
        }
        if carry != 0 {
            rem.push(carry as u32);
        }
        if cmp_mag(&rem, b) != Ordering::Less {
            rem = sub_mag(&rem, b);
            q[bit_idx / 32] |= 1u32 << (bit_idx % 32);
        }
    }
    while q.last() == Some(&0) {
        q.pop();
    }
    while rem.last() == Some(&0) {
        rem.pop();
    }
    (q, rem)
}
