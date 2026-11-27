pub mod array;
pub mod atomics;
pub mod collections;
pub mod date;
pub mod disposable;
pub mod error;
pub mod function;
pub mod globals;
pub mod iterator;
pub mod json;
pub mod number;
pub mod object;
pub mod promise;
pub mod proxy;
pub mod regexp;
pub mod string;
pub mod typed_array;

use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn set_method(
    rt: &mut JsRuntime,
    obj: ObjectId,
    name: &str,
    len: usize,
    cb: NativeCallback,
) {
    let f = rt.create_native_function(name, len, cb);
    rt.heap.get_obj_mut(obj).set_own_prop(
        PropertyKey::from(name),
        PropEntry::data(JsValue::Object(f), true, false, true),
    );
}

pub fn set_symbol_method(
    rt: &mut JsRuntime,
    obj: ObjectId,
    sym: u64,
    name: &str,
    len: usize,
    cb: NativeCallback,
) {
    let f = rt.create_native_function(name, len, cb);
    rt.heap.get_obj_mut(obj).set_own_prop(
        PropertyKey::Symbol(sym),
        PropEntry::data(JsValue::Object(f), true, false, true),
    );
}

pub fn set_global(rt: &mut JsRuntime, name: &str, val: JsValue) {
    let g = rt.global_object;
    rt.heap.get_obj_mut(g).set_own_prop(
        PropertyKey::from(name),
        PropEntry::data(val, true, false, true),
    );
}

pub fn set_getter(rt: &mut JsRuntime, obj: ObjectId, name: &str, cb: NativeCallback) {
    let g = rt.create_native_function(&format!("get {}", name), 0, cb);
    let _ = rt.define_property(
        obj,
        PropertyKey::from(name),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(g)),
            set: None,
            enumerable: Some(false),
            configurable: Some(true),
        },
    );
}

pub fn init_builtins(rt: &mut JsRuntime) {
    init_iterators_and_generators(rt);
    iterator::init(rt);
    object::init(rt);
    function::init(rt);
    array::init(rt);
    string::init(rt);
    number::init(rt);
    regexp::init(rt);
    date::init(rt);
    error::init(rt);
    disposable::init(rt);
    json::init(rt);
    collections::init(rt);
    promise::init(rt);
    proxy::init(rt);
    typed_array::init(rt);
    atomics::init(rt);
    globals::init(rt);
}

fn init_iterators_and_generators(rt: &mut JsRuntime) {
    let iter_proto = rt.intrinsics.iterator_proto;
    let sym_iter = rt.symbols.iterator;
    set_symbol_method(
        rt,
        iter_proto,
        sym_iter,
        "[Symbol.iterator]",
        0,
        Rc::new(|_, this, _| Ok(this)),
    );

    let to_str_tag = rt.symbols.to_string_tag;
    let iter_protos: [(ObjectId, &str); 5] = [
        (rt.intrinsics.array_iterator_proto, "Array Iterator"),
        (rt.intrinsics.string_iterator_proto, "String Iterator"),
        (rt.intrinsics.map_iterator_proto, "Map Iterator"),
        (rt.intrinsics.set_iterator_proto, "Set Iterator"),
        (rt.intrinsics.regexp_string_iterator_proto, "RegExp String Iterator"),
    ];
    for (p, tag) in iter_protos {
        set_method(
            rt,
            p,
            "next",
            0,
            Rc::new(|rt, this, _| {
                let JsValue::Object(oid) = this else {
                    return Err(JsError::type_err("Iterator.next called on non-object"));
                };
                step_builtin_iterator(rt, oid)
            }),
        );
        rt.heap.get_obj_mut(p).set_own_prop(
            PropertyKey::Symbol(to_str_tag),
            PropEntry::data(JsValue::String(tag.into()), false, false, true),
        );
    }

    let gen_proto = rt.intrinsics.generator_proto;
    rt.heap.get_obj_mut(gen_proto).set_own_prop(
        PropertyKey::from("constructor"),
        PropEntry::data(
            JsValue::Object(rt.intrinsics.generator_function_proto),
            false,
            false,
            true,
        ),
    );
    set_method(
        rt,
        gen_proto,
        "next",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Generator.prototype.next called on non-object"));
            };
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            rt.resume_generator(oid, 0, arg)
        }),
    );
    set_method(
        rt,
        gen_proto,
        "throw",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Generator.prototype.throw called on non-object"));
            };
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            rt.resume_generator(oid, 1, arg)
        }),
    );
    set_method(
        rt,
        gen_proto,
        "return",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Generator.prototype.return called on non-object"));
            };
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            rt.resume_generator(oid, 2, arg)
        }),
    );
    rt.heap.get_obj_mut(gen_proto).set_own_prop(
        PropertyKey::Symbol(to_str_tag),
        PropEntry::data(JsValue::String("Generator".into()), false, false, true),
    );

    let async_iter_proto = rt.intrinsics.async_iterator_proto;
    let sym_async_iter = rt.symbols.async_iterator;
    set_symbol_method(
        rt,
        async_iter_proto,
        sym_async_iter,
        "[Symbol.asyncIterator]",
        0,
        Rc::new(|_, this, _| Ok(this)),
    );
    let sym_async_dispose = rt.symbols.async_dispose;
    set_symbol_method(
        rt,
        async_iter_proto,
        sym_async_dispose,
        "[Symbol.asyncDispose]",
        0,
        Rc::new(|rt, this, _| {
            let out_p = rt.create_promise();
            let step: Result<Option<JsValue>, JsError> = (|| {
                let oid = rt.to_object(&this)?;
                let ret_fn = rt.get_property(oid, "return")?;
                if ret_fn.is_nullish() {
                    return Ok(None);
                }
                if !rt.is_callable(&ret_fn) {
                    return Err(JsError::type_err("return property is not callable"));
                }
                let r = rt.call_function(ret_fn, this, &[])?;
                Ok(Some(r))
            })();
            match step {
                Ok(None) => {
                    rt.resolve_promise(out_p, JsValue::Undefined);
                }
                Ok(Some(res)) => {
                    let inner_p = rt.promise_resolve_value(res);
                    let on_ok = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, _| {
                            rt.resolve_promise(out_p, JsValue::Undefined);
                            Ok(JsValue::Undefined)
                        }),
                    );
                    let on_err = rt.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            let err = a.first().cloned().unwrap_or(JsValue::Undefined);
                            rt.reject_promise(out_p, err);
                            Ok(JsValue::Undefined)
                        }),
                    );
                    rt.promise_then(
                        inner_p,
                        Some(JsValue::Object(on_ok)),
                        Some(JsValue::Object(on_err)),
                    );
                }
                Err(e) => {
                    let ev = rt.error_to_js_value(e);
                    rt.reject_promise(out_p, ev);
                }
            }
            Ok(JsValue::Object(out_p))
        }),
    );

    let async_gen_proto = rt.intrinsics.async_generator_proto;
    rt.heap.get_obj_mut(async_gen_proto).set_own_prop(
        PropertyKey::from("constructor"),
        PropEntry::data(
            JsValue::Object(rt.intrinsics.async_generator_function_proto),
            false,
            false,
            true,
        ),
    );
    rt.heap.get_obj_mut(async_gen_proto).set_own_prop(
        PropertyKey::Symbol(to_str_tag),
        PropEntry::data(JsValue::String("AsyncGenerator".into()), false, false, true),
    );
    set_method(
        rt,
        async_gen_proto,
        "next",
        1,
        Rc::new(|rt, this, args| {
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            let is_valid = matches!(this, JsValue::Object(oid) if matches!(rt.heap.get_obj(oid).kind, ObjectKind::Generator(ref g) if g.is_async));
            if !is_valid {
                let p = rt.create_promise();
                let ev = rt.error_to_js_value(JsError::type_err(
                    "AsyncGenerator.prototype.next requires that 'this' be an AsyncGenerator",
                ));
                rt.reject_promise(p, ev);
                return Ok(JsValue::Object(p));
            }
            let JsValue::Object(oid) = this else { unreachable!() };
            rt.resume_generator(oid, 0, arg)
        }),
    );
    set_method(
        rt,
        async_gen_proto,
        "throw",
        1,
        Rc::new(|rt, this, args| {
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            let is_valid = matches!(this, JsValue::Object(oid) if matches!(rt.heap.get_obj(oid).kind, ObjectKind::Generator(ref g) if g.is_async));
            if !is_valid {
                let p = rt.create_promise();
                let ev = rt.error_to_js_value(JsError::type_err(
                    "AsyncGenerator.prototype.throw requires that 'this' be an AsyncGenerator",
                ));
                rt.reject_promise(p, ev);
                return Ok(JsValue::Object(p));
            }
            let JsValue::Object(oid) = this else { unreachable!() };
            rt.resume_generator(oid, 1, arg)
        }),
    );
    set_method(
        rt,
        async_gen_proto,
        "return",
        1,
        Rc::new(|rt, this, args| {
            let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            let is_valid = matches!(this, JsValue::Object(oid) if matches!(rt.heap.get_obj(oid).kind, ObjectKind::Generator(ref g) if g.is_async));
            if !is_valid {
                let p = rt.create_promise();
                let ev = rt.error_to_js_value(JsError::type_err(
                    "AsyncGenerator.prototype.return requires that 'this' be an AsyncGenerator",
                ));
                rt.reject_promise(p, ev);
                return Ok(JsValue::Object(p));
            }
            let JsValue::Object(oid) = this else { unreachable!() };
            rt.resume_generator(oid, 2, arg)
        }),
    );
}

fn step_builtin_iterator(rt: &mut JsRuntime, oid: ObjectId) -> Result<JsValue, JsError> {
    let (kind, idx) = match &rt.heap.get_obj(oid).kind {
        ObjectKind::Iterator(it) => (it.kind.clone(), it.index),
        _ => return Err(JsError::type_err("Not a built-in iterator")),
    };
    match kind {
        BuiltinIteratorKind::Array { target, mode } => {
            if idx == usize::MAX {
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            let len = if matches!(rt.heap.get_obj(target).kind, ObjectKind::TypedArray(_)) {
                match rt.typed_array_effective_length(target) {
                    Some(l) => l,
                    None => return Err(JsError::type_err("TypedArray buffer is detached or out of bounds")),
                }
            } else {
                let len_v = rt.get_property(target, "length")?;
                to_length(rt.to_number(&len_v)?)
            };
            if idx >= len {
                if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                    it.index = usize::MAX;
                }
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                it.index = idx + 1;
            }
            let val = match mode {
                IterMode::Keys => JsValue::Number(idx as f64),
                IterMode::Values => rt.get_property(target, &idx.to_string())?,
                IterMode::Entries => {
                    let v = rt.get_property(target, &idx.to_string())?;
                    JsValue::Object(rt.create_array(vec![JsValue::Number(idx as f64), v]))
                }
            };
            Ok(rt.create_iter_result(val, false))
        }
        BuiltinIteratorKind::String { units } => {
            if idx >= units.len() {
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            let u = units[idx];
            let (next_idx, ch_str) =
                if (0xD800..=0xDBFF).contains(&u) && idx + 1 < units.len() {
                    let lo = units[idx + 1];
                    if (0xDC00..=0xDFFF).contains(&lo) {
                        (idx + 2, String::from_utf16_lossy(&[u, lo]))
                    } else {
                        (idx + 1, String::from_utf16_lossy(&[u]))
                    }
                } else {
                    (idx + 1, String::from_utf16_lossy(&[u]))
                };
            if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                it.index = next_idx;
            }
            Ok(rt.create_iter_result(JsValue::String(ch_str), false))
        }
        BuiltinIteratorKind::Map { target, mode } => {
            if idx == usize::MAX {
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            let entry = {
                let ObjectKind::Map(ref m) = rt.heap.get_obj(target).kind else {
                    return Ok(rt.create_iter_result(JsValue::Undefined, true));
                };
                let mut i = idx;
                let mut found = None;
                while i < m.entries.len() {
                    if let Some(pair) = &m.entries[i] {
                        found = Some((i + 1, pair.0.clone(), pair.1.clone()));
                        break;
                    }
                    i += 1;
                }
                found
            };
            if let Some((next_idx, k, v)) = entry {
                if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                    it.index = next_idx;
                }
                let val = match mode {
                    IterMode::Keys => k,
                    IterMode::Values => v,
                    IterMode::Entries => JsValue::Object(rt.create_array(vec![k, v])),
                };
                Ok(rt.create_iter_result(val, false))
            } else {
                if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                    it.index = usize::MAX;
                }
                Ok(rt.create_iter_result(JsValue::Undefined, true))
            }
        }
        BuiltinIteratorKind::Set { target, mode } => {
            if idx == usize::MAX {
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            let entry = {
                let ObjectKind::Set(ref s) = rt.heap.get_obj(target).kind else {
                    return Ok(rt.create_iter_result(JsValue::Undefined, true));
                };
                let mut i = idx;
                let mut found = None;
                while i < s.entries.len() {
                    if let Some(v) = &s.entries[i] {
                        found = Some((i + 1, v.clone()));
                        break;
                    }
                    i += 1;
                }
                found
            };
            if let Some((next_idx, v)) = entry {
                if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                    it.index = next_idx;
                }
                let val = match mode {
                    IterMode::Entries => JsValue::Object(rt.create_array(vec![v.clone(), v])),
                    IterMode::Keys | IterMode::Values => v,
                };
                Ok(rt.create_iter_result(val, false))
            } else {
                if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                    it.index = usize::MAX;
                }
                Ok(rt.create_iter_result(JsValue::Undefined, true))
            }
        }
        BuiltinIteratorKind::Keys(ks) => {
            if idx >= ks.len() {
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                it.index = idx + 1;
            }
            Ok(rt.create_iter_result(ks[idx].clone(), false))
        }
        BuiltinIteratorKind::RegExpMatchAll {
            regexp,
            input,
            global,
            done,
        } => {
            if done {
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            let exec_fn = rt.get_property(regexp, "exec")?;
            let m = if rt.is_callable(&exec_fn) {
                let res =
                    rt.call_function(exec_fn, JsValue::Object(regexp), &[JsValue::String(input)])?;
                if !res.is_null() && !res.is_object() {
                    return Err(JsError::type_err(
                        "RegExp exec method returned something other than an Object or null",
                    ));
                }
                res
            } else {
                crate::builtins::regexp::regexp_exec(rt, regexp, &input)?
            };
            if m.is_null() {
                if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                    if let BuiltinIteratorKind::RegExpMatchAll { ref mut done, .. } = it.kind {
                        *done = true;
                    }
                }
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            if !global {
                if let ObjectKind::Iterator(ref mut it) = rt.heap.get_obj_mut(oid).kind {
                    if let BuiltinIteratorKind::RegExpMatchAll { ref mut done, .. } = it.kind {
                        *done = true;
                    }
                }
            } else if let JsValue::Object(mid) = m {
                let s0 = rt.get_property(mid, "0")?;
                if rt.to_string(&s0)?.is_empty() {
                    let li = rt.get_property(regexp, "lastIndex")?;
                    let n = to_length(rt.to_number(&li)?);
                    let _ = rt.set_property(regexp, "lastIndex", JsValue::Number((n + 1) as f64));
                }
            }
            Ok(rt.create_iter_result(m, false))
        }
        BuiltinIteratorKind::Wrapped(_) | BuiltinIteratorKind::Helper(_) => {
            Err(JsError::type_err("Not a built-in iterator"))
        }
    }
}
