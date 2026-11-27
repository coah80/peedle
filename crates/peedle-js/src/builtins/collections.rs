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
    init_weak_ref(rt);
    init_finalization_registry(rt);
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
        "getOrInsert",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.getOrInsert called on non-object"));
            };
            let key = args.first().cloned().unwrap_or(JsValue::Undefined);
            let default_val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            match &mut rt.heap.get_obj_mut(oid).kind {
                ObjectKind::Map(m) => {
                    if let Some(existing) = m.get(&key) {
                        return Ok(existing.clone());
                    }
                    m.set(key, default_val.clone());
                    Ok(default_val)
                }
                _ => Err(JsError::type_err("Map.prototype.getOrInsert called on non-Map")),
            }
        }),
    );

    set_method(
        rt,
        proto,
        "getOrInsertComputed",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Map.prototype.getOrInsertComputed called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::Map(_)) {
                return Err(JsError::type_err("Map.prototype.getOrInsertComputed called on non-Map"));
            }
            let cb = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            let raw_key = args.first().cloned().unwrap_or(JsValue::Undefined);
            let key = match raw_key {
                JsValue::Number(n) if n == 0.0 => JsValue::Number(0.0),
                other => other,
            };
            if let ObjectKind::Map(ref m) = rt.heap.get_obj(oid).kind {
                if let Some(existing) = m.get(&key) {
                    return Ok(existing.clone());
                }
            }
            let val = rt.call_function(cb, JsValue::Undefined, &[key.clone()])?;
            if let ObjectKind::Map(ref mut m) = rt.heap.get_obj_mut(oid).kind {
                m.set(key, val.clone());
            }
            Ok(val)
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

    let ctor = rt.create_spec_constructor(
        "Map",
        0,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("Map")?;
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.map_proto)?;
            let map_obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
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
            let this_id = require_set(rt, &this)?;
            let (other_id, _, _, keys_fn) = get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            let (iter, next_fn) = get_keys_iterator(rt, other_id, keys_fn)?;
            let mut result = SetData::default();
            for v in clone_set_entries_by_id(rt, this_id) {
                result.add(v);
            }
            while let Some(v) = step_iter_with_next(rt, &iter, &next_fn)? {
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
            let this_id = require_set(rt, &this)?;
            let (other_id, other_size, has_fn, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            let mut result = SetData::default();
            let this_size = set_count(rt, this_id);
            if (this_size as f64) <= other_size {
                let mut i = 0;
                loop {
                    let elem = next_live_set_entry(rt, this_id, &mut i);
                    let Some(v) = elem else { break };
                    let in_other = rt
                        .call_function(has_fn.clone(), JsValue::Object(other_id), &[v.clone()])?
                        .to_boolean();
                    if in_other && set_has(rt, this_id, &v) {
                        result.add(v);
                    }
                }
            } else {
                let (iter, next_fn) = get_keys_iterator(rt, other_id, keys_fn)?;
                while let Some(v) = step_iter_with_next(rt, &iter, &next_fn)? {
                    if set_has(rt, this_id, &v) {
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
            let this_id = require_set(rt, &this)?;
            let (other_id, other_size, has_fn, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            let mut result = SetData::default();
            let snapshot = clone_set_entries_by_id(rt, this_id);
            for v in &snapshot {
                result.add(v.clone());
            }
            if (snapshot.len() as f64) <= other_size {
                for v in snapshot {
                    if rt
                        .call_function(has_fn.clone(), JsValue::Object(other_id), &[v.clone()])?
                        .to_boolean()
                    {
                        result.delete(&v);
                    }
                }
            } else {
                let (iter, next_fn) = get_keys_iterator(rt, other_id, keys_fn)?;
                while let Some(v) = step_iter_with_next(rt, &iter, &next_fn)? {
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
            let this_id = require_set(rt, &this)?;
            let (other_id, _, _, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            let (iter, next_fn) = get_keys_iterator(rt, other_id, keys_fn)?;
            let mut result = SetData::default();
            for v in clone_set_entries_by_id(rt, this_id) {
                result.add(v);
            }
            let mut seen_other = SetData::default();
            while let Some(v) = step_iter_with_next(rt, &iter, &next_fn)? {
                let in_result = result.has(&v);
                if set_has(rt, this_id, &v) {
                    if in_result {
                        result.delete(&v);
                    }
                } else if !in_result && !seen_other.has(&v) {
                    result.add(v.clone());
                }
                seen_other.add(v);
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
            let this_id = require_set(rt, &this)?;
            let (other_id, other_size, has_fn, _) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            if (set_count(rt, this_id) as f64) > other_size {
                return Ok(JsValue::Boolean(false));
            }
            let mut i = 0;
            loop {
                let Some(v) = next_live_set_entry(rt, this_id, &mut i) else { break };
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
            let this_id = require_set(rt, &this)?;
            let (other_id, other_size, _, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            if (set_count(rt, this_id) as f64) < other_size {
                return Ok(JsValue::Boolean(false));
            }
            let (iter, next_fn) = get_keys_iterator(rt, other_id, keys_fn)?;
            while let Some(v) = step_iter_with_next(rt, &iter, &next_fn)? {
                if !set_has(rt, this_id, &v) {
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
            let this_id = require_set(rt, &this)?;
            let (other_id, other_size, has_fn, keys_fn) =
                get_set_record(rt, args.first().unwrap_or(&JsValue::Undefined))?;
            if (set_count(rt, this_id) as f64) <= other_size {
                let mut i = 0;
                loop {
                    let Some(v) = next_live_set_entry(rt, this_id, &mut i) else { break };
                    if rt.call_function(has_fn.clone(), JsValue::Object(other_id), &[v])?.to_boolean() {
                        return Ok(JsValue::Boolean(false));
                    }
                }
            } else {
                let (iter, next_fn) = get_keys_iterator(rt, other_id, keys_fn)?;
                while let Some(v) = step_iter_with_next(rt, &iter, &next_fn)? {
                    if set_has(rt, this_id, &v) {
                        rt.iterator_close(&iter);
                        return Ok(JsValue::Boolean(false));
                    }
                }
            }
            Ok(JsValue::Boolean(true))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "Set",
        0,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("Set")?;
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.set_proto)?;
            let set_obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
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

fn require_set(rt: &JsRuntime, this: &JsValue) -> Result<ObjectId, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err("Set method called on non-object"));
    };
    if matches!(rt.heap.get_obj(*oid).kind, ObjectKind::Set(_)) {
        Ok(*oid)
    } else {
        Err(JsError::type_err("Set method called on non-Set"))
    }
}

fn clone_set_entries_by_id(rt: &JsRuntime, oid: ObjectId) -> Vec<JsValue> {
    match &rt.heap.get_obj(oid).kind {
        ObjectKind::Set(s) => s.entries.iter().flatten().cloned().collect(),
        _ => Vec::new(),
    }
}

fn set_count(rt: &JsRuntime, oid: ObjectId) -> usize {
    match &rt.heap.get_obj(oid).kind {
        ObjectKind::Set(s) => s.count,
        _ => 0,
    }
}

fn set_has(rt: &JsRuntime, oid: ObjectId, val: &JsValue) -> bool {
    match &rt.heap.get_obj(oid).kind {
        ObjectKind::Set(s) => s.has(val),
        _ => false,
    }
}

fn next_live_set_entry(rt: &JsRuntime, oid: ObjectId, idx: &mut usize) -> Option<JsValue> {
    let ObjectKind::Set(ref s) = rt.heap.get_obj(oid).kind else {
        return None;
    };
    while *idx < s.entries.len() {
        let cur = *idx;
        *idx += 1;
        if let Some(v) = &s.entries[cur] {
            return Some(v.clone());
        }
    }
    None
}

fn get_keys_iterator(
    rt: &mut JsRuntime,
    other_id: ObjectId,
    keys_fn: JsValue,
) -> Result<(JsValue, JsValue), JsError> {
    let iter = rt.call_function(keys_fn, JsValue::Object(other_id), &[])?;
    let JsValue::Object(iter_id) = iter else {
        return Err(JsError::type_err("Set-like keys() did not return an object"));
    };
    let next_fn = rt.get_property(iter_id, "next")?;
    if !rt.is_callable(&next_fn) {
        return Err(JsError::type_err("Iterator next is not callable"));
    }
    Ok((iter, next_fn))
}

fn step_iter_with_next(
    rt: &mut JsRuntime,
    iter: &JsValue,
    next_fn: &JsValue,
) -> Result<Option<JsValue>, JsError> {
    let res = rt.call_function(next_fn.clone(), iter.clone(), &[])?;
    let JsValue::Object(res_id) = res else {
        return Err(JsError::type_err("Iterator result is not an object"));
    };
    let done = rt.get_property(res_id, "done")?.to_boolean();
    if done {
        return Ok(None);
    }
    let val = rt.get_property(res_id, "value")?;
    Ok(Some(val))
}

enum WeakKey {
    Obj(ObjectId),
    Sym(u64),
}

fn can_be_held_weakly(rt: &JsRuntime, val: &JsValue) -> Option<WeakKey> {
    match val {
        JsValue::Object(oid) => Some(WeakKey::Obj(*oid)),
        JsValue::Symbol(sid) if rt.heap.symbol_key_for(*sid).is_none() => Some(WeakKey::Sym(*sid)),
        _ => None,
    }
}

fn weak_map_get(rt: &JsRuntime, oid: ObjectId, key: &WeakKey) -> Option<JsValue> {
    match key {
        WeakKey::Obj(k) => {
            if let ObjectKind::WeakMap(ref entries) = rt.heap.get_obj(oid).kind {
                entries.iter().find(|(ek, _)| ek == k).map(|(_, v)| v.clone())
            } else {
                None
            }
        }
        WeakKey::Sym(sid) => rt
            .heap
            .weak_map_symbols
            .get(&oid)
            .and_then(|v| v.iter().find(|(sk, _)| sk == sid).map(|(_, val)| val.clone())),
    }
}

fn weak_map_set(rt: &mut JsRuntime, oid: ObjectId, key: WeakKey, val: JsValue) {
    match key {
        WeakKey::Obj(k) => {
            if let ObjectKind::WeakMap(ref mut entries) = rt.heap.get_obj_mut(oid).kind {
                if let Some(pair) = entries.iter_mut().find(|(ek, _)| *ek == k) {
                    pair.1 = val;
                } else {
                    entries.push((k, val));
                }
            }
        }
        WeakKey::Sym(sid) => {
            let entries = rt.heap.weak_map_symbols.entry(oid).or_default();
            if let Some(pair) = entries.iter_mut().find(|(sk, _)| *sk == sid) {
                pair.1 = val;
            } else {
                entries.push((sid, val));
            }
        }
    }
}

fn weak_map_delete(rt: &mut JsRuntime, oid: ObjectId, key: &WeakKey) -> bool {
    match key {
        WeakKey::Obj(k) => {
            if let ObjectKind::WeakMap(ref mut entries) = rt.heap.get_obj_mut(oid).kind {
                if let Some(pos) = entries.iter().position(|(ek, _)| ek == k) {
                    entries.remove(pos);
                    return true;
                }
            }
            false
        }
        WeakKey::Sym(sid) => {
            if let Some(entries) = rt.heap.weak_map_symbols.get_mut(&oid) {
                if let Some(pos) = entries.iter().position(|(sk, _)| sk == sid) {
                    entries.remove(pos);
                    return true;
                }
            }
            false
        }
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
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakMap(_)) {
                return Err(JsError::type_err("WeakMap.prototype.get called on non-WeakMap"));
            }
            let Some(k) = args.first().and_then(|v| can_be_held_weakly(rt, v)) else {
                return Ok(JsValue::Undefined);
            };
            Ok(weak_map_get(rt, oid, &k).unwrap_or(JsValue::Undefined))
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
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakMap(_)) {
                return Err(JsError::type_err("WeakMap.prototype.has called on non-WeakMap"));
            }
            let Some(k) = args.first().and_then(|v| can_be_held_weakly(rt, v)) else {
                return Ok(JsValue::Boolean(false));
            };
            Ok(JsValue::Boolean(weak_map_get(rt, oid, &k).is_some()))
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
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakMap(_)) {
                return Err(JsError::type_err("WeakMap.prototype.set called on non-WeakMap"));
            }
            let Some(k) = args.first().and_then(|v| can_be_held_weakly(rt, v)) else {
                return Err(JsError::type_err("Invalid value used as weak map key"));
            };
            let val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            weak_map_set(rt, oid, k, val);
            Ok(JsValue::Object(oid))
        }),
    );

    set_method(
        rt,
        proto,
        "getOrInsert",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakMap.prototype.getOrInsert called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakMap(_)) {
                return Err(JsError::type_err("WeakMap.prototype.getOrInsert called on non-WeakMap"));
            }
            let Some(k) = args.first().and_then(|v| can_be_held_weakly(rt, v)) else {
                return Err(JsError::type_err("Invalid value used as weak map key"));
            };
            let default_val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if let Some(existing) = weak_map_get(rt, oid, &k) {
                return Ok(existing);
            }
            weak_map_set(rt, oid, k, default_val.clone());
            Ok(default_val)
        }),
    );

    set_method(
        rt,
        proto,
        "getOrInsertComputed",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakMap.prototype.getOrInsertComputed called on non-object"));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakMap(_)) {
                return Err(JsError::type_err("WeakMap.prototype.getOrInsertComputed called on non-WeakMap"));
            }
            let key_arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            let Some(k) = can_be_held_weakly(rt, &key_arg) else {
                return Err(JsError::type_err("Invalid value used as weak map key"));
            };
            let cb = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cb) {
                return Err(JsError::type_err("Callback is not a function"));
            }
            if let Some(existing) = weak_map_get(rt, oid, &k) {
                return Ok(existing);
            }
            let val = rt.call_function(cb, JsValue::Undefined, &[key_arg])?;
            weak_map_set(rt, oid, k, val.clone());
            Ok(val)
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
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakMap(_)) {
                return Err(JsError::type_err("WeakMap.prototype.delete called on non-WeakMap"));
            }
            let Some(k) = args.first().and_then(|v| can_be_held_weakly(rt, v)) else {
                return Ok(JsValue::Boolean(false));
            };
            Ok(JsValue::Boolean(weak_map_delete(rt, oid, &k)))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "WeakMap",
        0,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("WeakMap")?;
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.weak_map_proto)?;
            let wm = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::WeakMap(Vec::new()),
            ));
            if let Some(iterable) = args.first() {
                if !iterable.is_nullish() {
                    let adder = rt.get_property(wm, "set")?;
                    if !rt.is_callable(&adder) {
                        return Err(JsError::type_err("WeakMap.prototype.set is not a function"));
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
                            rt.call_function(adder.clone(), JsValue::Object(wm), &[k, v])?;
                            Ok(())
                        })();
                        if let Err(e) = res {
                            rt.iterator_close(&iter);
                            return Err(e);
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
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakSet(_)) {
                return Err(JsError::type_err("WeakSet.prototype.has called on non-WeakSet"));
            }
            let Some(k) = args.first().and_then(|v| can_be_held_weakly(rt, v)) else {
                return Ok(JsValue::Boolean(false));
            };
            let found = match k {
                WeakKey::Obj(kid) => match &rt.heap.get_obj(oid).kind {
                    ObjectKind::WeakSet(entries) => entries.contains(&kid),
                    _ => false,
                },
                WeakKey::Sym(sid) => rt
                    .heap
                    .weak_set_symbols
                    .get(&oid)
                    .is_some_and(|v| v.contains(&sid)),
            };
            Ok(JsValue::Boolean(found))
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
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakSet(_)) {
                return Err(JsError::type_err("WeakSet.prototype.add called on non-WeakSet"));
            }
            let Some(k) = args.first().and_then(|v| can_be_held_weakly(rt, v)) else {
                return Err(JsError::type_err("Invalid value used in weak set"));
            };
            match k {
                WeakKey::Obj(kid) => {
                    if let ObjectKind::WeakSet(ref mut entries) = rt.heap.get_obj_mut(oid).kind {
                        if !entries.contains(&kid) {
                            entries.push(kid);
                        }
                    }
                }
                WeakKey::Sym(sid) => {
                    let entries = rt.heap.weak_set_symbols.entry(oid).or_default();
                    if !entries.contains(&sid) {
                        entries.push(sid);
                    }
                }
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
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::WeakSet(_)) {
                return Err(JsError::type_err("WeakSet.prototype.delete called on non-WeakSet"));
            }
            let Some(k) = args.first().and_then(|v| can_be_held_weakly(rt, v)) else {
                return Ok(JsValue::Boolean(false));
            };
            let removed = match k {
                WeakKey::Obj(kid) => {
                    if let ObjectKind::WeakSet(ref mut entries) = rt.heap.get_obj_mut(oid).kind {
                        if let Some(pos) = entries.iter().position(|ek| *ek == kid) {
                            entries.remove(pos);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
                WeakKey::Sym(sid) => {
                    if let Some(entries) = rt.heap.weak_set_symbols.get_mut(&oid) {
                        if let Some(pos) = entries.iter().position(|sk| *sk == sid) {
                            entries.remove(pos);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
            };
            Ok(JsValue::Boolean(removed))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "WeakSet",
        0,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("WeakSet")?;
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.weak_set_proto)?;
            let ws = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::WeakSet(Vec::new()),
            ));
            if let Some(iterable) = args.first() {
                if !iterable.is_nullish() {
                    let adder = rt.get_property(ws, "add")?;
                    if !rt.is_callable(&adder) {
                        return Err(JsError::type_err("WeakSet.prototype.add is not a function"));
                    }
                    let iter = rt.get_iterator(iterable, false)?;
                    while let Some(item) = rt.iterator_step(&iter)? {
                        if let Err(e) =
                            rt.call_function(adder.clone(), JsValue::Object(ws), &[item])
                        {
                            rt.iterator_close(&iter);
                            return Err(e);
                        }
                    }
                }
            }
            Ok(JsValue::Object(ws))
        }),
    );
    set_global(rt, "WeakSet", JsValue::Object(ctor));
}

fn init_weak_ref(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.weak_ref_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("WeakRef".into()), false, false, true),
    );

    set_method(
        rt,
        proto,
        "deref",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("WeakRef.prototype.deref called on non-object"));
            };
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::WeakRef(target) => Ok(target.clone().unwrap_or(JsValue::Undefined)),
                _ => Err(JsError::type_err("WeakRef.prototype.deref called on non-WeakRef")),
            }
        }),
    );

    let ctor = rt.create_spec_constructor(
        "WeakRef",
        1,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("WeakRef")?;
            let target = args.first().cloned().unwrap_or(JsValue::Undefined);
            if can_be_held_weakly(rt, &target).is_none() {
                return Err(JsError::type_err("WeakRef target must be an object or non-registered symbol"));
            }
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.weak_ref_proto)?;
            let wr = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::WeakRef(Some(target)),
            ));
            Ok(JsValue::Object(wr))
        }),
    );
    set_global(rt, "WeakRef", JsValue::Object(ctor));
}

fn init_finalization_registry(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.finalization_registry_proto;
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("FinalizationRegistry".into()), false, false, true),
    );

    set_method(
        rt,
        proto,
        "register",
        2,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err(
                    "FinalizationRegistry.prototype.register called on non-object",
                ));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::FinalizationRegistry(_)) {
                return Err(JsError::type_err(
                    "FinalizationRegistry.prototype.register called on non-FinalizationRegistry",
                ));
            }
            let target = args.first().cloned().unwrap_or(JsValue::Undefined);
            let held_value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let unregister_token = args.get(2).cloned().unwrap_or(JsValue::Undefined);
            if can_be_held_weakly(rt, &target).is_none() {
                return Err(JsError::type_err(
                    "FinalizationRegistry target must be an object or non-registered symbol",
                ));
            }
            if target.same_value(&held_value) {
                return Err(JsError::type_err(
                    "FinalizationRegistry target and heldValue must not be the same",
                ));
            }
            let token_opt = if unregister_token.is_undefined() {
                None
            } else {
                if can_be_held_weakly(rt, &unregister_token).is_none() {
                    return Err(JsError::type_err(
                        "FinalizationRegistry unregisterToken must be an object, non-registered symbol, or undefined",
                    ));
                }
                Some(unregister_token)
            };
            if let ObjectKind::FinalizationRegistry(ref mut fr) = rt.heap.get_obj_mut(oid).kind {
                fr.cells.push(FinalizationCell {
                    target,
                    held_value,
                    unregister_token: token_opt,
                });
            }
            Ok(JsValue::Undefined)
        }),
    );

    set_method(
        rt,
        proto,
        "unregister",
        1,
        Rc::new(|rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err(
                    "FinalizationRegistry.prototype.unregister called on non-object",
                ));
            };
            if !matches!(rt.heap.get_obj(oid).kind, ObjectKind::FinalizationRegistry(_)) {
                return Err(JsError::type_err(
                    "FinalizationRegistry.prototype.unregister called on non-FinalizationRegistry",
                ));
            }
            let unregister_token = args.first().cloned().unwrap_or(JsValue::Undefined);
            if can_be_held_weakly(rt, &unregister_token).is_none() {
                return Err(JsError::type_err(
                    "FinalizationRegistry unregisterToken must be an object or non-registered symbol",
                ));
            }
            let mut removed = false;
            if let ObjectKind::FinalizationRegistry(ref mut fr) = rt.heap.get_obj_mut(oid).kind {
                let before = fr.cells.len();
                fr.cells.retain(|c| {
                    !matches!(&c.unregister_token, Some(tok) if tok.same_value(&unregister_token))
                });
                removed = fr.cells.len() < before;
            }
            Ok(JsValue::Boolean(removed))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "FinalizationRegistry",
        1,
        proto,
        Rc::new(|rt, _, args| {
            rt.require_new_target("FinalizationRegistry")?;
            let cleanup = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&cleanup) {
                return Err(JsError::type_err(
                    "FinalizationRegistry cleanup callback must be callable",
                ));
            }
            let obj_proto =
                rt.get_prototype_from_new_target(rt.intrinsics.finalization_registry_proto)?;
            let fr = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::FinalizationRegistry(FinalizationRegistryData {
                    cleanup_callback: cleanup,
                    cells: Vec::new(),
                }),
            ));
            Ok(JsValue::Object(fr))
        }),
    );
    set_global(rt, "FinalizationRegistry", JsValue::Object(ctor));
}
