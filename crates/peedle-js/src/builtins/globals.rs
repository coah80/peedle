use crate::builtins::typed_array::{alloc_typed_array, typed_array_proto_for};
use crate::builtins::{set_getter, set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use base64::Engine;
use std::collections::HashMap;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    let g = rt.global_object;
    rt.heap.get_obj_mut(g).set_own_prop(
        PropertyKey::from("undefined"),
        PropEntry::data(JsValue::Undefined, false, false, false),
    );
    rt.heap.get_obj_mut(g).set_own_prop(
        PropertyKey::from("NaN"),
        PropEntry::data(JsValue::Number(f64::NAN), false, false, false),
    );
    rt.heap.get_obj_mut(g).set_own_prop(
        PropertyKey::from("Infinity"),
        PropEntry::data(JsValue::Number(f64::INFINITY), false, false, false),
    );
    set_global(rt, "globalThis", JsValue::Object(g));

    let eval_fn = rt.create_native_function(
        "eval",
        1,
        Rc::new(|rt, _, args| {
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            match arg {
                JsValue::String(s) => rt.eval_in_env(&s, rt.global_env, false),
                other => Ok(other),
            }
        }),
    );
    rt.intrinsics.eval_fn = eval_fn;
    set_global(rt, "eval", JsValue::Object(eval_fn));

    let pi = rt.create_native_function("parseInt", 2, Rc::new(|rt, _, args| parse_int_impl(rt, args)));
    set_global(rt, "parseInt", JsValue::Object(pi));

    let pf = rt.create_native_function(
        "parseFloat",
        1,
        Rc::new(|rt, _, args| parse_float_impl(rt, args)),
    );
    set_global(rt, "parseFloat", JsValue::Object(pf));

    if let Ok(JsValue::Object(num_ctor)) = rt.get_property(g, "Number") {
        rt.heap.get_obj_mut(num_ctor).set_own_prop(
            PropertyKey::from("parseInt"),
            PropEntry::data(JsValue::Object(pi), true, false, true),
        );
        rt.heap.get_obj_mut(num_ctor).set_own_prop(
            PropertyKey::from("parseFloat"),
            PropEntry::data(JsValue::Object(pf), true, false, true),
        );
    }

    let isn = rt.create_native_function(
        "isNaN",
        1,
        Rc::new(|rt, _, args| {
            let n = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Boolean(n.is_nan()))
        }),
    );
    set_global(rt, "isNaN", JsValue::Object(isn));

    let isf = rt.create_native_function(
        "isFinite",
        1,
        Rc::new(|rt, _, args| {
            let n = rt.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Boolean(n.is_finite()))
        }),
    );
    set_global(rt, "isFinite", JsValue::Object(isf));

    let enc_uri = rt.create_native_function(
        "encodeURI",
        1,
        Rc::new(|rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::String(encode_uri_impl(&s, false)))
        }),
    );
    set_global(rt, "encodeURI", JsValue::Object(enc_uri));

    let enc_comp = rt.create_native_function(
        "encodeURIComponent",
        1,
        Rc::new(|rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::String(encode_uri_impl(&s, true)))
        }),
    );
    set_global(rt, "encodeURIComponent", JsValue::Object(enc_comp));

    let dec_uri = rt.create_native_function(
        "decodeURI",
        1,
        Rc::new(|rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::String(decode_uri_impl(&s, false)?))
        }),
    );
    set_global(rt, "decodeURI", JsValue::Object(dec_uri));

    let dec_comp = rt.create_native_function(
        "decodeURIComponent",
        1,
        Rc::new(|rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::String(decode_uri_impl(&s, true)?))
        }),
    );
    set_global(rt, "decodeURIComponent", JsValue::Object(dec_comp));

    let btoa_fn = rt.create_native_function(
        "btoa",
        1,
        Rc::new(|rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let mut bytes = Vec::with_capacity(s.len());
            for ch in s.chars() {
                if (ch as u32) > 0xFF {
                    return Err(JsError::error("btoa: string contains non-Latin1 character"));
                }
                bytes.push(ch as u8);
            }
            Ok(JsValue::String(
                base64::engine::general_purpose::STANDARD.encode(&bytes),
            ))
        }),
    );
    set_global(rt, "btoa", JsValue::Object(btoa_fn));

    let atob_fn = rt.create_native_function(
        "atob",
        1,
        Rc::new(|rt, _, args| {
            let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&cleaned)
                .map_err(|_| JsError::error("atob: invalid base64 string"))?;
            let out: String = decoded.into_iter().map(|b| b as char).collect();
            Ok(JsValue::String(out))
        }),
    );
    set_global(rt, "atob", JsValue::Object(atob_fn));

    let qm = rt.create_native_function(
        "queueMicrotask",
        1,
        Rc::new(|rt, _, args| {
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("queueMicrotask requires a callable callback"));
            }
            rt.queue_microtask(cb);
            Ok(JsValue::Undefined)
        }),
    );
    set_global(rt, "queueMicrotask", JsValue::Object(qm));

    let sc = rt.create_native_function(
        "structuredClone",
        1,
        Rc::new(|rt, _, args| {
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            let mut seen = HashMap::new();
            structured_clone_impl(rt, &val, &mut seen)
        }),
    );
    set_global(rt, "structuredClone", JsValue::Object(sc));

    init_symbol(rt);
    init_console(rt);
    init_text_codec(rt);
}

fn init_symbol(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.symbol_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("Symbol".into()), false, false, true),
    );
    set_getter(
        rt,
        proto,
        "description",
        Rc::new(|rt, this, _| {
            let sid = this_symbol(rt, &this)?;
            Ok(rt
                .heap
                .symbol_description(sid)
                .map(|s| JsValue::String(s.to_string()))
                .unwrap_or(JsValue::Undefined))
        }),
    );
    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let sid = this_symbol(rt, &this)?;
            let desc = rt.heap.symbol_description(sid).unwrap_or("");
            Ok(JsValue::String(format!("Symbol({})", desc)))
        }),
    );
    set_method(
        rt,
        proto,
        "valueOf",
        0,
        Rc::new(|rt, this, _| this_symbol(rt, &this).map(JsValue::Symbol)),
    );
    let to_prim_sym = rt.symbols.to_primitive;
    let to_prim_fn = rt.create_native_function(
        "[Symbol.toPrimitive]",
        1,
        Rc::new(|rt, this, _| this_symbol(rt, &this).map(JsValue::Symbol)),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(to_prim_sym),
        PropEntry::data(JsValue::Object(to_prim_fn), false, false, true),
    );

    let sym_fn = rt.create_spec_constructor(
        "Symbol",
        0,
        proto,
        Rc::new(|rt, _, args| {
            if matches!(&rt.native_new_target, Some(nt) if !nt.is_undefined()) {
                return Err(JsError::type_err("Symbol is not a constructor"));
            }
            let desc = match args.first() {
                None | Some(JsValue::Undefined) => None,
                Some(v) => Some(rt.to_string(v)?),
            };
            Ok(JsValue::Symbol(rt.heap.create_symbol(desc)))
        }),
    );

    let well_known = [
        ("iterator", rt.symbols.iterator),
        ("asyncIterator", rt.symbols.async_iterator),
        ("toStringTag", rt.symbols.to_string_tag),
        ("toPrimitive", rt.symbols.to_primitive),
        ("hasInstance", rt.symbols.has_instance),
        ("isConcatSpreadable", rt.symbols.is_concat_spreadable),
        ("species", rt.symbols.species),
        ("match", rt.symbols.match_sym),
        ("matchAll", rt.symbols.match_all),
        ("replace", rt.symbols.replace),
        ("search", rt.symbols.search),
        ("split", rt.symbols.split),
        ("unscopables", rt.symbols.unscopables),
        ("dispose", rt.symbols.dispose),
        ("asyncDispose", rt.symbols.async_dispose),
    ];
    for (name, id) in well_known {
        rt.heap.get_obj_mut(sym_fn).set_own_prop(
            PropertyKey::from(name),
            PropEntry::data(JsValue::Symbol(id), false, false, false),
        );
    }

    set_method(
        rt,
        sym_fn,
        "for",
        1,
        Rc::new(|rt, _, args| {
            let key = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Symbol(rt.heap.symbol_for(&key)))
        }),
    );
    set_method(
        rt,
        sym_fn,
        "keyFor",
        1,
        Rc::new(|rt, _, args| {
            let Some(JsValue::Symbol(id)) = args.first() else {
                return Err(JsError::type_err("Symbol.keyFor requires a symbol"));
            };
            Ok(rt
                .heap
                .symbol_key_for(*id)
                .map(JsValue::String)
                .unwrap_or(JsValue::Undefined))
        }),
    );

    set_global(rt, "Symbol", JsValue::Object(sym_fn));
}

fn this_symbol(rt: &JsRuntime, val: &JsValue) -> Result<u64, JsError> {
    match val {
        JsValue::Symbol(id) => Ok(*id),
        JsValue::Object(oid) => match rt.heap.get_obj(*oid).kind {
            ObjectKind::SymbolObj(id) => Ok(id),
            _ => Err(JsError::type_err("Symbol.prototype method requires a Symbol")),
        },
        _ => Err(JsError::type_err("Symbol.prototype method requires a Symbol")),
    }
}

fn init_console(rt: &mut JsRuntime) {
    let console = rt.create_object();
    for method in ["log", "info", "warn", "error", "debug"] {
        set_method(
            rt,
            console,
            method,
            0,
            Rc::new(|rt, _, args| {
                let mut parts = Vec::with_capacity(args.len());
                for a in args {
                    parts.push(rt.to_display_string(a));
                }
                rt.console_lines.push(parts.join(" "));
                Ok(JsValue::Undefined)
            }),
        );
    }
    set_method(
        rt,
        console,
        "assert",
        1,
        Rc::new(|rt, _, args| {
            let cond = args.first().map(|v| v.to_boolean()).unwrap_or(false);
            if !cond {
                let mut parts = vec!["Assertion failed".to_string()];
                for a in args.iter().skip(1) {
                    parts.push(rt.to_display_string(a));
                }
                rt.console_lines.push(parts.join(": "));
            }
            Ok(JsValue::Undefined)
        }),
    );
    set_global(rt, "console", JsValue::Object(console));
}

fn init_text_codec(rt: &mut JsRuntime) {
    let enc_proto = rt.create_object();
    set_method(
        rt,
        enc_proto,
        "encode",
        1,
        Rc::new(|rt, _, args| {
            let s = match args.first() {
                None | Some(JsValue::Undefined) => String::new(),
                Some(v) => rt.to_string(v)?,
            };
            let bytes = s.into_bytes();
            let len = bytes.len();
            let buf = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.array_buffer_proto),
                ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes,
                    detached: false,
                }),
            ));
            let u8_proto = typed_array_proto_for(rt, TypedArrayKind::Uint8);
            let ta = rt.heap.alloc_object(JsObject::new(
                Some(u8_proto),
                ObjectKind::TypedArray(TypedArrayData {
                    kind: TypedArrayKind::Uint8,
                    buffer: buf,
                    byte_offset: 0,
                    length: len,
                }),
            ));
            Ok(JsValue::Object(ta))
        }),
    );
    let enc_ctor = rt.create_native_constructor(
        "TextEncoder",
        0,
        enc_proto,
        Rc::new(move |rt, this, _| {
            if let JsValue::Object(oid) = this {
                let _ = rt.set_property(oid, "encoding", JsValue::String("utf-8".into()));
                return Ok(JsValue::Object(oid));
            }
            let obj = rt.create_object_with_proto(Some(enc_proto));
            let _ = rt.set_property(obj, "encoding", JsValue::String("utf-8".into()));
            Ok(JsValue::Object(obj))
        }),
    );
    set_global(rt, "TextEncoder", JsValue::Object(enc_ctor));

    let dec_proto = rt.create_object();
    set_method(
        rt,
        dec_proto,
        "decode",
        1,
        Rc::new(|rt, _, args| {
            let Some(JsValue::Object(oid)) = args.first() else {
                return Ok(JsValue::String(String::new()));
            };
            let bytes = match &rt.heap.get_obj(*oid).kind {
                ObjectKind::ArrayBuffer(ab) => {
                    if let Some(shared) = rt.shared_buffer_backing(*oid) {
                        shared.data.lock().unwrap().clone()
                    } else {
                        ab.bytes.clone()
                    }
                }
                ObjectKind::TypedArray(ta) => {
                    let es = ta.kind.element_size();
                    let off = ta.byte_offset;
                    let len = ta.length * es;
                    if let Some(shared) = rt.shared_buffer_backing(ta.buffer) {
                        let guard = shared.data.lock().unwrap();
                        guard[off..(off + len).min(guard.len())].to_vec()
                    } else if let ObjectKind::ArrayBuffer(ref ab) = rt.heap.get_obj(ta.buffer).kind {
                        ab.bytes[off..(off + len).min(ab.bytes.len())].to_vec()
                    } else {
                        Vec::new()
                    }
                }
                ObjectKind::DataView(dv) => {
                    if let Some(shared) = rt.shared_buffer_backing(dv.buffer) {
                        let guard = shared.data.lock().unwrap();
                        guard[dv.byte_offset..(dv.byte_offset + dv.byte_length).min(guard.len())].to_vec()
                    } else if let ObjectKind::ArrayBuffer(ref ab) = rt.heap.get_obj(dv.buffer).kind {
                        ab.bytes[dv.byte_offset..(dv.byte_offset + dv.byte_length).min(ab.bytes.len())]
                            .to_vec()
                    } else {
                        Vec::new()
                    }
                }
                _ => return Err(JsError::type_err("TextDecoder.decode requires a BufferSource")),
            };
            Ok(JsValue::String(String::from_utf8_lossy(&bytes).into_owned()))
        }),
    );
    let dec_ctor = rt.create_native_constructor(
        "TextDecoder",
        0,
        dec_proto,
        Rc::new(move |rt, this, args| {
            let enc = match args.first() {
                None | Some(JsValue::Undefined) => "utf-8".to_string(),
                Some(v) => rt.to_string(v)?.to_lowercase(),
            };
            if let JsValue::Object(oid) = this {
                let _ = rt.set_property(oid, "encoding", JsValue::String(enc));
                return Ok(JsValue::Object(oid));
            }
            let obj = rt.create_object_with_proto(Some(dec_proto));
            let _ = rt.set_property(obj, "encoding", JsValue::String(enc));
            Ok(JsValue::Object(obj))
        }),
    );
    set_global(rt, "TextDecoder", JsValue::Object(dec_ctor));
}

pub fn parse_int_impl(rt: &mut JsRuntime, args: &[JsValue]) -> Result<JsValue, JsError> {
    let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let mut trimmed = trim_js_whitespace(&s);
    let mut sign = 1.0_f64;
    if let Some(rest) = trimmed.strip_prefix('-') {
        sign = -1.0;
        trimmed = rest;
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        trimmed = rest;
    }
    let mut radix = match args.get(1) {
        None | Some(JsValue::Undefined) => 0i32,
        Some(v) => to_int32(rt.to_number(v)?),
    };
    let mut strip_prefix = true;
    if radix != 0 {
        if !(2..=36).contains(&radix) {
            return Ok(JsValue::Number(f64::NAN));
        }
        if radix != 16 {
            strip_prefix = false;
        }
    } else {
        radix = 10;
    }
    if strip_prefix {
        if let Some(rest) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
            trimmed = rest;
            radix = 16;
        }
    }
    let mut acc = 0.0_f64;
    let mut count = 0usize;
    let r = radix as u32;
    for b in trimmed.bytes() {
        let d = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'z' => (b - b'a' + 10) as u32,
            b'A'..=b'Z' => (b - b'A' + 10) as u32,
            _ => break,
        };
        if d >= r {
            break;
        }
        acc = acc * (r as f64) + d as f64;
        count += 1;
    }
    if count == 0 {
        Ok(JsValue::Number(f64::NAN))
    } else {
        Ok(JsValue::Number(sign * acc))
    }
}

pub fn parse_float_impl(rt: &mut JsRuntime, args: &[JsValue]) -> Result<JsValue, JsError> {
    let s = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let t = trim_js_whitespace(&s);
    if t.starts_with("Infinity") || t.starts_with("+Infinity") {
        return Ok(JsValue::Number(f64::INFINITY));
    }
    if t.starts_with("-Infinity") {
        return Ok(JsValue::Number(f64::NEG_INFINITY));
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    if matches!(bytes.get(i), Some(b'+' | b'-')) {
        i += 1;
    }
    let mut has_digits = false;
    while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
        has_digits = true;
        i += 1;
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            has_digits = true;
            i += 1;
        }
    }
    if !has_digits {
        return Ok(JsValue::Number(f64::NAN));
    }
    let end_before_exp = i;
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let exp_start = i;
        while bytes.get(i).is_some_and(|b| b.is_ascii_digit()) {
            i += 1;
        }
        if i == exp_start {
            i = end_before_exp;
        }
    }
    Ok(JsValue::Number(t[..i].parse::<f64>().unwrap_or(f64::NAN)))
}

fn encode_uri_impl(s: &str, component: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let keep = ch.is_ascii_alphanumeric()
            || matches!(ch, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')')
            || (!component
                && matches!(
                    ch,
                    ';' | '/' | '?' | ':' | '@' | '&' | '=' | '+' | '$' | ',' | '#'
                ));
        if keep {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

fn decode_uri_impl(s: &str, component: bool) -> Result<String, JsError> {
    let bytes = s.as_bytes();
    let mut raw: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(JsError::uri("URI malformed"));
            }
            let h1 = (bytes[i + 1] as char)
                .to_digit(16)
                .ok_or_else(|| JsError::uri("URI malformed"))?;
            let h2 = (bytes[i + 2] as char)
                .to_digit(16)
                .ok_or_else(|| JsError::uri("URI malformed"))?;
            let byte = ((h1 << 4) | h2) as u8;
            let reserved = matches!(
                byte,
                b';' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',' | b'#'
            );
            if !component && reserved {
                raw.extend_from_slice(&bytes[i..i + 3]);
            } else {
                raw.push(byte);
            }
            i += 3;
        } else {
            raw.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(raw).map_err(|_| JsError::uri("URI malformed"))
}

fn structured_clone_impl(
    rt: &mut JsRuntime,
    val: &JsValue,
    seen: &mut HashMap<ObjectId, ObjectId>,
) -> Result<JsValue, JsError> {
    let JsValue::Object(oid) = val else {
        return Ok(val.clone());
    };
    if let Some(&cloned) = seen.get(oid) {
        return Ok(JsValue::Object(cloned));
    }
    let kind_clone = match &rt.heap.get_obj(*oid).kind {
        ObjectKind::Function(_) | ObjectKind::NativeFunction(_) | ObjectKind::Promise(_) => {
            return Err(JsError::error("structuredClone: value cannot be cloned"));
        }
        ObjectKind::Date(ms) => {
            let c = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.date_proto),
                ObjectKind::Date(*ms),
            ));
            seen.insert(*oid, c);
            return Ok(JsValue::Object(c));
        }
        ObjectKind::RegExp(re) => {
            let src = re.original_source.clone();
            let fl = re.compiled.flags.canonical_string();
            let c = rt.create_regexp_object(&src, &fl)?;
            seen.insert(*oid, c);
            return Ok(JsValue::Object(c));
        }
        ObjectKind::ArrayBuffer(ab) => {
            if let Some(shared) = rt.shared_buffer_backing(*oid) {
                let c = rt.create_shared_array_buffer_from_backing(shared);
                seen.insert(*oid, c);
                return Ok(JsValue::Object(c));
            }
            let c = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.array_buffer_proto),
                ObjectKind::ArrayBuffer(ab.clone()),
            ));
            seen.insert(*oid, c);
            return Ok(JsValue::Object(c));
        }
        ObjectKind::TypedArray(ta) => {
            let ta_c = ta.clone();
            let c = alloc_typed_array(rt, ta_c.kind, ta_c.length)?;
            let src_bytes = if let Some(shared) = rt.shared_buffer_backing(ta_c.buffer) {
                let guard = shared.data.lock().unwrap();
                let len = ta_c.length * ta_c.kind.element_size();
                guard[ta_c.byte_offset..ta_c.byte_offset + len].to_vec()
            } else if let ObjectKind::ArrayBuffer(ref ab) = rt.heap.get_obj(ta_c.buffer).kind {
                let len = ta_c.length * ta_c.kind.element_size();
                ab.bytes[ta_c.byte_offset..ta_c.byte_offset + len].to_vec()
            } else {
                Vec::new()
            };
            let dst_buf = if let ObjectKind::TypedArray(ref dst) = rt.heap.get_obj(c).kind {
                dst.buffer
            } else {
                unreachable!()
            };
            if let ObjectKind::ArrayBuffer(ref mut ab) = rt.heap.get_obj_mut(dst_buf).kind {
                ab.bytes.copy_from_slice(&src_bytes);
            }
            seen.insert(*oid, c);
            return Ok(JsValue::Object(c));
        }
        ObjectKind::Array(_) => 1,
        ObjectKind::Map(_) => 2,
        ObjectKind::Set(_) => 3,
        _ => 0,
    };

    if kind_clone == 1 {
        let len_v = rt.get_property(*oid, "length")?;
        let len = to_length(rt.to_number(&len_v)?);
        let arr = rt.heap.alloc_object(JsObject::new(
            Some(rt.intrinsics.array_proto),
            ObjectKind::Array(ArrayData::with_len(len)),
        ));
        seen.insert(*oid, arr);
        for i in 0..len {
            let pk = PropertyKey::String(i.to_string());
            if rt.has_own_property(*oid, &pk)? {
                let v = rt.get_property_key(*oid, &pk)?;
                let cv = structured_clone_impl(rt, &v, seen)?;
                let _ = rt.set_property(arr, &i.to_string(), cv);
            }
        }
        return Ok(JsValue::Object(arr));
    }

    if kind_clone == 2 {
        let entries = match &rt.heap.get_obj(*oid).kind {
            ObjectKind::Map(m) => m.entries.clone(),
            _ => Vec::new(),
        };
        let dst = rt.heap.alloc_object(JsObject::new(
            Some(rt.intrinsics.map_proto),
            ObjectKind::Map(MapData::default()),
        ));
        seen.insert(*oid, dst);
        for pair in entries.into_iter().flatten() {
            let ck = structured_clone_impl(rt, &pair.0, seen)?;
            let cv = structured_clone_impl(rt, &pair.1, seen)?;
            if let ObjectKind::Map(ref mut m) = rt.heap.get_obj_mut(dst).kind {
                m.set(ck, cv);
            }
        }
        return Ok(JsValue::Object(dst));
    }

    if kind_clone == 3 {
        let entries = match &rt.heap.get_obj(*oid).kind {
            ObjectKind::Set(s) => s.entries.clone(),
            _ => Vec::new(),
        };
        let dst = rt.heap.alloc_object(JsObject::new(
            Some(rt.intrinsics.set_proto),
            ObjectKind::Set(SetData::default()),
        ));
        seen.insert(*oid, dst);
        for item in entries.into_iter().flatten() {
            let cv = structured_clone_impl(rt, &item, seen)?;
            if let ObjectKind::Set(ref mut s) = rt.heap.get_obj_mut(dst).kind {
                s.add(cv);
            }
        }
        return Ok(JsValue::Object(dst));
    }

    let dst = rt.create_object();
    seen.insert(*oid, dst);
    let keys = rt.own_keys(*oid)?;
    for k in keys {
        if let PropertyKey::String(s) = k {
            let pk = PropertyKey::String(s);
            if let Some(d) = rt.get_own_property_descriptor(*oid, &pk)? {
                if d.enumerable.unwrap_or(false) {
                    let v = rt.get_property_key(*oid, &pk)?;
                    let cv = structured_clone_impl(rt, &v, seen)?;
                    rt.define_property(dst, pk, PropertyDescriptor::data(cv, true, true, true))?;
                }
            }
        }
    }
    Ok(JsValue::Object(dst))
}
