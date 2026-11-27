use crate::builtins::string::expand_replacement;
use crate::builtins::{set_getter, set_global, set_method, set_symbol_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::collections::HashMap;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.regexp_proto;

    set_getter(
        rt,
        proto,
        "source",
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("RegExp.prototype.source called on non-object"));
            };
            if oid == rt.intrinsics.regexp_proto {
                return Ok(JsValue::String("(?:)".into()));
            }
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::RegExp(re) => Ok(JsValue::String(re.compiled.source.clone())),
                _ => Err(JsError::type_err("RegExp.prototype.source called on non-RegExp")),
            }
        }),
    );

    set_getter(
        rt,
        proto,
        "flags",
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("RegExp.prototype.flags called on non-object"));
            };
            let mut s = String::new();
            for (prop, ch) in [
                ("hasIndices", 'd'),
                ("global", 'g'),
                ("ignoreCase", 'i'),
                ("multiline", 'm'),
                ("dotAll", 's'),
                ("unicode", 'u'),
                ("sticky", 'y'),
            ] {
                if rt.get_property(oid, prop)?.to_boolean() {
                    s.push(ch);
                }
            }
            Ok(JsValue::String(s))
        }),
    );

    for (name, extractor) in [
        ("global", (|f: crate::regexp::RegExpFlags| f.global) as fn(crate::regexp::RegExpFlags) -> bool),
        ("ignoreCase", |f| f.ignore_case),
        ("multiline", |f| f.multiline),
        ("dotAll", |f| f.dot_all),
        ("unicode", |f| f.unicode),
        ("sticky", |f| f.sticky),
        ("hasIndices", |f| f.has_indices),
    ] {
        set_getter(
            rt,
            proto,
            name,
            Rc::new(move |rt, this, _| {
                let JsValue::Object(oid) = this else {
                    return Err(JsError::type_err("RegExp flag getter called on non-object"));
                };
                if oid == rt.intrinsics.regexp_proto {
                    return Ok(JsValue::Undefined);
                }
                match &rt.heap.get_obj(oid).kind {
                    ObjectKind::RegExp(re) => Ok(JsValue::Boolean(extractor(re.compiled.flags))),
                    _ => Err(JsError::type_err("RegExp flag getter called on non-RegExp")),
                }
            }),
        );
    }

    set_method(
        rt,
        proto,
        "exec",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("RegExp.prototype.exec called on non-object"));
            };
            let input = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            regexp_exec(rt, oid, &input)
        }),
    );

    set_method(
        rt,
        proto,
        "test",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("RegExp.prototype.test called on non-object"));
            };
            let input = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let exec_fn = rt.get_property(oid, "exec")?;
            let res = if rt.is_callable(&exec_fn) {
                rt.call_function(exec_fn, this, &[JsValue::String(input)])?
            } else {
                regexp_exec(rt, oid, &input)?
            };
            Ok(JsValue::Boolean(!res.is_null()))
        }),
    );

    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("RegExp.prototype.toString called on non-object"));
            };
            let src = rt.get_property(oid, "source")?;
            let fl = rt.get_property(oid, "flags")?;
            let src_s = rt.to_string(&src)?;
            let fl_s = rt.to_string(&fl)?;
            Ok(JsValue::String(format!("/{}/{}", src_s, fl_s)))
        }),
    );

    let sym_match = rt.symbols.match_sym;
    set_symbol_method(
        rt,
        proto,
        sym_match,
        "[Symbol.match]",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("RegExp.prototype[Symbol.match] called on non-object"));
            };
            let input = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let global = rt.get_property(oid, "global")?.to_boolean();
            if !global {
                let exec_fn = rt.get_property(oid, "exec")?;
                return rt.call_function(exec_fn, this, &[JsValue::String(input)]);
            }
            let _ = rt.set_property(oid, "lastIndex", JsValue::Number(0.0));
            let mut results = Vec::new();
            let exec_fn = rt.get_property(oid, "exec")?;
            loop {
                let m = rt.call_function(
                    exec_fn.clone(),
                    this.clone(),
                    &[JsValue::String(input.clone())],
                )?;
                if m.is_null() {
                    break;
                }
                let JsValue::Object(mid) = m else {
                    break;
                };
                let s0 = rt.get_property(mid, "0")?;
                let matched = rt.to_string(&s0)?;
                if matched.is_empty() {
                    let li = rt.get_property(oid, "lastIndex")?;
                    let n = to_length(rt.to_number(&li)?);
                    let _ = rt.set_property(oid, "lastIndex", JsValue::Number((n + 1) as f64));
                }
                results.push(JsValue::String(matched));
            }
            if results.is_empty() {
                Ok(JsValue::Null)
            } else {
                Ok(JsValue::Object(rt.create_array(results)))
            }
        }),
    );

    let sym_match_all = rt.symbols.match_all;
    set_symbol_method(
        rt,
        proto,
        sym_match_all,
        "[Symbol.matchAll]",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err(
                    "RegExp.prototype[Symbol.matchAll] called on non-object",
                ));
            };
            let input = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let (source, flags_str, global) = match &rt.heap.get_obj(oid).kind {
                ObjectKind::RegExp(re) => (
                    re.original_source.clone(),
                    re.compiled.flags.canonical_string(),
                    re.compiled.flags.global,
                ),
                _ => {
                    let src = rt.get_property(oid, "source")?;
                    let fl = rt.get_property(oid, "flags")?;
                    let fs = rt.to_string(&fl)?;
                    let g = fs.contains('g');
                    (rt.to_string(&src)?, fs, g)
                }
            };
            let clone = rt.create_regexp_object(&source, &flags_str)?;
            let li = rt.get_property(oid, "lastIndex")?;
            let _ = rt.set_property(clone, "lastIndex", li);
            let it = rt.create_builtin_iterator(BuiltinIteratorKind::RegExpMatchAll {
                regexp: clone,
                input,
                global,
                done: false,
            });
            Ok(JsValue::Object(it))
        }),
    );

    let sym_replace = rt.symbols.replace;
    set_symbol_method(
        rt,
        proto,
        sym_replace,
        "[Symbol.replace]",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err(
                    "RegExp.prototype[Symbol.replace] called on non-object",
                ));
            };
            let input = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let replace_val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let is_fn = rt.is_callable(&replace_val);
            let template = if is_fn {
                String::new()
            } else {
                rt.to_string(&replace_val)?
            };
            let global = rt.get_property(oid, "global")?.to_boolean();
            if global {
                let _ = rt.set_property(oid, "lastIndex", JsValue::Number(0.0));
            }
            let exec_fn = rt.get_property(oid, "exec")?;
            let mut matches = Vec::new();
            loop {
                let m = rt.call_function(
                    exec_fn.clone(),
                    this.clone(),
                    &[JsValue::String(input.clone())],
                )?;
                if m.is_null() {
                    break;
                }
                let JsValue::Object(mid) = m else {
                    break;
                };
                matches.push(mid);
                if !global {
                    break;
                }
                let s0 = rt.get_property(mid, "0")?;
                if rt.to_string(&s0)?.is_empty() {
                    let li = rt.get_property(oid, "lastIndex")?;
                    let n = to_length(rt.to_number(&li)?);
                    let _ = rt.set_property(oid, "lastIndex", JsValue::Number((n + 1) as f64));
                }
            }
            let units: Vec<u16> = input.encode_utf16().collect();
            let mut out = String::new();
            let mut next_pos = 0usize;
            for mid in matches {
                let len_v = rt.get_property(mid, "length")?;
                let n_caps = to_length(rt.to_number(&len_v)?).max(1);
                let m0 = rt.get_property(mid, "0")?;
                let matched = rt.to_string(&m0)?;
                let matched_len = matched.encode_utf16().count();
                let idx_v = rt.get_property(mid, "index")?;
                let pos = to_integer_or_infinity(rt.to_number(&idx_v)?)
                    .clamp(0.0, units.len() as f64) as usize;
                if pos < next_pos {
                    continue;
                }
                let mut caps: Vec<Option<String>> = Vec::with_capacity(n_caps - 1);
                for i in 1..n_caps {
                    let v = rt.get_property(mid, &i.to_string())?;
                    if v.is_undefined() {
                        caps.push(None);
                    } else {
                        caps.push(Some(rt.to_string(&v)?));
                    }
                }
                let groups_val = rt.get_property(mid, "groups")?;
                let repl = if is_fn {
                    let mut cb_args = vec![JsValue::String(matched)];
                    for c in &caps {
                        cb_args.push(match c {
                            Some(s) => JsValue::String(s.clone()),
                            None => JsValue::Undefined,
                        });
                    }
                    cb_args.push(JsValue::Number(pos as f64));
                    cb_args.push(JsValue::String(input.clone()));
                    if !groups_val.is_undefined() {
                        cb_args.push(groups_val);
                    }
                    let r = rt.call_function(replace_val.clone(), JsValue::Undefined, &cb_args)?;
                    rt.to_string(&r)?
                } else {
                    let named_map = if let JsValue::Object(gid) = groups_val {
                        let mut hm = HashMap::new();
                        for k in rt.own_keys(gid)? {
                            if let PropertyKey::String(s) = k {
                                let v = rt.get_property(gid, &s)?;
                                hm.insert(
                                    s,
                                    if v.is_undefined() {
                                        None
                                    } else {
                                        Some(rt.to_string(&v)?)
                                    },
                                );
                            }
                        }
                        Some(hm)
                    } else {
                        None
                    };
                    expand_replacement(
                        &template,
                        &units,
                        pos,
                        (pos + matched_len).min(units.len()),
                        &caps,
                        named_map.as_ref(),
                    )
                };
                out.push_str(&String::from_utf16_lossy(&units[next_pos..pos]));
                out.push_str(&repl);
                next_pos = (pos + matched_len).min(units.len());
            }
            out.push_str(&String::from_utf16_lossy(&units[next_pos..]));
            Ok(JsValue::String(out))
        }),
    );

    let sym_search = rt.symbols.search;
    set_symbol_method(
        rt,
        proto,
        sym_search,
        "[Symbol.search]",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err(
                    "RegExp.prototype[Symbol.search] called on non-object",
                ));
            };
            let input = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let prev = rt.get_property(oid, "lastIndex")?;
            let _ = rt.set_property(oid, "lastIndex", JsValue::Number(0.0));
            let exec_fn = rt.get_property(oid, "exec")?;
            let m = rt.call_function(exec_fn, this, &[JsValue::String(input)])?;
            let _ = rt.set_property(oid, "lastIndex", prev);
            if m.is_null() {
                Ok(JsValue::Number(-1.0))
            } else if let JsValue::Object(mid) = m {
                rt.get_property(mid, "index")
            } else {
                Ok(JsValue::Number(-1.0))
            }
        }),
    );

    let sym_split = rt.symbols.split;
    set_symbol_method(
        rt,
        proto,
        sym_split,
        "[Symbol.split]",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err(
                    "RegExp.prototype[Symbol.split] called on non-object",
                ));
            };
            let input = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let limit = match args.get(1) {
                None | Some(JsValue::Undefined) => u32::MAX as usize,
                Some(v) => to_uint32(rt.to_number(v)?) as usize,
            };
            if limit == 0 {
                return Ok(JsValue::Object(rt.create_array(Vec::new())));
            }
            let (source, mut flags_str) = match &rt.heap.get_obj(oid).kind {
                ObjectKind::RegExp(re) => (
                    re.original_source.clone(),
                    re.compiled.flags.canonical_string(),
                ),
                _ => {
                    let sp = rt.get_property(oid, "source")?;
                    let fp = rt.get_property(oid, "flags")?;
                    (rt.to_string(&sp)?, rt.to_string(&fp)?)
                }
            };
            if !flags_str.contains('y') {
                flags_str.push('y');
            }
            let splitter = rt.create_regexp_object(&source, &flags_str)?;
            let units: Vec<u16> = input.encode_utf16().collect();
            let size = units.len();
            if size == 0 {
                let m = regexp_exec(rt, splitter, &input)?;
                if !m.is_null() {
                    return Ok(JsValue::Object(rt.create_array(Vec::new())));
                }
                return Ok(JsValue::Object(rt.create_array(vec![JsValue::String(input)])));
            }
            let mut out = Vec::new();
            let mut p = 0usize;
            let mut q = 0usize;
            while q < size {
                let _ = rt.set_property(splitter, "lastIndex", JsValue::Number(q as f64));
                let m = regexp_exec(rt, splitter, &input)?;
                if m.is_null() {
                    q += 1;
                    continue;
                }
                let li_v = rt.get_property(splitter, "lastIndex")?;
                let e = to_length(rt.to_number(&li_v)?).min(size);
                if e == p {
                    q += 1;
                    continue;
                }
                out.push(JsValue::String(String::from_utf16_lossy(&units[p..q])));
                if out.len() == limit {
                    return Ok(JsValue::Object(rt.create_array(out)));
                }
                p = e;
                if let JsValue::Object(mid) = m {
                    let cap_len_v = rt.get_property(mid, "length")?;
                    let cap_len = to_length(rt.to_number(&cap_len_v)?);
                    for i in 1..cap_len {
                        let cap = rt.get_property(mid, &i.to_string())?;
                        out.push(cap);
                        if out.len() == limit {
                            return Ok(JsValue::Object(rt.create_array(out)));
                        }
                    }
                }
                q = p;
            }
            out.push(JsValue::String(String::from_utf16_lossy(&units[p..size])));
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );

    let ctor = rt.create_native_constructor(
        "RegExp",
        2,
        proto,
        Rc::new(|rt, _, args| {
            let pat_arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            let flags_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(pid) = pat_arg {
                if let ObjectKind::RegExp(ref re) = rt.heap.get_obj(pid).kind {
                    let orig = re.original_source.clone();
                    let orig_flags = re.compiled.flags.canonical_string();
                    let fl = if flags_arg.is_undefined() {
                        orig_flags
                    } else {
                        rt.to_string(&flags_arg)?
                    };
                    return Ok(JsValue::Object(rt.create_regexp_object(&orig, &fl)?));
                }
            }
            let pat = if pat_arg.is_undefined() {
                String::new()
            } else {
                rt.to_string(&pat_arg)?
            };
            let fl = if flags_arg.is_undefined() {
                String::new()
            } else {
                rt.to_string(&flags_arg)?
            };
            Ok(JsValue::Object(rt.create_regexp_object(&pat, &fl)?))
        }),
    );
    rt.intrinsics.regexp_ctor = ctor;
    let sym_species = rt.symbols.species;
    let sp_get = rt.create_native_function("get [Symbol.species]", 0, Rc::new(|_, this, _| Ok(this)));
    let _ = rt.define_property(
        ctor,
        PropertyKey::Symbol(sym_species),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(sp_get)),
            set: None,
            enumerable: Some(false),
            configurable: Some(true),
        },
    );
    set_global(rt, "RegExp", JsValue::Object(ctor));
}

fn regexp_exec(rt: &mut JsRuntime, oid: ObjectId, input: &str) -> Result<JsValue, JsError> {
    let compiled = match &rt.heap.get_obj(oid).kind {
        ObjectKind::RegExp(re) => re.compiled.clone(),
        _ => return Err(JsError::type_err("Method RegExp.prototype.exec called on non-RegExp")),
    };
    let global = compiled.flags.global;
    let sticky = compiled.flags.sticky;
    let has_indices = compiled.flags.has_indices;
    let li_val = rt.get_property(oid, "lastIndex")?;
    let mut last_index = if global || sticky {
        to_length(rt.to_number(&li_val)?)
    } else {
        0
    };
    let units: Vec<u16> = input.encode_utf16().collect();
    if last_index > units.len() {
        if global || sticky {
            let _ = rt.set_property(oid, "lastIndex", JsValue::Number(0.0));
        }
        return Ok(JsValue::Null);
    }
    let Some(m) = compiled.exec_utf16(&units, last_index) else {
        if global || sticky {
            let _ = rt.set_property(oid, "lastIndex", JsValue::Number(0.0));
        }
        return Ok(JsValue::Null);
    };
    if global || sticky {
        last_index = m.end;
        let _ = rt.set_property(oid, "lastIndex", JsValue::Number(last_index as f64));
    }
    let mut elems = Vec::with_capacity(m.captures.len());
    for cap in &m.captures {
        match cap {
            Some((s, e)) => elems.push(JsValue::String(String::from_utf16_lossy(&units[*s..*e]))),
            None => elems.push(JsValue::Undefined),
        }
    }
    let arr = rt.create_array(elems);
    let _ = rt.set_property(arr, "index", JsValue::Number(m.start as f64));
    let _ = rt.set_property(arr, "input", JsValue::String(input.to_string()));
    if !compiled.named_groups().is_empty() {
        let gobj = rt.create_object_with_proto(None);
        for (name, span) in &m.named_groups {
            let val = match span {
                Some((s, e)) => JsValue::String(String::from_utf16_lossy(&units[*s..*e])),
                None => JsValue::Undefined,
            };
            let _ = rt.set_property(gobj, name, val);
        }
        let _ = rt.set_property(arr, "groups", JsValue::Object(gobj));
    } else {
        let _ = rt.set_property(arr, "groups", JsValue::Undefined);
    }
    if has_indices {
        let mut idx_elems = Vec::with_capacity(m.captures.len());
        for cap in &m.captures {
            match cap {
                Some((s, e)) => {
                    let pair = rt.create_array(vec![
                        JsValue::Number(*s as f64),
                        JsValue::Number(*e as f64),
                    ]);
                    idx_elems.push(JsValue::Object(pair));
                }
                None => idx_elems.push(JsValue::Undefined),
            }
        }
        let indices_arr = rt.create_array(idx_elems);
        if !compiled.named_groups().is_empty() {
            let ig = rt.create_object_with_proto(None);
            for (name, span) in &m.named_groups {
                let v = match span {
                    Some((s, e)) => JsValue::Object(rt.create_array(vec![
                        JsValue::Number(*s as f64),
                        JsValue::Number(*e as f64),
                    ])),
                    None => JsValue::Undefined,
                };
                let _ = rt.set_property(ig, name, v);
            }
            let _ = rt.set_property(indices_arr, "groups", JsValue::Object(ig));
        } else {
            let _ = rt.set_property(indices_arr, "groups", JsValue::Undefined);
        }
        let _ = rt.set_property(arr, "indices", JsValue::Object(indices_arr));
    }
    Ok(JsValue::Object(arr))
}
