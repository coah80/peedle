use crate::builtins::{set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    init_number(rt);
    init_math(rt);
    init_bigint(rt);
    init_boolean(rt);
}

fn this_number(rt: &JsRuntime, val: &JsValue) -> Result<f64, JsError> {
    match val {
        JsValue::Number(n) => Ok(*n),
        JsValue::Object(oid) => {
            if let ObjectKind::NumberObj(n) = rt.heap.get_obj(*oid).kind {
                Ok(n)
            } else {
                Err(JsError::type_err(
                    "Number.prototype method requires that 'this' be a Number",
                ))
            }
        }
        _ => Err(JsError::type_err(
            "Number.prototype method requires that 'this' be a Number",
        )),
    }
}

fn init_number(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.number_proto;

    set_method(
        rt,
        proto,
        "valueOf",
        0,
        Rc::new(|rt, this, _| this_number(rt, &this).map(JsValue::Number)),
    );

    set_method(
        rt,
        proto,
        "toString",
        1,
        Rc::new(|rt, this, args| {
            let n = this_number(rt, &this)?;
            let radix = match args.first() {
                None | Some(JsValue::Undefined) => 10,
                Some(v) => {
                    let r = to_integer_or_infinity(rt.to_number(v)?);
                    if r < 2.0 || r > 36.0 {
                        return Err(JsError::range("toString() radix argument must be between 2 and 36"));
                    }
                    r as u32
                }
            };
            if radix == 10 {
                return Ok(JsValue::String(number_to_string(n)));
            }
            Ok(JsValue::String(format_number_radix(n, radix)))
        }),
    );

    set_method(
        rt,
        proto,
        "toFixed",
        1,
        Rc::new(|rt, this, args| {
            let n = this_number(rt, &this)?;
            let f = match args.first() {
                None | Some(JsValue::Undefined) => 0.0,
                Some(v) => to_integer_or_infinity(rt.to_number(v)?),
            };
            if f < 0.0 || f > 100.0 || !f.is_finite() {
                return Err(JsError::range("toFixed() digits argument must be between 0 and 100"));
            }
            if !n.is_finite() || n.abs() >= 1e21 {
                return Ok(JsValue::String(number_to_string(n)));
            }
            let prec = f as usize;
            Ok(JsValue::String(format!("{:.*}", prec, n)))
        }),
    );

    set_method(
        rt,
        proto,
        "toExponential",
        1,
        Rc::new(|rt, this, args| {
            let n = this_number(rt, &this)?;
            let frac_arg = args.first();
            let f_opt = match frac_arg {
                None | Some(JsValue::Undefined) => None,
                Some(v) => Some(to_integer_or_infinity(rt.to_number(v)?)),
            };
            if !n.is_finite() {
                return Ok(JsValue::String(number_to_string(n)));
            }
            let x = if n == 0.0 { 0.0 } else { n };
            match f_opt {
                None => {
                    let s = format!("{:e}", x);
                    Ok(JsValue::String(normalize_exp(&s)))
                }
                Some(f) => {
                    if f < 0.0 || f > 100.0 || !f.is_finite() {
                        return Err(JsError::range(
                            "toExponential() argument must be between 0 and 100",
                        ));
                    }
                    let s = format!("{:.*e}", f as usize, x);
                    Ok(JsValue::String(normalize_exp(&s)))
                }
            }
        }),
    );

    set_method(
        rt,
        proto,
        "toPrecision",
        1,
        Rc::new(|rt, this, args| {
            let n = this_number(rt, &this)?;
            match args.first() {
                None | Some(JsValue::Undefined) => Ok(JsValue::String(number_to_string(n))),
                Some(v) => {
                    let p = to_integer_or_infinity(rt.to_number(v)?);
                    if !n.is_finite() {
                        return Ok(JsValue::String(number_to_string(n)));
                    }
                    if p < 1.0 || p > 100.0 || !p.is_finite() {
                        return Err(JsError::range(
                            "toPrecision() argument must be between 1 and 100",
                        ));
                    }
                    let prec = p as usize;
                    if n == 0.0 {
                        if prec == 1 {
                            return Ok(JsValue::String("0".into()));
                        }
                        return Ok(JsValue::String(format!("0.{}", "0".repeat(prec - 1))));
                    }
                    let e = n.abs().log10().floor() as i32;
                    if e < -6 || e >= prec as i32 {
                        let s = format!("{:.*e}", prec - 1, n);
                        Ok(JsValue::String(normalize_exp(&s)))
                    } else {
                        let dec = (prec as i32 - 1 - e).max(0) as usize;
                        Ok(JsValue::String(format!("{:.*}", dec, n)))
                    }
                }
            }
        }),
    );

    set_method(
        rt,
        proto,
        "toLocaleString",
        0,
        Rc::new(|rt, this, _| {
            let n = this_number(rt, &this)?;
            Ok(JsValue::String(number_to_string(n)))
        }),
    );

    let ctor = rt.create_native_constructor(
        "Number",
        1,
        proto,
        Rc::new(|rt, this, args| {
            let is_new = matches!(
                this,
                JsValue::Object(oid) if rt.heap.get_obj(oid).prototype == Some(rt.intrinsics.number_proto)
            );
            let n = match args.first() {
                None => 0.0,
                Some(JsValue::BigInt(b)) => *b as f64,
                Some(v) => rt.to_number(v)?,
            };
            if is_new {
                let obj = rt.heap.alloc_object(JsObject::new(
                    Some(rt.intrinsics.number_proto),
                    ObjectKind::NumberObj(n),
                ));
                Ok(JsValue::Object(obj))
            } else {
                Ok(JsValue::Number(n))
            }
        }),
    );

    let consts = [
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", 5e-324),
        ("NaN", f64::NAN),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("POSITIVE_INFINITY", f64::INFINITY),
        ("EPSILON", f64::EPSILON),
        ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
        ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
    ];
    for (name, val) in consts {
        rt.heap.get_obj_mut(ctor).set_own_prop(
            PropertyKey::from(name),
            PropEntry::data(JsValue::Number(val), false, false, false),
        );
    }

    set_method(
        rt,
        ctor,
        "isNaN",
        1,
        Rc::new(|_, _, args| {
            let b = matches!(args.first(), Some(JsValue::Number(n)) if n.is_nan());
            Ok(JsValue::Boolean(b))
        }),
    );
    set_method(
        rt,
        ctor,
        "isFinite",
        1,
        Rc::new(|_, _, args| {
            let b = matches!(args.first(), Some(JsValue::Number(n)) if n.is_finite());
            Ok(JsValue::Boolean(b))
        }),
    );
    set_method(
        rt,
        ctor,
        "isInteger",
        1,
        Rc::new(|_, _, args| {
            let b = matches!(args.first(), Some(JsValue::Number(n)) if n.is_finite() && n.fract() == 0.0);
            Ok(JsValue::Boolean(b))
        }),
    );
    set_method(
        rt,
        ctor,
        "isSafeInteger",
        1,
        Rc::new(|_, _, args| {
            let b = matches!(
                args.first(),
                Some(JsValue::Number(n))
                    if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0
            );
            Ok(JsValue::Boolean(b))
        }),
    );
    set_method(
        rt,
        ctor,
        "parseInt",
        2,
        Rc::new(|rt, _, args| crate::builtins::globals::parse_int_impl(rt, args)),
    );
    set_method(
        rt,
        ctor,
        "parseFloat",
        1,
        Rc::new(|rt, _, args| crate::builtins::globals::parse_float_impl(rt, args)),
    );

    set_global(rt, "Number", JsValue::Object(ctor));
}

fn normalize_exp(s: &str) -> String {
    if let Some((m, e)) = s.split_once('e') {
        if let Ok(ei) = e.parse::<i32>() {
            return if ei >= 0 {
                format!("{}e+{}", m, ei)
            } else {
                format!("{}e{}", m, ei)
            };
        }
    }
    s.to_string()
}

fn format_number_radix(n: f64, radix: u32) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n == 0.0 {
        return "0".into();
    }
    if n.is_infinite() {
        return if n.is_sign_positive() {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let neg = n.is_sign_negative();
    let mut abs = n.abs();
    let int_part = abs.trunc();
    let mut frac = abs - int_part;
    let mut int_buf = Vec::new();
    let mut iv = int_part;
    if iv == 0.0 {
        int_buf.push(b'0');
    } else {
        let r = radix as f64;
        while iv >= 1.0 {
            let rem = (iv % r) as usize;
            int_buf.push(digits[rem.min(35)]);
            iv = (iv / r).floor();
        }
        int_buf.reverse();
    }
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push_str(&String::from_utf8_lossy(&int_buf));
    if frac > 0.0 {
        out.push('.');
        let r = radix as f64;
        for _ in 0..20 {
            if frac == 0.0 {
                break;
            }
            frac *= r;
            let d = frac.floor() as usize;
            out.push(digits[d.min(35)] as char);
            frac -= d as f64;
        }
    }
    let _ = &mut abs;
    out
}

fn init_math(rt: &mut JsRuntime) {
    let math = rt.create_object();
    let consts = [
        ("E", std::f64::consts::E),
        ("LN10", std::f64::consts::LN_10),
        ("LN2", std::f64::consts::LN_2),
        ("LOG2E", std::f64::consts::LOG2_E),
        ("LOG10E", std::f64::consts::LOG10_E),
        ("PI", std::f64::consts::PI),
        ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
        ("SQRT2", std::f64::consts::SQRT_2),
    ];
    for (k, v) in consts {
        rt.heap.get_obj_mut(math).set_own_prop(
            PropertyKey::from(k),
            PropEntry::data(JsValue::Number(v), false, false, false),
        );
    }
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(math).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("Math".into()), false, false, true),
    );

    let unary_fns: [(&str, fn(f64) -> f64); 25] = [
        ("abs", f64::abs),
        ("acos", f64::acos),
        ("acosh", f64::acosh),
        ("asin", f64::asin),
        ("asinh", f64::asinh),
        ("atan", f64::atan),
        ("atanh", f64::atanh),
        ("cbrt", f64::cbrt),
        ("ceil", f64::ceil),
        ("cos", f64::cos),
        ("cosh", f64::cosh),
        ("exp", f64::exp),
        ("expm1", f64::exp_m1),
        ("floor", f64::floor),
        ("fround", |x| (x as f32) as f64),
        ("log", f64::ln),
        ("log1p", f64::ln_1p),
        ("log10", f64::log10),
        ("log2", f64::log2),
        ("sin", f64::sin),
        ("sinh", f64::sinh),
        ("sqrt", f64::sqrt),
        ("tan", f64::tan),
        ("tanh", f64::tanh),
        ("trunc", f64::trunc),
    ];
    for (name, f) in unary_fns {
        set_method(
            rt,
            math,
            name,
            1,
            Rc::new(move |rt, _, args| {
                let x = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
                Ok(JsValue::Number(f(x)))
            }),
        );
    }

    set_method(
        rt,
        math,
        "round",
        1,
        Rc::new(|rt, _, args| {
            let x = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            if x.is_nan() || x.is_infinite() || x == 0.0 {
                return Ok(JsValue::Number(x));
            }
            if x < 0.0 && x >= -0.5 {
                return Ok(JsValue::Number(-0.0));
            }
            Ok(JsValue::Number((x + 0.5).floor()))
        }),
    );

    set_method(
        rt,
        math,
        "sign",
        1,
        Rc::new(|rt, _, args| {
            let x = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let r = if x.is_nan() || x == 0.0 {
                x
            } else if x > 0.0 {
                1.0
            } else {
                -1.0
            };
            Ok(JsValue::Number(r))
        }),
    );

    set_method(
        rt,
        math,
        "clz32",
        1,
        Rc::new(|rt, _, args| {
            let x = to_uint32(rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?);
            Ok(JsValue::Number(x.leading_zeros() as f64))
        }),
    );

    set_method(
        rt,
        math,
        "imul",
        2,
        Rc::new(|rt, _, args| {
            let a = to_int32(rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?);
            let b = to_int32(rt.to_number(args.get(1).unwrap_or(&JsValue::Undefined))?);
            Ok(JsValue::Number(a.wrapping_mul(b) as f64))
        }),
    );

    set_method(
        rt,
        math,
        "pow",
        2,
        Rc::new(|rt, _, args| {
            let a = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let b = rt.to_number(args.get(1).unwrap_or(&JsValue::Undefined))?;
            let r = if b.is_nan() {
                f64::NAN
            } else if b == 0.0 {
                1.0
            } else if a.abs() == 1.0 && b.is_infinite() {
                f64::NAN
            } else {
                a.powf(b)
            };
            Ok(JsValue::Number(r))
        }),
    );

    set_method(
        rt,
        math,
        "atan2",
        2,
        Rc::new(|rt, _, args| {
            let y = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            let x = rt.to_number(args.get(1).unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Number(y.atan2(x)))
        }),
    );

    set_method(
        rt,
        math,
        "max",
        2,
        Rc::new(|rt, _, args| {
            let mut coerced = Vec::with_capacity(args.len());
            for a in args {
                coerced.push(rt.to_number(a)?);
            }
            let mut max = f64::NEG_INFINITY;
            for n in coerced {
                if n.is_nan() {
                    return Ok(JsValue::Number(f64::NAN));
                }
                if n > max || (n == 0.0 && max == 0.0 && n.is_sign_positive()) {
                    max = n;
                }
            }
            Ok(JsValue::Number(max))
        }),
    );

    set_method(
        rt,
        math,
        "min",
        2,
        Rc::new(|rt, _, args| {
            let mut coerced = Vec::with_capacity(args.len());
            for a in args {
                coerced.push(rt.to_number(a)?);
            }
            let mut min = f64::INFINITY;
            for n in coerced {
                if n.is_nan() {
                    return Ok(JsValue::Number(f64::NAN));
                }
                if n < min || (n == 0.0 && min == 0.0 && n.is_sign_negative()) {
                    min = n;
                }
            }
            Ok(JsValue::Number(min))
        }),
    );

    set_method(
        rt,
        math,
        "hypot",
        2,
        Rc::new(|rt, _, args| {
            if args.is_empty() {
                return Ok(JsValue::Number(0.0));
            }
            let mut nums = Vec::with_capacity(args.len());
            let mut has_inf = false;
            let mut has_nan = false;
            for a in args {
                let n = rt.to_number(a)?;
                if n.is_infinite() {
                    has_inf = true;
                } else if n.is_nan() {
                    has_nan = true;
                }
                nums.push(n);
            }
            if has_inf {
                return Ok(JsValue::Number(f64::INFINITY));
            }
            if has_nan {
                return Ok(JsValue::Number(f64::NAN));
            }
            let sum: f64 = nums.iter().map(|x| x * x).sum();
            let res = sum.sqrt();
            Ok(JsValue::Number(if res == 0.0 { 0.0 } else { res }))
        }),
    );

    set_method(
        rt,
        math,
        "f16round",
        1,
        Rc::new(|rt, _, args| {
            let x = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Number(f64_to_f16_round(x)))
        }),
    );

    set_method(
        rt,
        math,
        "random",
        0,
        Rc::new(|_, _, _| {
            let mut buf = [0u8; 8];
            let _ = getrandom::fill(&mut buf);
            let bits = u64::from_le_bytes(buf) >> 11;
            let val = (bits as f64) / ((1u64 << 53) as f64);
            Ok(JsValue::Number(val))
        }),
    );

    set_global(rt, "Math", JsValue::Object(math));
}

fn f64_to_f16_round(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 || x.is_infinite() {
        return x;
    }
    let sign = if x.is_sign_negative() { -1.0 } else { 1.0 };
    let abs = x.abs();
    if abs >= 65520.0 {
        return sign * f64::INFINITY;
    }
    if abs < 2.0_f64.powi(-25) {
        return sign * 0.0;
    }
    if abs < 2.0_f64.powi(-14) {
        let scaled = abs * 2.0_f64.powi(24);
        let rounded = round_ties_to_even(scaled);
        return sign * rounded * 2.0_f64.powi(-24);
    }
    let exp = abs.log2().floor() as i32;
    let scale = 2.0_f64.powi(10 - exp);
    let rounded = round_ties_to_even(abs * scale) / scale;
    if rounded >= 65536.0 {
        sign * f64::INFINITY
    } else {
        sign * rounded
    }
}

fn round_ties_to_even(x: f64) -> f64 {
    let f = x.floor();
    let diff = x - f;
    if diff < 0.5 {
        f
    } else if diff > 0.5 {
        f + 1.0
    } else if (f % 2.0) == 0.0 {
        f
    } else {
        f + 1.0
    }
}

fn init_bigint(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.bigint_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("BigInt".into()), false, false, true),
    );
    set_method(
        rt,
        proto,
        "valueOf",
        0,
        Rc::new(|rt, this, _| match this {
            JsValue::BigInt(n) => Ok(JsValue::BigInt(n)),
            JsValue::Object(oid) => match rt.heap.get_obj(oid).kind {
                ObjectKind::BigIntObj(n) => Ok(JsValue::BigInt(n)),
                _ => Err(JsError::type_err("BigInt.prototype.valueOf requires a BigInt")),
            },
            _ => Err(JsError::type_err("BigInt.prototype.valueOf requires a BigInt")),
        }),
    );
    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, args| {
            let n = match this {
                JsValue::BigInt(b) => b,
                JsValue::Object(oid) => match rt.heap.get_obj(oid).kind {
                    ObjectKind::BigIntObj(b) => b,
                    _ => return Err(JsError::type_err("BigInt.prototype.toString requires a BigInt")),
                },
                _ => return Err(JsError::type_err("BigInt.prototype.toString requires a BigInt")),
            };
            let radix = match args.first() {
                None | Some(JsValue::Undefined) => 10,
                Some(v) => {
                    let r = to_integer_or_infinity(rt.to_number(v)?);
                    if r < 2.0 || r > 36.0 {
                        return Err(JsError::range("toString() radix must be between 2 and 36"));
                    }
                    r as u32
                }
            };
            if radix == 10 {
                return Ok(JsValue::String(n.to_string()));
            }
            Ok(JsValue::String(format_i128_radix(n, radix)))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "BigInt",
        1,
        proto,
        Rc::new(|rt, _, args| {
            if matches!(&rt.native_new_target, Some(nt) if !nt.is_undefined()) {
                return Err(JsError::type_err("BigInt is not a constructor"));
            }
            let val = args.first().unwrap_or(&JsValue::Undefined);
            let prim = rt.to_primitive(val, "number")?;
            match prim {
                JsValue::Number(n) => {
                    if !n.is_finite() || n.fract() != 0.0 {
                        return Err(JsError::range(format!(
                            "The number {} cannot be converted to a BigInt",
                            n
                        )));
                    }
                    Ok(JsValue::BigInt(n as i128))
                }
                other => Ok(JsValue::BigInt(rt.to_bigint(&other)?)),
            }
        }),
    );
    set_method(
        rt,
        ctor,
        "asIntN",
        2,
        Rc::new(|rt, _, args| {
            let bits = to_bigint_index(rt, args.first())?;
            let bi = rt.to_bigint(args.get(1).unwrap_or(&JsValue::Undefined))?;
            if bits == 0 {
                return Ok(JsValue::BigInt(0));
            }
            if bits >= 128 {
                return Ok(JsValue::BigInt(bi));
            }
            let mask = (1u128 << bits) - 1;
            let m = (bi as u128) & mask;
            let sign_bit = 1u128 << (bits - 1);
            let res = if m >= sign_bit {
                (m as i128).wrapping_sub((1u128 << bits) as i128)
            } else {
                m as i128
            };
            Ok(JsValue::BigInt(res))
        }),
    );
    set_method(
        rt,
        ctor,
        "asUintN",
        2,
        Rc::new(|rt, _, args| {
            let bits = to_bigint_index(rt, args.first())?;
            let bi = rt.to_bigint(args.get(1).unwrap_or(&JsValue::Undefined))?;
            if bits == 0 {
                return Ok(JsValue::BigInt(0));
            }
            if bits >= 128 {
                return Ok(JsValue::BigInt(bi));
            }
            let mask = (1u128 << bits) - 1;
            Ok(JsValue::BigInt(((bi as u128) & mask) as i128))
        }),
    );

    set_global(rt, "BigInt", JsValue::Object(ctor));
}

fn to_bigint_index(rt: &mut JsRuntime, arg: Option<&JsValue>) -> Result<usize, JsError> {
    match arg {
        None | Some(JsValue::Undefined) => Ok(0),
        Some(v) => {
            let n = to_integer_or_infinity(rt.to_number(v)?);
            if n < 0.0 || !n.is_finite() || n > 9_007_199_254_740_991.0 {
                return Err(JsError::range("Invalid BigInt bit width"));
            }
            Ok(n as usize)
        }
    }
}

fn format_i128_radix(n: i128, radix: u32) -> String {
    if n == 0 {
        return "0".into();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let neg = n < 0;
    let mut v = n.unsigned_abs();
    let r = radix as u128;
    let mut buf = Vec::new();
    while v > 0 {
        buf.push(digits[(v % r) as usize]);
        v /= r;
    }
    buf.reverse();
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&String::from_utf8_lossy(&buf));
    s
}

fn init_boolean(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.boolean_proto;
    set_method(
        rt,
        proto,
        "valueOf",
        0,
        Rc::new(|rt, this, _| this_bool(rt, &this).map(JsValue::Boolean)),
    );
    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let b = this_bool(rt, &this)?;
            Ok(JsValue::String(if b { "true" } else { "false" }.into()))
        }),
    );
    let ctor = rt.create_native_constructor(
        "Boolean",
        1,
        proto,
        Rc::new(|rt, this, args| {
            let is_new = matches!(
                this,
                JsValue::Object(oid) if rt.heap.get_obj(oid).prototype == Some(rt.intrinsics.boolean_proto)
            );
            let b = args.first().map(|v| v.to_boolean()).unwrap_or(false);
            if is_new {
                let obj = rt.heap.alloc_object(JsObject::new(
                    Some(rt.intrinsics.boolean_proto),
                    ObjectKind::BooleanObj(b),
                ));
                Ok(JsValue::Object(obj))
            } else {
                Ok(JsValue::Boolean(b))
            }
        }),
    );
    set_global(rt, "Boolean", JsValue::Object(ctor));
}

fn this_bool(rt: &JsRuntime, val: &JsValue) -> Result<bool, JsError> {
    match val {
        JsValue::Boolean(b) => Ok(*b),
        JsValue::Object(oid) => match rt.heap.get_obj(*oid).kind {
            ObjectKind::BooleanObj(b) => Ok(b),
            _ => Err(JsError::type_err("Boolean.prototype requires a Boolean")),
        },
        _ => Err(JsError::type_err("Boolean.prototype requires a Boolean")),
    }
}
