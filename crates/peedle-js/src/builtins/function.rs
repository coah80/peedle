use crate::builtins::{set_global, set_method, set_symbol_method};
use crate::compiler::Compiler;
use crate::error::JsError;
use crate::heap::*;
use crate::parser::Parser;
use crate::value::*;
use crate::vm::JsRuntime;
use std::rc::Rc;

pub fn init(rt: &mut JsRuntime) {
    let proto = rt.intrinsics.function_proto;
    let tte = rt.intrinsics.throw_type_error;
    rt.heap.object_realms.insert(tte, rt.current_realm_id);
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("length"),
        PropEntry::data(JsValue::Number(0.0), false, false, true),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::from("name"),
        PropEntry::data(JsValue::String(String::new()), false, false, true),
    );

    let _ = rt.define_property(
        proto,
        PropertyKey::from("caller"),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(tte)),
            set: Some(JsValue::Object(tte)),
            enumerable: Some(false),
            configurable: Some(true),
        },
    );
    let _ = rt.define_property(
        proto,
        PropertyKey::from("arguments"),
        PropertyDescriptor {
            value: None,
            writable: None,
            get: Some(JsValue::Object(tte)),
            set: Some(JsValue::Object(tte)),
            enumerable: Some(false),
            configurable: Some(true),
        },
    );

    set_method(
        rt,
        proto,
        "call",
        1,
        Rc::new(|rt, this, args| {
            if !rt.is_callable(&this) {
                return Err(JsError::type_err("Function.prototype.call called on non-function"));
            }
            let this_arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            let call_args = if args.len() > 1 { &args[1..] } else { &[] };
            rt.call_function(this, this_arg, call_args)
        }),
    );

    set_method(
        rt,
        proto,
        "apply",
        2,
        Rc::new(|rt, this, args| {
            if !rt.is_callable(&this) {
                return Err(JsError::type_err("Function.prototype.apply called on non-function"));
            }
            let this_arg = args.first().cloned().unwrap_or(JsValue::Undefined);
            let arg_list = match args.get(1) {
                None | Some(JsValue::Undefined) | Some(JsValue::Null) => Vec::new(),
                Some(JsValue::Object(oid)) => {
                    let len_v = rt.get_property(*oid, "length")?;
                    let len = to_length(rt.to_number(&len_v)?);
                    let mut list = Vec::with_capacity(len);
                    for i in 0..len {
                        list.push(rt.get_property(*oid, &i.to_string())?);
                    }
                    list
                }
                _ => {
                    return Err(JsError::type_err(
                        "CreateListFromArrayLike called on non-object",
                    ));
                }
            };
            rt.call_function(this, this_arg, &arg_list)
        }),
    );

    set_method(
        rt,
        proto,
        "bind",
        1,
        Rc::new(|rt, this, args| {
            if !rt.is_callable(&this) {
                return Err(JsError::type_err("Function.prototype.bind called on non-function"));
            }
            let JsValue::Object(target_id) = this else {
                return Err(JsError::type_err("Not a function"));
            };
            let bound_this = args.first().cloned().unwrap_or(JsValue::Undefined);
            let bound_args = if args.len() > 1 {
                args[1..].to_vec()
            } else {
                Vec::new()
            };
            let target_has_len = rt
                .get_own_property_descriptor(target_id, &PropertyKey::from("length"))?
                .is_some();
            let new_len = if target_has_len {
                let target_len = rt.get_property(target_id, "length")?;
                if let JsValue::Number(n) = target_len {
                    if n.is_infinite() {
                        if n.is_sign_positive() {
                            f64::INFINITY
                        } else {
                            0.0
                        }
                    } else {
                        let int_len = to_integer_or_infinity(n);
                        (int_len - bound_args.len() as f64).max(0.0)
                    }
                } else {
                    0.0
                }
            } else {
                0.0
            };
            let target_name = rt.get_property(target_id, "name")?;
            let orig_name = match target_name {
                JsValue::String(s) => s,
                _ => String::new(),
            };
            let bound_name = format!("bound {}", orig_name);
            let chunk = Rc::new(crate::bytecode::Chunk::new(false, false));
            let target_proto = rt.get_prototype_of(target_id)?;
            let bound_obj = rt.heap.alloc_object(JsObject::new(
                target_proto,
                ObjectKind::Function(JsFunction {
                    name: bound_name.clone(),
                    chunk,
                    params: Rc::new(Vec::new()),
                    env: rt.global_env,
                    is_async: false,
                    is_generator: false,
                    is_arrow: false,
                    is_method: false,
                    is_decl: false,
                    is_class_ctor: false,
                    is_derived_ctor: false,
                    strict: false,
                    home_object: None,
                    super_ctor: None,
                    bound_this: Some(bound_this),
                    bound_args,
                    bound_target: Some(target_id),
                    source: "function () { [native code] }".into(),
                    class_brand: None,
                    field_inits: Rc::new(Vec::new()),
                }),
            ));
            rt.heap.get_obj_mut(bound_obj).set_own_prop(
                PropertyKey::from("name"),
                PropEntry::data(JsValue::String(bound_name), false, false, true),
            );
            rt.heap.get_obj_mut(bound_obj).set_own_prop(
                PropertyKey::from("length"),
                PropEntry::data(JsValue::Number(new_len), false, false, true),
            );
            Ok(JsValue::Object(bound_obj))
        }),
    );

    set_method(
        rt,
        proto,
        "toString",
        0,
        Rc::new(|rt, this, _| {
            let JsValue::Object(oid) = this else {
                return Err(JsError::type_err(
                    "Function.prototype.toString requires that 'this' be a Function",
                ));
            };
            match &rt.heap.get_obj(oid).kind {
                ObjectKind::Function(f) => {
                    if f.source.is_empty() {
                        Ok(JsValue::String(format!(
                            "function {}() {{ [native code] }}",
                            f.name
                        )))
                    } else {
                        Ok(JsValue::String(f.source.clone()))
                    }
                }
                ObjectKind::NativeFunction(nf) => Ok(JsValue::String(format!(
                    "function {}() {{ [native code] }}",
                    nf.name
                ))),
                ObjectKind::Proxy(_) if rt.is_callable(&this) => {
                    Ok(JsValue::String("function () { [native code] }".into()))
                }
                _ => Err(JsError::type_err(
                    "Function.prototype.toString requires that 'this' be a Function",
                )),
            }
        }),
    );

    let sym_has_inst = rt.symbols.has_instance;
    let has_inst_fn = rt.create_native_function(
        "[Symbol.hasInstance]",
        1,
        Rc::new(|rt, this, args| {
            let lhs = args.first().unwrap_or(&JsValue::Undefined);
            let JsValue::Object(rhs_id) = this else {
                return Ok(JsValue::Boolean(false));
            };
            if !rt.is_callable(&this) {
                return Ok(JsValue::Boolean(false));
            }
            let target = match &rt.heap.get_obj(rhs_id).kind {
                ObjectKind::Function(f) => f.bound_target.unwrap_or(rhs_id),
                _ => rhs_id,
            };
            let JsValue::Object(lhs_id) = lhs else {
                return Ok(JsValue::Boolean(false));
            };
            let proto_val = rt.get_property(target, "prototype")?;
            let JsValue::Object(target_proto) = proto_val else {
                return Err(JsError::type_err(
                    "Function has non-object prototype in instanceof check",
                ));
            };
            let mut cur = rt.get_prototype_of(*lhs_id)?;
            while let Some(pid) = cur {
                if pid == target_proto {
                    return Ok(JsValue::Boolean(true));
                }
                cur = rt.get_prototype_of(pid)?;
            }
            Ok(JsValue::Boolean(false))
        }),
    );
    rt.heap.get_obj_mut(proto).set_own_prop(
        PropertyKey::Symbol(sym_has_inst),
        PropEntry::data(JsValue::Object(has_inst_fn), false, false, false),
    );
    let _ = set_symbol_method;

    let ctor = rt.create_spec_constructor(
        "Function",
        1,
        proto,
        Rc::new(|rt, _, args| {
            create_dynamic_function(rt, "function", rt.intrinsics.function_proto, None, args)
        }),
    );
    rt.intrinsics.function_ctor = ctor;
    set_global(rt, "Function", JsValue::Object(ctor));

    let sym_to_str_tag = rt.symbols.to_string_tag;

    let gen_fn_proto = rt.intrinsics.generator_function_proto;
    let gen_ctor = rt.create_spec_constructor(
        "GeneratorFunction",
        1,
        gen_fn_proto,
        Rc::new(|rt, _, args| {
            create_dynamic_function(
                rt,
                "function*",
                rt.intrinsics.generator_function_proto,
                Some(rt.intrinsics.generator_proto),
                args,
            )
        }),
    );
    rt.heap.get_obj_mut(gen_ctor).prototype = Some(ctor);
    rt.heap.get_obj_mut(gen_fn_proto).set_own_prop(
        PropertyKey::from("constructor"),
        PropEntry::data(JsValue::Object(gen_ctor), false, false, true),
    );
    rt.heap.get_obj_mut(gen_fn_proto).set_own_prop(
        PropertyKey::from("prototype"),
        PropEntry::data(JsValue::Object(rt.intrinsics.generator_proto), false, false, true),
    );
    rt.heap.get_obj_mut(gen_fn_proto).set_own_prop(
        PropertyKey::Symbol(sym_to_str_tag),
        PropEntry::data(JsValue::String("GeneratorFunction".into()), false, false, true),
    );
    rt.intrinsics.generator_function_ctor = gen_ctor;

    let async_fn_proto = rt.intrinsics.async_function_proto;
    let async_ctor = rt.create_spec_constructor(
        "AsyncFunction",
        1,
        async_fn_proto,
        Rc::new(|rt, _, args| {
            create_dynamic_function(
                rt,
                "async function",
                rt.intrinsics.async_function_proto,
                None,
                args,
            )
        }),
    );
    rt.heap.get_obj_mut(async_ctor).prototype = Some(ctor);
    rt.heap.get_obj_mut(async_fn_proto).set_own_prop(
        PropertyKey::from("constructor"),
        PropEntry::data(JsValue::Object(async_ctor), false, false, true),
    );
    rt.heap.get_obj_mut(async_fn_proto).set_own_prop(
        PropertyKey::Symbol(sym_to_str_tag),
        PropEntry::data(JsValue::String("AsyncFunction".into()), false, false, true),
    );
    rt.intrinsics.async_function_ctor = async_ctor;

    let async_gen_fn_proto = rt.intrinsics.async_generator_function_proto;
    let async_gen_ctor = rt.create_spec_constructor(
        "AsyncGeneratorFunction",
        1,
        async_gen_fn_proto,
        Rc::new(|rt, _, args| {
            create_dynamic_function(
                rt,
                "async function*",
                rt.intrinsics.async_generator_function_proto,
                Some(rt.intrinsics.async_generator_proto),
                args,
            )
        }),
    );
    rt.heap.get_obj_mut(async_gen_ctor).prototype = Some(ctor);
    rt.heap.get_obj_mut(async_gen_fn_proto).set_own_prop(
        PropertyKey::from("constructor"),
        PropEntry::data(JsValue::Object(async_gen_ctor), false, false, true),
    );
    rt.heap.get_obj_mut(async_gen_fn_proto).set_own_prop(
        PropertyKey::from("prototype"),
        PropEntry::data(
            JsValue::Object(rt.intrinsics.async_generator_proto),
            false,
            false,
            true,
        ),
    );
    rt.heap.get_obj_mut(async_gen_fn_proto).set_own_prop(
        PropertyKey::Symbol(sym_to_str_tag),
        PropEntry::data(
            JsValue::String("AsyncGeneratorFunction".into()),
            false,
            false,
            true,
        ),
    );
    rt.intrinsics.async_generator_function_ctor = async_gen_ctor;
}

fn create_dynamic_function(
    rt: &mut JsRuntime,
    prefix: &str,
    default_fn_proto: ObjectId,
    default_gen_proto: Option<ObjectId>,
    args: &[JsValue],
) -> Result<JsValue, JsError> {
    let (params_str, body_str) = if args.is_empty() {
        (String::new(), String::new())
    } else if args.len() == 1 {
        (String::new(), rt.to_string(&args[0])?)
    } else {
        let mut params = Vec::new();
        for a in &args[..args.len() - 1] {
            params.push(rt.to_string(a)?);
        }
        let body = rt.to_string(&args[args.len() - 1])?;
        (params.join(","), body)
    };
    let target_proto = rt.get_prototype_from_new_target(default_fn_proto)?;
    let src = format!("({} anonymous({}\n) {{\n{}\n}})", prefix, params_str, body_str);
    let program = Parser::parse_script(&src)?;
    let chunk = Compiler::compile_program(&program)?;
    let res = rt.exec_script_chunk(chunk, rt.global_env, program.strict)?;
    if let JsValue::Object(fn_id) = res {
        rt.heap.get_obj_mut(fn_id).prototype = Some(target_proto);
        if let Some(gp) = default_gen_proto {
            let fp = rt.create_object_with_proto(Some(gp));
            rt.heap.get_obj_mut(fn_id).set_own_prop(
                PropertyKey::from("prototype"),
                PropEntry::data(JsValue::Object(fp), true, false, false),
            );
        }
    }
    Ok(res)
}
