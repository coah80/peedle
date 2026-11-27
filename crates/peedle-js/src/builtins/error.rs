use crate::builtins::array::create_data_property_or_throw;
use crate::builtins::{set_global, set_method};
use crate::error::{ErrorKind, JsError};
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    let error_proto = rt.intrinsics.error_proto;
    rt.heap.get_obj_mut(error_proto).set_own_prop(
        PropertyKey::from("name"),
        PropEntry::data(JsValue::String("Error".into()), true, false, true),
    );
    rt.heap.get_obj_mut(error_proto).set_own_prop(
        PropertyKey::from("message"),
        PropEntry::data(JsValue::String(String::new()), true, false, true),
    );
    let stack_get = rt.create_native_function(
        "get stack",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Error.prototype.stack getter requires an object"));
            };
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::Error(e) => Ok(JsValue::String(e.stack.clone())),
                _ => Ok(JsValue::Undefined),
            }
        }),
    );
    let stack_set = rt.create_native_function(
        "set stack",
        1,
        Rc::new(move |rt, this, args| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Error.prototype.stack setter requires an object"));
            };
            let val = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !matches!(val, JsValue::String(_)) {
                return Err(JsError::type_err("Error.prototype.stack value must be a string"));
            }
            if oid == error_proto {
                return Err(JsError::type_err("Cannot set stack on Error.prototype"));
            }
            let key = PropertyKey::from("stack");
            let desc = rt.get_own_property_descriptor(oid, &key)?;
            if desc.is_none() {
                create_data_property_or_throw(rt, oid, key, val)?;
            } else {
                rt.set_property_key_strict(oid, key, val, true)?;
            }
            Ok(JsValue::Undefined)
        }),
    );
    let _ = rt.define_property(
        error_proto,
        PropertyKey::from("stack"),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(stack_get)),
            set: Some(JsValue::Object(stack_set)),
            enumerable: Some(false),
            configurable: Some(true),
        },
    );
    set_method(
        rt,
        error_proto,
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err("Error.prototype.toString called on non-object"));
            };
            let name_v = rt.get_property(oid, "name")?;
            let name = if name_v.is_undefined() {
                "Error".to_string()
            } else {
                rt.to_string(&name_v)?
            };
            let msg_v = rt.get_property(oid, "message")?;
            let msg = if msg_v.is_undefined() {
                String::new()
            } else {
                rt.to_string(&msg_v)?
            };
            if name.is_empty() {
                Ok(JsValue::String(msg))
            } else if msg.is_empty() {
                Ok(JsValue::String(name))
            } else {
                Ok(JsValue::String(format!("{}: {}", name, msg)))
            }
        }),
    );

    let err_ctor = make_error_ctor(rt, ErrorKind::Error, error_proto, None);
    set_method(
        rt,
        err_ctor,
        "isError",
        1,
        Rc::new(|rt, _, args| {
            let b = matches!(
                args.first(),
                Some(JsValue::Object(oid)) if matches!(rt.heap.get_obj(*oid).kind, ObjectKind::Error(_))
            );
            Ok(JsValue::Boolean(b))
        }),
    );
    set_global(rt, "Error", JsValue::Object(err_ctor));

    let subclasses = [
        (ErrorKind::TypeError, rt.intrinsics.type_error_proto),
        (ErrorKind::ReferenceError, rt.intrinsics.reference_error_proto),
        (ErrorKind::SyntaxError, rt.intrinsics.syntax_error_proto),
        (ErrorKind::RangeError, rt.intrinsics.range_error_proto),
        (ErrorKind::UriError, rt.intrinsics.uri_error_proto),
        (ErrorKind::EvalError, rt.intrinsics.eval_error_proto),
    ];

    for (kind, proto) in subclasses {
        rt.heap.get_obj_mut(proto).set_own_prop(
            PropertyKey::from("name"),
            PropEntry::data(JsValue::String(kind.name().into()), true, false, true),
        );
        rt.heap.get_obj_mut(proto).set_own_prop(
            PropertyKey::from("message"),
            PropEntry::data(JsValue::String(String::new()), true, false, true),
        );
        let ctor = make_error_ctor(rt, kind, proto, Some(err_ctor));
        set_global(rt, kind.name(), JsValue::Object(ctor));
    }

    let agg_proto = rt.intrinsics.aggregate_error_proto;
    rt.heap.get_obj_mut(agg_proto).set_own_prop(
        PropertyKey::from("name"),
        PropEntry::data(JsValue::String("AggregateError".into()), true, false, true),
    );
    rt.heap.get_obj_mut(agg_proto).set_own_prop(
        PropertyKey::from("message"),
        PropEntry::data(JsValue::String(String::new()), true, false, true),
    );
    let agg_ctor = rt.create_spec_constructor(
        "AggregateError",
        2,
        agg_proto,
        Rc::new(move |rt, _, args| {
            let obj_proto = rt.get_prototype_from_new_target(agg_proto)?;
            let has_msg = !matches!(args.get(1), None | Some(JsValue::Undefined));
            let msg = if has_msg {
                rt.to_string(&args[1])?
            } else {
                String::new()
            };
            let err_obj =
                rt.create_error_object_with_proto(ErrorKind::AggregateError, &msg, has_msg, obj_proto);
            if let Some(JsValue::Object(opts)) = args.get(2) {
                if rt.has_property(*opts, &PropertyKey::from("cause"))? {
                    let cause = rt.get_property(*opts, "cause")?;
                    rt.heap.get_obj_mut(err_obj).set_own_prop(
                        PropertyKey::from("cause"),
                        PropEntry::data(cause, true, false, true),
                    );
                }
            }
            let errors_arg = args.first().unwrap_or(&JsValue::Undefined);
            let errors = rt.collect_iterable(errors_arg)?;
            let err_arr = rt.create_array(errors);
            rt.heap.get_obj_mut(err_obj).set_own_prop(
                PropertyKey::from("errors"),
                PropEntry::data(JsValue::Object(err_arr), true, false, true),
            );
            Ok(JsValue::Object(err_obj))
        }),
    );
    rt.heap.get_obj_mut(agg_ctor).prototype = Some(err_ctor);
    set_global(rt, "AggregateError", JsValue::Object(agg_ctor));

    let sup_proto = rt.intrinsics.suppressed_error_proto;
    rt.heap.get_obj_mut(sup_proto).set_own_prop(
        PropertyKey::from("name"),
        PropEntry::data(JsValue::String("SuppressedError".into()), true, false, true),
    );
    rt.heap.get_obj_mut(sup_proto).set_own_prop(
        PropertyKey::from("message"),
        PropEntry::data(JsValue::String(String::new()), true, false, true),
    );
    let sup_ctor = rt.create_spec_constructor(
        "SuppressedError",
        3,
        sup_proto,
        Rc::new(move |rt, _, args| {
            let obj_proto = rt.get_prototype_from_new_target(sup_proto)?;
            let has_msg = !matches!(args.get(2), None | Some(JsValue::Undefined));
            let msg = if has_msg {
                rt.to_string(&args[2])?
            } else {
                String::new()
            };
            let err_obj = rt.create_error_object_with_proto(
                ErrorKind::SuppressedError,
                &msg,
                has_msg,
                obj_proto,
            );
            let error_val = args.first().cloned().unwrap_or(JsValue::Undefined);
            let suppressed_val = args.get(1).cloned().unwrap_or(JsValue::Undefined);
            rt.heap.get_obj_mut(err_obj).set_own_prop(
                PropertyKey::from("error"),
                PropEntry::data(error_val, true, false, true),
            );
            rt.heap.get_obj_mut(err_obj).set_own_prop(
                PropertyKey::from("suppressed"),
                PropEntry::data(suppressed_val, true, false, true),
            );
            Ok(JsValue::Object(err_obj))
        }),
    );
    rt.heap.get_obj_mut(sup_ctor).prototype = Some(err_ctor);
    set_global(rt, "SuppressedError", JsValue::Object(sup_ctor));
}

fn make_error_ctor(
    rt: &mut JsRuntime,
    kind: ErrorKind,
    proto: ObjectId,
    parent_ctor: Option<ObjectId>,
) -> ObjectId {
    let ctor = rt.create_spec_constructor(
        kind.name(),
        1,
        proto,
        Rc::new(move |rt, _, args| {
            let obj_proto = rt.get_prototype_from_new_target(proto)?;
            let has_msg = !matches!(args.first(), None | Some(JsValue::Undefined));
            let msg = if has_msg {
                rt.to_string(&args[0])?
            } else {
                String::new()
            };
            let obj = rt.create_error_object_with_proto(kind, &msg, has_msg, obj_proto);
            if let Some(JsValue::Object(opts)) = args.get(1) {
                if rt.has_property(*opts, &PropertyKey::from("cause"))? {
                    let cause = rt.get_property(*opts, "cause")?;
                    rt.heap.get_obj_mut(obj).set_own_prop(
                        PropertyKey::from("cause"),
                        PropEntry::data(cause, true, false, true),
                    );
                }
            }
            Ok(JsValue::Object(obj))
        }),
    );
    if let Some(p) = parent_ctor {
        rt.heap.get_obj_mut(ctor).prototype = Some(p);
    }
    ctor
}
