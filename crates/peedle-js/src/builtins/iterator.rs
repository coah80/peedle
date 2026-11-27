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
    };
    set_helper_state(rt, oid, IteratorHelperState::Completed);
    close_res?;
    Ok(rt.create_iter_result(JsValue::Undefined, true))
}
