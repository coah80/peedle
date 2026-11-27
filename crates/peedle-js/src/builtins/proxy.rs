use crate::builtins::{set_global, set_method};
use crate::error::JsError;
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    init_proxy(rt);
    init_reflect(rt);
}

fn create_proxy_obj(rt: &mut JsRuntime, args: &[JsValue]) -> Result<ObjectId, JsError> {
    let Some(JsValue::Object(target)) = args.first() else {
        return Err(JsError::type_err(
            "Cannot create proxy with a non-object as target or handler",
        ));
    };
    let Some(JsValue::Object(handler)) = args.get(1) else {
        return Err(JsError::type_err(
            "Cannot create proxy with a non-object as target or handler",
        ));
    };
    if let ObjectKind::Proxy(ref p) = rt.heap.get_obj(*target).kind {
        if p.revoked {
            return Err(JsError::type_err("Cannot create proxy with a revoked proxy as target"));
        }
    }
    if let ObjectKind::Proxy(ref p) = rt.heap.get_obj(*handler).kind {
        if p.revoked {
            return Err(JsError::type_err("Cannot create proxy with a revoked proxy as handler"));
        }
    }
    Ok(rt.heap.alloc_object(JsObject::new(
        None,
        ObjectKind::Proxy(ProxyData {
            target: *target,
            handler: *handler,
            revoked: false,
        }),
    )))
}

fn init_proxy(rt: &mut JsRuntime) {
    let ctor = rt.heap.alloc_object(JsObject::new(
        Some(rt.intrinsics.function_proto),
        ObjectKind::NativeFunction(NativeFn {
            name: "Proxy".into(),
            length: 2,
            callback: Rc::new(|rt, this, args| {
                if !this.is_object() {
                    return Err(JsError::type_err("Constructor Proxy requires 'new'"));
                }
                Ok(JsValue::Object(create_proxy_obj(rt, args)?))
            }),
            is_constructor: true,
        }),
    ));
    rt.heap.get_obj_mut(ctor).set_own_prop(
        PropertyKey::from("name"),
        PropEntry::data(JsValue::String("Proxy".into()), false, false, true),
    );
    rt.heap.get_obj_mut(ctor).set_own_prop(
        PropertyKey::from("length"),
        PropEntry::data(JsValue::Number(2.0), false, false, true),
    );

    set_method(
        rt,
        ctor,
        "revocable",
        2,
        Rc::new(|rt, _, args| {
            let proxy_id = create_proxy_obj(rt, args)?;
            let revoke = rt.create_native_function(
                "",
                0,
                Rc::new(move |rt, _, _| {
                    if let ObjectKind::Proxy(ref mut p) = rt.heap.get_obj_mut(proxy_id).kind {
                        p.revoked = true;
                    }
                    Ok(JsValue::Undefined)
                }),
            );
            let out = rt.create_object();
            let _ = rt.set_property(out, "proxy", JsValue::Object(proxy_id));
            let _ = rt.set_property(out, "revoke", JsValue::Object(revoke));
            Ok(JsValue::Object(out))
        }),
    );

    set_global(rt, "Proxy", JsValue::Object(ctor));
}

fn require_object_arg(args: &[JsValue], idx: usize, name: &str) -> Result<ObjectId, JsError> {
    match args.get(idx) {
        Some(JsValue::Object(oid)) => Ok(*oid),
        _ => Err(JsError::type_err(format!("Reflect.{} called on non-object", name))),
    }
}

fn extract_arg_list(rt: &mut JsRuntime, val: Option<&JsValue>) -> Result<Vec<JsValue>, JsError> {
    let Some(JsValue::Object(oid)) = val else {
        return Err(JsError::type_err("CreateListFromArrayLike called on non-object"));
    };
    let len_v = rt.get_property(*oid, "length")?;
    let len = to_length(rt.to_number(&len_v)?);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        out.push(rt.get_property(*oid, &i.to_string())?);
    }
    Ok(out)
}

fn init_reflect(rt: &mut JsRuntime) {
    let reflect = rt.create_object();
    let tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(reflect).set_own_prop(
        PropertyKey::Symbol(tag),
        PropEntry::data(JsValue::String("Reflect".into()), false, false, true),
    );

    set_method(
        rt,
        reflect,
        "apply",
        3,
        Rc::new(|rt, _, args| {
            let target = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_callable(&target) {
                return Err(JsError::type_err("Reflect.apply target is not callable"));
            }
            let this_arg = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            let list = extract_arg_list(rt, args.get(2))?;
            rt.call_function(target, this_arg, &list)
        }),
    );

    set_method(
        rt,
        reflect,
        "construct",
        2,
        Rc::new(|rt, _, args| {
            let target = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !rt.is_constructor(&target) {
                return Err(JsError::type_err("Reflect.construct target is not a constructor"));
            }
            let list = extract_arg_list(rt, args.get(1))?;
            let new_target = match args.get(2) {
                Some(nt) => {
                    if !rt.is_constructor(nt) {
                        return Err(JsError::type_err(
                            "Reflect.construct newTarget is not a constructor",
                        ));
                    }
                    nt.clone()
                }
                None => target.clone(),
            };
            rt.construct_with_new_target(target, &list, new_target)
        }),
    );

    set_method(
        rt,
        reflect,
        "defineProperty",
        3,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "defineProperty")?;
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            let desc = rt.to_property_descriptor(args.get(2).unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Boolean(rt.define_property(oid, key, desc)?))
        }),
    );

    set_method(
        rt,
        reflect,
        "deleteProperty",
        2,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "deleteProperty")?;
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Boolean(rt.delete_property(oid, &key)?))
        }),
    );

    set_method(
        rt,
        reflect,
        "get",
        2,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "get")?;
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            let receiver = args
                .get(2)
                .cloned()
                .unwrap_or(JsValue::Object(oid));
            rt.get_property_key_receiver(oid, &key, receiver)
        }),
    );

    set_method(
        rt,
        reflect,
        "getOwnPropertyDescriptor",
        2,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "getOwnPropertyDescriptor")?;
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            match rt.get_own_property_descriptor(oid, &key)? {
                Some(d) => Ok(rt.from_property_descriptor(&d)),
                None => Ok(JsValue::Undefined),
            }
        }),
    );

    set_method(
        rt,
        reflect,
        "getPrototypeOf",
        1,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "getPrototypeOf")?;
            Ok(rt
                .get_prototype_of(oid)?
                .map(JsValue::Object)
                .unwrap_or(JsValue::Null))
        }),
    );

    set_method(
        rt,
        reflect,
        "has",
        2,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "has")?;
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            Ok(JsValue::Boolean(rt.has_property(oid, &key)?))
        }),
    );

    set_method(
        rt,
        reflect,
        "isExtensible",
        1,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "isExtensible")?;
            Ok(JsValue::Boolean(rt.is_extensible(oid)?))
        }),
    );

    set_method(
        rt,
        reflect,
        "ownKeys",
        1,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "ownKeys")?;
            let keys = rt.own_keys(oid)?;
            let vals: Vec<JsValue> = keys.into_iter().map(|k| k.to_value()).collect();
            Ok(JsValue::Object(rt.create_array(vals)))
        }),
    );

    set_method(
        rt,
        reflect,
        "preventExtensions",
        1,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "preventExtensions")?;
            Ok(JsValue::Boolean(rt.prevent_extensions(oid)?))
        }),
    );

    set_method(
        rt,
        reflect,
        "set",
        3,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "set")?;
            let key = rt.to_property_key(args.get(1).unwrap_or(&JsValue::Undefined))?;
            let val = args.get(2).cloned().unwrap_or(JsValue::Undefined);
            let receiver = args
                .get(3)
                .cloned()
                .unwrap_or(JsValue::Object(oid));
            Ok(JsValue::Boolean(
                rt.set_property_key_receiver(oid, key, val, receiver)?,
            ))
        }),
    );

    set_method(
        rt,
        reflect,
        "setPrototypeOf",
        2,
        Rc::new(|rt, _, args| {
            let oid = require_object_arg(args, 0, "setPrototypeOf")?;
            let proto = match args.get(1) {
                Some(JsValue::Null) => None,
                Some(JsValue::Object(p)) => Some(*p),
                _ => {
                    return Err(JsError::type_err(
                        "Reflect.setPrototypeOf prototype must be an object or null",
                    ));
                }
            };
            Ok(JsValue::Boolean(rt.set_prototype_of(oid, proto)?))
        }),
    );

    set_global(rt, "Reflect", JsValue::Object(reflect));
}
