use crate::builtins::array::create_data_property_or_throw;
use crate::builtins::{set_global, set_method, set_symbol_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.iterator_proto;
    let helper_proto = rt.intrinsics.iterator_helper_proto;
    let wrap_proto = rt.intrinsics.wrap_for_valid_iterator_proto;
    let sym_to_string_tag = rt.symbols.to_string_tag;
    let sym_dispose = rt.symbols.dispose;

    let ctor = rt.create_spec_constructor(
        "Iterator",
        0,
        proto,
        Rc::new(|rt, _, _| {
            let nt = rt.native_new_target.clone().unwrap_or(JsValue::Undefined);
            if nt.is_undefined() || nt == JsValue::Object(rt.intrinsics.iterator_ctor) {
                return Err(JsError::type_err(
                    "Abstract class Iterator not directly constructable",
                ));
            }
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.iterator_proto)?;
            Ok(JsValue::Object(rt.create_object_with_proto(Some(obj_proto))))
        }),
    );
    rt.intrinsics.iterator_ctor = ctor;

    let ctor_get = rt.create_native_function(
        "get constructor",
        0,
        Rc::new(move |_, _, _| Ok(JsValue::Object(ctor))),
    );
    let ctor_set = rt.create_native_function(
        "set constructor",
        1,
        Rc::new(move |rt, this, args| {
            let v = args.first().cloned().unwrap_or(JsValue::Undefined);
            setter_ignoring_prototype(rt, &this, proto, PropertyKey::from("constructor"), v)
        }),
    );
    let _ = rt.define_property(
        proto,
        PropertyKey::from("constructor"),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(ctor_get)),
            set: Some(JsValue::Object(ctor_set)),
            enumerable: Some(false),
            configurable: Some(true),
        },
    );

    let tag_get = rt.create_native_function(
        "get [Symbol.toStringTag]",
        0,
        Rc::new(|_, _, _| Ok(JsValue::String("Iterator".into()))),
    );
    let tag_set = rt.create_native_function(
        "set [Symbol.toStringTag]",
        1,
        Rc::new(move |rt, this, args| {
            let v = args.first().cloned().unwrap_or(JsValue::Undefined);
            setter_ignoring_prototype(rt, &this, proto, PropertyKey::Symbol(sym_to_string_tag), v)
        }),
    );
    let _ = rt.define_property(
        proto,
        PropertyKey::Symbol(sym_to_string_tag),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(tag_get)),
            set: Some(JsValue::Object(tag_set)),
            enumerable: Some(false),
            configurable: Some(true),
        },
    );

    set_symbol_method(
        rt,
        proto,
        sym_dispose,
        "[Symbol.dispose]",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let ret_fn = rt.get_property(oid, "return")?;
            if !ret_fn.is_nullish() {
                if !rt.is_callable(&ret_fn) {
                    return Err(JsError::type_err("Iterator return is not callable"));
                }
                rt.call_function(ret_fn, this, &[])?;
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        ctor,
        "from",
        1,
        Rc::new(|rt, _, args| {
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            let rec = get_iterator_flattenable(rt, &val, true)?;
            if is_instance_of_iterator(rt, &rec.iterator)? {
                return Ok(rec.iterator);
            }
            let obj = rt.create_builtin_iterator(BuiltinIteratorKind::Wrapped(rec));
            Ok(JsValue::Object(obj))
        }),
    );

    set_method(
        rt,
        ctor,
        "concat",
        0,
        Rc::new(|rt, _, args| {
            let sym_iter = PropertyKey::Symbol(rt.symbols.iterator);
            let mut iterables = Vec::with_capacity(args.len());
            for item in args {
                let JsValue::Object(oid) = item else {
                    return Err(JsError::type_err("Iterator.concat argument must be an object"));
                };
                let method = rt.get_property_key(*oid, &sym_iter)?;
                if !rt.is_callable(&method) {
                    return Err(JsError::type_err(
                        "Iterator.concat argument is not iterable",
                    ));
                }
                iterables.push((item.clone(), method));
            }
            let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
                IteratorHelperData {
                    state: IteratorHelperState::SuspendedStart,
                    kind: IteratorHelperKind::Concat {
                        iterables,
                        pos: 0,
                        inner: None,
                    },
                },
            ));
            Ok(JsValue::Object(helper))
        }),
    );

    set_method(
        rt,
        ctor,
        "zip",
        1,
        Rc::new(|rt, _, args| iterator_zip(rt, args)),
    );

    set_method(
        rt,
        ctor,
        "zipKeyed",
        1,
        Rc::new(|rt, _, args| iterator_zip_keyed(rt, args)),
    );

    set_method(
        rt,
        proto,
        "map",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.map requires an object"));
            };
            let mapper = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&mapper) {
                rt.iterator_close(&this);
                return Err(JsError::type_err("mapper is not a function"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
                IteratorHelperData {
                    state: IteratorHelperState::SuspendedStart,
                    kind: IteratorHelperKind::Map {
                        iterated,
                        mapper,
                        counter: 0,
                    },
                },
            ));
            Ok(JsValue::Object(helper))
        }),
    );

    set_method(
        rt,
        proto,
        "filter",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.filter requires an object"));
            };
            let predicate = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&predicate) {
                rt.iterator_close(&this);
                return Err(JsError::type_err("predicate is not a function"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
                IteratorHelperData {
                    state: IteratorHelperState::SuspendedStart,
                    kind: IteratorHelperKind::Filter {
                        iterated,
                        predicate,
                        counter: 0,
                    },
                },
            ));
            Ok(JsValue::Object(helper))
        }),
    );

    set_method(
        rt,
        proto,
        "take",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.take requires an object"));
            };
            let limit_val = args.first().unwrap_or(&JsValue::Undefined);
            let num_limit = match rt.to_number(limit_val) {
                Ok(n) => n,
                Err(e) => {
                    rt.iterator_close(&this);
                    return Err(e);
                }
            };
            if num_limit.is_nan() {
                rt.iterator_close(&this);
                return Err(JsError::range("limit must not be NaN"));
            }
            let int_limit = to_integer_or_infinity(num_limit);
            if int_limit < 0.0 || (int_limit.is_finite() && int_limit > 9_007_199_254_740_991.0) {
                rt.iterator_close(&this);
                return Err(JsError::range("limit out of range"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
                IteratorHelperData {
                    state: IteratorHelperState::SuspendedStart,
                    kind: IteratorHelperKind::Take {
                        iterated,
                        remaining: int_limit,
                    },
                },
            ));
            Ok(JsValue::Object(helper))
        }),
    );

    set_method(
        rt,
        proto,
        "drop",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.drop requires an object"));
            };
            let limit_val = args.first().unwrap_or(&JsValue::Undefined);
            let num_limit = match rt.to_number(limit_val) {
                Ok(n) => n,
                Err(e) => {
                    rt.iterator_close(&this);
                    return Err(e);
                }
            };
            if num_limit.is_nan() {
                rt.iterator_close(&this);
                return Err(JsError::range("limit must not be NaN"));
            }
            let int_limit = to_integer_or_infinity(num_limit);
            if int_limit < 0.0 || (int_limit.is_finite() && int_limit > 9_007_199_254_740_991.0) {
                rt.iterator_close(&this);
                return Err(JsError::range("limit out of range"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
                IteratorHelperData {
                    state: IteratorHelperState::SuspendedStart,
                    kind: IteratorHelperKind::Drop {
                        iterated,
                        remaining: int_limit,
                    },
                },
            ));
            Ok(JsValue::Object(helper))
        }),
    );

    set_method(
        rt,
        proto,
        "flatMap",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.flatMap requires an object"));
            };
            let mapper = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&mapper) {
                rt.iterator_close(&this);
                return Err(JsError::type_err("mapper is not a function"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
                IteratorHelperData {
                    state: IteratorHelperState::SuspendedStart,
                    kind: IteratorHelperKind::FlatMap {
                        iterated,
                        mapper,
                        counter: 0,
                        inner: None,
                    },
                },
            ));
            Ok(JsValue::Object(helper))
        }),
    );

    set_method(
        rt,
        proto,
        "reduce",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.reduce requires an object"));
            };
            let reducer = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&reducer) {
                rt.iterator_close(&this);
                return Err(JsError::type_err("reducer is not a function"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let (mut acc, mut counter) = if args.len() >= 2 {
                (args[1].clone(), 0usize)
            } else {
                match step_record(rt, &iterated)? {
                    Some(first) => (first, 1usize),
                    None => {
                        return Err(JsError::type_err(
                            "Reduce of empty iterator with no initial value",
                        ));
                    }
                }
            };
            while let Some(v) = step_record(rt, &iterated)? {
                match rt.call_function(
                    reducer.clone(),
                    JsValue::Undefined,
                    &[acc, v, JsValue::Number(counter as f64)],
                ) {
                    Ok(next_acc) => acc = next_acc,
                    Err(e) => {
                        rt.iterator_close(&iterated.iterator);
                        return Err(e);
                    }
                }
                counter += 1;
            }
            Ok(acc)
        }),
    );

    set_method(
        rt,
        proto,
        "toArray",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.toArray requires an object"));
            };
            let iterated = get_iterator_direct(rt, &this)?;
            let mut items = Vec::new();
            while let Some(v) = step_record(rt, &iterated)? {
                items.push(v);
            }
            Ok(JsValue::Object(rt.create_array(items)))
        }),
    );

    set_method(
        rt,
        proto,
        "forEach",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.forEach requires an object"));
            };
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                rt.iterator_close(&this);
                return Err(JsError::type_err("forEach callback is not a function"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let mut counter = 0usize;
            while let Some(v) = step_record(rt, &iterated)? {
                if let Err(e) = rt.call_function(
                    cb.clone(),
                    JsValue::Undefined,
                    &[v, JsValue::Number(counter as f64)],
                ) {
                    rt.iterator_close(&iterated.iterator);
                    return Err(e);
                }
                counter += 1;
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "some",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.some requires an object"));
            };
            let pred = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&pred) {
                rt.iterator_close(&this);
                return Err(JsError::type_err("predicate is not a function"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let mut counter = 0usize;
            while let Some(v) = step_record(rt, &iterated)? {
                let res = match rt.call_function(
                    pred.clone(),
                    JsValue::Undefined,
                    &[v, JsValue::Number(counter as f64)],
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        rt.iterator_close(&iterated.iterator);
                        return Err(e);
                    }
                };
                if res.to_boolean() {
                    rt.iterator_close_checked(&iterated.iterator, false)?;
                    return Ok(JsValue::Boolean(true));
                }
                counter += 1;
            }
            Ok(JsValue::Boolean(false))
        }),
    );

    set_method(
        rt,
        proto,
        "every",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.every requires an object"));
            };
            let pred = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&pred) {
                rt.iterator_close(&this);
                return Err(JsError::type_err("predicate is not a function"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let mut counter = 0usize;
            while let Some(v) = step_record(rt, &iterated)? {
                let res = match rt.call_function(
                    pred.clone(),
                    JsValue::Undefined,
                    &[v, JsValue::Number(counter as f64)],
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        rt.iterator_close(&iterated.iterator);
                        return Err(e);
                    }
                };
                if !res.to_boolean() {
                    rt.iterator_close_checked(&iterated.iterator, false)?;
                    return Ok(JsValue::Boolean(false));
                }
                counter += 1;
            }
            Ok(JsValue::Boolean(true))
        }),
    );

    set_method(
        rt,
        proto,
        "find",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err("Iterator.prototype.find requires an object"));
            };
            let pred = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&pred) {
                rt.iterator_close(&this);
                return Err(JsError::type_err("predicate is not a function"));
            }
            let iterated = get_iterator_direct(rt, &this)?;
            let mut counter = 0usize;
            while let Some(v) = step_record(rt, &iterated)? {
                let res = match rt.call_function(
                    pred.clone(),
                    JsValue::Undefined,
                    &[v.clone(), JsValue::Number(counter as f64)],
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        rt.iterator_close(&iterated.iterator);
                        return Err(e);
                    }
                };
                if res.to_boolean() {
                    rt.iterator_close_checked(&iterated.iterator, false)?;
                    return Ok(v);
                }
                counter += 1;
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "includes",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err(
                    "Iterator.prototype.includes requires an object",
                ));
            };
            let search = args.first().cloned().unwrap_or(JsValue::Undefined);
            let to_skip = match args.get(1) {
                None | Some(JsValue::Undefined) => 0.0,
                Some(JsValue::Number(n)) => {
                    if !n.is_infinite() && !(n.is_finite() && n.fract() == 0.0) {
                        rt.iterator_close(&this);
                        return Err(JsError::type_err(
                            "skippedElements must be an integral number or infinity",
                        ));
                    }
                    if *n < 0.0 || (n.is_finite() && *n > 9_007_199_254_740_991.0) {
                        rt.iterator_close(&this);
                        return Err(JsError::range("skippedElements out of range"));
                    }
                    *n
                }
                Some(_) => {
                    rt.iterator_close(&this);
                    return Err(JsError::type_err("skippedElements must be a number"));
                }
            };
            let iterated = get_iterator_direct(rt, &this)?;
            let mut skipped = 0.0_f64;
            while let Some(v) = step_record(rt, &iterated)? {
                if skipped < to_skip {
                    skipped += 1.0;
                    continue;
                }
                if v.same_value_zero(&search) {
                    rt.iterator_close_checked(&iterated.iterator, false)?;
                    return Ok(JsValue::Boolean(true));
                }
            }
            Ok(JsValue::Boolean(false))
        }),
    );

    set_method(
        rt,
        proto,
        "join",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err(
                    "Iterator.prototype.join requires an object",
                ));
            };
            let sep = match args.first() {
                None | Some(JsValue::Undefined) => ",".to_string(),
                Some(v) => match rt.to_string(v) {
                    Ok(s) => s,
                    Err(e) => {
                        rt.iterator_close(&this);
                        return Err(e);
                    }
                },
            };
            let iterated = get_iterator_direct(rt, &this)?;
            let mut out = String::new();
            let mut first = true;
            while let Some(v) = step_record(rt, &iterated)? {
                if !first {
                    out.push_str(&sep);
                }
                first = false;
                if !v.is_nullish() {
                    match rt.to_string(&v) {
                        Ok(s) => out.push_str(&s),
                        Err(e) => {
                            rt.iterator_close(&iterated.iterator);
                            return Err(e);
                        }
                    }
                }
            }
            Ok(JsValue::String(out))
        }),
    );

    set_method(
        rt,
        proto,
        "chunks",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err(
                    "Iterator.prototype.chunks requires an object",
                ));
            };
            let chunk_size = match args.first() {
                Some(JsValue::Number(n)) if n.is_finite() && n.fract() == 0.0 => {
                    if *n < 1.0 || *n > 4_294_967_295.0 {
                        rt.iterator_close(&this);
                        return Err(JsError::range("chunkSize out of range"));
                    }
                    *n as usize
                }
                _ => {
                    rt.iterator_close(&this);
                    return Err(JsError::type_err("chunkSize must be an integral number"));
                }
            };
            let iterated = get_iterator_direct(rt, &this)?;
            let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
                IteratorHelperData {
                    state: IteratorHelperState::SuspendedStart,
                    kind: IteratorHelperKind::Chunks {
                        iterated,
                        chunk_size,
                        exhausted: false,
                    },
                },
            ));
            Ok(JsValue::Object(helper))
        }),
    );

    set_method(
        rt,
        proto,
        "windows",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(_) = this else {
                return Err(JsError::type_err(
                    "Iterator.prototype.windows requires an object",
                ));
            };
            let window_size = match args.first() {
                Some(JsValue::Number(n)) if n.is_finite() && n.fract() == 0.0 => {
                    if *n < 1.0 || *n > 4_294_967_295.0 {
                        rt.iterator_close(&this);
                        return Err(JsError::range("windowSize out of range"));
                    }
                    *n as usize
                }
                _ => {
                    rt.iterator_close(&this);
                    return Err(JsError::type_err("windowSize must be an integral number"));
                }
            };
            let allow_partial = match args.get(1) {
                None | Some(JsValue::Undefined) => false,
                Some(JsValue::String(s)) if s == "only-full" => false,
                Some(JsValue::String(s)) if s == "allow-partial" => true,
                _ => {
                    rt.iterator_close(&this);
                    return Err(JsError::type_err(
                        "undersized must be 'only-full' or 'allow-partial'",
                    ));
                }
            };
            let iterated = get_iterator_direct(rt, &this)?;
            let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
                IteratorHelperData {
                    state: IteratorHelperState::SuspendedStart,
                    kind: IteratorHelperKind::Windows {
                        iterated,
                        window_size,
                        allow_partial,
                        buffer: Vec::new(),
                        exhausted: false,
                    },
                },
            ));
            Ok(JsValue::Object(helper))
        }),
    );

    rt.heap.get_obj_mut(helper_proto).set_own_prop(
        PropertyKey::Symbol(sym_to_string_tag),
        PropEntry::data(JsValue::String("Iterator Helper".into()), false, false, true),
    );
    set_method(
        rt,
        helper_proto,
        "next",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Iterator Helper next requires an object"));
            };
            helper_next(rt, oid)
        }),
    );
    set_method(
        rt,
        helper_proto,
        "return",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Iterator Helper return requires an object"));
            };
            helper_return(rt, oid)
        }),
    );

    set_method(
        rt,
        wrap_proto,
        "next",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Wrapped iterator next requires an object"));
            };
            let rec = match &rt.heap.get_obj(oid).kind {
                ObjectKind::Iterator(IteratorData {
                    kind: BuiltinIteratorKind::Wrapped(r),
                    ..
                }) => r.clone(),
                _ => return Err(JsError::type_err("Incompatible receiver for wrapped iterator")),
            };
            rt.call_function(rec.next_method, rec.iterator, &[])
        }),
    );
    set_method(
        rt,
        wrap_proto,
        "return",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Wrapped iterator return requires an object"));
            };
            let rec = match &rt.heap.get_obj(oid).kind {
                ObjectKind::Iterator(IteratorData {
                    kind: BuiltinIteratorKind::Wrapped(r),
                    ..
                }) => r.clone(),
                _ => return Err(JsError::type_err("Incompatible receiver for wrapped iterator")),
            };
            let JsValue::Object(inner_id) = rec.iterator else {
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            };
            let ret_fn = rt.get_property(inner_id, "return")?;
            if ret_fn.is_nullish() {
                return Ok(rt.create_iter_result(JsValue::Undefined, true));
            }
            if !rt.is_callable(&ret_fn) {
                return Err(JsError::type_err("Iterator return is not callable"));
            }
            rt.call_function(ret_fn, rec.iterator, &[])
        }),
    );

    set_global(rt, "Iterator", JsValue::Object(ctor));
}

fn setter_ignoring_prototype(
    rt: &mut JsRuntime,
    this: &JsValue,
    home: ObjectId,
    key: PropertyKey,
    val: JsValue,
) -> Result<JsValue, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("Setter called on non-object"));
    };
    if *oid == home {
        return Err(JsError::type_err("Cannot assign to property on Iterator.prototype"));
    }
    let desc = rt.get_own_property_descriptor(*oid, &key)?;
    if desc.is_none() {
        create_data_property_or_throw(rt, *oid, key, val)?;
    } else {
        rt.set_property_key_strict(*oid, key, val, true)?;
    }
    Ok(JsValue::Undefined)
}

fn is_instance_of_iterator(rt: &mut JsRuntime, val: &JsValue) -> Result<bool, JsError> {
    let JsValue::Object(mut cur) = val else {
        return Ok(false);
    };
    let target = rt.intrinsics.iterator_proto;
    while let Some(p) = rt.get_prototype_of(cur)? {
        if p == target {
            return Ok(true);
        }
        cur = p;
    }
    Ok(false)
}

fn get_iterator_direct(rt: &mut JsRuntime, obj: &JsValue) -> Result<IterRecord, JsError> {
    let JsValue::Object(oid) = obj else {
        return Err(JsError::type_err("Iterator must be an object"));
    };
    let next_method = rt.get_property(*oid, "next")?;
    Ok(IterRecord {
        iterator: obj.clone(),
        next_method,
    })
}

fn get_iterator_flattenable(
    rt: &mut JsRuntime,
    obj: &JsValue,
    allow_strings: bool,
) -> Result<IterRecord, JsError> {
    let is_ok = match obj {
        JsValue::Object(_) => true,
        JsValue::String(_) => allow_strings,
        _ => false,
    };
    if !is_ok {
        return Err(JsError::type_err("Value is not an object"));
    }
    let sym_iter = PropertyKey::Symbol(rt.symbols.iterator);
    let method = match obj {
        JsValue::Object(oid) => rt.get_property_key(*oid, &sym_iter)?,
        JsValue::String(_) => {
            let proto = rt.intrinsics.string_proto;
            rt.get_property_key_receiver(proto, &sym_iter, obj.clone())?
        }
        _ => unreachable!(),
    };
    let iterator = if method.is_nullish() {
        obj.clone()
    } else {
        if !rt.is_callable(&method) {
            return Err(JsError::type_err("Symbol.iterator is not callable"));
        }
        rt.call_function(method, obj.clone(), &[])?
    };
    let JsValue::Object(_) = iterator else {
        return Err(JsError::type_err("Iterator must be an object"));
    };
    get_iterator_direct(rt, &iterator)
}

fn step_record(rt: &mut JsRuntime, rec: &IterRecord) -> Result<Option<JsValue>, JsError> {
    let res = rt.call_function(rec.next_method.clone(), rec.iterator.clone(), &[])?;
    let JsValue::Object(res_id) = res else {
        return Err(JsError::type_err("Iterator result is not an object"));
    };
    if rt.get_property(res_id, "done")?.to_boolean() {
        Ok(None)
    } else {
        Ok(Some(rt.get_property(res_id, "value")?))
    }
}

fn set_helper_state(rt: &mut JsRuntime, oid: ObjectId, state: IteratorHelperState) {
    if let ObjectKind::Iterator(IteratorData {
        kind: BuiltinIteratorKind::Helper(ref mut hd),
        ..
    }) = rt.heap.get_obj_mut(oid).kind
    {
        hd.state = state;
    }
}

fn helper_next(rt: &mut JsRuntime, oid: ObjectId) -> Result<JsValue, JsError> {
    let (state, kind) = match &rt.heap.get_obj(oid).kind {
        ObjectKind::Iterator(IteratorData {
            kind: BuiltinIteratorKind::Helper(hd),
            ..
        }) => (hd.state, hd.kind.clone()),
        _ => return Err(JsError::type_err("Incompatible receiver for Iterator Helper")),
    };
    match state {
        IteratorHelperState::Executing => {
            return Err(JsError::type_err("Iterator Helper is already executing"));
        }
        IteratorHelperState::Completed => {
            return Ok(rt.create_iter_result(JsValue::Undefined, true));
        }
        IteratorHelperState::SuspendedStart | IteratorHelperState::SuspendedYield => {}
    }
    set_helper_state(rt, oid, IteratorHelperState::Executing);

    let step_res: Result<(Option<JsValue>, IteratorHelperKind), JsError> = match kind {
        IteratorHelperKind::Map {
            iterated,
            mapper,
            mut counter,
        } => (|| {
            let Some(v) = step_record(rt, &iterated)? else {
                return Ok((
                    None,
                    IteratorHelperKind::Map {
                        iterated,
                        mapper,
                        counter,
                    },
                ));
            };
            let mapped = match rt.call_function(
                mapper.clone(),
                JsValue::Undefined,
                &[v, JsValue::Number(counter as f64)],
            ) {
                Ok(m) => m,
                Err(e) => {
                    rt.iterator_close(&iterated.iterator);
                    return Err(e);
                }
            };
            counter += 1;
            Ok((
                Some(mapped),
                IteratorHelperKind::Map {
                    iterated,
                    mapper,
                    counter,
                },
            ))
        })(),
        IteratorHelperKind::Filter {
            iterated,
            predicate,
            mut counter,
        } => (|| loop {
            let Some(v) = step_record(rt, &iterated)? else {
                return Ok((
                    None,
                    IteratorHelperKind::Filter {
                        iterated,
                        predicate,
                        counter,
                    },
                ));
            };
            let ok = match rt.call_function(
                predicate.clone(),
                JsValue::Undefined,
                &[v.clone(), JsValue::Number(counter as f64)],
            ) {
                Ok(r) => r.to_boolean(),
                Err(e) => {
                    rt.iterator_close(&iterated.iterator);
                    return Err(e);
                }
            };
            counter += 1;
            if ok {
                return Ok((
                    Some(v),
                    IteratorHelperKind::Filter {
                        iterated,
                        predicate,
                        counter,
                    },
                ));
            }
        })(),
        IteratorHelperKind::Take {
            iterated,
            mut remaining,
        } => (|| {
            if remaining == 0.0 {
                rt.iterator_close_checked(&iterated.iterator, false)?;
                return Ok((None, IteratorHelperKind::Take { iterated, remaining }));
            }
            if remaining.is_finite() {
                remaining -= 1.0;
            }
            let Some(v) = step_record(rt, &iterated)? else {
                return Ok((None, IteratorHelperKind::Take { iterated, remaining }));
            };
            Ok((Some(v), IteratorHelperKind::Take { iterated, remaining }))
        })(),
        IteratorHelperKind::Drop {
            iterated,
            mut remaining,
        } => (|| {
            while remaining > 0.0 {
                if remaining.is_finite() {
                    remaining -= 1.0;
                }
                if step_record(rt, &iterated)?.is_none() {
                    return Ok((None, IteratorHelperKind::Drop { iterated, remaining }));
                }
            }
            let Some(v) = step_record(rt, &iterated)? else {
                return Ok((None, IteratorHelperKind::Drop { iterated, remaining }));
            };
            Ok((Some(v), IteratorHelperKind::Drop { iterated, remaining }))
        })(),
        IteratorHelperKind::FlatMap {
            iterated,
            mapper,
            mut counter,
            mut inner,
        } => (|| loop {
            if let Some(inn) = inner.take() {
                match step_record(rt, &inn) {
                    Ok(Some(iv)) => {
                        return Ok((
                            Some(iv),
                            IteratorHelperKind::FlatMap {
                                iterated,
                                mapper,
                                counter,
                                inner: Some(inn),
                            },
                        ));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        rt.iterator_close(&iterated.iterator);
                        return Err(e);
                    }
                }
            }
            let Some(v) = step_record(rt, &iterated)? else {
                return Ok((
                    None,
                    IteratorHelperKind::FlatMap {
                        iterated,
                        mapper,
                        counter,
                        inner: None,
                    },
                ));
            };
            let mapped = match rt.call_function(
                mapper.clone(),
                JsValue::Undefined,
                &[v, JsValue::Number(counter as f64)],
            ) {
                Ok(m) => m,
                Err(e) => {
                    rt.iterator_close(&iterated.iterator);
                    return Err(e);
                }
            };
            counter += 1;
            let next_inner = match get_iterator_flattenable(rt, &mapped, false) {
                Ok(r) => r,
                Err(e) => {
                    rt.iterator_close(&iterated.iterator);
                    return Err(e);
                }
            };
            inner = Some(next_inner);
        })(),
        IteratorHelperKind::Concat {
            iterables,
            mut pos,
            mut inner,
        } => (|| loop {
            if let Some(inn) = inner.take() {
                match step_record(rt, &inn)? {
                    Some(v) => {
                        return Ok((
                            Some(v),
                            IteratorHelperKind::Concat {
                                iterables,
                                pos,
                                inner: Some(inn),
                            },
                        ));
                    }
                    None => {}
                }
            }
            if pos >= iterables.len() {
                return Ok((
                    None,
                    IteratorHelperKind::Concat {
                        iterables,
                        pos,
                        inner: None,
                    },
                ));
            }
            let (obj, method) = iterables[pos].clone();
            pos += 1;
            let iter = rt.call_function(method, obj, &[])?;
            let JsValue::Object(_) = iter else {
                return Err(JsError::type_err("Iterator must be an object"));
            };
            let rec = get_iterator_direct(rt, &iter)?;
            inner = Some(rec);
        })(),
        IteratorHelperKind::Chunks {
            iterated,
            chunk_size,
            mut exhausted,
        } => (|| {
            if exhausted {
                return Ok((
                    None,
                    IteratorHelperKind::Chunks {
                        iterated,
                        chunk_size,
                        exhausted: true,
                    },
                ));
            }
            let mut buf = Vec::new();
            while buf.len() < chunk_size {
                match step_record(rt, &iterated)? {
                    Some(v) => buf.push(v),
                    None => {
                        exhausted = true;
                        break;
                    }
                }
            }
            if buf.is_empty() {
                Ok((
                    None,
                    IteratorHelperKind::Chunks {
                        iterated,
                        chunk_size,
                        exhausted: true,
                    },
                ))
            } else {
                let arr = rt.create_array(buf);
                Ok((
                    Some(JsValue::Object(arr)),
                    IteratorHelperKind::Chunks {
                        iterated,
                        chunk_size,
                        exhausted,
                    },
                ))
            }
        })(),
        IteratorHelperKind::Windows {
            iterated,
            window_size,
            allow_partial,
            mut buffer,
            mut exhausted,
        } => (|| {
            if exhausted {
                return Ok((
                    None,
                    IteratorHelperKind::Windows {
                        iterated,
                        window_size,
                        allow_partial,
                        buffer,
                        exhausted: true,
                    },
                ));
            }
            if buffer.len() == window_size {
                buffer.remove(0);
                match step_record(rt, &iterated)? {
                    Some(v) => {
                        buffer.push(v);
                        let arr = rt.create_array(buffer.clone());
                        Ok((
                            Some(JsValue::Object(arr)),
                            IteratorHelperKind::Windows {
                                iterated,
                                window_size,
                                allow_partial,
                                buffer,
                                exhausted: false,
                            },
                        ))
                    }
                    None => Ok((
                        None,
                        IteratorHelperKind::Windows {
                            iterated,
                            window_size,
                            allow_partial,
                            buffer: Vec::new(),
                            exhausted: true,
                        },
                    )),
                }
            } else {
                while buffer.len() < window_size {
                    match step_record(rt, &iterated)? {
                        Some(v) => buffer.push(v),
                        None => {
                            exhausted = true;
                            if allow_partial && !buffer.is_empty() {
                                let arr = rt.create_array(std::mem::take(&mut buffer));
                                return Ok((
                                    Some(JsValue::Object(arr)),
                                    IteratorHelperKind::Windows {
                                        iterated,
                                        window_size,
                                        allow_partial,
                                        buffer: Vec::new(),
                                        exhausted: true,
                                    },
                                ));
                            }
                            return Ok((
                                None,
                                IteratorHelperKind::Windows {
                                    iterated,
                                    window_size,
                                    allow_partial,
                                    buffer: Vec::new(),
                                    exhausted: true,
                                },
                            ));
                        }
                    }
                }
                let arr = rt.create_array(buffer.clone());
                Ok((
                    Some(JsValue::Object(arr)),
                    IteratorHelperKind::Windows {
                        iterated,
                        window_size,
                        allow_partial,
                        buffer,
                        exhausted: false,
                    },
                ))
            }
        })(),
        IteratorHelperKind::Zip {
            mut iters,
            mode,
            padding,
            keys,
        } => (|| {
            let count = iters.len();
            if count == 0 {
                return Ok((
                    None,
                    IteratorHelperKind::Zip {
                        iters,
                        mode,
                        padding,
                        keys,
                    },
                ));
            }
            let mut results = Vec::with_capacity(count);
            for i in 0..count {
                let Some(rec) = iters[i].clone() else {
                    results.push(padding[i].clone());
                    continue;
                };
                match step_record(rt, &rec) {
                    Err(e) => {
                        iters[i] = None;
                        iterator_close_all(rt, &iters, Some(e))?;
                        unreachable!();
                    }
                    Ok(Some(v)) => results.push(v),
                    Ok(None) => {
                        iters[i] = None;
                        match mode {
                            ZipMode::Shortest => {
                                iterator_close_all(rt, &iters, None)?;
                                return Ok((
                                    None,
                                    IteratorHelperKind::Zip {
                                        iters,
                                        mode,
                                        padding,
                                        keys,
                                    },
                                ));
                            }
                            ZipMode::Strict => {
                                if i != 0 {
                                    iterator_close_all(
                                        rt,
                                        &iters,
                                        Some(JsError::type_err(
                                            "Iterator.zip strict mode: iterators have different lengths",
                                        )),
                                    )?;
                                    unreachable!();
                                }
                                for k in 1..count {
                                    let Some(rk) = iters[k].clone() else {
                                        continue;
                                    };
                                    match step_record(rt, &rk) {
                                        Err(e) => {
                                            iters[k] = None;
                                            iterator_close_all(rt, &iters, Some(e))?;
                                            unreachable!();
                                        }
                                        Ok(None) => {
                                            iters[k] = None;
                                        }
                                        Ok(Some(_)) => {
                                            iterator_close_all(
                                                rt,
                                                &iters,
                                                Some(JsError::type_err(
                                                    "Iterator.zip strict mode: iterators have different lengths",
                                                )),
                                            )?;
                                            unreachable!();
                                        }
                                    }
                                }
                                return Ok((
                                    None,
                                    IteratorHelperKind::Zip {
                                        iters,
                                        mode,
                                        padding,
                                        keys,
                                    },
                                ));
                            }
                            ZipMode::Longest => {
                                if iters.iter().all(|it| it.is_none()) {
                                    return Ok((
                                        None,
                                        IteratorHelperKind::Zip {
                                            iters,
                                            mode,
                                            padding,
                                            keys,
                                        },
                                    ));
                                }
                                results.push(padding[i].clone());
                            }
                        }
                    }
                }
            }
            let out_val = if let Some(ref ks) = keys {
                let obj = rt.create_object_with_proto(None);
                for (k, v) in ks.iter().zip(results) {
                    rt.heap.get_obj_mut(obj).set_own_prop(
                        k.clone(),
                        PropEntry::data(v, true, true, true),
                    );
                }
                JsValue::Object(obj)
            } else {
                JsValue::Object(rt.create_array(results))
            };
            Ok((
                Some(out_val),
                IteratorHelperKind::Zip {
                    iters,
                    mode,
                    padding,
                    keys,
                },
            ))
        })(),
    };

    match step_res {
        Ok((Some(val), next_kind)) => {
            if let ObjectKind::Iterator(IteratorData {
                kind: BuiltinIteratorKind::Helper(ref mut hd),
                ..
            }) = rt.heap.get_obj_mut(oid).kind
            {
                hd.state = IteratorHelperState::SuspendedYield;
                hd.kind = next_kind;
            }
            Ok(rt.create_iter_result(val, false))
        }
        Ok((None, next_kind)) => {
            if let ObjectKind::Iterator(IteratorData {
                kind: BuiltinIteratorKind::Helper(ref mut hd),
                ..
            }) = rt.heap.get_obj_mut(oid).kind
            {
                hd.state = IteratorHelperState::Completed;
                hd.kind = next_kind;
            }
            Ok(rt.create_iter_result(JsValue::Undefined, true))
        }
        Err(e) => {
            set_helper_state(rt, oid, IteratorHelperState::Completed);
            Err(e)
        }
    }
}

fn helper_return(rt: &mut JsRuntime, oid: ObjectId) -> Result<JsValue, JsError> {
    let (state, kind) = match &rt.heap.get_obj(oid).kind {
        ObjectKind::Iterator(IteratorData {
            kind: BuiltinIteratorKind::Helper(hd),
            ..
        }) => (hd.state, hd.kind.clone()),
        _ => return Err(JsError::type_err("Incompatible receiver for Iterator Helper")),
    };
    let was_suspended_start = matches!(state, IteratorHelperState::SuspendedStart);
    match state {
        IteratorHelperState::Executing => {
            return Err(JsError::type_err("Iterator Helper is already executing"));
        }
        IteratorHelperState::Completed => {
            return Ok(rt.create_iter_result(JsValue::Undefined, true));
        }
        IteratorHelperState::SuspendedStart | IteratorHelperState::SuspendedYield => {}
    }
    if was_suspended_start {
        set_helper_state(rt, oid, IteratorHelperState::Completed);
    } else {
        set_helper_state(rt, oid, IteratorHelperState::Executing);
    }
    let close_res: Result<(), JsError> = match kind {
        IteratorHelperKind::Map { iterated, .. }
        | IteratorHelperKind::Filter { iterated, .. }
        | IteratorHelperKind::Take { iterated, .. }
        | IteratorHelperKind::Drop { iterated, .. } => {
            rt.iterator_close_checked(&iterated.iterator, false)
        }
        IteratorHelperKind::FlatMap {
            iterated, inner, ..
        } => {
            let r1 = if let Some(inn) = inner {
                rt.iterator_close_checked(&inn.iterator, false)
            } else {
                Ok(())
            };
            match r1 {
                Ok(()) => rt.iterator_close_checked(&iterated.iterator, false),
                Err(e) => {
                    rt.iterator_close(&iterated.iterator);
                    Err(e)
                }
            }
        }
        IteratorHelperKind::Concat { inner, .. } => {
            if let Some(inn) = inner {
                rt.iterator_close_checked(&inn.iterator, false)
            } else {
                Ok(())
            }
        }
        IteratorHelperKind::Chunks {
            iterated,
            exhausted,
            ..
        }
        | IteratorHelperKind::Windows {
            iterated,
            exhausted,
            ..
        } => {
            if exhausted {
                Ok(())
            } else {
                rt.iterator_close_checked(&iterated.iterator, false)
            }
        }
        IteratorHelperKind::Zip { iters, .. } => iterator_close_all(rt, &iters, None),
    };
    set_helper_state(rt, oid, IteratorHelperState::Completed);
    close_res?;
    Ok(rt.create_iter_result(JsValue::Undefined, true))
}

fn parse_zip_options(
    rt: &mut JsRuntime,
    options_val: Option<&JsValue>,
) -> Result<(ZipMode, Option<JsValue>), JsError> {
    let opts_oid = match options_val {
        None | Some(JsValue::Undefined) => None,
        Some(JsValue::Object(o)) => Some(*o),
        _ => return Err(JsError::type_err("options must be an object or undefined")),
    };
    let mode_val = if let Some(o) = opts_oid {
        rt.get_property(o, "mode")?
    } else {
        JsValue::Undefined
    };
    let mode = match &mode_val {
        JsValue::Undefined => ZipMode::Shortest,
        JsValue::String(s) if s == "shortest" => ZipMode::Shortest,
        JsValue::String(s) if s == "longest" => ZipMode::Longest,
        JsValue::String(s) if s == "strict" => ZipMode::Strict,
        _ => return Err(JsError::type_err("Invalid mode option for Iterator.zip")),
    };
    let padding_opt = if mode == ZipMode::Longest {
        let p = if let Some(o) = opts_oid {
            rt.get_property(o, "padding")?
        } else {
            JsValue::Undefined
        };
        if p.is_undefined() {
            None
        } else if matches!(p, JsValue::Object(_)) {
            Some(p)
        } else {
            return Err(JsError::type_err("padding option must be an object or undefined"));
        }
    } else {
        None
    };
    Ok((mode, padding_opt))
}

fn iterator_close_all(
    rt: &mut JsRuntime,
    iters: &[Option<IterRecord>],
    mut err: Option<JsError>,
) -> Result<(), JsError> {
    for it in iters.iter().rev().flatten() {
        if err.is_some() {
            rt.iterator_close(&it.iterator);
        } else if let Err(e) = rt.iterator_close_checked(&it.iterator, false) {
            err = Some(e);
        }
    }
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn get_sync_iterator(rt: &mut JsRuntime, obj: &JsValue) -> Result<IterRecord, JsError> {
    let JsValue::Object(oid) = obj else {
        return Err(JsError::type_err("Value is not iterable"));
    };
    let sym_iter = PropertyKey::Symbol(rt.symbols.iterator);
    let method = rt.get_property_key(*oid, &sym_iter)?;
    if !rt.is_callable(&method) {
        return Err(JsError::type_err("Value is not iterable"));
    }
    let iter = rt.call_function(method, obj.clone(), &[])?;
    let JsValue::Object(_) = iter else {
        return Err(JsError::type_err("Iterator must be an object"));
    };
    get_iterator_direct(rt, &iter)
}

fn iterator_zip(rt: &mut JsRuntime, args: &[JsValue]) -> Result<JsValue, JsError> {
    let iterables = args.first().cloned().unwrap_or(JsValue::Undefined);
    let JsValue::Object(_) = iterables else {
        return Err(JsError::type_err("Iterator.zip iterables must be an object"));
    };
    let (mode, padding_opt) = parse_zip_options(rt, args.get(1))?;
    let input_iter = get_sync_iterator(rt, &iterables)?;
    let mut iters: Vec<Option<IterRecord>> = Vec::new();
    loop {
        let item = match step_record(rt, &input_iter) {
            Ok(Some(v)) => v,
            Ok(None) => break,
            Err(e) => {
                iterator_close_all(rt, &iters, Some(e))?;
                unreachable!();
            }
        };
        match get_iterator_flattenable(rt, &item, false) {
            Ok(rec) => iters.push(Some(rec)),
            Err(e) => {
                let _ = iterator_close_all(rt, &iters, Some(e.clone()));
                rt.iterator_close(&input_iter.iterator);
                return Err(e);
            }
        }
    }
    let mut padding = Vec::with_capacity(iters.len());
    if mode == ZipMode::Longest {
        if let Some(pad_obj) = padding_opt {
            let pad_iter = match get_sync_iterator(rt, &pad_obj) {
                Ok(r) => r,
                Err(e) => {
                    iterator_close_all(rt, &iters, Some(e))?;
                    unreachable!();
                }
            };
            let mut using_padding = true;
            for _ in 0..iters.len() {
                if using_padding {
                    match step_record(rt, &pad_iter) {
                        Ok(Some(v)) => padding.push(v),
                        Ok(None) => {
                            using_padding = false;
                            padding.push(JsValue::Undefined);
                        }
                        Err(e) => {
                            iterator_close_all(rt, &iters, Some(e))?;
                            unreachable!();
                        }
                    }
                } else {
                    padding.push(JsValue::Undefined);
                }
            }
            if using_padding {
                if let Err(e) = rt.iterator_close_checked(&pad_iter.iterator, false) {
                    iterator_close_all(rt, &iters, Some(e))?;
                    unreachable!();
                }
            }
        } else {
            padding.resize(iters.len(), JsValue::Undefined);
        }
    }
    let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
        IteratorHelperData {
            state: IteratorHelperState::SuspendedStart,
            kind: IteratorHelperKind::Zip {
                iters,
                mode,
                padding,
                keys: None,
            },
        },
    ));
    Ok(JsValue::Object(helper))
}

fn iterator_zip_keyed(rt: &mut JsRuntime, args: &[JsValue]) -> Result<JsValue, JsError> {
    let iterables = args.first().cloned().unwrap_or(JsValue::Undefined);
    let JsValue::Object(iterables_oid) = iterables else {
        return Err(JsError::type_err(
            "Iterator.zipKeyed iterables must be an object",
        ));
    };
    let (mode, padding_opt) = parse_zip_options(rt, args.get(1))?;
    let all_keys = rt.own_keys(iterables_oid)?;
    let mut keys: Vec<PropertyKey> = Vec::new();
    let mut iters: Vec<Option<IterRecord>> = Vec::new();
    for k in all_keys {
        let desc = match rt.get_own_property_descriptor(iterables_oid, &k) {
            Ok(d) => d,
            Err(e) => {
                iterator_close_all(rt, &iters, Some(e))?;
                unreachable!();
            }
        };
        if let Some(d) = desc {
            if d.enumerable == Some(true) {
                let val = match rt.get_property_key(iterables_oid, &k) {
                    Ok(v) => v,
                    Err(e) => {
                        iterator_close_all(rt, &iters, Some(e))?;
                        unreachable!();
                    }
                };
                if !val.is_undefined() {
                    match get_iterator_flattenable(rt, &val, false) {
                        Ok(rec) => {
                            keys.push(k);
                            iters.push(Some(rec));
                        }
                        Err(e) => {
                            iterator_close_all(rt, &iters, Some(e))?;
                            unreachable!();
                        }
                    }
                }
            }
        }
    }
    let mut padding = Vec::with_capacity(iters.len());
    if mode == ZipMode::Longest {
        if let Some(JsValue::Object(pad_oid)) = padding_opt {
            for k in &keys {
                match rt.get_property_key(pad_oid, k) {
                    Ok(v) => padding.push(v),
                    Err(e) => {
                        iterator_close_all(rt, &iters, Some(e))?;
                        unreachable!();
                    }
                }
            }
        } else {
            padding.resize(iters.len(), JsValue::Undefined);
        }
    }
    let helper = rt.create_builtin_iterator(BuiltinIteratorKind::Helper(
        IteratorHelperData {
            state: IteratorHelperState::SuspendedStart,
            kind: IteratorHelperKind::Zip {
                iters,
                mode,
                padding,
                keys: Some(keys),
            },
        },
    ));
    Ok(JsValue::Object(helper))
}
