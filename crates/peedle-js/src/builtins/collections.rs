use crate::builtins::{set_getter, set_global, set_method, set_symbol_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    init_map(rt);
    init_set(rt);
    init_weak_map(rt);
    init_weak_set(rt);
}

fn init_map(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.map_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("Map".into()), false, false, true),
    );

    set_getter(
        rt,
        proto,
        "size",
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.size called on non-object"));
            };
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::Map(m) => Ok(JsValue::Number(m.count as f64)),
                _ => Err(JsError::type_err("Map.prototype.size called on non-Map")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "get",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.get called on non-object"));
            };
            let key = args.first().unwrap_or(&JsValue::Undefined);
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::Map(m) => Ok(m.get(key).cloned().unwrap_or(JsValue::Undefined)),
                _ => Err(JsError::type_err("Map.prototype.get called on non-Map")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "has",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.has called on non-object"));
            };
            let key = args.first().unwrap_or(&JsValue::Undefined);
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::Map(m) => Ok(JsValue::Boolean(m.has(key))),
                _ => Err(JsError::type_err("Map.prototype.has called on non-Map")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "set",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.set called on non-object"));
            };
            let key = args.first().cloned().unwrap_or(JsValue::Undefined);
            let val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::Map(m) => {
                    m.set(key, val);
                    Ok(JsValue::Object(oid))
                }
                _ => Err(JsError::type_err("Map.prototype.set called on non-Map")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "delete",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.delete called on non-object"));
            };
            let key = args.first().unwrap_or(&JsValue::Undefined);
            match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::Map(m) => Ok(JsValue::Boolean(m.delete(key))),
                _ => Err(JsError::type_err("Map.prototype.delete called on non-Map")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "clear",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.clear called on non-object"));
            };
            match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::Map(m) => {
                    m.clear();
                    Ok(JsValue::Undefined)
                }
                _ => Err(JsError::type_err("Map.prototype.clear called on non-Map")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "forEach",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.forEach called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::Map(_)) {
                return Err(JsError::type_err("Map.prototype.forEach called on non-Map"));
            }
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut i = 0;
            loop {
                let next = match &rt.heap.get_obj(oid).kind {
                    ObjectKind::Map(m) => {
                        let mut found = None;
                        while i < m.entries.len() {
                            if let Some((k, v)) = &m.entries[i] {
                                found = Some((k.clone(), v.clone()));
                                i += 1;
                                break;
                            }
                            i += 1;
                        }
                        found
                    }
                    _ => None,
                };
                let Some((k, v)) = next else { break };
                rt.call_function(cb.clone(), this_arg.clone(), &[v, k, JsValue::Object(oid)])?;
            }
            Ok(JsValue::Undefined)
        }),
    );

    let entries_fn = rt.create_native_function(
        "entries",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.entries called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::Map(_)) {
                return Err(JsError::type_err("Map.prototype.entries called on non-Map"));
            }
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Map {
                    target: oid,
                    mode: IterMode::Entries,
                },
            )))
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("entries"),
        PropEntry::data(JsValue::Object(entries_fn), true, false, true),
    );
    let sym_iter = rt.symbols.iterator;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_iter),
        PropEntry::data(JsValue::Object(entries_fn), true, false, true),
    );
    set_method(
        rt,
        proto,
        "keys",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.keys called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::Map(_)) {
                return Err(JsError::type_err("Map.prototype.keys called on non-Map"));
            }
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Map {
                    target: oid,
                    mode: IterMode::Keys,
                },
            )))
        }),
    );
    set_method(
        rt,
        proto,
        "values",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.values called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::Map(_)) {
                return Err(JsError::type_err("Map.prototype.values called on non-Map"));
            }
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Map {
                    target: oid,
                    mode: IterMode::Values,
                },
            )))
        }),
    );

    let ctor = rt.create_native_constructor(
        "Map",
        0,
        proto,
        Rc::new(|rt, this, args| {
            if !this.is_object() {
                return Err(JsError::type_err("Constructor Map requires 'new'"));
            }
            let map_obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.map_proto),
                ObjectKind::Map(MapData::default()),
            ));
            if let Some(iterable) = args.first() {
                if !iterable.is_nullish() {
                    let adder = rt.get_property(map_obj, "set")?;
                    if !rt.is_callable(&adder) {
                        return Err(JsError::type_err("Map.prototype.set is not a function"));
                    }
                    let iter = rt.get_iterator(iterable, false)?;
                    while let Some(item) = rt.iterator_step(&iter)? {
                        let res = (|| -> Result<(), JsError> {
                            let JsValue::Object(iid) = item else {
                                return Err(JsError::type_err(
                                    "Iterator value is not an entry object",
                                ));
                            };
                            let k = rt.get_property(iid, "0")?;
                            let v = rt.get_property(iid, "1")?;
                            rt.call_function(
                                adder.clone(),
                                JsValue::Object(map_obj),
                                &[k, v],
                            )?;
                            Ok(())
                        })();
                        if let Err(e) = res {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    }
                }
            }
            Ok(JsValue::Object(map_obj))
        }),
    );

    set_method(
        rt,
        ctor,
        "groupBy",
        2,
        Rc::new(|rt, _, args| {
            let items = rt.collect_iterable(args.first().unwrap_or(&JsValue::Undefined))?;
            let cb = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Map.groupBy callback is not a function"));
            }
            let map_obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.map_proto),
                ObjectKind::Map(MapData::default()),
            ));
            for (i, item) in items.into_iter().enumerate() {
                let k = rt.call_function(
                    cb.clone(),
                    JsValue::Undefined,
                    &[item.clone(), JsValue::Number(i as f64)],
                )?;
                let existing = match &rt.heap.get_obj(map_obj).kind {
                    ObjectKind::Map(m) => m.get(&k).cloned(),
                    _ => None,
                };
                if let Some(JsValue::Object(arr_id)) = existing {
                    if let ObjectKind::Array(ref mut arr) = rt.heap.get_obj_mut(arr_id).kind {
                        let idx = arr.length;
                        arr.set(idx, item);
                    }
                } else {
                    let arr = rt.create_array(vec![item]);
                    if let ObjectKind::Map(ref mut m) = rt.heap.get_obj_mut(map_obj).kind {
                        m.set(k, JsValue::Object(arr));
                    }
                }
            }
            Ok(JsValue::Object(map_obj))
        }),
    );

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

    let _ = set_symbol_method;
    set_global(rt, "Map", JsValue::Object(ctor));
}

fn init_set(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.set_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("Set".into()), false, false, true),
    );

    set_getter(
        rt,
        proto,
        "size",
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Set.prototype.size called on non-object"));
            };
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::Set(s) => Ok(JsValue::Number(s.count as f64)),
                _ => Err(JsError::type_err("Set.prototype.size called on non-Set")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "has",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Set.prototype.has called on non-object"));
            };
            let v = args.first().unwrap_or(&JsValue::Undefined);
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::Set(s) => Ok(JsValue::Boolean(s.has(v))),
                _ => Err(JsError::type_err("Set.prototype.has called on non-Set")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "add",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Set.prototype.add called on non-object"));
            };
            let v = args.first().cloned().unwrap_or(JsValue::Undefined);
            match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::Set(s) => {
                    s.add(v);
                    Ok(JsValue::Object(oid))
                }
                _ => Err(JsError::type_err("Set.prototype.add called on non-Set")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "delete",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Set.prototype.delete called on non-object"));
            };
            let v = args.first().unwrap_or(&JsValue::Undefined);
            match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::Set(s) => Ok(JsValue::Boolean(s.delete(v))),
                _ => Err(JsError::type_err("Set.prototype.delete called on non-Set")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "clear",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Set.prototype.clear called on non-object"));
            };
            match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::Set(s) => {
                    s.clear();
                    Ok(JsValue::Undefined)
                }
                _ => Err(JsError::type_err("Set.prototype.clear called on non-Set")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "forEach",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Set.prototype.forEach called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::Set(_)) {
                return Err(JsError::type_err("Set.prototype.forEach called on non-Set"));
            }
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let mut i = 0;
            loop {
                let next = match &rt.heap.get_obj(oid).kind {
                    ObjectKind::Set(s) => {
                        let mut found = None;
                        while i < s.entries.len() {
                            if let Some(v) = &s.entries[i] {
                                found = Some(v.clone());
                                i += 1;
                                break;
                            }
                            i += 1;
                        }
                        found
                    }
                    _ => None,
                };
                let Some(v) = next else { break };
                rt.call_function(
                    cb.clone(),
                    this_arg.clone(),
                    &[v.clone(), v, JsValue::Object(oid)],
                )?;
            }
            Ok(JsValue::Undefined)
        }),
    );

    let values_fn = rt.create_native_function(
        "values",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Set.prototype.values called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::Set(_)) {
                return Err(JsError::type_err("Set.prototype.values called on non-Set"));
            }
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Set {
                    target: oid,
                    mode: IterMode::Values,
                },
            )))
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("values"),
        PropEntry::data(JsValue::Object(values_fn), true, false, true),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("keys"),
        PropEntry::data(JsValue::Object(values_fn), true, false, true),
    );
    let sym_iter = rt.symbols.iterator;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_iter),
        PropEntry::data(JsValue::Object(values_fn), true, false, true),
    );
    set_method(
        rt,
        proto,
        "entries",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Set.prototype.entries called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::Set(_)) {
                return Err(JsError::type_err("Set.prototype.entries called on non-Set"));
            }
            Ok(JsValue::Object(rt.create_builtin_iterator(
                BuiltinIteratorKind::Set {
                    target: oid,
                    mode: IterMode::Entries,
                },
            )))
        }),
    );

    set_method(
        rt,
        proto,
        "union",
        1,
        Rc::new(|rt, this, args| {
            let base = clone_set_entries(rt, &this)?;
            let (other_id, _, _, keys_fn) = get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            let iter = rt.call_function(keys_fn, JsValue::Object(other_id), &[])?;
            let mut result = SetData::default();
            for v in base {
                result.add(v);
            }
            while let Some(v) = rt.iterator_step(&iter)? {
                result.add(v);
            }
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.set_proto),
                ObjectKind::Set(result),
            ));
            Ok(JsValue::Object(obj))
        }),
    );
    set_method(
        rt,
        proto,
        "intersection",
        1,
        Rc::new(|rt, this, args| {
            let base = clone_set_entries(rt, &this)?;
            let (other_id, other_size, has_fn, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            let mut result = SetData::default();
            if base.len() as f64 <= other_size {
                for v in base {
                    if rt.call_function(has_fn.clone(), JsValue::Object(other_id), &[v.clone()])?.to_boolean() {
                        result.add(v);
                    }
                }
            } else {
                let iter = rt.call_function(keys_fn, JsValue::Object(other_id), &[])?;
                while let Some(v) = rt.iterator_step(&iter)? {
                    if base.iter().any(|b| b.same_value_zero(&v)) {
                        result.add(v);
                    }
                }
            }
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.set_proto),
                ObjectKind::Set(result),
            ));
            Ok(JsValue::Object(obj))
        }),
    );
    set_method(
        rt,
        proto,
        "difference",
        1,
        Rc::new(|rt, this, args| {
            let base = clone_set_entries(rt, &this)?;
            let (other_id, other_size, has_fn, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            let mut result = SetData::default();
            for v in &base {
                result.add(v.clone());
            }
            if base.len() as f64 <= other_size {
                for v in base {
                    if rt.call_function(has_fn.clone(), JsValue::Object(other_id), &[v.clone()])?.to_boolean() {
                        result.delete(&v);
                    }
                }
            } else {
                let iter = rt.call_function(keys_fn, JsValue::Object(other_id), &[])?;
                while let Some(v) = rt.iterator_step(&iter)? {
                    result.delete(&v);
                }
            }
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.set_proto),
                ObjectKind::Set(result),
            ));
            Ok(JsValue::Object(obj))
        }),
    );
    set_method(
        rt,
        proto,
        "symmetricDifference",
        1,
        Rc::new(|rt, this, args| {
            let base = clone_set_entries(rt, &this)?;
            let (other_id, _, _, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            let iter = rt.call_function(keys_fn, JsValue::Object(other_id), &[])?;
            let mut result = SetData::default();
            for v in &base {
                result.add(v.clone());
            }
            let mut seen_other = SetData::default();
            while let Some(v) = rt.iterator_step(&iter)? {
                if !seen_other.has(&v) {
                    seen_other.add(v.clone());
                    if result.has(&v) {
                        result.delete(&v);
                    } else {
                        result.add(v);
                    }
                }
            }
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.set_proto),
                ObjectKind::Set(result),
            ));
            Ok(JsValue::Object(obj))
        }),
    );
    set_method(
        rt,
        proto,
        "isSubsetOf",
        1,
        Rc::new(|rt, this, args| {
            let base = clone_set_entries(rt, &this)?;
            let (other_id, other_size, has_fn, _) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            if base.len() as f64 > other_size {
                return Ok(JsValue::Boolean(false));
            }
            for v in base {
                if !rt.call_function(has_fn.clone(), JsValue::Object(other_id), &[v])?.to_boolean() {
                    return Ok(JsValue::Boolean(false));
                }
            }
            Ok(JsValue::Boolean(true))
        }),
    );
    set_method(
        rt,
        proto,
        "isSupersetOf",
        1,
        Rc::new(|rt, this, args| {
            let base = clone_set_entries(rt, &this)?;
            let (other_id, other_size, _, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            if (base.len() as f64) < other_size {
                return Ok(JsValue::Boolean(false));
            }
            let iter = rt.call_function(keys_fn, JsValue::Object(other_id), &[])?;
            while let Some(v) = rt.iterator_step(&iter)? {
                if !base.iter().any(|b| b.same_value_zero(&v)) {
                    rt.iterator_close(&iter);
                    return Ok(JsValue::Boolean(false));
                }
            }
            Ok(JsValue::Boolean(true))
        }),
    );
    set_method(
        rt,
        proto,
        "isDisjointFrom",
        1,
        Rc::new(|rt, this, args| {
            let base = clone_set_entries(rt, &this)?;
            let (other_id, other_size, has_fn, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            if base.len() as f64 <= other_size {
                for v in base {
                    if rt.call_function(has_fn.clone(), JsValue::Object(other_id), &[v])?.to_boolean() {
                        return Ok(JsValue::Boolean(false));
                    }
                }
            } else {
                let iter = rt.call_function(keys_fn, JsValue::Object(other_id), &[])?;
                while let Some(v) = rt.iterator_step(&iter)? {
                    if base.iter().any(|b| b.same_value_zero(&v)) {
                        rt.iterator_close(&iter);
                        return Ok(JsValue::Boolean(false));
                    }
                }
            }
            Ok(JsValue::Boolean(true))
        }),
    );

    let ctor = rt.create_native_constructor(
        "Set",
        0,
        proto,
        Rc::new(|rt, this, args| {
            if !this.is_object() {
                return Err(JsError::type_err("Constructor Set requires 'new'"));
            }
            let set_obj = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.set_proto),
                ObjectKind::Set(SetData::default()),
            ));
            if let Some(iterable) = args.first() {
                if !iterable.is_nullish() {
                    let adder = rt.get_property(set_obj, "add")?;
                    if !rt.is_callable(&adder) {
                        return Err(JsError::type_err("Set.prototype.add is not a function"));
                    }
                    let iter = rt.get_iterator(iterable, false)?;
                    while let Some(item) = rt.iterator_step(&iter)? {
                        if let Err(e) =
                            rt.call_function(adder.clone(), JsValue::Object(set_obj), &[item])
                        {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    }
                }
            }
            Ok(JsValue::Object(set_obj))
        }),
    );

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

    set_global(rt, "Set", JsValue::Object(ctor));
}

fn clone_set_entries(rt: &JsRuntime, this: &JsValue) -> Result<Vec<JsValue>, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("Set method called on non-object"));
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::Set(s) => Ok(s.entries.iter().flatten().cloned().collect()),
        _ => Err(JsError::type_err("Set method called on non-Set")),
    }
}

fn get_set_record(
    rt: &mut JsRuntime,
    arg: &JsValue,
) -> Result<(ObjectId, f64, JsValue, JsValue), JsError> {
    let JsValue::Object(oid) = arg else {
        return Err(JsError::type_err("Set-like argument must be an object"));
    };
    let raw_size = rt.get_property(*oid, "size")?;
    let num_size = rt.to_number(&raw_size)?;
    if num_size.is_nan() {
        return Err(JsError::type_err("Set-like size must not be NaN"));
    }
    let int_size = to_integer_or_infinity(num_size);
    if int_size < 0.0 {
        return Err(JsError::range("Set-like size must be non-negative"));
    }
    let has_fn = rt.get_property(*oid, "has")?;
    if !rt.is_callable(&has_fn) {
        return Err(JsError::type_err("Set-like 'has' is not callable"));
    }
    let keys_fn = rt.get_property(*oid, "keys")?;
    if !rt.is_callable(&keys_fn) {
        return Err(JsError::type_err("Set-like 'keys' is not callable"));
    }
    Ok((*oid, int_size, has_fn, keys_fn))
}

fn init_weak_map(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.weak_map_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("WeakMap".into()), false, false, true),
    );

    set_method(
        rt,
        proto,
        "get",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakMap.prototype.get called on non-object"));
            };
            let ObjectKind::WeakMap(ref entries) = rt.heap.get_obj(oid).kind else {
                return Err(JsError::type_err("WeakMap.prototype.get called on non-WeakMap"));
            };
            let Some(JsValue::Object(k)) = args.first() else {
                return Ok(JsValue::Undefined);
            };
            Ok(entries
                .iter()
                .find(|(ek, _)| ek == k)
                .map(|(_, v)| v.clone())
                .unwrap_or(JsValue::Undefined))
        }),
    );

    set_method(
        rt,
        proto,
        "has",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakMap.prototype.has called on non-object"));
            };
            let ObjectKind::WeakMap(ref entries) = rt.heap.get_obj(oid).kind else {
                return Err(JsError::type_err("WeakMap.prototype.has called on non-WeakMap"));
            };
            let Some(JsValue::Object(k)) = args.first() else {
                return Ok(JsValue::Boolean(false));
            };
            Ok(JsValue::Boolean(entries.iter().any(|(ek, _)| ek == k)))
        }),
    );

    set_method(
        rt,
        proto,
        "set",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakMap.prototype.set called on non-object"));
            };
            let Some(JsValue::Object(k)) = args.first() else {
                return Err(JsError::type_err("Invalid value used as weak map key"));
            };
            let val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let ObjectKind::WeakMap(ref mut entries) = rt.heap.get_obj_mut(oid).kind else {
                return Err(JsError::type_err("WeakMap.prototype.set called on non-WeakMap"));
            };
            if let Some(pair) = entries.iter_mut().find(|(ek, _)| ek == k) {
                pair.1 = val;
            } else {
                entries.push((*k, val));
            }
            Ok(JsValue::Object(oid))
        }),
    );

    set_method(
        rt,
        proto,
        "delete",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakMap.prototype.delete called on non-object"));
            };
            let Some(JsValue::Object(k)) = args.first() else {
                return Ok(JsValue::Boolean(false));
            };
            let ObjectKind::WeakMap(ref mut entries) = rt.heap.get_obj_mut(oid).kind else {
                return Err(JsError::type_err("WeakMap.prototype.delete called on non-WeakMap"));
            };
            if let Some(pos) = entries.iter().position(|(ek, _)| ek == k) {
                entries.remove(pos);
                Ok(JsValue::Boolean(true))
            } else {
                Ok(JsValue::Boolean(false))
            }
        }),
    );

    let ctor = rt.create_native_constructor(
        "WeakMap",
        0,
        proto,
        Rc::new(|rt, _, args| {
            let wm = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.weak_map_proto),
                ObjectKind::WeakMap(Vec::new()),
            ));
            if let Some(iterable) = args.first() {
                if !iterable.is_nullish() {
                    let items = rt.collect_iterable(iterable)?;
                    for item in items {
                        let JsValue::Object(iid) = item else {
                            return Err(JsError::type_err("Iterator value is not an entry object"));
                        };
                        let k = rt.get_property(iid, "0")?;
                        let v = rt.get_property(iid, "1")?;
                        let JsValue::Object(kid) = k else {
                            return Err(JsError::type_err("Invalid value used as weak map key"));
                        };
                        if let ObjectKind::WeakMap(ref mut entries) =
                            rt.heap.get_obj_mut(wm).kind
                        {
                            entries.push((kid, v));
                        }
                    }
                }
            }
            Ok(JsValue::Object(wm))
        }),
    );
    set_global(rt, "WeakMap", JsValue::Object(ctor));
}

fn init_weak_set(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.weak_set_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("WeakSet".into()), false, false, true),
    );

    set_method(
        rt,
        proto,
        "has",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakSet.prototype.has called on non-object"));
            };
            let ObjectKind::WeakSet(ref entries) = rt.heap.get_obj(oid).kind else {
                return Err(JsError::type_err("WeakSet.prototype.has called on non-WeakSet"));
            };
            let Some(JsValue::Object(k)) = args.first() else {
                return Ok(JsValue::Boolean(false));
            };
            Ok(JsValue::Boolean(entries.contains(k)))
        }),
    );

    set_method(
        rt,
        proto,
        "add",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakSet.prototype.add called on non-object"));
            };
            let Some(JsValue::Object(k)) = args.first() else {
                return Err(JsError::type_err("Invalid value used in weak set"));
            };
            let ObjectKind::WeakSet(ref mut entries) = rt.heap.get_obj_mut(oid).kind else {
                return Err(JsError::type_err("WeakSet.prototype.add called on non-WeakSet"));
            };
            if !entries.contains(k) {
                entries.push(*k);
            }
            Ok(JsValue::Object(oid))
        }),
    );

    set_method(
        rt,
        proto,
        "delete",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakSet.prototype.delete called on non-object"));
            };
            let Some(JsValue::Object(k)) = args.first() else {
                return Ok(JsValue::Boolean(false));
            };
            let ObjectKind::WeakSet(ref mut entries) = rt.heap.get_obj_mut(oid).kind else {
                return Err(JsError::type_err("WeakSet.prototype.delete called on non-WeakSet"));
            };
            if let Some(pos) = entries.iter().position(|ek| ek == k) {
                entries.remove(pos);
                Ok(JsValue::Boolean(true))
            } else {
                Ok(JsValue::Boolean(false))
            }
        }),
    );

    let ctor = rt.create_native_constructor(
        "WeakSet",
        0,
        proto,
        Rc::new(|rt, _, args| {
            let ws = rt.heap.alloc_object(JsObject::new(
                Some(rt.intrinsics.weak_set_proto),
                ObjectKind::WeakSet(Vec::new()),
            ));
            if let Some(iterable) = args.first() {
                if !iterable.is_nullish() {
                    let items = rt.collect_iterable(iterable)?;
                    for item in items {
                        let JsValue::Object(kid) = item else {
                            return Err(JsError::type_err("Invalid value used in weak set"));
                        };
                        if let ObjectKind::WeakSet(ref mut entries) =
                            rt.heap.get_obj_mut(ws).kind
                        {
                            if !entries.contains(&kid) {
                                entries.push(kid);
                            }
                        }
                    }
                }
            }
            Ok(JsValue::Object(ws))
        }),
    );
    set_global(rt, "WeakSet", JsValue::Object(ctor));
}
