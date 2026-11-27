use crate::builtins::{set_global, set_method};
use crate::compiler::Compiler;
use crate::error::{ErrorKind, JsError};
use crate::heap::*;
use crate::parser::Parser;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    init_abstract_module_source(rt);
    init_shadow_realm(rt);
}

fn init_abstract_module_source(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.abstract_module_source_proto;
    let ctor = rt.create_spec_constructor(
        "AbstractModuleSource",
        0,
        proto,
        Rc::new(|_, _, _| {
            Err(JsError::type_err(
                "AbstractModuleSource cannot be constructed directly",
            ))
        }),
    );
    rt.intrinsics.abstract_module_source_ctor = ctor;
    let sym_tag = rt.symbols.to_string_tag;
    let tag_getter = rt.create_native_function(
        "get [Symbol.toStringTag]",
        0,
        Rc::new(|_, _, _| Ok(JsValue::Undefined)),
    );
    let _ = rt.define_property(
        proto,
        PropertyKey::Symbol(sym_tag),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(tag_getter)),
            set: None,
            enumerable: Some(false),
            configurable: Some(true),
        },
    );
}

fn init_shadow_realm(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.shadow_realm_proto;
    let sym_tag = rt.symbols.to_string_tag;
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_tag),
        PropEntry::data(JsValue::String("ShadowRealm".into()), false, false, true),
    );

    set_method(
        rt,
        proto,
        "evaluate",
        1,
        Rc::new(|rt, this, args| {
            let eval_realm = validate_shadow_realm(rt, &this)?;
            let Some(JsValue::String(src)) = args.first() else {
                return Err(JsError::type_err(
                    "ShadowRealm.prototype.evaluate requires a string argument",
                ));
            };
            let src = src.clone();
            let caller_realm = rt.current_realm_id;
            let program = Parser::parse_script(&src).map_err(|_| {
                let err = rt.create_realm_error_object(
                    caller_realm,
                    ErrorKind::SyntaxError,
                    "SyntaxError in ShadowRealm.prototype.evaluate",
                );
                JsError::Thrown(JsValue::Object(err))
            })?;
            let chunk = Compiler::compile_program(&program).map_err(|_| {
                let err = rt.create_realm_error_object(
                    caller_realm,
                    ErrorKind::SyntaxError,
                    "SyntaxError in ShadowRealm.prototype.evaluate",
                );
                JsError::Thrown(JsValue::Object(err))
            })?;
            let prev = rt.switch_realm(eval_realm);
            let eval_res = (|| -> Result<JsValue, JsError> {
                let lex_env = rt.heap.alloc_env(EnvRecord::new(
                    Some(rt.global_env),
                    false,
                    program.strict,
                ));
                if program.strict {
                    rt.hoist_into_env(&chunk, lex_env, true)?;
                } else {
                    for vn in &chunk.hoisted_vars {
                        let g = rt.global_object;
                        let pk = PropertyKey::String(vn.clone());
                        if rt.heap.get_obj(g).get_own_prop(&pk).is_none() {
                            rt.heap.get_obj_mut(g).set_own_prop(
                                pk,
                                PropEntry::data(JsValue::Undefined, true, true, true),
                            );
                        }
                    }
                    for (fn_name, tmpl) in &chunk.hoisted_funcs {
                        let fn_id = rt.create_js_function(tmpl, rt.global_env);
                        let g = rt.global_object;
                        rt.heap.get_obj_mut(g).set_own_prop(
                            PropertyKey::String(fn_name.clone()),
                            PropEntry::data(JsValue::Object(fn_id), true, true, true),
                        );
                    }
                    for (name, is_const) in &chunk.lex_names {
                        rt.heap.get_env_mut(lex_env).bindings.insert(
                            name.clone(),
                            Binding {
                                value: JsValue::Undefined,
                                mutable: !is_const,
                                initialized: false,
                                deletable: false,
                                strict_immutable: *is_const,
                            },
                        );
                    }
                }
                let mut frame = crate::bytecode::CallFrame::new(chunk, lex_env, None);
                rt.active_frames.push(lex_env);
                let r = rt.run_frame(&mut frame);
                rt.active_frames.pop();
                match r? {
                    crate::exec::FrameOutcome::Return(v) => Ok(v),
                    _ => Ok(JsValue::Undefined),
                }
            })();
            rt.switch_realm(prev);
            let val = match eval_res {
                Ok(v) => v,
                Err(_) => {
                    return Err(caller_type_error(
                        rt,
                        caller_realm,
                        "ShadowRealm evaluation threw an error",
                    ));
                }
            };
            get_wrapped_value(rt, caller_realm, val)
        }),
    );

    set_method(
        rt,
        proto,
        "importValue",
        2,
        Rc::new(|rt, this, args| {
            let eval_realm = validate_shadow_realm(rt, &this)?;
            let specifier = rt.to_string(args.first().unwrap_or(&JsValue::Undefined))?;
            let Some(JsValue::String(export_name)) = args.get(1) else {
                return Err(JsError::type_err(
                    "ShadowRealm.prototype.importValue exportName must be a string",
                ));
            };
            let export_name = export_name.clone();
            let caller_realm = rt.current_realm_id;
            let out_p = rt.create_promise();
            match rt.load_module_in_realm(eval_realm, &specifier) {
                Err(_) => {
                    let err_obj = rt.create_realm_error_object(
                        caller_realm,
                        ErrorKind::TypeError,
                        "ShadowRealm.prototype.importValue failed to load module",
                    );
                    rt.reject_promise(out_p, JsValue::Object(err_obj));
                }
                Ok((ns_id, eval_p)) => {
                    let resolve_export = move |rt: &mut JsRuntime| {
                        let prev = rt.switch_realm(caller_realm);
                        let res = (|| -> Result<JsValue, JsError> {
                            if !rt.has_own_property(ns_id, &PropertyKey::String(export_name.clone()))? {
                                return Err(caller_type_error(
                                    rt,
                                    caller_realm,
                                    "ShadowRealm.prototype.importValue export not found",
                                ));
                            }
                            let val = rt.get_property(ns_id, &export_name).map_err(|_| {
                                caller_type_error(
                                    rt,
                                    caller_realm,
                                    "ShadowRealm.prototype.importValue export access failed",
                                )
                            })?;
                            get_wrapped_value(rt, caller_realm, val)
                        })();
                        rt.switch_realm(prev);
                        match res {
                            Ok(v) => rt.resolve_promise(out_p, v),
                            Err(e) => {
                                let ev = rt.error_to_js_value(e);
                                rt.reject_promise(out_p, ev);
                            }
                        }
                    };
                    if let Some(ep) = eval_p {
                        let on_ok = rt.create_native_function(
                            "",
                            1,
                            Rc::new(move |rt, _, _| {
                                resolve_export(rt);
                                Ok(JsValue::Undefined)
                            }),
                        );
                        let on_err = rt.create_native_function(
                            "",
                            1,
                            Rc::new(move |rt, _, _| {
                                let err_obj = rt.create_realm_error_object(
                                    caller_realm,
                                    ErrorKind::TypeError,
                                    "ShadowRealm.prototype.importValue module evaluation failed",
                                );
                                rt.reject_promise(out_p, JsValue::Object(err_obj));
                                Ok(JsValue::Undefined)
                            }),
                        );
                        rt.promise_then(
                            ep,
                            Some(JsValue::Object(on_ok)),
                            Some(JsValue::Object(on_err)),
                        );
                    } else {
                        let job = rt.create_native_function(
                            "",
                            0,
                            Rc::new(move |rt, _, _| {
                                resolve_export(rt);
                                Ok(JsValue::Undefined)
                            }),
                        );
                        rt.queue_microtask(JsValue::Object(job));
                    }
                }
            }
            Ok(JsValue::Object(out_p))
        }),
    );

    let ctor = rt.create_spec_constructor(
        "ShadowRealm",
        0,
        proto,
        Rc::new(|rt, _, _| {
            rt.require_new_target("ShadowRealm")?;
            let obj_proto = rt.get_prototype_from_new_target(rt.intrinsics.shadow_realm_proto)?;
            let realm_id = rt.create_realm();
            let obj = rt.heap.alloc_object(JsObject::new(
                Some(obj_proto),
                ObjectKind::ShadowRealm(realm_id),
            ));
            Ok(JsValue::Object(obj))
        }),
    );
    set_global(rt, "ShadowRealm", JsValue::Object(ctor));
}

fn validate_shadow_realm(rt: &JsRuntime, this: &JsValue) -> Result<usize, JsError> {
    let JsValue::Object(oid) = this else {
        return Err(JsError::type_err(
            "ShadowRealm method called on non-ShadowRealm",
        ));
    };
    match &rt.heap.get_obj(*oid).kind {
        ObjectKind::ShadowRealm(id) => Ok(*id),
        _ => Err(JsError::type_err(
            "ShadowRealm method called on non-ShadowRealm",
        )),
    }
}

fn caller_type_error(rt: &mut JsRuntime, caller_realm: usize, msg: &str) -> JsError {
    let err = rt.create_realm_error_object(caller_realm, ErrorKind::TypeError, msg);
    JsError::Thrown(JsValue::Object(err))
}

fn is_revoked_proxy(rt: &JsRuntime, mut oid: ObjectId) -> bool {
    for _ in 0..256 {
        match &rt.heap.get_obj(oid).kind {
            ObjectKind::Proxy(p) => {
                if p.revoked {
                    return true;
                }
                oid = p.target;
            }
            _ => return false,
        }
    }
    false
}

pub fn get_wrapped_value(
    rt: &mut JsRuntime,
    caller_realm: usize,
    val: JsValue,
) -> Result<JsValue, JsError> {
    let JsValue::Object(oid) = val else {
        return Ok(val);
    };
    if !rt.is_callable(&JsValue::Object(oid)) {
        return Err(caller_type_error(
            rt,
            caller_realm,
            "ShadowRealm cannot pass non-callable object across realm boundary",
        ));
    }
    wrapped_function_create(rt, caller_realm, oid)
}

pub fn wrapped_function_create(
    rt: &mut JsRuntime,
    caller_realm: usize,
    target: ObjectId,
) -> Result<JsValue, JsError> {
    if is_revoked_proxy(rt, target) {
        return Err(caller_type_error(
            rt,
            caller_realm,
            "Cannot wrap a revoked Proxy",
        ));
    }
    let meta_res = (|| -> Result<(f64, String), JsError> {
        let has_len = rt.has_own_property(target, &PropertyKey::from("length"))?;
        let mut len = 0.0f64;
        if has_len {
            let tlen = rt.get_property(target, "length")?;
            if let JsValue::Number(n) = tlen {
                if n == f64::INFINITY {
                    len = f64::INFINITY;
                } else if n == f64::NEG_INFINITY {
                    len = 0.0;
                } else {
                    len = to_integer_or_infinity(n).max(0.0);
                }
            }
        }
        let tname = rt.get_property(target, "name")?;
        let name = match tname {
            JsValue::String(s) => s,
            _ => String::new(),
        };
        Ok((len, name))
    })();
    let (len, name) = match meta_res {
        Ok(m) => m,
        Err(_) => {
            return Err(caller_type_error(
                rt,
                caller_realm,
                "Failed to copy name and length for wrapped function",
            ));
        }
    };
    let fn_proto = if caller_realm == rt.current_realm_id {
        rt.intrinsics.function_proto
    } else {
        rt.realms[caller_realm].intrinsics.function_proto
    };
    let mut obj = JsObject::new(
        Some(fn_proto),
        ObjectKind::WrappedFunction {
            target,
            caller_realm,
        },
    );
    obj.set_own_prop(
        PropertyKey::from("length"),
        PropEntry::data(JsValue::Number(len), false, false, true),
    );
    obj.set_own_prop(
        PropertyKey::from("name"),
        PropEntry::data(JsValue::String(name), false, false, true),
    );
    let wf = rt.heap.alloc_object(obj);
    rt.heap.object_realms.insert(wf, caller_realm);
    Ok(JsValue::Object(wf))
}

pub fn call_wrapped_function(
    rt: &mut JsRuntime,
    target: ObjectId,
    caller_realm: usize,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    let target_realm = match rt.get_function_realm(target) {
        Ok(r) => r,
        Err(_) => {
            return Err(caller_type_error(
                rt,
                caller_realm,
                "Wrapped function target is invalid or revoked",
            ));
        }
    };
    let mut wrapped_args = Vec::with_capacity(args.len());
    for a in args {
        let wa = match get_wrapped_value(rt, target_realm, a.clone()) {
            Ok(v) => v,
            Err(_) => {
                return Err(caller_type_error(
                    rt,
                    caller_realm,
                    "Wrapped function argument could not be wrapped",
                ));
            }
        };
        wrapped_args.push(wa);
    }
    let result = match rt.call_function(JsValue::Object(target), JsValue::Undefined, &wrapped_args)
    {
        Ok(v) => v,
        Err(_) => {
            return Err(caller_type_error(
                rt,
                caller_realm,
                "Wrapped function threw an exception",
            ));
        }
    };
    get_wrapped_value(rt, caller_realm, result)
}
