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
    let agg_ctor = rt.create_native_constructor(
        "AggregateError",
        2,
        agg_proto,
        Rc::new(|rt, _, args| {
            let errors_arg = args.first().unwrap_or(&JsValue::Undefined);
            let errors = rt.collect_iterable(errors_arg)?;
            let msg = match args.get(1) {
                None | Some(JsValue::Undefined) => String::new(),
                Some(v) => rt.to_string(v)?,
            };
            let err_obj = rt.create_error_object(ErrorKind::AggregateError, &msg);
            let err_arr = rt.create_array(errors);
            rt.heap.get_obj_mut(err_obj).set_own_prop(
                PropertyKey::from("errors"),
                PropEntry::data(JsValue::Object(err_arr), true, false, true),
            );
            if let Some(JsValue::Object(opts)) = args.get(2) {
                if rt.has_property(*opts, &PropertyKey::from("cause"))? {
                    let cause = rt.get_property(*opts, "cause")?;
                    rt.heap.get_obj_mut(err_obj).set_own_prop(
                        PropertyKey::from("cause"),
                        PropEntry::data(cause, true, false, true),
                    );
                }
            }
            Ok(JsValue::Object(err_obj))
        }),
    );
    rt.heap.get_obj_mut(agg_ctor).prototype = Some(err_ctor);
    set_global(rt, "AggregateError", JsValue::Object(agg_ctor));
}

fn make_error_ctor(
    rt: &mut JsRuntime,
    kind: ErrorKind,
    proto: ObjectId,
    parent_ctor: Option<ObjectId>,
) -> ObjectId {
    let ctor = rt.create_native_constructor(
        kind.name(),
        1,
        proto,
        Rc::new(move |rt, _, args| {
            let msg = match args.first() {
                None | Some(JsValue::Undefined) => String::new(),
                Some(v) => rt.to_string(v)?,
            };
            let obj = rt.create_error_object(kind, &msg);
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
