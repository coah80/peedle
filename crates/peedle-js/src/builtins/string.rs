use crate::builtins::{set_global, set_method, set_symbol_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;
use unicode_normalization::UnicodeNormalization;

fn require_string_this(rt: &mut JsRuntime, this: &JsValue) -> Result<String, JsError> {
    if this.is_nullish() {
        return Err(JsError::type_err(
            "String.prototype method called on null or undefined",
        ));
    }
    rt.to_string(this)
}

fn to_u16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn from_u16(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

pub fn init(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.string_proto;

    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, _| this_string_value(rt, &this).map(JsValue::String)),
    );
    set_method(
        rt,
        proto,
        "valueOf",
        0,
        Rc::new(|rt, this, _| this_string_value(rt, &this).map(JsValue::String)),
    );

    set_method(
        rt,
        proto,
        "charAt",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let pos = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let units = to_u16(&s);
            if pos < 0.0 || pos >= units.len() as f64 {
                Ok(JsValue::String(String::new()))
            } else {
                Ok(JsValue::String(from_u16(&[units[pos as usize]])))
            }
        }),
    );

    set_method(
        rt,
        proto,
        "charCodeAt",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let pos = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let units = to_u16(&s);
            if pos < 0.0 || pos >= units.len() as f64 {
                Ok(JsValue::Number(f64::NAN))
            } else {
                Ok(JsValue::Number(units[pos as usize] as f64))
            }
        }),
    );

    set_method(
        rt,
        proto,
        "codePointAt",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let pos = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let units = to_u16(&s);
            if pos < 0.0 || pos >= units.len() as f64 {
                return Ok(JsValue::Undefined);
            }
            let idx = pos as usize;
            let first = units[idx];
            if (0xD800..=0xDBFF).contains(&first) && idx + 1 < units.len() {
                let second = units[idx + 1];
                if (0xDC00..=0xDFFF).contains(&second) {
                    let cp = 0x10000
                        + (((first as u32) - 0xD800) << 10)
                        + ((second as u32) - 0xDC00);
                    return Ok(JsValue::Number(cp as f64));
                }
            }
            Ok(JsValue::Number(first as f64))
        }),
    );

    set_method(
        rt,
        proto,
        "at",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let units = to_u16(&s);
            let rel = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            let k = if rel >= 0.0 {
                rel
            } else {
                units.len() as f64 + rel
            };
            if k < 0.0 || k >= units.len() as f64 {
                Ok(JsValue::Undefined)
            } else {
                Ok(JsValue::String(from_u16(&[units[k as usize]])))
            }
        }),
    );

    set_method(
        rt,
        proto,
        "indexOf",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let search = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let units = to_u16(&s);
            let needle = to_u16(&search);
            let pos = match args.get(1) {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    n.clamp(0.0, units.len() as f64) as usize
                }
            };
            if needle.is_empty() {
                return Ok(JsValue::Number(pos as f64));
            }
            if needle.len() > units.len() {
                return Ok(JsValue::Number(-1.0));
            }
            for i in pos..=(units.len() - needle.len()) {
                if units[i..i + needle.len()] == needle[..] {
                    return Ok(JsValue::Number(i as f64));
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        proto,
        "lastIndexOf",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let search = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let units = to_u16(&s);
            let needle = to_u16(&search);
            let pos = match args.get(1) {
                None | Some(JsValue::Undefined) => units.len(),
                Some(v) => {
                    let n = rt.to_number(v)?;
                    if n.is_nan() {
                        units.len()
                    } else {
                        to_integer_or_infinity(n).clamp(0.0, units.len() as f64) as usize
                    }
                }
            };
            if needle.is_empty() {
                return Ok(JsValue::Number(pos as f64));
            }
            if needle.len() > units.len() {
                return Ok(JsValue::Number(-1.0));
            }
            let max_i = pos.min(units.len() - needle.len());
            for i in (0..=max_i).rev() {
                if units[i..i + needle.len()] == needle[..] {
                    return Ok(JsValue::Number(i as f64));
                }
            }
            Ok(JsValue::Number(-1.0))
        }),
    );

    set_method(
        rt,
        proto,
        "includes",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let search_arg = args.first().unwrap_or(&JsValue::Undefined);
            if is_regexp_like(rt, search_arg)? {
                return Err(JsError::type_err(
                    "First argument to String.prototype.includes must not be a regular expression",
                ));
            }
            let search = rt.to_string(search_arg)?;
            let units = to_u16(&s);
            let needle = to_u16(&search);
            let pos = match args.get(1) {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    to_integer_or_infinity(rt.to_number(v)?).clamp(0.0, units.len() as f64) as usize
                }
            };
            if needle.is_empty() {
                return Ok(JsValue::Boolean(true));
            }
            if pos + needle.len() > units.len() {
                return Ok(JsValue::Boolean(false));
            }
            for i in pos..=(units.len() - needle.len()) {
                if units[i..i + needle.len()] == needle[..] {
                    return Ok(JsValue::Boolean(true));
                }
            }
            Ok(JsValue::Boolean(false))
        }),
    );

    set_method(
        rt,
        proto,
        "startsWith",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let search_arg = args.first().unwrap_or(&JsValue::Undefined);
            if is_regexp_like(rt, search_arg)? {
                return Err(JsError::type_err(
                    "First argument to String.prototype.startsWith must not be a regular expression",
                ));
            }
            let search = rt.to_string(search_arg)?;
            let units = to_u16(&s);
            let needle = to_u16(&search);
            let pos = match args.get(1) {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    to_integer_or_infinity(rt.to_number(v)?).clamp(0.0, units.len() as f64) as usize
                }
            };
            if pos + needle.len() > units.len() {
                return Ok(JsValue::Boolean(false));
            }
            Ok(JsValue::Boolean(units[pos..pos + needle.len()] == needle[..]))
        }),
    );

    set_method(
        rt,
        proto,
        "endsWith",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let search_arg = args.first().unwrap_or(&JsValue::Undefined);
            if is_regexp_like(rt, search_arg)? {
                return Err(JsError::type_err(
                    "First argument to String.prototype.endsWith must not be a regular expression",
                ));
            }
            let search = rt.to_string(search_arg)?;
            let units = to_u16(&s);
            let needle = to_u16(&search);
            let end = match args.get(1) {
                None | Some(JsValue::Undefined) => units.len(),
                Some(v) => {
                    to_integer_or_infinity(rt.to_number(v)?).clamp(0.0, units.len() as f64) as usize
                }
            };
            if needle.len() > end {
                return Ok(JsValue::Boolean(false));
            }
            Ok(JsValue::Boolean(
                units[end - needle.len()..end] == needle[..],
            ))
        }),
    );

    set_method(
        rt,
        proto,
        "slice",
        2,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let units = to_u16(&s);
            let len = units.len();
            let start = match args.first() {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    if n < 0.0 {
                        (len as f64 + n).max(0.0) as usize
                    } else {
                        (n as usize).min(len)
                    }
                }
            };
            let end = match args.get(1) {
                None | Some(JsValue::Undefined) => len,
                Some(v) => {
                    let n = to_integer_or_infinity(rt.to_number(v)?);
                    if n < 0.0 {
                        (len as f64 + n).max(0.0) as usize
                    } else {
                        (n as usize).min(len)
                    }
                }
            };
            if start >= end {
                Ok(JsValue::String(String::new()))
            } else {
                Ok(JsValue::String(from_u16(&units[start..end])))
            }
        }),
    );

    set_method(
        rt,
        proto,
        "substring",
        2,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let units = to_u16(&s);
            let len = units.len();
            let s1 = match args.first() {
                None | Some(JsValue::Undefined) => 0usize,
                Some(v) => {
                    to_integer_or_infinity(rt.to_number(v)?).clamp(0.0, len as f64) as usize
                }
            };
            let s2 = match args.get(1) {
                None | Some(JsValue::Undefined) => len,
                Some(v) => {
                    to_integer_or_infinity(rt.to_number(v)?).clamp(0.0, len as f64) as usize
                }
            };
            let (from, to) = if s1 <= s2 { (s1, s2) } else { (s2, s1) };
            Ok(JsValue::String(from_u16(&units[from..to])))
        }),
    );

    set_method(
        rt,
        proto,
        "substr",
        2,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let units = to_u16(&s);
            let len = units.len();
            let raw_start = match args.first() {
                None | Some(JsValue::Undefined) => 0.0,
                Some(v) => to_integer_or_infinity(rt.to_number(v)?),
            };
            let start = if raw_start < 0.0 {
                (len as f64 + raw_start).max(0.0) as usize
            } else {
                (raw_start as usize).min(len)
            };
            let count = match args.get(1) {
                None | Some(JsValue::Undefined) => len - start,
                Some(v) => {
                    to_integer_or_infinity(rt.to_number(v)?).clamp(0.0, (len - start) as f64)
                        as usize
                }
            };
            Ok(JsValue::String(from_u16(&units[start..start + count])))
        }),
    );

    set_method(
        rt,
        proto,
        "split",
        2,
        Rc::new(|rt, this, args| {
            if this.is_nullish() {
                return Err(JsError::type_err(
                    "String.prototype.split called on null or undefined",
                ));
            }
            let sep = args.first().cloned().unwrap_or(JsValue::Undefined);
            let lim_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(sep_id) = sep {
                let sym = PropertyKey::Symbol(rt.symbols.split);
                let splitter = rt.get_property_key(sep_id, &sym)?;
                if !splitter.is_nullish() {
                    return rt.call_function(splitter, sep, &[this, lim_arg]);
                }
            }
            let s = rt.to_string(&this)?;
            let limit = if lim_arg.is_undefined() {
                u32::MAX as usize
            } else {
                to_uint32(rt.to_number(&lim_arg)?) as usize
            };
            let sep_s = rt.to_string(&sep)?;
            if limit == 0 {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            }
            if sep.is_undefined() {
                return Ok(JsValue::Object(rt.create_array(vec![JsValue::String(s)])));
            }
            let units = to_u16(&s);
            let sep_u = to_u16(&sep_s);
            let mut parts = Vec::new();
            if sep_u.is_empty() {
                for u in units.iter().take(limit) {
                    parts.push(JsValue::String(from_u16(&[*u])));
                }
            } else {
                let mut pos = 0usize;
                while parts.len() < limit {
                    if let Some(found) = find_u16(&units, &sep_u, pos) {
                        parts.push(JsValue::String(from_u16(&units[pos..found])));
                        pos = found + sep_u.len();
                    } else {
                        parts.push(JsValue::String(from_u16(&units[pos..])));
                        break;
                    }
                }
            }
            Ok(JsValue::Object(rt.create_array(parts)))
        }),
    );

    set_method(
        rt,
        proto,
        "replace",
        2,
        Rc::new(|rt, this, args| {
            if this.is_nullish() {
                return Err(JsError::type_err("String.prototype.replace called on null or undefined"));
            }
            let search = args.first().cloned().unwrap_or(JsValue::Undefined);
            let replace_val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(sid) = search {
                let sym = PropertyKey::Symbol(rt.symbols.replace);
                let replacer = rt.get_property_key(sid, &sym)?;
                if !replacer.is_nullish() {
                    return rt.call_function(replacer, search, &[this, replace_val]);
                }
            }
            let s = rt.to_string(&this)?;
            let search_s = rt.to_string(&search)?;
            let is_fn = rt.is_callable(&replace_val);
            let template = if is_fn {
                String::new()
            } else {
                rt.to_string(&replace_val)?
            };
            let units = to_u16(&s);
            let needle = to_u16(&search_s);
            let Some(pos) = find_u16(&units, &needle, 0) else {
                return Ok(JsValue::String(s));
            };
            let repl = if is_fn {
                let r = rt.call_function(
                    replace_val,
                    JsValue::Undefined,
                    &[
                        JsValue::String(search_s.clone()),
                        JsValue::Number(pos as f64),
                        JsValue::String(s.clone()),
                    ],
                )?;
                rt.to_string(&r)?
            } else {
                expand_replacement(&template, &units, pos, pos + needle.len(), &[], None)
            };
            let mut out = from_u16(&units[..pos]);
            out.push_str(&repl);
            out.push_str(&from_u16(&units[pos + needle.len()..]));
            Ok(JsValue::String(out))
        }),
    );

    set_method(
        rt,
        proto,
        "replaceAll",
        2,
        Rc::new(|rt, this, args| {
            if this.is_nullish() {
                return Err(JsError::type_err(
                    "String.prototype.replaceAll called on null or undefined",
                ));
            }
            let search = args.first().cloned().unwrap_or(JsValue::Undefined);
            let replace_val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(sid) = search {
                if is_regexp_like(rt, &search)? {
                    let flags = rt.get_property(sid, "flags")?;
                    let fs = rt.to_string(&flags)?;
                    if !fs.contains('g') {
                        return Err(JsError::type_err(
                            "String.prototype.replaceAll called with a non-global RegExp argument",
                        ));
                    }
                }
                let sym = PropertyKey::Symbol(rt.symbols.replace);
                let replacer = rt.get_property_key(sid, &sym)?;
                if !replacer.is_nullish() {
                    return rt.call_function(replacer, search, &[this, replace_val]);
                }
            }
            let s = rt.to_string(&this)?;
            let search_s = rt.to_string(&search)?;
            let is_fn = rt.is_callable(&replace_val);
            let template = if is_fn {
                String::new()
            } else {
                rt.to_string(&replace_val)?
            };
            let units = to_u16(&s);
            let needle = to_u16(&search_s);
            let mut out = String::new();
            if needle.is_empty() {
                for i in 0..=units.len() {
                    let repl = if is_fn {
                        let r = rt.call_function(
                            replace_val.clone(),
                            JsValue::Undefined,
                            &[
                                JsValue::String(String::new()),
                                JsValue::Number(i as f64),
                                JsValue::String(s.clone()),
                            ],
                        )?;
                        rt.to_string(&r)?
                    } else {
                        expand_replacement(&template, &units, i, i, &[], None)
                    };
                    out.push_str(&repl);
                    if i < units.len() {
                        out.push_str(&from_u16(&[units[i]]));
                    }
                }
                return Ok(JsValue::String(out));
            }
            let mut pos = 0usize;
            while let Some(found) = find_u16(&units, &needle, pos) {
                out.push_str(&from_u16(&units[pos..found]));
                let repl = if is_fn {
                    let r = rt.call_function(
                        replace_val.clone(),
                        JsValue::Undefined,
                        &[
                            JsValue::String(search_s.clone()),
                            JsValue::Number(found as f64),
                            JsValue::String(s.clone()),
                        ],
                    )?;
                    rt.to_string(&r)?
                } else {
                    expand_replacement(&template, &units, found, found + needle.len(), &[], None)
                };
                out.push_str(&repl);
                pos = found + needle.len();
            }
            out.push_str(&from_u16(&units[pos..]));
            Ok(JsValue::String(out))
        }),
    );

    set_method(
        rt,
        proto,
        "match",
        1,
        Rc::new(|rt, this, args| {
            if this.is_nullish() {
                return Err(JsError::type_err("String.prototype.match called on null or undefined"));
            }
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(aid) = arg {
                let sym = PropertyKey::Symbol(rt.symbols.match_sym);
                let matcher = rt.get_property_key(aid, &sym)?;
                if !matcher.is_nullish() {
                    return rt.call_function(matcher, arg, &[this]);
                }
            }
            let s = rt.to_string(&this)?;
            let pat = if arg.is_undefined() {
                String::new()
            } else {
                rt.to_string(&arg)?
            };
            let re = rt.create_regexp_object(&pat, "")?;
            let sym = PropertyKey::Symbol(rt.symbols.match_sym);
            let matcher = rt.get_property_key(re, &sym)?;
            rt.call_function(matcher, JsValue::Object(re), &[JsValue::String(s)])
        }),
    );

    set_method(
        rt,
        proto,
        "matchAll",
        1,
        Rc::new(|rt, this, args| {
            if this.is_nullish() {
                return Err(JsError::type_err(
                    "String.prototype.matchAll called on null or undefined",
                ));
            }
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(aid) = arg {
                if is_regexp_like(rt, &arg)? {
                    let flags = rt.get_property(aid, "flags")?;
                    if !rt.to_string(&flags)?.contains('g') {
                        return Err(JsError::type_err(
                            "String.prototype.matchAll called with a non-global RegExp argument",
                        ));
                    }
                }
                let sym = PropertyKey::Symbol(rt.symbols.match_all);
                let matcher = rt.get_property_key(aid, &sym)?;
                if !matcher.is_nullish() {
                    return rt.call_function(matcher, arg, &[this]);
                }
            }
            let s = rt.to_string(&this)?;
            let pat = if arg.is_undefined() {
                String::new()
            } else {
                rt.to_string(&arg)?
            };
            let re = rt.create_regexp_object(&pat, "g")?;
            let sym = PropertyKey::Symbol(rt.symbols.match_all);
            let matcher = rt.get_property_key(re, &sym)?;
            rt.call_function(matcher, JsValue::Object(re), &[JsValue::String(s)])
        }),
    );

    set_method(
        rt,
        proto,
        "search",
        1,
        Rc::new(|rt, this, args| {
            if this.is_nullish() {
                return Err(JsError::type_err("String.prototype.search called on null or undefined"));
            }
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(aid) = arg {
                let sym = PropertyKey::Symbol(rt.symbols.search);
                let searcher = rt.get_property_key(aid, &sym)?;
                if !searcher.is_nullish() {
                    return rt.call_function(searcher, arg, &[this]);
                }
            }
            let s = rt.to_string(&this)?;
            let pat = if arg.is_undefined() {
                String::new()
            } else {
                rt.to_string(&arg)?
            };
            let re = rt.create_regexp_object(&pat, "")?;
            let sym = PropertyKey::Symbol(rt.symbols.search);
            let searcher = rt.get_property_key(re, &sym)?;
            rt.call_function(searcher, JsValue::Object(re), &[JsValue::String(s)])
        }),
    );

    set_method(
        rt,
        proto,
        "toLowerCase",
        0,
        Rc::new(|rt, this, _| Ok(JsValue::String(require_string_this(rt, &this)?.to_lowercase()))),
    );
    set_method(
        rt,
        proto,
        "toLocaleLowerCase",
        0,
        Rc::new(|rt, this, _| Ok(JsValue::String(require_string_this(rt, &this)?.to_lowercase()))),
    );
    set_method(
        rt,
        proto,
        "toUpperCase",
        0,
        Rc::new(|rt, this, _| Ok(JsValue::String(require_string_this(rt, &this)?.to_uppercase()))),
    );
    set_method(
        rt,
        proto,
        "toLocaleUpperCase",
        0,
        Rc::new(|rt, this, _| Ok(JsValue::String(require_string_this(rt, &this)?.to_uppercase()))),
    );
    set_method(
        rt,
        proto,
        "trim",
        0,
        Rc::new(|rt, this, _| {
            let s = require_string_this(rt, &this)?;
            Ok(JsValue::String(trim_js_whitespace(&s).to_string()))
        }),
    );
    let trim_start = rt.create_native_function(
        "trimStart",
        0,
        Rc::new(|rt, this, _| {
            let s = require_string_this(rt, &this)?;
            Ok(JsValue::String(s.trim_start_matches(is_js_whitespace).to_string()))
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("trimStart"),
        PropEntry::data(JsValue::Object(trim_start), true, false, true),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("trimLeft"),
        PropEntry::data(JsValue::Object(trim_start), true, false, true),
    );
    let trim_end = rt.create_native_function(
        "trimEnd",
        0,
        Rc::new(|rt, this, _| {
            let s = require_string_this(rt, &this)?;
            Ok(JsValue::String(s.trim_end_matches(is_js_whitespace).to_string()))
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("trimEnd"),
        PropEntry::data(JsValue::Object(trim_end), true, false, true),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("trimRight"),
        PropEntry::data(JsValue::Object(trim_end), true, false, true),
    );

    set_method(
        rt,
        proto,
        "padStart",
        1,
        Rc::new(|rt, this, args| pad_string(rt, &this, args, true)),
    );
    set_method(
        rt,
        proto,
        "padEnd",
        1,
        Rc::new(|rt, this, args| pad_string(rt, &this, args, false)),
    );

    set_method(
        rt,
        proto,
        "repeat",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let n = to_integer_or_infinity(
                rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?,
            );
            if n < 0.0 || n.is_infinite() {
                return Err(JsError::range("Invalid count value in String.prototype.repeat"));
            }
            let count = n as usize;
            if count == 0 || s.is_empty() {
                return Ok(JsValue::String(String::new()));
            }
            if count.saturating_mul(s.len()) > 64 * 1024 * 1024 {
                return Err(JsError::range("String repeat result too large"));
            }
            Ok(JsValue::String(s.repeat(count)))
        }),
    );

    set_method(
        rt,
        proto,
        "concat",
        1,
        Rc::new(|rt, this, args| {
            let mut out = require_string_this(rt, &this)?;
            for a in args {
                out.push_str(&rt.to_string(a)?);
            }
            Ok(JsValue::String(out))
        }),
    );

    set_method(
        rt,
        proto,
        "normalize",
        0,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let form = match args.first() {
                None | Some(JsValue::Undefined) => "NFC".to_string(),
                Some(v) => rt.to_string(v)?,
            };
            let res: String = match form.as_str() {
                "NFC" => s.nfc().collect(),
                "NFD" => s.nfd().collect(),
                "NFKC" => s.nfkc().collect(),
                "NFKD" => s.nfkd().collect(),
                _ => {
                    return Err(JsError::range(format!(
                        "The normalization form should be one of NFC, NFD, NFKC, NFKD (got {})",
                        form
                    )));
                }
            };
            Ok(JsValue::String(res))
        }),
    );

    set_method(
        rt,
        proto,
        "localeCompare",
        1,
        Rc::new(|rt, this, args| {
            let s = require_string_this(rt, &this)?;
            let that = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let ord = match s.nfc().cmp(that.nfc()) {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
            };
            Ok(JsValue::Number(ord))
        }),
    );

    set_method(
        rt,
        proto,
        "isWellFormed",
        0,
        Rc::new(|rt, this, _| {
            let _ = require_string_this(rt, &this)?;
            Ok(JsValue::Boolean(true))
        }),
    );
    set_method(
        rt,
        proto,
        "toWellFormed",
        0,
        Rc::new(|rt, this, _| Ok(JsValue::String(require_string_this(rt, &this)?))),
    );

    let sym_iter = rt.symbols.iterator;
    set_symbol_method(
        rt,
        proto,
        sym_iter,
        "[Symbol.iterator]",
        0,
        Rc::new(|rt, this, _| {
            let s = require_string_this(rt, &this)?;
            let it = rt.create_builtin_iterator(BuiltinIteratorKind::String {
                units: to_u16(&s),
            });
            Ok(JsValue::Object(it))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "String",
        1,
        proto,
        Rc::new(|rt, _, args| {
            let new_target = rt.native_new_target.clone();
            let is_new = matches!(&new_target, Some(nt) if !nt.is_undefined());
            let s = match args.first() {
                None => String::new(),
                Some(JsValue::Symbol(id)) if !is_new => {
                    let desc = rt.heap.symbol_description(*id).unwrap_or("");
                    format!("Symbol({})", desc)
                }
                Some(v) => rt.to_string(v)?,
            };
            if is_new {
                let target_proto = if let Some(nt) = new_target {
                    rt.get_prototype_from_constructor(&nt, rt.intrinsics.string_proto)?
                } else {
                    rt.intrinsics.string_proto
                };
                let utf16_len = s.encode_utf16().count() as f64;
                let obj = rt.heap.alloc_object(JsObject::new(
                    Some(target_proto),
                    ObjectKind::StringObj(s),
                ));
                rt.heap.get_obj_mut(obj).set_own_prop(
                    PropertyKey::from("length"),
                    PropEntry::data(JsValue::Number(utf16_len), false, false, false),
                );
                Ok(JsValue::Object(obj))
            } else {
                Ok(JsValue::String(s))
            }
        }),
    );

    set_method(
        rt,
        ctor,
        "fromCharCode",
        1,
        Rc::new(|rt, _, args| {
            let mut units = Vec::with_capacity(args.len());
            for a in args {
                let n = to_uint32(rt.to_number(a)?) as u16;
                units.push(n);
            }
            Ok(JsValue::String(from_u16(&units)))
        }),
    );

    set_method(
        rt,
        ctor,
        "fromCodePoint",
        1,
        Rc::new(|rt, _, args| {
            let mut out = String::new();
            for a in args {
                let n = rt.to_number(a)?;
                if !n.is_finite() || n.fract() != 0.0 || n < 0.0 || n > 0x10FFFF as f64 {
                    return Err(JsError::range(format!("Invalid code point {}", n)));
                }
                if let Some(ch) = char::from_u32(n as u32) {
                    out.push(ch);
                } else {
                    out.push('\u{FFFD}');
                }
            }
            Ok(JsValue::String(out))
        }),
    );

    set_method(
        rt,
        ctor,
        "raw",
        1,
        Rc::new(|rt, _, args| {
            let tmpl = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let raw_v = rt.get_property(tmpl, "raw")?;
            let raw_id = rt.to_object(&raw_v)?;
            let len_v = rt.get_property(raw_id, "length")?;
            let len = to_length(rt.to_number(&len_v)?);
            if len == 0 {
                return Ok(JsValue::String(String::new()));
            }
            let mut out = String::new();
            for i in 0..len {
                let seg = rt.get_property(raw_id, &i.to_string())?;
                out.push_str(&rt.to_string(&seg)?);
                if i + 1 < len {
                    if let Some(sub) = args.get(i + 1) {
                        out.push_str(&rt.to_string(sub)?);
                    }
                }
            }
            Ok(JsValue::String(out))
        }),
    );

    set_global(rt, "String", JsValue::Object(ctor));
}

fn this_string_value(rt: &JsRuntime, val: &JsValue) -> Result<String, JsError> {
    match val {
        JsValue::String(s) => Ok(s.clone()),
        JsValue::Object(oid) => {
            if let ObjectKind::StringObj(ref s) = rt.heap.get_obj(*oid).kind {
                Ok(s.clone())
            } else {
                Err(JsError::type_err(
                    "String.prototype.valueOf requires that 'this' be a String",
                ))
            }
        }
        _ => Err(JsError::type_err(
            "String.prototype.valueOf requires that 'this' be a String",
        )),
    }
}

fn pad_string(
    rt: &mut JsRuntime,
    this: &JsValue,
    args: &[JsValue],
    start: bool,
) -> Result<JsValue, JsError> {
    let s = require_string_this(rt, this)?;
    let units = to_u16(&s);
    let max_len = to_length(rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?);
    if max_len <= units.len() {
        return Ok(JsValue::String(s));
    }
    let filler = match args.get(1) {
        None | Some(JsValue::Undefined) => " ".to_string(),
        Some(v) => rt.to_string(v)?,
    };
    let fill_u = to_u16(&filler);
    if fill_u.is_empty() {
        return Ok(JsValue::String(s));
    }
    let needed = max_len - units.len();
    let mut pad = Vec::with_capacity(needed);
    while pad.len() < needed {
        let rem = needed - pad.len();
        pad.extend_from_slice(&fill_u[..rem.min(fill_u.len())]);
    }
    let mut res = Vec::with_capacity(max_len);
    if start {
        res.extend_from_slice(&pad);
        res.extend_from_slice(&units);
    } else {
        res.extend_from_slice(&units);
        res.extend_from_slice(&pad);
    }
    Ok(JsValue::String(from_u16(&res)))
}

fn is_regexp_like(rt: &mut JsRuntime, val: &JsValue) -> Result<bool, JsError> {
    let JsValue::Object(oid) = val else {
        return Ok(false);
    };
    let sym = PropertyKey::Symbol(rt.symbols.match_sym);
    let m = rt.get_property_key(*oid, &sym)?;
    if !m.is_undefined() {
        return Ok(m.to_boolean());
    }
    Ok(matches!(rt.heap.get_obj(*oid).kind, ObjectKind::RegExp(_)))
}

fn find_u16(haystack: &[u16], needle: &[u16], start: usize) -> Option<usize> {
    if needle.is_empty() {
        return if start <= haystack.len() {
            Some(start)
        } else {
            None
        };
    }
    if start + needle.len() > haystack.len() {
        return None;
    }
    for i in start..=(haystack.len() - needle.len()) {
        if haystack[i..i + needle.len()] == needle[..] {
            return Some(i);
        }
    }
    None
}

pub fn expand_replacement(
    template: &str,
    input: &[u16],
    match_start: usize,
    match_end: usize,
    captures: &[Option<String>],
    named: Option<&std::collections::HashMap<String, Option<String>>>,
) -> String {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() {
            match chars[i + 1] {
                '$' => {
                    out.push('$');
                    i += 2;
                    continue;
                }
                '&' => {
                    out.push_str(&from_u16(&input[match_start..match_end]));
                    i += 2;
                    continue;
                }
                '`' => {
                    out.push_str(&from_u16(&input[..match_start]));
                    i += 2;
                    continue;
                }
                '\'' => {
                    out.push_str(&from_u16(&input[match_end..]));
                    i += 2;
                    continue;
                }
                '<' if named.is_some() => {
                    if let Some(close) = chars[i + 2..].iter().position(|&c| c == '>') {
                        let name: String = chars[i + 2..i + 2 + close].iter().collect();
                        if let Some(Some(val)) = named.and_then(|m| m.get(&name)) {
                            out.push_str(val);
                        }
                        i = i + 2 + close + 1;
                        continue;
                    }
                }
                c if c.is_ascii_digit() => {
                    let d1 = (c as usize) - ('0' as usize);
                    if i + 2 < chars.len() && chars[i + 2].is_ascii_digit() {
                        let d2 = d1 * 10 + ((chars[i + 2] as usize) - ('0' as usize));
                        if d2 >= 1 && d2 <= captures.len() {
                            if let Some(ref s) = captures[d2 - 1] {
                                out.push_str(s);
                            }
                            i += 3;
                            continue;
                        }
                    }
                    if d1 >= 1 && d1 <= captures.len() {
                        if let Some(ref s) = captures[d1 - 1] {
                            out.push_str(s);
                        }
                        i += 2;
                        continue;
                    }
                }
                _ => {}
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn expand_replacement_rt(
    rt: &mut JsRuntime,
    template: &str,
    matched: &str,
    input: &[u16],
    match_start: usize,
    match_end: usize,
    captures: &[Option<String>],
    groups_obj: Option<ObjectId>,
) -> Result<String, JsError> {
    let chars: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() {
            match chars[i + 1] {
                '$' => {
                    out.push('$');
                    i += 2;
                    continue;
                }
                '&' => {
                    out.push_str(matched);
                    i += 2;
                    continue;
                }
                '`' => {
                    out.push_str(&from_u16(&input[..match_start]));
                    i += 2;
                    continue;
                }
                '\'' => {
                    out.push_str(&from_u16(&input[match_end..]));
                    i += 2;
                    continue;
                }
                '<' if groups_obj.is_some() => {
                    if let Some(close) = chars[i + 2..].iter().position(|&c| c == '>') {
                        let name: String = chars[i + 2..i + 2 + close].iter().collect();
                        let gid = groups_obj.unwrap();
                        let v = rt.get_property(gid, &name)?;
                        if !v.is_undefined() {
                            out.push_str(&rt.to_string(&v)?);
                        }
                        i = i + 2 + close + 1;
                        continue;
                    }
                }
                c if c.is_ascii_digit() => {
                    let d1 = (c as usize) - ('0' as usize);
                    if i + 2 < chars.len() && chars[i + 2].is_ascii_digit() {
                        let d2 = d1 * 10 + ((chars[i + 2] as usize) - ('0' as usize));
                        if d2 >= 1 && d2 <= captures.len() {
                            if let Some(ref s) = captures[d2 - 1] {
                                out.push_str(s);
                            }
                            i += 3;
                            continue;
                        }
                    }
                    if d1 >= 1 && d1 <= captures.len() {
                        if let Some(ref s) = captures[d1 - 1] {
                            out.push_str(s);
                        }
                        i += 2;
                        continue;
                    }
                }
                _ => {}
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    Ok(out)
}
