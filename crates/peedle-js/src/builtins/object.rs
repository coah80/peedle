use crate::builtins::{set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.object_proto;

    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, _| Ok(JsValue::String(object_to_string(rt, &this)?))),
    );
    set_method(
        rt,
        proto,
        "toLocaleString",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            let to_str = rt.get_property(oid, "toString")?;
            rt.call_function(to_str, this, &[])
        }),
    );
    set_method(
        rt,
        proto,
        "valueOf",
        0,
        Rc::new(|rt, this, _| Ok(JsValue::Object(rt.to_object(&this)?))),
    );
    set_method(
        rt,
        proto,
        "hasOwnProperty",
        1,
        Rc::new(|rt, this, args| {
            let key = rt.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
            let oid = rt.to_object(&this)?;
            Ok(JsValue::Boolean(rt.has_own_property(oid, &key)?))
        }),
    );
    set_method(
        rt,
        proto,
        "isPrototypeOf",
        1,
        Rc::new(|rt, this, args| {
            let Some(JsValue::Object(mut cur)) = args.first().copied_opt() else {
                return Ok(JsValue::Boolean(false));
            };
            let target = rt.to_object(&this)?;
            while let Some(p) = rt.get_prototype_of(cur)? {
                if p == target {
                    return Ok(JsValue::Boolean(true));
                }
                cur = p;
            }
            Ok(JsValue::Boolean(false))
        }),
    );
    set_method(
        rt,
        proto,
        "propertyIsEnumerable",
        1,
        Rc::new(|rt, this, args| {
            let key = rt.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
            let oid = rt.to_object(&this)?;
            let desc = rt.get_own_property_descriptor(oid, &key)?;
            Ok(JsValue::Boolean(
                desc.and_then(|d| d.enumerable).unwrap_or(false),
            ))
        }),
    );

    let proto_getter = rt.create_native_function(
        "get __proto__",
        0,
        Rc::new(|rt, this, _| {
            let oid = rt.to_object(&this)?;
            Ok(rt
                .get_prototype_of(oid)?
                .map(JsValue::Object)
                .unwrap_or(JsValue::Null))
        }),
    );
    let proto_setter = rt.create_native_function(
        "set __proto__",
        1,
        Rc::new(|rt, this, args| {
            if this.is_nullish() {
                return Err(JsError::type_err("Cannot convert undefined or null to object"));
            }
            let JsValue::Object(oid) = this else {
                return Ok(JsValue::Undefined);
            };
            match args.first() {
                Some(JsValue::Object(p)) => {
                    if !rt.set_prototype_of(oid, Some(*p))? {
                        return Err(JsError::type_err("Cannot set prototype of object"));
                    }
                }
                Some(JsValue::Null) => {
                    if !rt.set_prototype_of(oid, None)? {
                        return Err(JsError::type_err("Cannot set prototype of object"));
                    }
                }
                _ => {}
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.define_property(
        proto,
        PropertyKey::from("__proto__"),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(proto_getter)),
            set: Some(JsValue::Object(proto_setter)),
            enumerable: Some(false),
            configurable: Some(true),
        },
    );

    set_method(
        rt,
        proto,
        "__defineGetter__",
        2,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let getter = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&getter) {
                return Err(JsError::type_err("Getter must be a function"));
            }
            let key = rt.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
            let desc = PropertyDescriptor {
                value: None,
                writable: None,
                get: Some(getter),
                set: None,
                enumerable: Some(true),
                configurable: Some(true),
            };
            if !rt.define_property(oid, key, desc)? {
                return Err(JsError::type_err("Cannot define getter"));
            }
            Ok(JsValue::Undefined)
        }),
    );
    set_method(
        rt,
        proto,
        "__defineSetter__",
        2,
        Rc::new(|rt, this, args| {
            let oid = rt.to_object(&this)?;
            let setter = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&setter) {
                return Err(JsError::type_err("Setter must be a function"));
            }
            let key = rt.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
            let desc = PropertyDescriptor {
                value: None,
                writable: None,
                get: None,
                set: Some(setter),
                enumerable: Some(true),
                configurable: Some(true),
            };
            if !rt.define_property(oid, key, desc)? {
                return Err(JsError::type_err("Cannot define setter"));
            }
            Ok(JsValue::Undefined)
        }),
    );
    set_method(
        rt,
        proto,
        "__lookupGetter__",
        1,
        Rc::new(|rt, this, args| {
            let mut cur = rt.to_object(&this)?;
            let key = rt.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
            loop {
                if let Some(d) = rt.get_own_property_descriptor(cur, &key)? {
                    if d.is_accessor() {
                        return Ok(d.get.unwrap_or(JsValue::Undefined));
                    }
                    return Ok(JsValue::Undefined);
                }
                match rt.get_prototype_of(cur)? {
                    Some(p) => cur = p,
                    None => return Ok(JsValue::Undefined),
                }
            }
        }),
    );
    set_method(
        rt,
        proto,
        "__lookupSetter__",
        1,
        Rc::new(|rt, this, args| {
            let mut cur = rt.to_object(&this)?;
            let key = rt.to_property_key(args.first().unwrap_or(&JsValue::Undefined))?;
            loop {
                if let Some(d) = rt.get_own_property_descriptor(cur, &key)? {
                    if d.is_accessor() {
                        return Ok(d.set.unwrap_or(JsValue::Undefined));
                    }
                    return Ok(JsValue::Undefined);
                }
                match rt.get_prototype_of(cur)? {
                    Some(p) => cur = p,
                    None => return Ok(JsValue::Undefined),
                }
            }
        }),
    );

    let ctor = rt.create_spec_constructor(
        "Object",
        1,
        proto,
        Rc::new(|rt, _, args| {
            if let Some(JsValue::Object(nt_id)) = rt.native_new_target {
                if nt_id != rt.intrinsics.object_ctor {
                    let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.object_proto)?;
                    return Ok(JsValue::Object(rt.create_object_with_proto(Some(obj_proto))));
                }
            }
            match args.first() {
                None | Some(JsValue::Undefined) | Some(JsValue::Null) => {
                    Ok(JsValue::Object(rt.create_object()))
                }
                Some(v) => Ok(JsValue::Object(rt.to_object(v)?)),
            }
        }),
    );
    rt.intrinsics.object_ctor = ctor;

    set_method(
        rt,
        ctor,
        "create",
        2,
        Rc::new(|rt, _, args| {
            let proto_arg = args.first().unwrap_or(&JsValue::Undefined);
            let p = match proto_arg {
                JsValue::Null => None,
                JsValue::Object(id) => Some(*id),
                _ => {
                    return Err(JsError::type_err(
                        "Object prototype may only be an Object or null",
                    ));
                }
            };
            let obj = rt.create_object_with_proto(p);
            if let Some(props) = args.get(1) {
                if !props.is_undefined() {
                    define_properties_impl(rt, obj, props)?;
                }
            }
            Ok(JsValue::Object(obj))
        }),
    );
    set_method(
        rt,
        ctor,
        "defineProperty",
        3,
        Rc::new(|rt, _, args| {
            let Some(JsValue::Object(oid)) = args.first() else {
                return Err(JsError::type_err("Object.defineProperty called on non-object"));
            };
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            let desc = rt.to_property_descriptor(args.get(2).unwrap_or(&JsValue::Undefined))?;
            if !rt.define_property(*oid, key, desc)? {
                return Err(JsError::type_err("Cannot define property"));
            }
            Ok(JsValue::Object(*oid))
        }),
    );
    set_method(
        rt,
        ctor,
        "defineProperties",
        2,
        Rc::new(|rt, _, args| {
            let Some(JsValue::Object(oid)) = args.first() else {
                return Err(JsError::type_err("Object.defineProperties called on non-object"));
            };
            let props = args.get(1).unwrap_or(&JsValue::Undefined);
            define_properties_impl(rt, *oid, props)?;
            Ok(JsValue::Object(*oid))
        }),
    );
    set_method(
        rt,
        ctor,
        "getOwnPropertyDescriptor",
        2,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            match rt.get_own_property_descriptor(oid, &key)? {
                Some(d) => Ok(rt.from_property_descriptor(&d)),
                None => Ok(JsValue::Undefined),
            }
        }),
    );
    set_method(
        rt,
        ctor,
        "getOwnPropertyDescriptors",
        1,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let keys = rt.own_keys(oid)?;
            let out = rt.create_object();
            for k in keys {
                if let Some(d) = rt.get_own_property_descriptor(oid, &k)? {
                    let dobj = rt.from_property_descriptor(&d);
                    rt.define_property(out, k, PropertyDescriptor::data(dobj, true, true, true))?;
                }
            }
            Ok(JsValue::Object(out))
        }),
    );
    set_method(
        rt,
        ctor,
        "keys",
        1,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let keys = rt.own_keys(oid)?;
            let mut out = Vec::new();
            for k in keys {
                if let PropertyKey::String(s) = k {
                    if let Some(d) =
                        rt.get_own_property_descriptor(oid, &PropertyKey::String(s.clone()))?
                    {
                        if d.enumerable.unwrap_or(false) {
                            out.push(JsValue::String(s));
                        }
                    }
                }
            }
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );
    set_method(
        rt,
        ctor,
        "values",
        1,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let keys = rt.own_keys(oid)?;
            let mut out = Vec::new();
            for k in keys {
                if let PropertyKey::String(s) = k {
                    let pk = PropertyKey::String(s);
                    if let Some(d) = rt.get_own_property_descriptor(oid, &pk)? {
                        if d.enumerable.unwrap_or(false) {
                            out.push(rt.get_property_key(oid, &pk)?);
                        }
                    }
                }
            }
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );
    set_method(
        rt,
        ctor,
        "entries",
        1,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let keys = rt.own_keys(oid)?;
            let mut out = Vec::new();
            for k in keys {
                if let PropertyKey::String(s) = k {
                    let pk = PropertyKey::String(s.clone());
                    if let Some(d) = rt.get_own_property_descriptor(oid, &pk)? {
                        if d.enumerable.unwrap_or(false) {
                            let v = rt.get_property_key(oid, &pk)?;
                            let pair = rt.create_array(vec![JsValue::String(s), v]);
                            out.push(JsValue::Object(pair));
                        }
                    }
                }
            }
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );
    set_method(
        rt,
        ctor,
        "fromEntries",
        1,
        Rc::new(|rt, _, args| {
            let iterable = args.first().unwrap_or(&JsValue::Undefined);
            if iterable.is_nullish() {
                return Err(JsError::type_err("Object.fromEntries requires an iterable"));
            }
            let out = rt.create_object();
            let iter = rt.get_iterator(iterable, false)?;
            let JsValue::Object(iter_id) = iter else {
                return Err(JsError::type_err("Iterator must be an object"));
            };
            let next_fn = rt.get_property(iter_id, "next")?;
            loop {
                let res = rt.call_function(next_fn.clone(), iter.clone(), &[])?;
                let JsValue::Object(res_id) = res else {
                    return Err(JsError::type_err("Iterator result is not an object"));
                };
                if rt.get_property(res_id, "done")?.to_boolean() {
                    break;
                }
                let item = rt.get_property(res_id, "value")?;
                let step_res: Result<(), JsError> = (|| {
                    let JsValue::Object(iid) = item else {
                        return Err(JsError::type_err("Iterator value is not an entry object"));
                    };
                    let k = rt.get_property(iid, "0")?;
                    let v = rt.get_property(iid, "1")?;
                    let pk = rt.to_property_key(&k)?;
                    rt.define_property(out, pk, PropertyDescriptor::data(v, true, true, true))?;
                    Ok(())
                })();
                if let Err(e) = step_res {
                    rt.iterator_close(&iter);
                    return Err(e);
                }
            }
            Ok(JsValue::Object(out))
        }),
    );
    set_method(
        rt,
        ctor,
        "getOwnPropertyNames",
        1,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let keys = rt.own_keys(oid)?;
            let out: Vec<JsValue> = keys
                .into_iter()
                .filter_map(|k| match k {
                    PropertyKey::String(s) => Some(JsValue::String(s)),
                    _ => None,
                })
                .collect();
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );
    set_method(
        rt,
        ctor,
        "getOwnPropertySymbols",
        1,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let keys = rt.own_keys(oid)?;
            let out: Vec<JsValue> = keys
                .into_iter()
                .filter_map(|k| match k {
                    PropertyKey::Symbol(id) => Some(JsValue::Symbol(id)),
                    _ => None,
                })
                .collect();
            Ok(JsValue::Object(rt.create_array(out)))
        }),
    );
    set_method(
        rt,
        ctor,
        "getPrototypeOf",
        1,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            Ok(rt
                .get_prototype_of(oid)?
                .map(JsValue::Object)
                .unwrap_or(JsValue::Null))
        }),
    );
    set_method(
        rt,
        ctor,
        "setPrototypeOf",
        2,
        Rc::new(|rt, _, args| {
            let target = args.first().unwrap_or(&JsValue::Undefined);
            if target.is_nullish() {
                return Err(JsError::type_err("Object.setPrototypeOf called on null or undefined"));
            }
            let proto = match args.get(1) {
                Some(JsValue::Null) => None,
                Some(JsValue::Object(id)) => Some(*id),
                _ => {
                    return Err(JsError::type_err(
                        "Object prototype may only be an Object or null",
                    ));
                }
            };
            if let JsValue::Object(oid) = target {
                if !rt.set_prototype_of(*oid, proto)? {
                    return Err(JsError::type_err("Cannot set prototype of object"));
                }
            }
            Ok(target.clone())
        }),
    );
    set_method(
        rt,
        ctor,
        "assign",
        2,
        Rc::new(|rt, _, args| {
            let target_id = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            for src in args.iter().skip(1) {
                if src.is_nullish() {
                    continue;
                }
                let src_id = rt.to_object(src)?;
                let keys = rt.own_keys(src_id)?;
                for k in keys {
                    if let Some(d) = rt.get_own_property_descriptor(src_id, &k)? {
                        if d.enumerable.unwrap_or(false) {
                            let val = rt.get_property_key(src_id, &k)?;
                            rt.set_property_key_strict(target_id, k, val, true)?;
                        }
                    }
                }
            }
            Ok(JsValue::Object(target_id))
        }),
    );
    set_method(
        rt,
        ctor,
        "is",
        2,
        Rc::new(|_, _, args| {
            let a = args.first().unwrap_or(&JsValue::Undefined);
            let b = args.get(1).unwrap_or(&JsValue::Undefined);
            Ok(JsValue::Boolean(a.same_value(b)))
        }),
    );
    set_method(
        rt,
        ctor,
        "hasOwn",
        2,
        Rc::new(|rt, _, args| {
            let oid = rt.to_object(args.first().unwrap_or(&JsValue::Undefined))?;
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Boolean(rt.has_own_property(oid, &key)?))
        }),
    );
    set_method(
        rt,
        ctor,
        "freeze",
        1,
        Rc::new(|rt, _, args| {
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(oid) = val {
                set_integrity_level(rt, oid, true)?;
            }
            Ok(val)
        }),
    );
    set_method(
        rt,
        ctor,
        "seal",
        1,
        Rc::new(|rt, _, args| {
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(oid) = val {
                set_integrity_level(rt, oid, false)?;
            }
            Ok(val)
        }),
    );
    set_method(
        rt,
        ctor,
        "preventExtensions",
        1,
        Rc::new(|rt, _, args| {
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Object(oid) = val {
                if !rt.prevent_extensions(oid)? {
                    return Err(JsError::type_err("Cannot prevent extensions"));
                }
            }
            Ok(val)
        }),
    );
    set_method(
        rt,
        ctor,
        "isExtensible",
        1,
        Rc::new(|rt, _, args| {
            let Some(JsValue::Object(oid)) = args.first() else {
                return Ok(JsValue::Boolean(false));
            };
            Ok(JsValue::Boolean(rt.is_extensible(*oid)?))
        }),
    );
    set_method(
        rt,
        ctor,
        "isFrozen",
        1,
        Rc::new(|rt, _, args| {
            let Some(JsValue::Object(oid)) = args.first() else {
                return Ok(JsValue::Boolean(true));
            };
            Ok(JsValue::Boolean(test_integrity_level(rt, *oid, true)?))
        }),
    );
    set_method(
        rt,
        ctor,
        "isSealed",
        1,
        Rc::new(|rt, _, args| {
            let Some(JsValue::Object(oid)) = args.first() else {
                return Ok(JsValue::Boolean(true));
            };
            Ok(JsValue::Boolean(test_integrity_level(rt, *oid, false)?))
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
                return Err(JsError::type_err("Object.groupBy callback is not a function"));
            }
            let out = rt.create_object_with_proto(None);
            for (i, item) in items.into_iter().enumerate() {
                let kv = rt.call_function(
                    cb.clone(),
                    JsValue::Undefined,
                    &[item.clone(), JsValue::Number(i as f64)],
                )?;
                let pk = rt.to_property_key(&kv)?;
                let group = if rt.has_own_property(out, &pk)? {
                    rt.get_property_key(out, &pk)?
                } else {
                    let arr = rt.create_array(Vec::new());
                    rt.define_property(
                        out,
                        pk,
                        PropertyDescriptor::data(JsValue::Object(arr), true, true, true),
                    )?;
                    JsValue::Object(arr)
                };
                if let JsValue::Object(arr_id) = group {
                    if let ObjectKind::Array(ref mut arr) = rt.heap.get_obj_mut(arr_id).kind {
                        let idx = arr.length;
                        arr.set(idx, item);
                    }
                }
            }
            Ok(JsValue::Object(out))
        }),
    );

    set_global(rt, "Object", JsValue::Object(ctor));
}

fn define_properties_impl(
    rt: &mut JsRuntime,
    obj: ObjectId,
    props: &JsValue,
) -> Result<(), JsError> {
    let props_id = rt.to_object(props)?;
    let keys = rt.own_keys(props_id)?;
    let mut descs = Vec::new();
    for k in keys {
        if let Some(d) = rt.get_own_property_descriptor(props_id, &k)? {
            if d.enumerable.unwrap_or(false) {
                let v = rt.get_property_key(props_id, &k)?;
                let pd = rt.to_property_descriptor(&v)?;
                descs.push((k, pd));
            }
        }
    }
    for (k, pd) in descs {
        if !rt.define_property(obj, k, pd)? {
            return Err(JsError::type_err("Cannot define property"));
        }
    }
    Ok(())
}

fn set_integrity_level(rt: &mut JsRuntime, oid: ObjectId, freeze: bool) -> Result<(), JsError> {
    if !rt.prevent_extensions(oid)? {
        return Err(JsError::type_err("Cannot prevent extensions"));
    }
    let keys = rt.own_keys(oid)?;
    for k in keys {
        if let Some(d) = rt.get_own_property_descriptor(oid, &k)? {
            let mut next = PropertyDescriptor::empty();
            next.configurable = Some(false);
            if freeze && !d.is_accessor() {
                next.writable = Some(false);
            }
            if !rt.define_property(oid, k, next)? {
                return Err(JsError::type_err("Cannot define property"));
            }
        }
    }
    Ok(())
}

fn test_integrity_level(
    rt: &mut JsRuntime,
    oid: ObjectId,
    frozen: bool,
) -> Result<bool, JsError> {
    if rt.is_extensible(oid)? {
        return Ok(false);
    }
    let keys = rt.own_keys(oid)?;
    for k in keys {
        if let Some(d) = rt.get_own_property_descriptor(oid, &k)? {
            if d.configurable == Some(true) {
                return Ok(false);
            }
            if frozen && !d.is_accessor() && d.writable == Some(true) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub fn object_to_string(rt: &mut JsRuntime, val: &JsValue) -> Result<String, JsError> {
    match val {
        JsValue::Undefined => return Ok("[object Undefined]".into()),
        JsValue::Null => return Ok("[object Null]".into()),
        _ => {}
    }
    let oid = rt.to_object(val)?;
    let builtin_tag = match &rt.heap.get_obj(oid).kind {
        ObjectKind::Array(_) => "Array",
        ObjectKind::Arguments(_) => "Arguments",
        ObjectKind::Function(_) | ObjectKind::NativeFunction(_) => "Function",
        ObjectKind::Error(_) => "Error",
        ObjectKind::BooleanObj(_) => "Boolean",
        ObjectKind::NumberObj(_) => "Number",
        ObjectKind::StringObj(_) => "String",
        ObjectKind::Date(_) => "Date",
        ObjectKind::RegExp(_) => "RegExp",
        ObjectKind::Map(_) => "Map",
        ObjectKind::Set(_) => "Set",
        ObjectKind::WeakMap(_) => "WeakMap",
        ObjectKind::WeakSet(_) => "WeakSet",
        ObjectKind::WeakRef(_) => "WeakRef",
        ObjectKind::FinalizationRegistry(_) => "FinalizationRegistry",
        ObjectKind::Promise(_) => "Promise",
        ObjectKind::ArrayBuffer(_) => "ArrayBuffer",
        ObjectKind::DataView(_) => "DataView",
        ObjectKind::TypedArray(ta) => ta.kind.name(),
        ObjectKind::Proxy(p) => {
            if !p.revoked && rt.is_callable(&JsValue::Object(p.target)) {
                "Function"
            } else {
                "Object"
            }
        }
        _ => "Object",
    };
    let tag_key = PropertyKey::Symbol(rt.symbols.to_string_tag);
    if let Ok(JsValue::String(tag)) = rt.get_property_key(oid, &tag_key) {
        return Ok(format!("[object {}]", tag));
    }
    Ok(format!("[object {}]", builtin_tag))
}

trait OptValCopy {
    fn copied_opt(self) -> Option<JsValue>;
}

impl OptValCopy for Option<&JsValue> {
    fn copied_opt(self) -> Option<JsValue> {
        self.cloned()
    }
}
