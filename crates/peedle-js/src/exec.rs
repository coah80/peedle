use crate::ast::{BinaryOp, MethodKind, Param, Pattern, UnaryOp};
use crate::bytecode::*;
use crate::compiler::Compiler;
use crate::error::{ErrorKind, JsError};
use crate::heap::*;
use crate::value::*;
use crate::vm::JsRuntime;
use std::collections::HashSet;
use std::rc::Rc;

pub enum FrameOutcome {
    Return(JsValue),
    Yield(JsValue),
    YieldDelegate(JsValue),
    Await(JsValue),
}

fn pattern_has_expr(pat: &Pattern) -> bool {
    match pat {
        Pattern::Ident(_) => false,
        Pattern::Assign { .. } => true,
        Pattern::Array { elements, rest } => {
            elements
                .iter()
                .flatten()
                .any(pattern_has_expr)
                || rest.as_deref().is_some_and(pattern_has_expr)
        }
        Pattern::Object { props, rest } => {
            props.iter().any(|p| {
                matches!(p.key, crate::ast::PropKey::Computed(_)) || pattern_has_expr(&p.value)
            }) || rest.as_deref().is_some_and(pattern_has_expr)
        }
        Pattern::Member(_) => true,
    }
}

impl JsRuntime {
    pub fn hoist_into_env(
        &mut self,
        chunk: &Chunk,
        env: EnvId,
        _strict: bool,
    ) -> Result<(), JsError> {
        let is_global = self.heap.get_env(env).object_env == Some(self.global_object);
        if is_global {
            for (name, _) in &chunk.lex_names {
                let e = self.heap.get_env(env);
                if e.lex_names.contains(name) || e.var_names.contains(name) {
                    return Err(JsError::syntax(format!(
                        "Identifier '{}' has already been declared",
                        name
                    )));
                }
                let key = PropertyKey::String(name.clone());
                if let Some(ex) = self.heap.get_obj(self.global_object).get_own_prop(&key) {
                    if !ex.configurable {
                        return Err(JsError::syntax(format!(
                            "Identifier '{}' has already been declared",
                            name
                        )));
                    }
                }
            }
            for name in &chunk.hoisted_vars {
                if self.heap.get_env(env).lex_names.contains(name) {
                    return Err(JsError::syntax(format!(
                        "Identifier '{}' has already been declared",
                        name
                    )));
                }
            }
            for (name, _) in &chunk.hoisted_funcs {
                if self.heap.get_env(env).lex_names.contains(name) {
                    return Err(JsError::syntax(format!(
                        "Identifier '{}' has already been declared",
                        name
                    )));
                }
            }
            for (name, _) in &chunk.hoisted_funcs {
                let key = PropertyKey::String(name.clone());
                let existing = self.heap.get_obj(self.global_object).get_own_prop(&key).cloned();
                if let Some(ex) = existing {
                    if !ex.configurable {
                        if matches!(ex.value, PropValue::Accessor { .. })
                            || !ex.writable
                            || !ex.enumerable
                        {
                            return Err(JsError::type_err("Cannot redefine global function"));
                        }
                    }
                } else if !self.heap.get_obj(self.global_object).extensible {
                    return Err(JsError::type_err(
                        "Cannot define global function on non-extensible object",
                    ));
                }
            }
            for name in &chunk.hoisted_vars {
                let key = PropertyKey::String(name.clone());
                if !self.has_own_property(self.global_object, &key)?
                    && !self.heap.get_obj(self.global_object).extensible
                {
                    return Err(JsError::type_err(
                        "Cannot define global var on non-extensible object",
                    ));
                }
            }
            for (name, tmpl) in &chunk.hoisted_funcs {
                let fn_id = self.create_js_function(tmpl, env);
                let key = PropertyKey::String(name.clone());
                let existing = self.heap.get_obj(self.global_object).get_own_prop(&key).cloned();
                if let Some(ex) = existing {
                    if ex.configurable {
                        self.heap.get_obj_mut(self.global_object).set_own_prop(
                            key,
                            PropEntry::data(JsValue::Object(fn_id), true, true, false),
                        );
                    } else {
                        self.set_property(self.global_object, name, JsValue::Object(fn_id))?;
                    }
                } else {
                    self.heap.get_obj_mut(self.global_object).set_own_prop(
                        key,
                        PropEntry::data(JsValue::Object(fn_id), true, true, false),
                    );
                }
                self.heap.get_env_mut(env).var_names.insert(name.clone());
            }
            for name in &chunk.hoisted_vars {
                let key = PropertyKey::String(name.clone());
                if !self.has_own_property(self.global_object, &key)? {
                    self.heap.get_obj_mut(self.global_object).set_own_prop(
                        key,
                        PropEntry::data(JsValue::Undefined, true, true, false),
                    );
                }
                self.heap.get_env_mut(env).var_names.insert(name.clone());
            }
            for (name, is_const) in &chunk.lex_names {
                let e = self.heap.get_env_mut(env);
                e.lex_names.insert(name.clone());
                e.bindings.insert(
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
            return Ok(());
        }
        for name in &chunk.hoisted_vars {
            let e = self.heap.get_env_mut(env);
            e.bindings.entry(name.clone()).or_insert(Binding {
                value: JsValue::Undefined,
                mutable: true,
                initialized: true,
                deletable: false,
                strict_immutable: false,
            });
        }
        for (name, tmpl) in &chunk.hoisted_funcs {
            let fn_id = self.create_js_function(tmpl, env);
            self.heap.get_env_mut(env).bindings.insert(
                name.clone(),
                Binding {
                    value: JsValue::Object(fn_id),
                    mutable: true,
                    initialized: true,
                    deletable: false,
                    strict_immutable: false,
                },
            );
        }
        for (name, is_const) in &chunk.lex_names {
            let e = self.heap.get_env_mut(env);
            e.lex_names.insert(name.clone());
            e.bindings.insert(
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
        Ok(())
    }

    pub fn call_function(
        &mut self,
        callee: JsValue,
        this_arg: JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, JsError> {
        self.call_internal(callee, this_arg, args, None)
    }

    pub fn construct(
        &mut self,
        ctor: JsValue,
        args: &[JsValue],
    ) -> Result<JsValue, JsError> {
        self.construct_with_new_target(ctor.clone(), args, ctor)
    }

    pub fn construct_with_new_target(
        &mut self,
        ctor: JsValue,
        args: &[JsValue],
        new_target: JsValue,
    ) -> Result<JsValue, JsError> {
        let JsValue::Object(ctor_id) = ctor else {
            return Err(JsError::type_err("Value is not a constructor"));
        };

        let proxy_info = match &self.heap.get_obj(ctor_id).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err(
                    "Cannot perform 'construct' on a revoked Proxy",
                ));
            }
            if !self.is_constructor(&JsValue::Object(target)) {
                return Err(JsError::type_err("Proxy target is not a constructor"));
            }
            let trap = self.get_property(handler, "construct")?;
            if !trap.is_nullish() {
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy construct trap is not callable"));
                }
                let arg_arr = self.create_array(args.to_vec());
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), JsValue::Object(arg_arr), new_target],
                )?;
                if !res.is_object() {
                    return Err(JsError::type_err("Proxy construct trap must return an object"));
                }
                return Ok(res);
            }
            return self.construct_with_new_target(JsValue::Object(target), args, new_target);
        }

        if let ObjectKind::Function(ref f) = self.heap.get_obj(ctor_id).kind {
            if let Some(bt) = f.bound_target {
                let mut combined = f.bound_args.clone();
                combined.extend_from_slice(args);
                let nt = if new_target == JsValue::Object(ctor_id) {
                    JsValue::Object(bt)
                } else {
                    new_target
                };
                return self.construct_with_new_target(JsValue::Object(bt), &combined, nt);
            }
        }

        if !self.is_constructor(&JsValue::Object(ctor_id)) {
            return Err(JsError::type_err("Value is not a constructor"));
        }

        let native_handles_nt = match &self.heap.get_obj(ctor_id).kind {
            ObjectKind::NativeFunction(nf) => Some(nf.handles_new_target),
            _ => None,
        };
        if let Some(handles_nt) = native_handles_nt {
            if handles_nt {
                return self.call_internal(
                    JsValue::Object(ctor_id),
                    JsValue::Undefined,
                    args,
                    Some(new_target),
                );
            }
            let default_proto = self
                .get_property(ctor_id, "prototype")?
                .as_object()
                .unwrap_or(self.intrinsics.object_proto);
            let proto = self.get_prototype_from_constructor(&new_target, default_proto)?;
            let instance = self.create_object_with_proto(Some(proto));
            let res = self.call_internal(
                JsValue::Object(ctor_id),
                JsValue::Object(instance),
                args,
                Some(new_target),
            )?;
            return if res.is_object() {
                Ok(res)
            } else {
                Ok(JsValue::Object(instance))
            };
        }

        let is_derived = match &self.heap.get_obj(ctor_id).kind {
            ObjectKind::Function(f) => f.is_derived_ctor,
            _ => false,
        };

        let this_obj = if is_derived {
            None
        } else {
            let proto = self.get_prototype_from_constructor(&new_target, self.intrinsics.object_proto)?;
            Some(self.create_object_with_proto(Some(proto)))
        };

        if let Some(inst) = this_obj {
            self.init_instance_fields(ctor_id, inst)?;
        }

        let this_val = this_obj.map(JsValue::Object).unwrap_or(JsValue::Undefined);
        let res = self.call_js_function_internal(
            ctor_id,
            this_val,
            args,
            Some(new_target),
            true,
            this_obj,
        )?;
        if res.is_object() {
            Ok(res)
        } else if let Some(inst) = this_obj {
            Ok(JsValue::Object(inst))
        } else {
            Ok(res)
        }
    }

    pub fn init_instance_fields(&mut self, ctor_id: ObjectId, inst: ObjectId) -> Result<(), JsError> {
        let (brand, field_inits, env) = match &self.heap.get_obj(ctor_id).kind {
            ObjectKind::Function(f) => (f.class_brand, f.field_inits.clone(), f.env),
            _ => return Ok(()),
        };
        if let Some(b) = brand {
            self.heap.get_obj_mut(inst).class_brands.push(b);
        }
        for fi in field_inits.iter() {
            let val = if let Some(ref chunk) = fi.init_chunk {
                let mut fenv = EnvRecord::new(Some(env), true, true);
                fenv.this_binding = Some(JsValue::Object(inst));
                let eid = self.heap.alloc_env(fenv);
                self.exec_script_chunk(chunk.clone(), eid, true)?
            } else {
                JsValue::Undefined
            };
            match &fi.key {
                FieldKeySpec::Name(name) => {
                    self.define_property(
                        inst,
                        PropertyKey::String(name.clone()),
                        PropertyDescriptor::data(val, true, true, true),
                    )?;
                }
                FieldKeySpec::Computed(key_box) => {
                    let kv = self.get_property(*key_box, "k")?;
                    let pk = self.to_property_key(&kv)?;
                    self.define_property(
                        inst,
                        pk,
                        PropertyDescriptor::data(val, true, true, true),
                    )?;
                }
                FieldKeySpec::Private(name) => {
                    let b = brand.unwrap_or(0);
                    self.heap
                        .get_obj_mut(inst)
                        .private_fields
                        .insert((b, name.clone()), PropEntry::data(val, true, false, false));
                }
            }
        }
        Ok(())
    }

    fn call_internal(
        &mut self,
        callee: JsValue,
        this_arg: JsValue,
        args: &[JsValue],
        new_target: Option<JsValue>,
    ) -> Result<JsValue, JsError> {
        if self.call_depth >= self.max_call_depth {
            return Err(JsError::range("Maximum call stack size exceeded"));
        }
        let JsValue::Object(oid) = callee else {
            return Err(JsError::type_err(format!(
                "{} is not a function",
                self.to_display_string(&callee)
            )));
        };

        let proxy_info = match &self.heap.get_obj(oid).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err("Cannot perform 'apply' on a revoked Proxy"));
            }
            if !self.is_callable(&JsValue::Object(target)) {
                return Err(JsError::type_err("Proxy target is not callable"));
            }
            let trap = self.get_property(handler, "apply")?;
            if !trap.is_nullish() {
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy apply trap is not callable"));
                }
                let arg_arr = self.create_array(args.to_vec());
                return self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), this_arg, JsValue::Object(arg_arr)],
                );
            }
            return self.call_internal(JsValue::Object(target), this_arg, args, new_target);
        }

        let native_cb = match &self.heap.get_obj(oid).kind {
            ObjectKind::NativeFunction(nf) => Some(nf.callback.clone()),
            _ => None,
        };
        if let Some(cb) = native_cb {
            let fn_realm = self
                .heap
                .object_realms
                .get(&oid)
                .copied()
                .unwrap_or(self.current_realm_id);
            let prev_realm = self.switch_realm(fn_realm);
            self.call_depth += 1;
            let prev_nt = self.native_new_target.replace(new_target.unwrap_or(JsValue::Undefined));
            let res = cb(self, this_arg, args);
            let res = match res {
                Err(e @ JsError::Native { object: None, .. }) if prev_realm != fn_realm => {
                    Err(JsError::Thrown(self.error_to_js_value(e)))
                }
                other => other,
            };
            self.native_new_target = prev_nt;
            self.call_depth -= 1;
            self.switch_realm(prev_realm);
            return res;
        }

        let bound_info = match &self.heap.get_obj(oid).kind {
            ObjectKind::Function(f) => f.bound_target.map(|bt| (bt, f.bound_this.clone(), f.bound_args.clone())),
            _ => None,
        };
        if let Some((bt, bthis, mut bargs)) = bound_info {
            bargs.extend_from_slice(args);
            let actual_this = bthis.unwrap_or(this_arg);
            return self.call_internal(JsValue::Object(bt), actual_this, &bargs, new_target);
        }

        if matches!(self.heap.get_obj(oid).kind, ObjectKind::Function(_)) {
            return self.call_js_function_internal(oid, this_arg, args, new_target, false, None);
        }

        Err(JsError::type_err("Value is not a function"))
    }

    fn call_js_function_internal(
        &mut self,
        fn_id: ObjectId,
        this_arg: JsValue,
        args: &[JsValue],
        new_target: Option<JsValue>,
        is_constructor: bool,
        this_for_ctor: Option<ObjectId>,
    ) -> Result<JsValue, JsError> {
        let fn_realm = self
            .heap
            .object_realms
            .get(&fn_id)
            .copied()
            .unwrap_or(self.current_realm_id);
        let prev_realm = self.switch_realm(fn_realm);
        let res = self.call_js_function_body(
            fn_id,
            this_arg,
            args,
            new_target,
            is_constructor,
            this_for_ctor,
            prev_realm,
        );
        let res = match res {
            Err(e @ JsError::Native { object: None, .. }) if prev_realm != fn_realm => {
                Err(JsError::Thrown(self.error_to_js_value(e)))
            }
            other => other,
        };
        self.switch_realm(prev_realm);
        res
    }

    fn call_js_function_body(
        &mut self,
        fn_id: ObjectId,
        this_arg: JsValue,
        args: &[JsValue],
        new_target: Option<JsValue>,
        is_constructor: bool,
        this_for_ctor: Option<ObjectId>,
        caller_realm: usize,
    ) -> Result<JsValue, JsError> {
        let func = match &self.heap.get_obj(fn_id).kind {
            ObjectKind::Function(f) => f.clone(),
            _ => return Err(JsError::type_err("Not a JS function")),
        };

        if func.is_class_ctor && !is_constructor {
            return Err(JsError::type_err(
                "Class constructor cannot be invoked without 'new'",
            ));
        }

        let parent_env = if !func.name.is_empty() && !func.is_arrow {
            let mut name_env = EnvRecord::new(Some(func.env), false, func.strict);
            name_env.bindings.insert(
                func.name.clone(),
                Binding {
                    value: JsValue::Object(fn_id),
                    mutable: false,
                    initialized: true,
                    deletable: false,
                    strict_immutable: false,
                },
            );
            self.heap.alloc_env(name_env)
        } else {
            func.env
        };

        let mut env_rec = EnvRecord::new(Some(parent_env), true, func.strict);
        if !func.is_arrow {
            let bound_this = if func.strict || is_constructor {
                this_arg
            } else if this_arg.is_nullish() {
                JsValue::Object(self.global_object)
            } else if !this_arg.is_object() {
                JsValue::Object(self.to_object(&this_arg)?)
            } else {
                this_arg
            };
            env_rec.this_binding = Some(bound_this);
            env_rec.new_target = Some(new_target.unwrap_or(JsValue::Undefined));
            env_rec.allow_new_target = true;
            env_rec.allow_super_call = func.is_derived_ctor;
            env_rec.allow_super_prop = func.home_object.is_some();
        }
        if func.class_brand.is_some() {
            env_rec.allow_private = true;
        }
        if let Some(home) = func.home_object {
            env_rec.super_proto = self.heap.get_obj(home).prototype;
        }
        let fn_env = self.heap.alloc_env(env_rec);

        let mut all_param_names = Vec::new();
        for p in func.params.iter() {
            crate::compiler::collect_pattern_names(&p.pattern, &mut all_param_names, false);
        }
        let has_param_exprs = func.params.iter().any(|p| pattern_has_expr(&p.pattern));
        let param_has_arguments = all_param_names.iter().any(|(n, _)| n == "arguments");

        if !func.is_arrow && !param_has_arguments {
            let is_simple_params = func
                .params
                .iter()
                .all(|p| !p.rest && matches!(p.pattern, crate::ast::Pattern::Ident(_)));
            let (mapped_env, mapped_params) = if !func.strict && is_simple_params {
                let map_len = args.len().min(func.params.len());
                let mut m = vec![None; map_len];
                let mut seen = HashSet::new();
                for i in (0..map_len).rev() {
                    if let crate::ast::Pattern::Ident(ref pname) = func.params[i].pattern {
                        if seen.insert(pname.clone()) {
                            m[i] = Some(pname.clone());
                        }
                    }
                }
                (Some(fn_env), m)
            } else {
                (None, Vec::new())
            };
            let args_obj = self.heap.alloc_object(JsObject::new(
                Some(self.intrinsics.object_proto),
                ObjectKind::Arguments(ArgumentsData {
                    values: args.to_vec(),
                    strict: func.strict,
                    mapped_env,
                    mapped_params,
                }),
            ));
            self.heap.get_obj_mut(args_obj).set_own_prop(
                PropertyKey::from("length"),
                PropEntry::data(JsValue::Number(args.len() as f64), true, false, true),
            );
            let sym_iter = PropertyKey::Symbol(self.symbols.iterator);
            let arr_vals = self.get_property_key(self.intrinsics.array_proto, &sym_iter)?;
            self.heap.get_obj_mut(args_obj).set_own_prop(
                sym_iter,
                PropEntry::data(arr_vals, true, false, true),
            );
            if !func.strict && is_simple_params {
                self.heap.get_obj_mut(args_obj).set_own_prop(
                    PropertyKey::from("callee"),
                    PropEntry::data(JsValue::Object(fn_id), true, false, true),
                );
            } else {
                let tte = self.intrinsics.throw_type_error;
                self.heap.get_obj_mut(args_obj).set_own_prop(
                    PropertyKey::from("callee"),
                    PropEntry::accessor(
                        Some(JsValue::Object(tte)),
                        Some(JsValue::Object(tte)),
                        false,
                        false,
                    ),
                );
            }
            self.heap.get_env_mut(fn_env).bindings.insert(
                "arguments".to_string(),
                Binding {
                    value: JsValue::Object(args_obj),
                    mutable: true,
                    initialized: true,
                    deletable: false,
                    strict_immutable: false,
                },
            );
        }

        if has_param_exprs {
            let e = self.heap.get_env_mut(fn_env);
            e.is_param_env = true;
            for (pn, _) in &all_param_names {
                e.lex_names.insert(pn.clone());
            }
            if !func.is_arrow {
                e.lex_names.insert("arguments".to_string());
            }
        }

        let bind_res = self.bind_params(&func.params, args, fn_env);
        if let Err(e) = bind_res {
            if func.is_async && !func.is_generator {
                let promise = self.create_promise();
                let ev = self.error_to_js_value(e);
                self.reject_promise(promise, ev);
                return Ok(JsValue::Object(promise));
            }
            return Err(e);
        }

        let body_env = if has_param_exprs {
            let mut var_env_rec = EnvRecord::new(Some(fn_env), true, func.strict);
            for vn in &func.chunk.hoisted_vars {
                let init_val = self
                    .heap
                    .get_env(fn_env)
                    .bindings
                    .get(vn)
                    .map(|b| b.value.clone())
                    .unwrap_or(JsValue::Undefined);
                var_env_rec.bindings.insert(
                    vn.clone(),
                    Binding {
                        value: init_val,
                        mutable: true,
                        initialized: true,
                        deletable: false,
                        strict_immutable: false,
                    },
                );
            }
            let vid = self.heap.alloc_env(var_env_rec);
            self.hoist_into_env(&func.chunk, vid, func.strict)?;
            vid
        } else {
            self.hoist_into_env(&func.chunk, fn_env, func.strict)?;
            fn_env
        };

        let mut frame = CallFrame::new(func.chunk.clone(), body_env, Some(fn_id));
        frame.is_constructor = is_constructor;
        frame.this_for_ctor = this_for_ctor;
        frame.is_derived_ctor = func.is_derived_ctor;
        frame.is_generator = func.is_generator;
        frame.is_async = func.is_async;

        if func.is_generator {
            let gen_proto = if func.is_async {
                self.intrinsics.async_generator_proto
            } else {
                self.get_property(fn_id, "prototype")
                    .ok()
                    .and_then(|v| v.as_object())
                    .unwrap_or(self.intrinsics.generator_proto)
            };
            let gen_obj = self.heap.alloc_object(JsObject::new(
                Some(gen_proto),
                ObjectKind::Generator(GeneratorData {
                    state: GeneratorState::SuspendedStart,
                    frame: Some(Box::new(frame)),
                    is_async: func.is_async,
                    delegating: None,
                    result_promise: None,
                }),
            ));
            return Ok(JsValue::Object(gen_obj));
        }

        if func.is_async {
            let promise = self.create_promise();
            frame.async_promise = Some(promise);
            self.run_async_frame(frame, promise);
            return Ok(JsValue::Object(promise));
        }

        self.call_depth += 1;
        self.call_stack_callees.push(fn_id);
        self.active_frames.push(fn_env);
        let res = self.run_frame(&mut frame);
        self.active_frames.pop();
        self.call_stack_callees.pop();
        self.call_depth -= 1;

        match res? {
            FrameOutcome::Return(v) => {
                if is_constructor && func.is_derived_ctor && !v.is_object() {
                    if !v.is_undefined() {
                        let err = self.create_realm_error_object(
                            caller_realm,
                            ErrorKind::TypeError,
                            "Derived constructors may only return object or undefined",
                        );
                        return Err(JsError::Thrown(JsValue::Object(err)));
                    }
                    if let Some(inst) = frame.this_for_ctor {
                        return Ok(JsValue::Object(inst));
                    }
                    let err = self.create_realm_error_object(
                        caller_realm,
                        ErrorKind::ReferenceError,
                        "Must call super constructor in derived class before returning",
                    );
                    return Err(JsError::Thrown(JsValue::Object(err)));
                }
                Ok(v)
            }
            _ => Ok(JsValue::Undefined),
        }
    }

    pub fn run_async_frame(&mut self, mut frame: CallFrame, promise: ObjectId) {
        self.active_frames.push(frame.env);
        let res = self.run_frame(&mut frame);
        self.active_frames.pop();
        match res {
            Ok(FrameOutcome::Return(v)) => {
                self.resolve_promise(promise, v);
            }
            Ok(FrameOutcome::Await(awaited)) => {
                let awaited_p = self.promise_resolve_value(awaited);
                let holder = self.heap.alloc_object(JsObject::new(
                    None,
                    ObjectKind::Generator(GeneratorData {
                        state: GeneratorState::SuspendedYield,
                        frame: Some(Box::new(frame)),
                        is_async: true,
                        delegating: None,
                        result_promise: Some(promise),
                    }),
                ));
                let on_ok = self.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, args| {
                        let val = args.first().cloned().unwrap_or(JsValue::Undefined);
                        let opt_frame = match &mut rt.heap.get_obj_mut(holder).kind {
                            ObjectKind::Generator(g) => g.frame.take(),
                            _ => None,
                        };
                        if let Some(mut fr) = opt_frame {
                            fr.stack.push(val);
                            rt.run_async_frame(*fr, promise);
                        }
                        Ok(JsValue::Undefined)
                    }),
                );
                let on_err = self.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, args| {
                        let err = args.first().cloned().unwrap_or(JsValue::Undefined);
                        let opt_frame = match &mut rt.heap.get_obj_mut(holder).kind {
                            ObjectKind::Generator(g) => g.frame.take(),
                            _ => None,
                        };
                        if let Some(mut fr) = opt_frame {
                            if let Err(e) = rt.unwind_throw(&mut fr, JsError::thrown(err)) {
                                let ev = rt.error_to_js_value(e);
                                rt.reject_promise(promise, ev);
                            } else {
                                rt.run_async_frame(*fr, promise);
                            }
                        }
                        Ok(JsValue::Undefined)
                    }),
                );
                self.promise_then(awaited_p, Some(JsValue::Object(on_ok)), Some(JsValue::Object(on_err)));
            }
            Ok(_) => {
                self.resolve_promise(promise, JsValue::Undefined);
            }
            Err(e) => {
                let ev = self.error_to_js_value(e);
                self.reject_promise(promise, ev);
            }
        }
    }

    pub fn promise_resolve_value(&mut self, val: JsValue) -> ObjectId {
        if let JsValue::Object(oid) = val {
            if matches!(self.heap.get_obj(oid).kind, ObjectKind::Promise(_)) {
                return oid;
            }
        }
        let p = self.create_promise();
        self.resolve_promise(p, val);
        p
    }

    fn pipe_async_gen_result(&mut self, res: Result<JsValue, JsError>, out_p: ObjectId) {
        match res {
            Ok(v) => self.resolve_promise(out_p, v),
            Err(e) => {
                let ev = self.error_to_js_value(e);
                self.reject_promise(out_p, ev);
            }
        }
    }

    fn resume_async_delegate(
        &mut self,
        gen_id: ObjectId,
        del_iter: JsValue,
        mode: u8,
        arg: JsValue,
    ) -> ObjectId {
        let out_p = self.create_promise();
        self.step_async_delegate(gen_id, del_iter, mode, arg, out_p);
        out_p
    }

    fn step_async_delegate(
        &mut self,
        gen_id: ObjectId,
        del_iter: JsValue,
        mode: u8,
        arg: JsValue,
        out_p: ObjectId,
    ) {
        let JsValue::Object(d_oid) = del_iter else {
            if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                g.delegating = None;
            }
            let err = self.error_to_js_value(JsError::type_err("Delegated iterator is not an object"));
            let res = self.resume_generator(gen_id, 1, err);
            self.pipe_async_gen_result(res, out_p);
            return;
        };

        let call_res: Result<Option<JsValue>, JsError> = (|| match mode {
            0 => {
                let next_fn = self.get_property(d_oid, "next")?;
                if !self.is_callable(&next_fn) {
                    return Err(JsError::type_err("Delegated iterator next is not callable"));
                }
                let r = self.call_function(next_fn, del_iter.clone(), &[arg.clone()])?;
                Ok(Some(r))
            }
            1 => {
                let throw_fn = self.get_property(d_oid, "throw")?;
                if throw_fn.is_nullish() {
                    return Ok(None);
                }
                if !self.is_callable(&throw_fn) {
                    return Err(JsError::type_err("Delegated iterator throw is not callable"));
                }
                let r = self.call_function(throw_fn, del_iter.clone(), &[arg.clone()])?;
                Ok(Some(r))
            }
            2 => {
                let ret_fn = self.get_property(d_oid, "return")?;
                if ret_fn.is_nullish() {
                    return Ok(None);
                }
                if !self.is_callable(&ret_fn) {
                    return Err(JsError::type_err("Delegated iterator return is not callable"));
                }
                let r = self.call_function(ret_fn, del_iter.clone(), &[arg.clone()])?;
                Ok(Some(r))
            }
            _ => Ok(None),
        })();

        match call_res {
            Err(e) => {
                if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                    g.delegating = None;
                }
                let ev = self.error_to_js_value(e);
                let res = self.resume_generator(gen_id, 1, ev);
                self.pipe_async_gen_result(res, out_p);
            }
            Ok(None) => {
                if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                    g.delegating = None;
                }
                if mode == 1 {
                    self.iterator_close(&del_iter);
                    let err = self.error_to_js_value(JsError::type_err(
                        "The iterator does not provide a 'throw' method",
                    ));
                    let res = self.resume_generator(gen_id, 1, err);
                    self.pipe_async_gen_result(res, out_p);
                } else {
                    let arg_p = self.promise_resolve_value(arg);
                    let on_ok = self.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                            let res = rt.resume_generator(gen_id, 2, v);
                            rt.pipe_async_gen_result(res, out_p);
                            Ok(JsValue::Undefined)
                        }),
                    );
                    let on_err = self.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                            let res = rt.resume_generator(gen_id, 1, v);
                            rt.pipe_async_gen_result(res, out_p);
                            Ok(JsValue::Undefined)
                        }),
                    );
                    self.promise_then(
                        arg_p,
                        Some(JsValue::Object(on_ok)),
                        Some(JsValue::Object(on_err)),
                    );
                }
            }
            Ok(Some(inner_val)) => {
                let inner_p = self.promise_resolve_value(inner_val);
                let on_ok = self.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, a| {
                        let resolved = a.first().cloned().unwrap_or(JsValue::Undefined);
                        let step: Result<(bool, JsValue), JsError> = (|| {
                            let JsValue::Object(r_oid) = resolved else {
                                return Err(JsError::type_err("Iterator result is not an object"));
                            };
                            let done = rt.get_property(r_oid, "done")?.to_boolean();
                            let val = rt.get_property(r_oid, "value")?;
                            Ok((done, val))
                        })();
                        match step {
                            Err(e) => {
                                if let ObjectKind::Generator(ref mut g) =
                                    rt.heap.get_obj_mut(gen_id).kind
                                {
                                    g.delegating = None;
                                }
                                let ev = rt.error_to_js_value(e);
                                let res = rt.resume_generator(gen_id, 1, ev);
                                rt.pipe_async_gen_result(res, out_p);
                            }
                            Ok((true, val)) => {
                                if let ObjectKind::Generator(ref mut g) =
                                    rt.heap.get_obj_mut(gen_id).kind
                                {
                                    g.delegating = None;
                                }
                                let resume_mode = if mode == 2 { 2 } else { 0 };
                                let res = rt.resume_generator(gen_id, resume_mode, val);
                                rt.pipe_async_gen_result(res, out_p);
                            }
                            Ok((false, val)) => {
                                let ir = rt.create_iter_result(val, false);
                                rt.resolve_promise(out_p, ir);
                            }
                        }
                        Ok(JsValue::Undefined)
                    }),
                );
                let on_err = self.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, a| {
                        let err = a.first().cloned().unwrap_or(JsValue::Undefined);
                        if let ObjectKind::Generator(ref mut g) = rt.heap.get_obj_mut(gen_id).kind {
                            g.delegating = None;
                        }
                        let res = rt.resume_generator(gen_id, 1, err);
                        rt.pipe_async_gen_result(res, out_p);
                        Ok(JsValue::Undefined)
                    }),
                );
                self.promise_then(
                    inner_p,
                    Some(JsValue::Object(on_ok)),
                    Some(JsValue::Object(on_err)),
                );
            }
        }
    }

    pub fn resume_generator(
        &mut self,
        gen_id: ObjectId,
        mode: u8,
        arg: JsValue,
    ) -> Result<JsValue, JsError> {
        let (state, is_async, delegating) = match &self.heap.get_obj(gen_id).kind {
            ObjectKind::Generator(g) => (g.state, g.is_async, g.delegating.clone()),
            _ => return Err(JsError::type_err("Receiver is not a Generator")),
        };

        if let Some(del_iter) = delegating {
            if is_async {
                let p = self.resume_async_delegate(gen_id, del_iter, mode, arg);
                return Ok(JsValue::Object(p));
            }
            if mode == 0 {
                if let JsValue::Object(d_oid) = del_iter {
                    let next_fn = self.get_property(d_oid, "next")?;
                    let res = self.call_function(next_fn, del_iter.clone(), &[arg.clone()])?;
                    if let JsValue::Object(r_oid) = res {
                        let done = self.get_property(r_oid, "done")?.to_boolean();
                        let val = self.get_property(r_oid, "value")?;
                        if !done {
                            return Ok(self.create_iter_result(val, false));
                        }
                        if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                            g.delegating = None;
                        }
                        return self.resume_generator(gen_id, 0, val);
                    }
                }
            }
        }

        if state == GeneratorState::Completed {
            if is_async {
                return match mode {
                    1 => {
                        let p = self.create_promise();
                        self.reject_promise(p, arg);
                        Ok(JsValue::Object(p))
                    }
                    2 => {
                        let r = self.create_iter_result(arg, true);
                        Ok(JsValue::Object(self.promise_resolve_value(r)))
                    }
                    _ => {
                        let r = self.create_iter_result(JsValue::Undefined, true);
                        Ok(JsValue::Object(self.promise_resolve_value(r)))
                    }
                };
            }
            return match mode {
                1 => Err(JsError::thrown(arg)),
                2 => Ok(self.create_iter_result(arg, true)),
                _ => Ok(self.create_iter_result(JsValue::Undefined, true)),
            };
        }
        if state == GeneratorState::Executing {
            let err = JsError::type_err("Generator is already running");
            if is_async {
                let p = self.create_promise();
                let ev = self.error_to_js_value(err);
                self.reject_promise(p, ev);
                return Ok(JsValue::Object(p));
            }
            return Err(err);
        }

        let mut frame = match &mut self.heap.get_obj_mut(gen_id).kind {
            ObjectKind::Generator(g) => {
                g.state = GeneratorState::Executing;
                *g.frame.take().unwrap()
            }
            _ => unreachable!(),
        };

        if state == GeneratorState::SuspendedStart {
            match mode {
                1 => {
                    if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                        g.state = GeneratorState::Completed;
                    }
                    if is_async {
                        let p = self.create_promise();
                        self.reject_promise(p, arg);
                        return Ok(JsValue::Object(p));
                    }
                    return Err(JsError::thrown(arg));
                }
                2 => {
                    if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                        g.state = GeneratorState::Completed;
                    }
                    let r = self.create_iter_result(arg, true);
                    if is_async {
                        return Ok(JsValue::Object(self.promise_resolve_value(r)));
                    }
                    return Ok(r);
                }
                _ => {}
            }
        } else {
            match mode {
                0 => frame.stack.push(arg),
                1 => {
                    if let Err(e) = self.unwind_throw(&mut frame, JsError::thrown(arg)) {
                        if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                            g.state = GeneratorState::Completed;
                        }
                        if is_async {
                            let p = self.create_promise();
                            let ev = self.error_to_js_value(e);
                            self.reject_promise(p, ev);
                            return Ok(JsValue::Object(p));
                        }
                        return Err(e);
                    }
                }
                2 => {
                    match self.unwind_return(&mut frame, arg) {
                        Ok(Some(ret_val)) => {
                            if let ObjectKind::Generator(ref mut g) =
                                self.heap.get_obj_mut(gen_id).kind
                            {
                                g.state = GeneratorState::Completed;
                            }
                            let r = self.create_iter_result(ret_val, true);
                            if is_async {
                                return Ok(JsValue::Object(self.promise_resolve_value(r)));
                            }
                            return Ok(r);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            if let ObjectKind::Generator(ref mut g) =
                                self.heap.get_obj_mut(gen_id).kind
                            {
                                g.state = GeneratorState::Completed;
                            }
                            if is_async {
                                let p = self.create_promise();
                                let ev = self.error_to_js_value(e);
                                self.reject_promise(p, ev);
                                return Ok(JsValue::Object(p));
                            }
                            return Err(e);
                        }
                    }
                }
                _ => {}
            }
        }

        self.active_frames.push(frame.env);
        let outcome = self.run_frame(&mut frame);
        self.active_frames.pop();

        match outcome {
            Ok(FrameOutcome::Return(v)) => {
                if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                    g.state = GeneratorState::Completed;
                }
                let r = self.create_iter_result(v, true);
                if is_async {
                    Ok(JsValue::Object(self.promise_resolve_value(r)))
                } else {
                    Ok(r)
                }
            }
            Ok(FrameOutcome::Yield(v)) => {
                if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                    g.state = GeneratorState::SuspendedYield;
                    g.frame = Some(Box::new(frame));
                }
                if is_async {
                    let p = self.promise_resolve_value(v);
                    let out_p = self.create_promise();
                    let on_ok = self.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            let av = a.first().cloned().unwrap_or(JsValue::Undefined);
                            let res = rt.create_iter_result(av, false);
                            rt.resolve_promise(out_p, res);
                            Ok(JsValue::Undefined)
                        }),
                    );
                    let on_err = self.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, a| {
                            let err_v = a.first().cloned().unwrap_or(JsValue::Undefined);
                            let res = rt.resume_generator(gen_id, 1, err_v);
                            rt.pipe_async_gen_result(res, out_p);
                            Ok(JsValue::Undefined)
                        }),
                    );
                    self.promise_then(
                        p,
                        Some(JsValue::Object(on_ok)),
                        Some(JsValue::Object(on_err)),
                    );
                    Ok(JsValue::Object(out_p))
                } else {
                    let r = self.create_iter_result(v, false);
                    Ok(r)
                }
            }
            Ok(FrameOutcome::YieldDelegate(iterable)) => {
                match self.get_iterator(&iterable, is_async) {
                    Ok(iter) => {
                        if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind
                        {
                            g.state = GeneratorState::SuspendedYield;
                            g.frame = Some(Box::new(frame));
                            g.delegating = Some(iter);
                        }
                        self.resume_generator(gen_id, 0, JsValue::Undefined)
                    }
                    Err(e) => {
                        if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind
                        {
                            g.state = GeneratorState::SuspendedYield;
                            g.frame = Some(Box::new(frame));
                        }
                        let ev = self.error_to_js_value(e);
                        self.resume_generator(gen_id, 1, ev)
                    }
                }
            }
            Ok(FrameOutcome::Await(awaited)) => {
                let p = self.promise_resolve_value(awaited);
                let out_p = self.create_promise();
                if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                    g.state = GeneratorState::SuspendedYield;
                    g.frame = Some(Box::new(frame));
                }
                let on_ok = self.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, a| {
                        let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                        let res = rt.resume_generator(gen_id, 0, v);
                        rt.pipe_async_gen_result(res, out_p);
                        Ok(JsValue::Undefined)
                    }),
                );
                let on_err = self.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, a| {
                        let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                        let res = rt.resume_generator(gen_id, 1, v);
                        rt.pipe_async_gen_result(res, out_p);
                        Ok(JsValue::Undefined)
                    }),
                );
                self.promise_then(p, Some(JsValue::Object(on_ok)), Some(JsValue::Object(on_err)));
                Ok(JsValue::Object(out_p))
            }
            Err(e) => {
                if let ObjectKind::Generator(ref mut g) = self.heap.get_obj_mut(gen_id).kind {
                    g.state = GeneratorState::Completed;
                }
                if is_async {
                    let p = self.create_promise();
                    let ev = self.error_to_js_value(e);
                    self.reject_promise(p, ev);
                    Ok(JsValue::Object(p))
                } else {
                    Err(e)
                }
            }
        }
    }

    fn bind_params(&mut self, params: &[Param], args: &[JsValue], env: EnvId) -> Result<(), JsError> {
        let mut param_names = Vec::new();
        for p in params {
            crate::compiler::collect_pattern_names(&p.pattern, &mut param_names, false);
        }
        let has_default = params
            .iter()
            .any(|p| p.rest || !matches!(p.pattern, Pattern::Ident(_)));
        if has_default {
            for (name, _) in &param_names {
                self.heap.get_env_mut(env).bindings.insert(
                    name.clone(),
                    Binding {
                        value: JsValue::Undefined,
                        mutable: true,
                        initialized: false,
                        deletable: false,
                        strict_immutable: false,
                    },
                );
            }
        }
        for (i, param) in params.iter().enumerate() {
            if param.rest {
                let rest_vals = if i < args.len() {
                    args[i..].to_vec()
                } else {
                    Vec::new()
                };
                let arr = self.create_array(rest_vals);
                self.bind_pattern_value(&param.pattern, JsValue::Object(arr), env)?;
                break;
            }
            let val = args.get(i).cloned().unwrap_or(JsValue::Undefined);
            self.bind_pattern_value(&param.pattern, val, env)?;
        }
        Ok(())
    }

    fn bind_pattern_value(
        &mut self,
        pat: &Pattern,
        val: JsValue,
        env: EnvId,
    ) -> Result<(), JsError> {
        match pat {
            Pattern::Ident(name) => {
                self.heap.get_env_mut(env).bindings.insert(
                    name.clone(),
                    Binding {
                        value: val,
                        mutable: true,
                        initialized: true,
                        deletable: false,
                    strict_immutable: false,
                    },
                );
            }
            Pattern::Assign { left, right } => {
                let actual = if val.is_undefined() {
                    let infer_name = match left.as_ref() {
                        Pattern::Ident(n) => Some(n.as_str()),
                        _ => None,
                    };
                    let c = Compiler::compile_expr_chunk_with_name(
                        right,
                        self.heap.get_env(env).strict,
                        infer_name,
                    )?;
                    self.exec_script_chunk(c, env, self.heap.get_env(env).strict)?
                } else {
                    val
                };
                self.bind_pattern_value(left, actual, env)?;
            }
            Pattern::Array { elements, rest } => {
                let iter = self.get_iterator(&val, false)?;
                let mut done = false;
                for el in elements {
                    let next = if done {
                        JsValue::Undefined
                    } else {
                        match self.iterator_step(&iter)? {
                            Some(v) => v,
                            None => {
                                done = true;
                                JsValue::Undefined
                            }
                        }
                    };
                    if let Some(sub) = el {
                        if let Err(e) = self.bind_pattern_value(sub, next, env) {
                            if !done {
                                self.iterator_close(&iter);
                            }
                            return Err(e);
                        }
                    }
                }
                if let Some(rest_pat) = rest {
                    let mut rest_vals = Vec::new();
                    if !done {
                        while let Some(v) = self.iterator_step(&iter)? {
                            rest_vals.push(v);
                        }
                    }
                    let arr = self.create_array(rest_vals);
                    self.bind_pattern_value(rest_pat, JsValue::Object(arr), env)?;
                } else if !done {
                    self.iterator_close(&iter);
                }
            }
            Pattern::Object { props, rest } => {
                let oid = self.to_object(&val)?;
                let mut excluded = HashSet::new();
                for p in props {
                    let key = match &p.key {
                        crate::ast::PropKey::Ident(s) | crate::ast::PropKey::String(s) => {
                            excluded.insert(s.clone());
                            PropertyKey::String(s.clone())
                        }
                        crate::ast::PropKey::Number(n) => {
                            let s = number_to_string(*n);
                            excluded.insert(s.clone());
                            PropertyKey::String(s)
                        }
                        crate::ast::PropKey::Computed(e) => {
                            let c = Compiler::compile_expr_chunk(e, self.heap.get_env(env).strict)?;
                            let kv = self.exec_script_chunk(c, env, self.heap.get_env(env).strict)?;
                            let pk = self.to_property_key(&kv)?;
                            if let PropertyKey::String(ref s) = pk {
                                excluded.insert(s.clone());
                            }
                            pk
                        }
                        crate::ast::PropKey::Private(s) => PropertyKey::String(s.clone()),
                    };
                    let pv = self.get_property_key(oid, &key)?;
                    self.bind_pattern_value(&p.value, pv, env)?;
                }
                if let Some(rest_pat) = rest {
                    let rest_obj = self.create_object();
                    let keys = self.own_keys(oid)?;
                    for k in keys {
                        if let PropertyKey::String(ref s) = k {
                            if excluded.contains(s) {
                                continue;
                            }
                        }
                        if let Some(desc) = self.get_own_property_descriptor(oid, &k)? {
                            if desc.enumerable.unwrap_or(false) {
                                let v = self.get_property_key(oid, &k)?;
                                self.define_property(
                                    rest_obj,
                                    k,
                                    PropertyDescriptor::data(v, true, true, true),
                                )?;
                            }
                        }
                    }
                    self.bind_pattern_value(rest_pat, JsValue::Object(rest_obj), env)?;
                }
            }
            Pattern::Member(_) => {}
        }
        Ok(())
    }

    pub fn run_frame(&mut self, frame: &mut CallFrame) -> Result<FrameOutcome, JsError> {
        loop {
            if let Some(limit) = self.max_steps {
                self.step_count += 1;
                if self.step_count > limit {
                    return Err(JsError::internal("Execution step limit exceeded"));
                }
            }
            if frame.ip >= frame.chunk.ops.len() {
                return Ok(FrameOutcome::Return(JsValue::Undefined));
            }
            let op = frame.chunk.ops[frame.ip].clone();
            frame.ip += 1;

            let step_res = self.exec_op(frame, op);
            match step_res {
                Ok(None) => continue,
                Ok(Some(outcome)) => return Ok(outcome),
                Err(err) => {
                    self.unwind_throw(frame, err)?;
                }
            }
        }
    }

    pub fn unwind_throw(&mut self, frame: &mut CallFrame, err: JsError) -> Result<(), JsError> {
        while let Some(handler) = frame.handlers.last_mut() {
            if !handler.in_catch && !handler.in_finally {
                if let Some(catch_ip) = handler.catch_ip {
                    handler.in_catch = true;
                    let target_stack = handler.stack_len;
                    let target_env = handler.env_depth;
                    self.restore_env_depth(frame, target_env);
                    frame.stack.truncate(target_stack);
                    let err_val = self.error_to_js_value(err);
                    frame.stack.push(err_val);
                    frame.ip = catch_ip as usize;
                    return Ok(());
                }
            }
            if !handler.in_finally {
                if let Some(fin_ip) = handler.finally_ip {
                    handler.in_finally = true;
                    handler.pending = PendingCompletion::Throw(err);
                    let target_stack = handler.stack_len;
                    let target_env = handler.env_depth;
                    self.restore_env_depth(frame, target_env);
                    frame.stack.truncate(target_stack);
                    frame.ip = fin_ip as usize;
                    return Ok(());
                }
            }
            frame.handlers.pop();
        }
        Err(err)
    }

    pub fn unwind_return(
        &mut self,
        frame: &mut CallFrame,
        val: JsValue,
    ) -> Result<Option<JsValue>, JsError> {
        while let Some(handler) = frame.handlers.last_mut() {
            if !handler.in_finally {
                if let Some(fin_ip) = handler.finally_ip {
                    handler.in_finally = true;
                    handler.pending = PendingCompletion::Return(val);
                    let target_stack = handler.stack_len;
                    let target_env = handler.env_depth;
                    self.restore_env_depth(frame, target_env);
                    frame.stack.truncate(target_stack);
                    frame.ip = fin_ip as usize;
                    return Ok(None);
                }
            }
            frame.handlers.pop();
        }
        Ok(Some(val))
    }

    fn unwind_jump(
        &mut self,
        frame: &mut CallFrame,
        target: u32,
        env_depth: u32,
        stack_depth: u32,
        handler_depth: u32,
    ) {
        while frame.handlers.len() > handler_depth as usize {
            let handler = frame.handlers.last_mut().unwrap();
            if !handler.in_finally {
                if let Some(fin_ip) = handler.finally_ip {
                    handler.in_finally = true;
                    handler.pending = PendingCompletion::Jump {
                        target,
                        env_depth,
                        stack_depth,
                        handler_depth,
                    };
                    let s_len = handler.stack_len;
                    let e_len = handler.env_depth;
                    self.restore_env_depth(frame, e_len);
                    frame.stack.truncate(s_len);
                    frame.ip = fin_ip as usize;
                    return;
                }
            }
            frame.handlers.pop();
        }
        self.restore_env_depth(frame, env_depth as usize);
        frame.stack.truncate(stack_depth as usize);
        frame.ip = target as usize;
    }

    fn restore_env_depth(&mut self, frame: &mut CallFrame, depth: usize) {
        while frame.env_stack.len() > depth {
            frame.env = frame.env_stack.pop().unwrap();
        }
    }

    fn exec_op(&mut self, frame: &mut CallFrame, op: Op) -> Result<Option<FrameOutcome>, JsError> {
        match op {
            Op::PushUndefined => frame.stack.push(JsValue::Undefined),
            Op::PushNull => frame.stack.push(JsValue::Null),
            Op::PushTrue => frame.stack.push(JsValue::Boolean(true)),
            Op::PushFalse => frame.stack.push(JsValue::Boolean(false)),
            Op::PushConst(idx) => {
                let c = frame.chunk.consts[idx as usize].clone();
                let val = match c {
                    ConstValue::Number(n) => JsValue::Number(n),
                    ConstValue::BigInt(n) => JsValue::BigInt(n),
                    ConstValue::String(s) => JsValue::String(s),
                    ConstValue::RegExp(p, f) => JsValue::Object(self.create_regexp_object(&p, &f)?),
                    _ => JsValue::Undefined,
                };
                frame.stack.push(val);
            }
            Op::Pop => {
                frame.stack.pop();
            }
            Op::Dup => {
                let v = frame.stack.last().cloned().unwrap_or(JsValue::Undefined);
                frame.stack.push(v);
            }
            Op::Dup2 => {
                let len = frame.stack.len();
                let a = frame.stack[len - 2].clone();
                let b = frame.stack[len - 1].clone();
                frame.stack.push(a);
                frame.stack.push(b);
            }
            Op::Swap => {
                let len = frame.stack.len();
                frame.stack.swap(len - 1, len - 2);
            }
            Op::Rot3 => {
                let len = frame.stack.len();
                let c = frame.stack.pop().unwrap();
                let b = frame.stack.pop().unwrap();
                let a = frame.stack.pop().unwrap();
                frame.stack.push(b);
                frame.stack.push(c);
                frame.stack.push(a);
                let _ = len;
            }
            Op::PushThis => {
                let t = self.resolve_this(frame.env);
                frame.stack.push(t);
            }
            Op::PushNewTarget => {
                let nt = self.resolve_new_target(frame.env);
                frame.stack.push(nt);
            }
            Op::PushImportMeta => {
                let meta = self.create_object();
                frame.stack.push(JsValue::Object(meta));
            }
            Op::EnterScope => {
                let strict = self.heap.get_env(frame.env).strict;
                let child = self
                    .heap
                    .alloc_env(EnvRecord::new(Some(frame.env), false, strict));
                frame.env_stack.push(frame.env);
                frame.env = child;
            }
            Op::ExitScope => {
                if let Some(prev) = frame.env_stack.pop() {
                    frame.env = prev;
                }
            }
            Op::EnterWith => {
                let val = frame.stack.pop().unwrap();
                let oid = self.to_object(&val)?;
                let strict = self.heap.get_env(frame.env).strict;
                let mut rec = EnvRecord::new(Some(frame.env), false, strict);
                rec.object_env = Some(oid);
                let child = self.heap.alloc_env(rec);
                frame.env_stack.push(frame.env);
                frame.env = child;
            }
            Op::CloneLoopEnv(cidx) => {
                let ConstValue::Names(names) = frame.chunk.consts[cidx as usize].clone() else {
                    unreachable!();
                };
                let old_env = frame.env;
                let parent = self.heap.get_env(old_env).parent;
                let strict = self.heap.get_env(old_env).strict;
                let mut new_rec = EnvRecord::new(parent, false, strict);
                for n in &names {
                    if let Some(b) = self.heap.get_env(old_env).bindings.get(n).cloned() {
                        new_rec.bindings.insert(n.clone(), b);
                    }
                }
                let new_env = self.heap.alloc_env(new_rec);
                frame.env = new_env;
            }
            Op::DeclareVar(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let target_env = self.nearest_var_env(frame.env);
                let e = self.heap.get_env_mut(target_env);
                e.bindings.entry(name).or_insert(Binding {
                    value: JsValue::Undefined,
                    mutable: true,
                    initialized: true,
                    deletable: false,
                    strict_immutable: false,
                });
            }
            Op::DeclareLet(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let e = self.heap.get_env_mut(frame.env);
                e.lex_names.insert(name.clone());
                e.bindings.insert(
                    name,
                    Binding {
                        value: JsValue::Undefined,
                        mutable: true,
                        initialized: false,
                        deletable: false,
                        strict_immutable: false,
                    },
                );
            }
            Op::DeclareConst(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let e = self.heap.get_env_mut(frame.env);
                e.lex_names.insert(name.clone());
                e.bindings.insert(
                    name,
                    Binding {
                        value: JsValue::Undefined,
                        mutable: false,
                        initialized: false,
                        deletable: false,
                        strict_immutable: true,
                    },
                );
            }
            Op::InitBinding(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = frame.stack.pop().unwrap();
                let env_mut = self.heap.get_env_mut(frame.env);
                if let Some(b) = env_mut.bindings.get_mut(&name) {
                    b.value = val;
                    b.initialized = true;
                } else {
                    env_mut.bindings.insert(
                        name,
                        Binding {
                            value: val,
                            mutable: true,
                            initialized: true,
                            deletable: false,
                    strict_immutable: false,
                        },
                    );
                }
            }
            Op::GetVar(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = self.get_var(frame.env, &name)?;
                frame.stack.push(val);
            }
            Op::GetVarTypeof(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = match self.get_var(frame.env, &name) {
                    Ok(v) => v,
                    Err(JsError::Native {
                        kind: ErrorKind::ReferenceError,
                        message,
                        ..
                    }) if !message.contains("before initialization") => JsValue::Undefined,
                    Err(e) => return Err(e),
                };
                let t = self.typeof_value(&val);
                frame.stack.push(JsValue::String(t.to_string()));
            }
            Op::SetVar(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = frame.stack.pop().unwrap();
                self.set_var(frame.env, &name, val, frame.chunk.strict)?;
            }
            Op::DeleteVar(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let res = self.delete_var(frame.env, &name)?;
                frame.stack.push(JsValue::Boolean(res));
            }
            Op::GetProp(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let obj_val = frame.stack.pop().unwrap();
                let val = self.get_value_prop(&obj_val, &PropertyKey::String(name))?;
                frame.stack.push(val);
            }
            Op::SetProp(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = frame.stack.pop().unwrap();
                let obj_val = frame.stack.pop().unwrap();
                self.set_value_prop(
                    &obj_val,
                    PropertyKey::String(name),
                    val.clone(),
                    frame.chunk.strict,
                )?;
                frame.stack.push(val);
            }
            Op::DeleteProp(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let obj_val = frame.stack.pop().unwrap();
                let oid = self.to_object(&obj_val)?;
                let ok = self.delete_property(oid, &PropertyKey::String(name))?;
                if !ok && frame.chunk.strict {
                    return Err(JsError::type_err("Cannot delete property"));
                }
                frame.stack.push(JsValue::Boolean(ok));
            }
            Op::GetElem => {
                let key_val = frame.stack.pop().unwrap();
                let obj_val = frame.stack.pop().unwrap();
                let key = self.to_property_key(&key_val)?;
                let val = self.get_value_prop(&obj_val, &key)?;
                frame.stack.push(val);
            }
            Op::SetElem => {
                let val = frame.stack.pop().unwrap();
                let key_val = frame.stack.pop().unwrap();
                let obj_val = frame.stack.pop().unwrap();
                let key = self.to_property_key(&key_val)?;
                self.set_value_prop(&obj_val, key, val.clone(), frame.chunk.strict)?;
                frame.stack.push(val);
            }
            Op::DeleteElem => {
                let key_val = frame.stack.pop().unwrap();
                let obj_val = frame.stack.pop().unwrap();
                let oid = self.to_object(&obj_val)?;
                let key = self.to_property_key(&key_val)?;
                let ok = self.delete_property(oid, &key)?;
                if !ok && frame.chunk.strict {
                    return Err(JsError::type_err("Cannot delete property"));
                }
                frame.stack.push(JsValue::Boolean(ok));
            }
            Op::GetPrivate(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let obj_val = frame.stack.pop().unwrap();
                let JsValue::Object(oid) = obj_val else {
                    return Err(JsError::type_err("Cannot read private member from non-object"));
                };
                let val = self.get_private_field(oid, &name)?;
                frame.stack.push(val);
            }
            Op::SetPrivate(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = frame.stack.pop().unwrap();
                let obj_val = frame.stack.pop().unwrap();
                let JsValue::Object(oid) = obj_val else {
                    return Err(JsError::type_err("Cannot write private member to non-object"));
                };
                self.set_private_field(oid, &name, val.clone())?;
                frame.stack.push(val);
            }
            Op::HasPrivate(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let obj_val = frame.stack.pop().unwrap();
                let JsValue::Object(oid) = obj_val else {
                    return Err(JsError::type_err(
                        "Right-hand side of private 'in' must be an object",
                    ));
                };
                let found = self
                    .heap
                    .get_obj(oid)
                    .private_fields
                    .keys()
                    .any(|(_, k)| k == &name);
                frame.stack.push(JsValue::Boolean(found));
            }
            Op::GetSuperProp(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let super_proto = self
                    .resolve_super_proto(frame.env)
                    .ok_or_else(|| JsError::type_err("Cannot read super property"))?;
                let this_val = self.resolve_this(frame.env);
                let val = self.get_property_key_receiver(
                    super_proto,
                    &PropertyKey::String(name),
                    this_val,
                )?;
                frame.stack.push(val);
            }
            Op::SetSuperProp(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = frame.stack.pop().unwrap();
                let super_proto = self
                    .resolve_super_proto(frame.env)
                    .ok_or_else(|| JsError::type_err("Cannot set super property"))?;
                let this_val = self.resolve_this(frame.env);
                self.set_property_key_receiver(
                    super_proto,
                    PropertyKey::String(name),
                    val.clone(),
                    this_val,
                )?;
                frame.stack.push(val);
            }
            Op::GetSuperElem => {
                let key_val = frame.stack.pop().unwrap();
                let key = self.to_property_key(&key_val)?;
                let super_proto = self
                    .resolve_super_proto(frame.env)
                    .ok_or_else(|| JsError::type_err("Cannot read super property"))?;
                let this_val = self.resolve_this(frame.env);
                let val = self.get_property_key_receiver(super_proto, &key, this_val)?;
                frame.stack.push(val);
            }
            Op::SetSuperElem => {
                let val = frame.stack.pop().unwrap();
                let key_val = frame.stack.pop().unwrap();
                let key = self.to_property_key(&key_val)?;
                let super_proto = self
                    .resolve_super_proto(frame.env)
                    .ok_or_else(|| JsError::type_err("Cannot set super property"))?;
                let this_val = self.resolve_this(frame.env);
                self.set_property_key_receiver(super_proto, key, val.clone(), this_val)?;
                frame.stack.push(val);
            }
            Op::CallSuper(argc) => {
                let start = frame.stack.len() - argc as usize;
                let args: Vec<JsValue> = frame.stack.drain(start..).collect();
                let res = self.exec_super_call(frame, &args)?;
                frame.stack.push(res);
            }
            Op::CallSuperSpread => {
                let arr = frame.stack.pop().unwrap();
                let args = self.extract_args_array(arr);
                let res = self.exec_super_call(frame, &args)?;
                frame.stack.push(res);
            }
            Op::CreateObject => {
                let obj = self.create_object();
                frame.stack.push(JsValue::Object(obj));
            }
            Op::CreateArray(n) => {
                let start = frame.stack.len() - n as usize;
                let vals: Vec<JsValue> = frame.stack.drain(start..).collect();
                let arr = self.create_array(vals);
                frame.stack.push(JsValue::Object(arr));
            }
            Op::ArrayPush => {
                let val = frame.stack.pop().unwrap();
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(oid).kind {
                    let idx = arr.length;
                    arr.set(idx, val);
                }
            }
            Op::ArrayPushHole => {
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(oid).kind {
                    arr.elements.push(None);
                    arr.length += 1;
                }
            }
            Op::ArraySpread => {
                let iter_val = frame.stack.pop().unwrap();
                let items = self.collect_iterable(&iter_val)?;
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(oid).kind {
                    for it in items {
                        let idx = arr.length;
                        arr.set(idx, it);
                    }
                }
            }
            Op::ObjectSetProp(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = frame.stack.pop().unwrap();
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if name == "__proto__" {
                    if let JsValue::Object(pid) = val {
                        let _ = self.set_prototype_of(oid, Some(pid));
                    } else if val.is_null() {
                        let _ = self.set_prototype_of(oid, None);
                    }
                } else {
                    if let JsValue::Object(fid) = val {
                        if let ObjectKind::Function(ref mut f) = self.heap.get_obj_mut(fid).kind {
                            f.home_object = Some(oid);
                        }
                    }
                    self.heap.get_obj_mut(oid).set_own_prop(
                        PropertyKey::String(name),
                        PropEntry::data(val, true, true, true),
                    );
                }
            }
            Op::ObjectSetOwnProp(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = frame.stack.pop().unwrap();
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let JsValue::Object(fid) = val {
                    if let ObjectKind::Function(ref mut f) = self.heap.get_obj_mut(fid).kind {
                        f.home_object = Some(oid);
                    }
                }
                self.heap.get_obj_mut(oid).set_own_prop(
                    PropertyKey::String(name),
                    PropEntry::data(val, true, true, true),
                );
            }
            Op::ObjectSetComputed => {
                let val = frame.stack.pop().unwrap();
                let key_val = frame.stack.pop().unwrap();
                let key = self.to_property_key(&key_val)?;
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let JsValue::Object(fid) = val {
                    if let ObjectKind::Function(ref mut f) = self.heap.get_obj_mut(fid).kind {
                        f.home_object = Some(oid);
                    }
                }
                self.heap
                    .get_obj_mut(oid)
                    .set_own_prop(key, PropEntry::data(val, true, true, true));
            }
            Op::ObjectSetGetter(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let getter = frame.stack.pop().unwrap();
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let JsValue::Object(fid) = getter {
                    let fobj = self.heap.get_obj_mut(fid);
                    if let ObjectKind::Function(ref mut f) = fobj.kind {
                        f.home_object = Some(oid);
                    }
                    fobj.remove_own_prop(&PropertyKey::from("prototype"));
                }
                self.define_property(
                    oid,
                    PropertyKey::String(name),
                    PropertyDescriptor {
                        value: None,
                        writable: None,
                        get: Some(getter),
                        set: None,
                        enumerable: Some(true),
                        configurable: Some(true),
                    },
                )?;
            }
            Op::ObjectSetSetter(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let setter = frame.stack.pop().unwrap();
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let JsValue::Object(fid) = setter {
                    let fobj = self.heap.get_obj_mut(fid);
                    if let ObjectKind::Function(ref mut f) = fobj.kind {
                        f.home_object = Some(oid);
                    }
                    fobj.remove_own_prop(&PropertyKey::from("prototype"));
                }
                self.define_property(
                    oid,
                    PropertyKey::String(name),
                    PropertyDescriptor {
                        value: None,
                        writable: None,
                        get: None,
                        set: Some(setter),
                        enumerable: Some(true),
                        configurable: Some(true),
                    },
                )?;
            }
            Op::ObjectSetGetterComputed => {
                let getter = frame.stack.pop().unwrap();
                let key_val = frame.stack.pop().unwrap();
                let key = self.to_property_key(&key_val)?;
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let JsValue::Object(fid) = getter {
                    let fobj = self.heap.get_obj_mut(fid);
                    if let ObjectKind::Function(ref mut f) = fobj.kind {
                        f.home_object = Some(oid);
                    }
                    fobj.remove_own_prop(&PropertyKey::from("prototype"));
                }
                self.define_property(
                    oid,
                    key,
                    PropertyDescriptor {
                        value: None,
                        writable: None,
                        get: Some(getter),
                        set: None,
                        enumerable: Some(true),
                        configurable: Some(true),
                    },
                )?;
            }
            Op::ObjectSetSetterComputed => {
                let setter = frame.stack.pop().unwrap();
                let key_val = frame.stack.pop().unwrap();
                let key = self.to_property_key(&key_val)?;
                let JsValue::Object(oid) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if let JsValue::Object(fid) = setter {
                    let fobj = self.heap.get_obj_mut(fid);
                    if let ObjectKind::Function(ref mut f) = fobj.kind {
                        f.home_object = Some(oid);
                    }
                    fobj.remove_own_prop(&PropertyKey::from("prototype"));
                }
                self.define_property(
                    oid,
                    key,
                    PropertyDescriptor {
                        value: None,
                        writable: None,
                        get: None,
                        set: Some(setter),
                        enumerable: Some(true),
                        configurable: Some(true),
                    },
                )?;
            }
            Op::ObjectSpread => {
                let src = frame.stack.pop().unwrap();
                let JsValue::Object(dst_id) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                if !src.is_nullish() {
                    let src_id = self.to_object(&src)?;
                    let keys = self.own_keys(src_id)?;
                    for k in keys {
                        if let Some(desc) = self.get_own_property_descriptor(src_id, &k)? {
                            if desc.enumerable.unwrap_or(false) {
                                let v = self.get_property_key(src_id, &k)?;
                                self.heap
                                    .get_obj_mut(dst_id)
                                    .set_own_prop(k, PropEntry::data(v, true, true, true));
                            }
                        }
                    }
                }
            }
            Op::ObjectRest(cidx) => {
                let ConstValue::Names(excluded) = frame.chunk.consts[cidx as usize].clone() else {
                    unreachable!();
                };
                let src = frame.stack.pop().unwrap();
                let src_id = self.to_object(&src)?;
                let dst = self.create_object();
                let keys = self.own_keys(src_id)?;
                for k in keys {
                    if let PropertyKey::String(ref s) = k {
                        if excluded.contains(s) {
                            continue;
                        }
                    }
                    if let Some(desc) = self.get_own_property_descriptor(src_id, &k)? {
                        if desc.enumerable.unwrap_or(false) {
                            let v = self.get_property_key(src_id, &k)?;
                            self.heap
                                .get_obj_mut(dst)
                                .set_own_prop(k, PropEntry::data(v, true, true, true));
                        }
                    }
                }
                frame.stack.push(JsValue::Object(dst));
            }
            Op::CreateFunction(idx) => {
                let ConstValue::Func(tmpl) = frame.chunk.consts[idx as usize].clone() else {
                    unreachable!();
                };
                let fn_id = self.create_js_function(&tmpl, frame.env);
                frame.stack.push(JsValue::Object(fn_id));
            }
            Op::CreateClass(idx) => {
                let ConstValue::Class(tmpl) = frame.chunk.consts[idx as usize].clone() else {
                    unreachable!();
                };
                let cls_id = self.instantiate_class(frame, &tmpl)?;
                frame.stack.push(JsValue::Object(cls_id));
            }
            Op::Unary(uop) => {
                let val = frame.stack.pop().unwrap();
                let res = self.exec_unary(uop, val)?;
                frame.stack.push(res);
            }
            Op::Binary(bop) => {
                let right = frame.stack.pop().unwrap();
                let left = frame.stack.pop().unwrap();
                let res = self.exec_binary(bop, left, right)?;
                frame.stack.push(res);
            }
            Op::ToNumber => {
                let v = frame.stack.pop().unwrap();
                if let JsValue::BigInt(n) = v {
                    frame.stack.push(JsValue::BigInt(n));
                } else {
                    let n = self.to_number(&v)?;
                    frame.stack.push(JsValue::Number(n));
                }
            }
            Op::ToString => {
                let v = frame.stack.pop().unwrap();
                let s = self.to_string(&v)?;
                frame.stack.push(JsValue::String(s));
            }
            Op::ToPropertyKey => {
                let key = frame.stack.pop().unwrap();
                if let Some(base) = frame.stack.last() {
                    if base.is_nullish() {
                        return Err(JsError::type_err(
                            "Cannot read properties of null or undefined",
                        ));
                    }
                }
                let pk = self.to_property_key(&key)?;
                frame.stack.push(pk.to_value());
            }
            Op::ToPropertyKeyOnly => {
                let key = frame.stack.pop().unwrap();
                let pk = self.to_property_key(&key)?;
                frame.stack.push(pk.to_value());
            }
            Op::RequireObjectCoercible => {
                if let Some(v) = frame.stack.last() {
                    if v.is_nullish() {
                        return Err(JsError::type_err(
                            "Cannot destructure property of null or undefined",
                        ));
                    }
                }
            }
            Op::Inc => {
                let v = frame.stack.pop().unwrap();
                match v {
                    JsValue::BigInt(n) => frame.stack.push(JsValue::BigInt(n.wrapping_add(1))),
                    JsValue::Number(n) => frame.stack.push(JsValue::Number(n + 1.0)),
                    other => {
                        let n = self.to_number(&other)?;
                        frame.stack.push(JsValue::Number(n + 1.0));
                    }
                }
            }
            Op::Dec => {
                let v = frame.stack.pop().unwrap();
                match v {
                    JsValue::BigInt(n) => frame.stack.push(JsValue::BigInt(n.wrapping_sub(1))),
                    JsValue::Number(n) => frame.stack.push(JsValue::Number(n - 1.0)),
                    other => {
                        let n = self.to_number(&other)?;
                        frame.stack.push(JsValue::Number(n - 1.0));
                    }
                }
            }
            Op::Jump(target) => frame.ip = target as usize,
            Op::JumpIfFalse(target) => {
                let v = frame.stack.pop().unwrap();
                if !v.to_boolean() {
                    frame.ip = target as usize;
                }
            }
            Op::JumpIfTrue(target) => {
                let v = frame.stack.pop().unwrap();
                if v.to_boolean() {
                    frame.ip = target as usize;
                }
            }
            Op::JumpIfNullish(target) => {
                let v = frame.stack.pop().unwrap();
                if v.is_nullish() {
                    frame.ip = target as usize;
                }
            }
            Op::JumpIfNotNullish(target) => {
                let v = frame.stack.pop().unwrap();
                if !v.is_nullish() {
                    frame.ip = target as usize;
                }
            }
            Op::JumpIfNotUndefined(target) => {
                let v = frame.stack.last().unwrap();
                if !v.is_undefined() {
                    frame.ip = target as usize;
                }
            }
            Op::Call(argc) => {
                let start = frame.stack.len() - argc as usize;
                let args: Vec<JsValue> = frame.stack.drain(start..).collect();
                let callee = frame.stack.pop().unwrap();
                let res = self.call_function(callee, JsValue::Undefined, &args)?;
                frame.stack.push(res);
            }
            Op::CallMethod(argc) => {
                let start = frame.stack.len() - argc as usize;
                let args: Vec<JsValue> = frame.stack.drain(start..).collect();
                let this_val = frame.stack.pop().unwrap();
                let callee = frame.stack.pop().unwrap();
                let res = self.call_function(callee, this_val, &args)?;
                frame.stack.push(res);
            }
            Op::CallSpread => {
                let arr = frame.stack.pop().unwrap();
                let callee = frame.stack.pop().unwrap();
                let args = self.extract_args_array(arr);
                let res = self.call_function(callee, JsValue::Undefined, &args)?;
                frame.stack.push(res);
            }
            Op::CallMethodSpread => {
                let arr = frame.stack.pop().unwrap();
                let this_val = frame.stack.pop().unwrap();
                let callee = frame.stack.pop().unwrap();
                let args = self.extract_args_array(arr);
                let res = self.call_function(callee, this_val, &args)?;
                frame.stack.push(res);
            }
            Op::CallDirectEval(argc) => {
                let start = frame.stack.len() - argc as usize;
                let args: Vec<JsValue> = frame.stack.drain(start..).collect();
                let callee = frame.stack.pop().unwrap();
                if callee == JsValue::Object(self.intrinsics.eval_fn) {
                    let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
                    let res = match arg {
                        JsValue::String(s) => self.eval_in_env(&s, frame.env, frame.chunk.strict)?,
                        other => other,
                    };
                    frame.stack.push(res);
                } else {
                    let res = self.call_function(callee, JsValue::Undefined, &args)?;
                    frame.stack.push(res);
                }
            }
            Op::CallDirectEvalSpread => {
                let arr = frame.stack.pop().unwrap();
                let callee = frame.stack.pop().unwrap();
                let args = self.extract_args_array(arr);
                if callee == JsValue::Object(self.intrinsics.eval_fn) {
                    let arg = args.first().cloned().unwrap_or(JsValue::Undefined);
                    let res = match arg {
                        JsValue::String(s) => self.eval_in_env(&s, frame.env, frame.chunk.strict)?,
                        other => other,
                    };
                    frame.stack.push(res);
                } else {
                    let res = self.call_function(callee, JsValue::Undefined, &args)?;
                    frame.stack.push(res);
                }
            }
            Op::New(argc) => {
                let start = frame.stack.len() - argc as usize;
                let args: Vec<JsValue> = frame.stack.drain(start..).collect();
                let ctor = frame.stack.pop().unwrap();
                let res = self.construct(ctor, &args)?;
                frame.stack.push(res);
            }
            Op::NewSpread => {
                let arr = frame.stack.pop().unwrap();
                let ctor = frame.stack.pop().unwrap();
                let args = self.extract_args_array(arr);
                let res = self.construct(ctor, &args)?;
                frame.stack.push(res);
            }
            Op::Return => {
                let val = frame.stack.pop().unwrap_or(JsValue::Undefined);
                if let Some(final_val) = self.unwind_return(frame, val)? {
                    return Ok(Some(FrameOutcome::Return(final_val)));
                }
            }
            Op::Throw => {
                let val = frame.stack.pop().unwrap_or(JsValue::Undefined);
                return Err(JsError::thrown(val));
            }
            Op::TryPush {
                catch_ip,
                finally_ip,
            } => {
                frame.handlers.push(TryHandler {
                    catch_ip,
                    finally_ip,
                    stack_len: frame.stack.len(),
                    env_depth: frame.env_stack.len(),
                    in_catch: false,
                    in_finally: false,
                    pending: PendingCompletion::Normal,
                    saved_last_value: None,
                });
            }
            Op::TryPop => {
                frame.handlers.pop();
            }
            Op::FinallyStart => {
                if let Some(handler) = frame.handlers.last_mut() {
                    handler.in_finally = true;
                    handler.saved_last_value = Some(frame.last_value.clone());
                    frame.last_value = JsValue::Undefined;
                }
            }
            Op::FinallyEnd => {
                if let Some(handler) = frame.handlers.pop() {
                    if let Some(saved) = handler.saved_last_value {
                        frame.last_value = saved;
                    }
                    match handler.pending {
                        PendingCompletion::Normal => {}
                        PendingCompletion::Throw(e) => return Err(e),
                        PendingCompletion::Return(v) => {
                            if let Some(fv) = self.unwind_return(frame, v)? {
                                return Ok(Some(FrameOutcome::Return(fv)));
                            }
                        }
                        PendingCompletion::Jump {
                            target,
                            env_depth,
                            stack_depth,
                            handler_depth,
                        } => {
                            self.unwind_jump(frame, target, env_depth, stack_depth, handler_depth);
                        }
                    }
                }
            }
            Op::Break {
                target,
                env_depth,
                stack_depth,
                handler_depth,
            } => {
                self.unwind_jump(frame, target, env_depth, stack_depth, handler_depth);
            }
            Op::ForInInit => {
                let val = frame.stack.pop().unwrap();
                let keys = self.collect_for_in_keys(&val)?;
                let iter_obj = self.create_builtin_iterator(BuiltinIteratorKind::Keys(keys));
                frame.stack.push(JsValue::Object(iter_obj));
            }
            Op::ForInNext(done_target) => {
                let JsValue::Object(iter_id) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                let next_key = match &mut self.heap.get_obj_mut(iter_id).kind {
                    ObjectKind::Iterator(it) => {
                        if let BuiltinIteratorKind::Keys(ref ks) = it.kind {
                            if it.index < ks.len() {
                                let k = ks[it.index].clone();
                                it.index += 1;
                                Some(k)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(k) = next_key {
                    frame.stack.push(k);
                } else {
                    frame.ip = done_target as usize;
                }
            }
            Op::GetIterator(is_async) => {
                let val = frame.stack.pop().unwrap();
                let iter = self.get_iterator(&val, is_async)?;
                frame.stack.push(iter);
            }
            Op::IteratorNext(done_target) => {
                let iter = frame.stack.last().cloned().unwrap();
                match self.iterator_step(&iter) {
                    Ok(Some(val)) => {
                        frame.stack.push(val);
                    }
                    Ok(None) => {
                        *frame.stack.last_mut().unwrap() = JsValue::Undefined;
                        frame.ip = done_target as usize;
                    }
                    Err(e) => {
                        *frame.stack.last_mut().unwrap() = JsValue::Undefined;
                        return Err(e);
                    }
                }
            }
            Op::IteratorNextValue => {
                let iter = frame.stack.last().cloned().unwrap();
                if iter.is_undefined() {
                    frame.stack.push(JsValue::Undefined);
                } else {
                    match self.iterator_step(&iter) {
                        Ok(Some(val)) => frame.stack.push(val),
                        Ok(None) => {
                            *frame.stack.last_mut().unwrap() = JsValue::Undefined;
                            frame.stack.push(JsValue::Undefined);
                        }
                        Err(e) => {
                            *frame.stack.last_mut().unwrap() = JsValue::Undefined;
                            return Err(e);
                        }
                    }
                }
            }
            Op::IteratorNextValueAt(depth) => {
                let slot = frame.stack.len() - 1 - depth as usize;
                let iter = frame.stack[slot].clone();
                if iter.is_undefined() {
                    frame.stack.push(JsValue::Undefined);
                } else {
                    match self.iterator_step(&iter) {
                        Ok(Some(val)) => frame.stack.push(val),
                        Ok(None) => {
                            frame.stack[slot] = JsValue::Undefined;
                            frame.stack.push(JsValue::Undefined);
                        }
                        Err(e) => {
                            frame.stack[slot] = JsValue::Undefined;
                            return Err(e);
                        }
                    }
                }
            }
            Op::IteratorRest => {
                let iter = frame.stack.last().cloned().unwrap();
                let mut rest = Vec::new();
                if !iter.is_undefined() {
                    loop {
                        match self.iterator_step(&iter) {
                            Ok(Some(v)) => rest.push(v),
                            Ok(None) => {
                                *frame.stack.last_mut().unwrap() = JsValue::Undefined;
                                break;
                            }
                            Err(e) => {
                                *frame.stack.last_mut().unwrap() = JsValue::Undefined;
                                return Err(e);
                            }
                        }
                    }
                }
                let arr = self.create_array(rest);
                frame.stack.push(JsValue::Object(arr));
            }
            Op::IteratorRestAt(depth) => {
                let slot = frame.stack.len() - 1 - depth as usize;
                let iter = frame.stack[slot].clone();
                let mut rest = Vec::new();
                if !iter.is_undefined() {
                    loop {
                        match self.iterator_step(&iter) {
                            Ok(Some(v)) => rest.push(v),
                            Ok(None) => {
                                frame.stack[slot] = JsValue::Undefined;
                                break;
                            }
                            Err(e) => {
                                frame.stack[slot] = JsValue::Undefined;
                                return Err(e);
                            }
                        }
                    }
                }
                let arr = self.create_array(rest);
                frame.stack.push(JsValue::Object(arr));
            }
            Op::IteratorClose => {
                if let Some(iter) = frame.stack.last().cloned() {
                    if !iter.is_undefined() {
                        *frame.stack.last_mut().unwrap() = JsValue::Undefined;
                        let suppress = frame.handlers.last().is_some_and(|h| {
                            matches!(h.pending, PendingCompletion::Throw(_))
                        });
                        self.iterator_close_checked(&iter, suppress)?;
                    }
                }
            }
            Op::Yield => {
                let val = frame.stack.pop().unwrap_or(JsValue::Undefined);
                return Ok(Some(FrameOutcome::Yield(val)));
            }
            Op::YieldDelegate => {
                let val = frame.stack.pop().unwrap_or(JsValue::Undefined);
                return Ok(Some(FrameOutcome::YieldDelegate(val)));
            }
            Op::Await => {
                let val = frame.stack.pop().unwrap_or(JsValue::Undefined);
                return Ok(Some(FrameOutcome::Await(val)));
            }
            Op::CreateTemplateObject(idx) => {
                let ConstValue::TemplateStrings {
                    site_id,
                    raw,
                    cooked,
                } = frame.chunk.consts[idx as usize].clone()
                else {
                    unreachable!();
                };
                let cache_key = (self.current_realm_id, site_id);
                if let Some(&cached) = self.template_cache.get(&cache_key) {
                    frame.stack.push(JsValue::Object(cached));
                } else {
                    let cooked_vals: Vec<JsValue> = cooked
                        .into_iter()
                        .map(|c| c.map(JsValue::String).unwrap_or(JsValue::Undefined))
                        .collect();
                    let raw_vals: Vec<JsValue> = raw.into_iter().map(JsValue::String).collect();
                    let arr = self.create_array(cooked_vals.clone());
                    let raw_arr = self.create_array(raw_vals.clone());
                    let raw_len = raw_vals.len();
                    for (i, v) in raw_vals.into_iter().enumerate() {
                        let o = self.heap.get_obj_mut(raw_arr);
                        if let ObjectKind::Array(ref mut a) = o.kind {
                            a.has_custom_index_props = true;
                        }
                        o.set_own_prop(
                            PropertyKey::String(i.to_string()),
                            PropEntry::data(v, false, true, false),
                        );
                    }
                    self.heap.get_obj_mut(raw_arr).set_own_prop(
                        PropertyKey::from("length"),
                        PropEntry::data(JsValue::Number(raw_len as f64), false, false, false),
                    );
                    self.heap.non_writable_array_lengths.insert(raw_arr);
                    self.heap.get_obj_mut(raw_arr).extensible = false;
                    let arr_len = cooked_vals.len();
                    for (i, v) in cooked_vals.into_iter().enumerate() {
                        let o = self.heap.get_obj_mut(arr);
                        if let ObjectKind::Array(ref mut a) = o.kind {
                            a.has_custom_index_props = true;
                        }
                        o.set_own_prop(
                            PropertyKey::String(i.to_string()),
                            PropEntry::data(v, false, true, false),
                        );
                    }
                    self.heap.get_obj_mut(arr).set_own_prop(
                        PropertyKey::from("length"),
                        PropEntry::data(JsValue::Number(arr_len as f64), false, false, false),
                    );
                    self.heap.non_writable_array_lengths.insert(arr);
                    self.heap.get_obj_mut(arr).set_own_prop(
                        PropertyKey::from("raw"),
                        PropEntry::data(JsValue::Object(raw_arr), false, false, false),
                    );
                    self.heap.get_obj_mut(arr).extensible = false;
                    self.template_cache.insert(cache_key, arr);
                    frame.stack.push(JsValue::Object(arr));
                }
            }
            Op::DynamicImport => {
                let spec_val = frame.stack.pop().unwrap();
                let spec = self.to_string(&spec_val)?;
                let p = self.create_promise();
                match self.load_module(&spec) {
                    Ok(ns) => self.resolve_promise(p, JsValue::Object(ns)),
                    Err(e) => {
                        let ev = self.error_to_js_value(e);
                        self.reject_promise(p, ev);
                    }
                }
                frame.stack.push(JsValue::Object(p));
            }
            Op::ExportBinding(idx) => {
                let name = get_const_str(&frame.chunk, idx).to_string();
                let val = frame.stack.pop().unwrap();
                if let Some(&ns) = self.current_module_exports.last() {
                    if name == "*" {
                        if let JsValue::Object(src_ns) = val {
                            let pairs = match &self.heap.get_obj(src_ns).kind {
                                ObjectKind::ModuleNamespace(m) => m.exports.clone(),
                                _ => Vec::new(),
                            };
                            if let ObjectKind::ModuleNamespace(ref mut m) =
                                self.heap.get_obj_mut(ns).kind
                            {
                                for (k, v) in pairs {
                                    if k != "default" {
                                        m.exports.push((k, v));
                                    }
                                }
                            }
                        }
                    } else if let ObjectKind::ModuleNamespace(ref mut m) =
                        self.heap.get_obj_mut(ns).kind
                    {
                        if let Some(existing) = m.exports.iter_mut().find(|(k, _)| k == &name) {
                            existing.1 = val;
                        } else {
                            m.exports.push((name, val));
                        }
                    }
                }
            }
            Op::ImportModule(idx) => {
                let spec = get_const_str(&frame.chunk, idx).to_string();
                let ns = self.load_module(&spec)?;
                frame.stack.push(JsValue::Object(ns));
            }
            Op::ImportBinding(imp_idx, loc_idx) => {
                let imported = get_const_str(&frame.chunk, imp_idx).to_string();
                let local = get_const_str(&frame.chunk, loc_idx).to_string();
                let JsValue::Object(ns) = frame.stack.last().copied_or_ref() else {
                    unreachable!();
                };
                let val = self.get_property(ns, &imported)?;
                self.heap.get_env_mut(frame.env).bindings.insert(
                    local,
                    Binding {
                        value: val,
                        mutable: false,
                        initialized: true,
                        deletable: false,
                    strict_immutable: false,
                    },
                );
            }
            Op::ImportNamespace(loc_idx) => {
                let local = get_const_str(&frame.chunk, loc_idx).to_string();
                let val = frame.stack.last().cloned().unwrap();
                self.heap.get_env_mut(frame.env).bindings.insert(
                    local,
                    Binding {
                        value: val,
                        mutable: false,
                        initialized: true,
                        deletable: false,
                    strict_immutable: false,
                    },
                );
            }
            Op::SetLastValue => {
                if let Some(v) = frame.stack.last().cloned() {
                    frame.last_value = v;
                }
            }
            Op::PushLastValue => {
                frame.stack.push(frame.last_value.clone());
            }
            Op::Debugger => {}
            Op::AddDisposable {
                stack_name,
                is_async,
            } => {
                let name = get_const_str(&frame.chunk, stack_name).to_string();
                let val = frame.stack.last().cloned().unwrap_or(JsValue::Undefined);
                let stack_val = self.get_var(frame.env, &name)?;
                let JsValue::Object(stack_oid) = stack_val else {
                    return Err(JsError::internal("Invalid dispose stack"));
                };
                if val.is_nullish() {
                    if is_async {
                        let entry = self.create_array(vec![
                            val,
                            JsValue::Undefined,
                            JsValue::Boolean(true),
                        ]);
                        if let ObjectKind::Array(ref mut arr) =
                            self.heap.get_obj_mut(stack_oid).kind
                        {
                            let len = arr.length;
                            arr.set(len, JsValue::Object(entry));
                        }
                    }
                } else if let JsValue::Object(oid) = val {
                    let method = if is_async {
                        let async_key = PropertyKey::Symbol(self.symbols.async_dispose);
                        let m = self.get_property_key(oid, &async_key)?;
                        if m.is_nullish() {
                            let sync_key = PropertyKey::Symbol(self.symbols.dispose);
                            let sm = self.get_property_key(oid, &sync_key)?;
                            if !self.is_callable(&sm) {
                                return Err(JsError::type_err("Object is not async disposable"));
                            }
                            sm
                        } else if !self.is_callable(&m) {
                            return Err(JsError::type_err("Symbol.asyncDispose is not callable"));
                        } else {
                            m
                        }
                    } else {
                        let sync_key = PropertyKey::Symbol(self.symbols.dispose);
                        let sm = self.get_property_key(oid, &sync_key)?;
                        if !self.is_callable(&sm) {
                            return Err(JsError::type_err("Object is not disposable"));
                        }
                        sm
                    };
                    let entry = self.create_array(vec![
                        val,
                        method,
                        JsValue::Boolean(is_async),
                    ]);
                    if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(stack_oid).kind {
                        let len = arr.length;
                        arr.set(len, JsValue::Object(entry));
                    }
                } else {
                    return Err(JsError::type_err("Disposable resource must be an object"));
                }
            }
            Op::DisposeNext {
                stack_name,
                sync_ip,
                end_ip,
            } => {
                let name = get_const_str(&frame.chunk, stack_name).to_string();
                let stack_val = self.get_var(frame.env, &name)?;
                let JsValue::Object(stack_oid) = stack_val else {
                    frame.ip = end_ip as usize;
                    return Ok(None);
                };
                let popped = match &mut self.heap.get_obj_mut(stack_oid).kind {
                    ObjectKind::Array(ref mut arr) if arr.length > 0 => {
                        let idx = arr.length - 1;
                        let item = arr.get(idx).cloned();
                        arr.truncate(idx);
                        item
                    }
                    _ => None,
                };
                let Some(JsValue::Object(entry_oid)) = popped else {
                    frame.ip = end_ip as usize;
                    return Ok(None);
                };
                let (res_val, method_val, is_async_val) = match &self.heap.get_obj(entry_oid).kind {
                    ObjectKind::Array(a) => (
                        a.get(0).cloned().unwrap_or(JsValue::Undefined),
                        a.get(1).cloned().unwrap_or(JsValue::Undefined),
                        a.get(2).cloned().unwrap_or(JsValue::Boolean(false)).to_boolean(),
                    ),
                    _ => (JsValue::Undefined, JsValue::Undefined, false),
                };
                let call_res = if self.is_callable(&method_val) {
                    self.call_function(method_val, res_val, &[])?
                } else {
                    JsValue::Undefined
                };
                if is_async_val {
                    frame.stack.push(call_res);
                } else {
                    frame.ip = sync_ip as usize;
                }
            }
        }
        Ok(None)
    }

    fn exec_super_call(
        &mut self,
        frame: &mut CallFrame,
        args: &[JsValue],
    ) -> Result<JsValue, JsError> {
        let Some(callee) = frame.callee else {
            return Err(JsError::reference("super() outside constructor"));
        };
        let super_ctor = match &self.heap.get_obj(callee).kind {
            ObjectKind::Function(f) => f.super_ctor,
            _ => None,
        }
        .ok_or_else(|| JsError::type_err("Super constructor is not valid"))?;
        let new_target = self.resolve_new_target(frame.env);
        let result =
            self.construct_with_new_target(JsValue::Object(super_ctor), args, new_target)?;
        if let JsValue::Object(inst) = result {
            frame.this_for_ctor = Some(inst);
            frame.super_called = true;
            let fn_env = self.nearest_function_env(frame.env);
            self.heap.get_env_mut(fn_env).this_binding = Some(JsValue::Object(inst));
            self.init_instance_fields(callee, inst)?;
        }
        Ok(result)
    }

    fn instantiate_class(
        &mut self,
        frame: &mut CallFrame,
        tmpl: &ClassTemplate,
    ) -> Result<ObjectId, JsError> {
        let mut computed_count = 0usize;
        for m in &tmpl.methods {
            if m.is_computed {
                computed_count += 1;
            }
        }
        for f in &tmpl.fields {
            if f.is_computed {
                computed_count += 1;
            }
        }
        let comp_start = frame.stack.len() - computed_count;
        let computed_vals: Vec<JsValue> = frame.stack.drain(comp_start..).collect();
        let mut comp_iter = computed_vals.into_iter();

        let super_class = if tmpl.has_super {
            let sv = frame.stack.pop().unwrap();
            match sv {
                JsValue::Null => None,
                JsValue::Object(id) if self.is_constructor(&JsValue::Object(id)) => Some(id),
                _ => {
                    return Err(JsError::type_err(
                        "Class extends value is not a constructor or null",
                    ));
                }
            }
        } else {
            None
        };

        let proto_parent = if let Some(sc) = super_class {
            let p = self.get_property(sc, "prototype")?;
            match p {
                JsValue::Null => None,
                JsValue::Object(pid) => Some(pid),
                _ => {
                    return Err(JsError::type_err(
                        "Class extends value does not have valid prototype property",
                    ));
                }
            }
        } else {
            Some(self.intrinsics.object_proto)
        };

        let proto = self.create_object_with_proto(proto_parent);
        let brand = self.heap.alloc_class_brand();

        let class_env = self
            .heap
            .alloc_env(EnvRecord::new(Some(frame.env), false, true));

        let mut instance_fields = Vec::new();
        let mut static_fields = Vec::new();

        let mut method_keys = Vec::with_capacity(tmpl.methods.len());
        for m in &tmpl.methods {
            let key = if m.is_computed {
                let kv = comp_iter.next().unwrap();
                self.to_property_key(&kv)?
            } else {
                PropertyKey::String(m.name.clone().unwrap_or_default())
            };
            method_keys.push(key);
        }

        for f in &tmpl.fields {
            let spec = if f.is_private {
                FieldKeySpec::Private(f.name.clone().unwrap_or_default())
            } else if f.is_computed {
                let kv = comp_iter.next().unwrap();
                let pk = self.to_property_key(&kv)?;
                let box_obj = self.create_object();
                let _ = self.set_property(box_obj, "k", pk.to_value());
                FieldKeySpec::Computed(box_obj)
            } else {
                FieldKeySpec::Name(f.name.clone().unwrap_or_default())
            };
            let fi = ClassFieldInit {
                key: spec,
                init_chunk: f.init.clone(),
                is_private: f.is_private,
            };
            if f.is_static {
                static_fields.push(fi);
            } else {
                instance_fields.push(fi);
            }
        }

        let ctor_tmpl = if let Some(ref ct) = tmpl.constructor {
            ct.clone()
        } else if super_class.is_some() {
            let mut c = Chunk::new(true, false);
            let args_idx = c.add_string("arguments".into());
            c.emit(Op::GetVar(args_idx));
            c.emit(Op::CallSuperSpread);
            c.emit(Op::Return);
            Rc::new(FuncTemplate {
                name: tmpl.name.clone(),
                chunk: Rc::new(c),
                params: Rc::new(Vec::new()),
                is_async: false,
                is_generator: false,
                is_arrow: false,
                strict: true,
                source: String::new(),
            })
        } else {
            let mut c = Chunk::new(true, false);
            c.emit(Op::PushUndefined);
            c.emit(Op::Return);
            Rc::new(FuncTemplate {
                name: tmpl.name.clone(),
                chunk: Rc::new(c),
                params: Rc::new(Vec::new()),
                is_async: false,
                is_generator: false,
                is_arrow: false,
                strict: true,
                source: String::new(),
            })
        };

        let ctor_id = self.create_js_function(&ctor_tmpl, class_env);
        let ctor_proto = super_class.unwrap_or(self.intrinsics.function_proto);
        self.heap.get_obj_mut(ctor_id).prototype = Some(ctor_proto);
        if let ObjectKind::Function(ref mut f) = self.heap.get_obj_mut(ctor_id).kind {
            f.is_class_ctor = true;
            f.is_derived_ctor = super_class.is_some();
            f.super_ctor = super_class;
            f.home_object = Some(proto);
            f.class_brand = Some(brand);
            f.field_inits = Rc::new(instance_fields);
            if let Some(ref n) = tmpl.name {
                f.name = n.clone();
            }
        }
        if let Some(ref n) = tmpl.name {
            self.heap.get_obj_mut(ctor_id).set_own_prop(
                PropertyKey::from("name"),
                PropEntry::data(JsValue::String(n.clone()), false, false, true),
            );
            self.heap.get_env_mut(class_env).bindings.insert(
                n.clone(),
                Binding {
                    value: JsValue::Object(ctor_id),
                    mutable: false,
                    initialized: true,
                    deletable: false,
                    strict_immutable: false,
                },
            );
        }
        self.heap.get_obj_mut(ctor_id).set_own_prop(
            PropertyKey::from("prototype"),
            PropEntry::data(JsValue::Object(proto), false, false, false),
        );
        self.heap.get_obj_mut(proto).set_own_prop(
            PropertyKey::from("constructor"),
            PropEntry::data(JsValue::Object(ctor_id), true, false, true),
        );

        for (m, key) in tmpl.methods.iter().zip(method_keys.into_iter()) {
            let target = if m.is_static { ctor_id } else { proto };
            let fn_id = self.create_js_function(&m.func, class_env);
            if let ObjectKind::Function(ref mut f) = self.heap.get_obj_mut(fn_id).kind {
                f.home_object = Some(target);
                f.class_brand = Some(brand);
            }
            if !m.func.is_generator {
                self.heap
                    .get_obj_mut(fn_id)
                    .remove_own_prop(&PropertyKey::from("prototype"));
            }
            if m.is_private {
                let pname = m.name.clone().unwrap_or_default();
                let pf = &mut self.heap.get_obj_mut(target).private_fields;
                match m.kind {
                    MethodKind::Get => {
                        if let Some(existing) = pf.get_mut(&(brand, pname.clone())) {
                            if let PropValue::Accessor { ref mut get, .. } = existing.value {
                                *get = Some(JsValue::Object(fn_id));
                            } else {
                                *existing = PropEntry::accessor(
                                    Some(JsValue::Object(fn_id)),
                                    None,
                                    false,
                                    false,
                                );
                            }
                        } else {
                            pf.insert(
                                (brand, pname),
                                PropEntry::accessor(Some(JsValue::Object(fn_id)), None, false, false),
                            );
                        }
                    }
                    MethodKind::Set => {
                        if let Some(existing) = pf.get_mut(&(brand, pname.clone())) {
                            if let PropValue::Accessor { ref mut set, .. } = existing.value {
                                *set = Some(JsValue::Object(fn_id));
                            } else {
                                *existing = PropEntry::accessor(
                                    None,
                                    Some(JsValue::Object(fn_id)),
                                    false,
                                    false,
                                );
                            }
                        } else {
                            pf.insert(
                                (brand, pname),
                                PropEntry::accessor(None, Some(JsValue::Object(fn_id)), false, false),
                            );
                        }
                    }
                    _ => {
                        pf.insert(
                            (brand, pname),
                            PropEntry::data(JsValue::Object(fn_id), false, false, false),
                        );
                    }
                }
            } else {
                let ok = match m.kind {
                    MethodKind::Get => self.define_property(
                        target,
                        key,
                        PropertyDescriptor {
                            value: None,
                            writable: None,
                            get: Some(JsValue::Object(fn_id)),
                            set: None,
                            enumerable: Some(false),
                            configurable: Some(true),
                        },
                    )?,
                    MethodKind::Set => self.define_property(
                        target,
                        key,
                        PropertyDescriptor {
                            value: None,
                            writable: None,
                            get: None,
                            set: Some(JsValue::Object(fn_id)),
                            enumerable: Some(false),
                            configurable: Some(true),
                        },
                    )?,
                    _ => self.define_property(
                        target,
                        key,
                        PropertyDescriptor::data(JsValue::Object(fn_id), true, false, true),
                    )?,
                };
                if !ok {
                    return Err(JsError::type_err("Cannot define class method"));
                }
            }
        }

        for sf in static_fields {
            let val = if let Some(ref chunk) = sf.init_chunk {
                let mut fenv = EnvRecord::new(Some(class_env), true, true);
                fenv.this_binding = Some(JsValue::Object(ctor_id));
                fenv.allow_super_prop = true;
                fenv.allow_new_target = true;
                fenv.allow_private = true;
                let eid = self.heap.alloc_env(fenv);
                self.exec_script_chunk(chunk.clone(), eid, true)?
            } else {
                JsValue::Undefined
            };
            match sf.key {
                FieldKeySpec::Name(name) => {
                    self.define_property(
                        ctor_id,
                        PropertyKey::String(name),
                        PropertyDescriptor::data(val, true, true, true),
                    )?;
                }
                FieldKeySpec::Computed(key_box) => {
                    let kv = self.get_property(key_box, "k")?;
                    let pk = self.to_property_key(&kv)?;
                    self.define_property(
                        ctor_id,
                        pk,
                        PropertyDescriptor::data(val, true, true, true),
                    )?;
                }
                FieldKeySpec::Private(name) => {
                    self.heap
                        .get_obj_mut(ctor_id)
                        .private_fields
                        .insert((brand, name), PropEntry::data(val, true, false, false));
                }
            }
        }

        for sb in &tmpl.static_blocks {
            let mut senv = EnvRecord::new(Some(class_env), true, true);
            senv.this_binding = Some(JsValue::Object(ctor_id));
            senv.allow_super_prop = true;
            senv.allow_new_target = true;
            senv.allow_private = true;
            let eid = self.heap.alloc_env(senv);
            self.exec_script_chunk(sb.clone(), eid, true)?;
        }

        Ok(ctor_id)
    }

    fn get_private_field(&mut self, oid: ObjectId, name: &str) -> Result<JsValue, JsError> {
        let mut cur = Some(oid);
        while let Some(id) = cur {
            let found = self
                .heap
                .get_obj(id)
                .private_fields
                .iter()
                .find(|((_, k), _)| k == name)
                .map(|(_, e)| e.clone());
            if let Some(entry) = found {
                return match entry.value {
                    PropValue::Data(v) => Ok(v),
                    PropValue::Accessor { get, .. } => {
                        if let Some(g) = get {
                            self.call_function(g, JsValue::Object(oid), &[])
                        } else {
                            Err(JsError::type_err("Private accessor has no getter"))
                        }
                    }
                };
            }
            cur = self.heap.get_obj(id).prototype;
        }
        Err(JsError::type_err(format!(
            "Cannot read private member #{} from object",
            name
        )))
    }

    fn set_private_field(
        &mut self,
        oid: ObjectId,
        name: &str,
        val: JsValue,
    ) -> Result<(), JsError> {
        let key_found = self
            .heap
            .get_obj(oid)
            .private_fields
            .keys()
            .find(|(_, k)| k == name)
            .cloned();
        if let Some(k) = key_found {
            let entry = self.heap.get_obj(oid).private_fields.get(&k).unwrap().clone();
            match entry.value {
                PropValue::Data(_) => {
                    if !entry.writable {
                        return Err(JsError::type_err("Private method is not writable"));
                    }
                    if let Some(e) = self.heap.get_obj_mut(oid).private_fields.get_mut(&k) {
                        e.value = PropValue::Data(val);
                    }
                    return Ok(());
                }
                PropValue::Accessor { set, .. } => {
                    if let Some(s) = set {
                        self.call_function(s, JsValue::Object(oid), &[val])?;
                        return Ok(());
                    }
                    return Err(JsError::type_err("Private accessor has no setter"));
                }
            }
        }
        let mut cur = self.heap.get_obj(oid).prototype;
        while let Some(id) = cur {
            let found = self
                .heap
                .get_obj(id)
                .private_fields
                .iter()
                .find(|((_, k), _)| k == name)
                .map(|(_, e)| e.clone());
            if let Some(entry) = found {
                if let PropValue::Accessor { set: Some(s), .. } = entry.value {
                    self.call_function(s, JsValue::Object(oid), &[val])?;
                    return Ok(());
                }
            }
            cur = self.heap.get_obj(id).prototype;
        }
        Err(JsError::type_err(format!(
            "Cannot write private member #{} to object",
            name
        )))
    }

    fn extract_args_array(&self, val: JsValue) -> Vec<JsValue> {
        if let JsValue::Object(oid) = val {
            match &self.heap.get_obj(oid).kind {
                ObjectKind::Array(arr) => {
                    return (0..arr.length)
                        .map(|i| arr.get(i).cloned().unwrap_or(JsValue::Undefined))
                        .collect();
                }
                ObjectKind::Arguments(args) => {
                    return args.values.clone();
                }
                _ => {}
            }
        }
        Vec::new()
    }

    fn collect_for_in_keys(&mut self, val: &JsValue) -> Result<Vec<JsValue>, JsError> {
        if val.is_nullish() {
            return Ok(Vec::new());
        }
        let oid = self.to_object(val)?;
        let mut keys = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = Some(oid);
        while let Some(id) = cur {
            let own = self.own_keys(id)?;
            for k in own {
                let PropertyKey::String(s) = k else {
                    continue;
                };
                if !seen.insert(s.clone()) {
                    continue;
                }
                if let Some(desc) =
                    self.get_own_property_descriptor(id, &PropertyKey::String(s.clone()))?
                {
                    if desc.enumerable.unwrap_or(false) {
                        keys.push(JsValue::String(s));
                    }
                }
            }
            cur = self.get_prototype_of(id)?;
        }
        Ok(keys)
    }

    fn get_value_prop(&mut self, val: &JsValue, key: &PropertyKey) -> Result<JsValue, JsError> {
        match val {
            JsValue::Undefined => Err(JsError::type_err(format!(
                "Cannot read properties of undefined (reading '{}')",
                key
            ))),
            JsValue::Null => Err(JsError::type_err(format!(
                "Cannot read properties of null (reading '{}')",
                key
            ))),
            JsValue::Object(id) => self.get_property_key(*id, key),
            JsValue::String(s) => {
                if let PropertyKey::String(ref k) = key {
                    if k == "length" {
                        return Ok(JsValue::Number(s.encode_utf16().count() as f64));
                    }
                    if let Some(idx) = parse_array_index(k) {
                        if let Some(u) = s.encode_utf16().nth(idx as usize) {
                            return Ok(JsValue::String(String::from_utf16_lossy(&[u])));
                        }
                        return Ok(JsValue::Undefined);
                    }
                }
                self.get_property_key_receiver(self.intrinsics.string_proto, key, val.clone())
            }
            JsValue::Number(_) => {
                self.get_property_key_receiver(self.intrinsics.number_proto, key, val.clone())
            }
            JsValue::Boolean(_) => {
                self.get_property_key_receiver(self.intrinsics.boolean_proto, key, val.clone())
            }
            JsValue::Symbol(_) => {
                self.get_property_key_receiver(self.intrinsics.symbol_proto, key, val.clone())
            }
            JsValue::BigInt(_) => {
                self.get_property_key_receiver(self.intrinsics.bigint_proto, key, val.clone())
            }
        }
    }

    fn set_value_prop(
        &mut self,
        val: &JsValue,
        key: PropertyKey,
        new_val: JsValue,
        strict: bool,
    ) -> Result<(), JsError> {
        match val {
            JsValue::Undefined | JsValue::Null => Err(JsError::type_err(format!(
                "Cannot set properties of {} (setting '{}')",
                if val.is_null() { "null" } else { "undefined" },
                key
            ))),
            JsValue::Object(id) => {
                self.set_property_key_strict(*id, key, new_val, strict)?;
                Ok(())
            }
            _ => {
                if strict {
                    Err(JsError::type_err(format!(
                        "Cannot create property '{}' on primitive",
                        key
                    )))
                } else {
                    Ok(())
                }
            }
        }
    }

    fn resolve_this(&self, mut env: EnvId) -> JsValue {
        loop {
            let rec = self.heap.get_env(env);
            if let Some(ref t) = rec.this_binding {
                return t.clone();
            }
            let Some(p) = rec.parent else {
                return JsValue::Object(self.global_object);
            };
            env = p;
        }
    }

    fn resolve_new_target(&self, mut env: EnvId) -> JsValue {
        loop {
            let rec = self.heap.get_env(env);
            if let Some(ref nt) = rec.new_target {
                return nt.clone();
            }
            let Some(p) = rec.parent else {
                return JsValue::Undefined;
            };
            env = p;
        }
    }

    fn resolve_super_proto(&self, mut env: EnvId) -> Option<ObjectId> {
        loop {
            let rec = self.heap.get_env(env);
            if let Some(sp) = rec.super_proto {
                return Some(sp);
            }
            env = rec.parent?;
        }
    }

    fn nearest_var_env(&self, mut env: EnvId) -> EnvId {
        loop {
            let rec = self.heap.get_env(env);
            if rec.is_function_scope || rec.parent.is_none() {
                return env;
            }
            env = rec.parent.unwrap();
        }
    }

    fn nearest_function_env(&self, mut env: EnvId) -> EnvId {
        loop {
            let rec = self.heap.get_env(env);
            if rec.is_function_scope || rec.parent.is_none() {
                return env;
            }
            env = rec.parent.unwrap();
        }
    }

    pub fn get_var(&mut self, mut env: EnvId, name: &str) -> Result<JsValue, JsError> {
        loop {
            let (binding, obj_env, parent) = {
                let rec = self.heap.get_env(env);
                (rec.bindings.get(name).cloned(), rec.object_env, rec.parent)
            };
            if let Some(b) = binding {
                if !b.initialized {
                    return Err(JsError::reference(format!(
                        "Cannot access '{}' before initialization",
                        name
                    )));
                }
                return Ok(b.value);
            }
            if let Some(oid) = obj_env {
                let key = PropertyKey::String(name.to_string());
                if self.has_property(oid, &key)? {
                    if parent.is_some() {
                        let unscop = PropertyKey::Symbol(self.symbols.unscopables);
                        let u = self.get_property_key(oid, &unscop)?;
                        if let JsValue::Object(uid) = u {
                            if self.get_property(uid, name)?.to_boolean() {
                                if let Some(p) = parent {
                                    env = p;
                                    continue;
                                }
                            }
                        }
                    }
                    return self.get_property(oid, name);
                }
            }
            let Some(p) = parent else {
                return Err(JsError::reference(format!("{} is not defined", name)));
            };
            env = p;
        }
    }

    pub fn set_var(
        &mut self,
        mut env: EnvId,
        name: &str,
        val: JsValue,
        strict: bool,
    ) -> Result<(), JsError> {
        loop {
            let (has_binding, obj_env, parent) = {
                let rec = self.heap.get_env(env);
                (rec.bindings.contains_key(name), rec.object_env, rec.parent)
            };
            if has_binding {
                let b = self.heap.get_env_mut(env).bindings.get_mut(name).unwrap();
                if !b.initialized {
                    return Err(JsError::reference(format!(
                        "Cannot access '{}' before initialization",
                        name
                    )));
                }
                if !b.mutable {
                    if strict || b.strict_immutable {
                        return Err(JsError::type_err(format!(
                            "Assignment to constant variable '{}'",
                            name
                        )));
                    }
                    return Ok(());
                }
                b.value = val;
                return Ok(());
            }
            if let Some(oid) = obj_env {
                let key = PropertyKey::String(name.to_string());
                if self.has_property(oid, &key)? {
                    if parent.is_some() {
                        let unscop = PropertyKey::Symbol(self.symbols.unscopables);
                        let u = self.get_property_key(oid, &unscop)?;
                        if let JsValue::Object(uid) = u {
                            if self.get_property(uid, name)?.to_boolean() {
                                if let Some(p) = parent {
                                    env = p;
                                    continue;
                                }
                            }
                        }
                    }
                    self.set_property_key_strict(oid, key, val, strict)?;
                    return Ok(());
                }
            }
            if let Some(p) = parent {
                env = p;
            } else {
                if strict {
                    return Err(JsError::reference(format!("{} is not defined", name)));
                }
                self.set_property(self.global_object, name, val)?;
                return Ok(());
            }
        }
    }

    fn delete_var(&mut self, mut env: EnvId, name: &str) -> Result<bool, JsError> {
        loop {
            let (binding_del, obj_env, parent) = {
                let rec = self.heap.get_env(env);
                (
                    rec.bindings.get(name).map(|b| b.deletable),
                    rec.object_env,
                    rec.parent,
                )
            };
            if let Some(del) = binding_del {
                if del {
                    self.heap.get_env_mut(env).bindings.remove(name);
                    return Ok(true);
                }
                return Ok(false);
            }
            if let Some(oid) = obj_env {
                let key = PropertyKey::String(name.to_string());
                if self.has_property(oid, &key)? {
                    return self.delete_property(oid, &key);
                }
            }
            let Some(p) = parent else {
                return Ok(true);
            };
            env = p;
        }
    }

    fn exec_unary(&mut self, op: UnaryOp, val: JsValue) -> Result<JsValue, JsError> {
        match op {
            UnaryOp::Plus => Ok(JsValue::Number(self.to_number(&val)?)),
            UnaryOp::Minus => match val {
                JsValue::BigInt(n) => Ok(JsValue::BigInt(n.wrapping_neg())),
                other => Ok(JsValue::Number(-self.to_number(&other)?)),
            },
            UnaryOp::Not => Ok(JsValue::Boolean(!val.to_boolean())),
            UnaryOp::BitNot => match val {
                JsValue::BigInt(n) => Ok(JsValue::BigInt(!n)),
                other => Ok(JsValue::Number((!to_int32(self.to_number(&other)?)) as f64)),
            },
            UnaryOp::TypeOf => Ok(JsValue::String(self.typeof_value(&val).to_string())),
            UnaryOp::Void => Ok(JsValue::Undefined),
            UnaryOp::Delete => Ok(JsValue::Boolean(true)),
        }
    }

    fn exec_binary(
        &mut self,
        op: BinaryOp,
        left: JsValue,
        right: JsValue,
    ) -> Result<JsValue, JsError> {
        match op {
            BinaryOp::Add => {
                let l = self.to_primitive(&left, "default")?;
                let r = self.to_primitive(&right, "default")?;
                if matches!(l, JsValue::String(_)) || matches!(r, JsValue::String(_)) {
                    let ls = self.to_string(&l)?;
                    let rs = self.to_string(&r)?;
                    return Ok(JsValue::String(format!("{}{}", ls, rs)));
                }
                if let (JsValue::BigInt(a), JsValue::BigInt(b)) = (&l, &r) {
                    return Ok(JsValue::BigInt(a.wrapping_add(*b)));
                }
                if matches!(l, JsValue::BigInt(_)) || matches!(r, JsValue::BigInt(_)) {
                    return Err(JsError::type_err(
                        "Cannot mix BigInt and other types, use explicit conversions",
                    ));
                }
                let a = self.to_number(&l)?;
                let b = self.to_number(&r)?;
                Ok(JsValue::Number(a + b))
            }
            BinaryOp::Sub => self.numeric_binop(left, right, |a, b| a - b, |a, b| Ok(a.wrapping_sub(b))),
            BinaryOp::Mul => self.numeric_binop(left, right, |a, b| a * b, |a, b| Ok(a.wrapping_mul(b))),
            BinaryOp::Div => self.numeric_binop(
                left,
                right,
                |a, b| a / b,
                |a, b| {
                    if b == 0 {
                        Err(JsError::range("Division by zero"))
                    } else {
                        Ok(a.wrapping_div(b))
                    }
                },
            ),
            BinaryOp::Rem => self.numeric_binop(
                left,
                right,
                |a, b| a % b,
                |a, b| {
                    if b == 0 {
                        Err(JsError::range("Division by zero"))
                    } else {
                        Ok(a.wrapping_rem(b))
                    }
                },
            ),
            BinaryOp::Exp => self.numeric_binop(
                left,
                right,
                |a, b| {
                    if b.is_nan() {
                        f64::NAN
                    } else if b == 0.0 {
                        1.0
                    } else if a.abs() == 1.0 && b.is_infinite() {
                        f64::NAN
                    } else {
                        a.powf(b)
                    }
                },
                |a, b| {
                    if b < 0 {
                        Err(JsError::range("Exponent must be positive"))
                    } else {
                        Ok(a.wrapping_pow(b as u32))
                    }
                },
            ),
            BinaryOp::Shl => self.bitwise_binop(
                left,
                right,
                |a, b| (a.wrapping_shl((b as u32) & 0x1f)) as f64,
                |a, b| a.wrapping_shl(b as u32),
            ),
            BinaryOp::Shr => self.bitwise_binop(
                left,
                right,
                |a, b| (a.wrapping_shr((b as u32) & 0x1f)) as f64,
                |a, b| a.wrapping_shr(b as u32),
            ),
            BinaryOp::UShr => {
                if matches!(left, JsValue::BigInt(_)) || matches!(right, JsValue::BigInt(_)) {
                    return Err(JsError::type_err("BigInts have no unsigned right shift"));
                }
                let a = to_uint32(self.to_number(&left)?);
                let b = to_uint32(self.to_number(&right)?) & 0x1f;
                Ok(JsValue::Number((a >> b) as f64))
            }
            BinaryOp::BitAnd => {
                self.bitwise_binop(left, right, |a, b| (a & b) as f64, |a, b| a & b)
            }
            BinaryOp::BitOr => {
                self.bitwise_binop(left, right, |a, b| (a | b) as f64, |a, b| a | b)
            }
            BinaryOp::BitXor => {
                self.bitwise_binop(left, right, |a, b| (a ^ b) as f64, |a, b| a ^ b)
            }
            BinaryOp::Eq => Ok(JsValue::Boolean(self.abstract_eq(&left, &right)?)),
            BinaryOp::Neq => Ok(JsValue::Boolean(!self.abstract_eq(&left, &right)?)),
            BinaryOp::StrictEq => Ok(JsValue::Boolean(left.strict_eq(&right))),
            BinaryOp::StrictNeq => Ok(JsValue::Boolean(!left.strict_eq(&right))),
            BinaryOp::Lt => Ok(JsValue::Boolean(
                self.relational_lt(&left, &right)?.unwrap_or(false),
            )),
            BinaryOp::Gt => Ok(JsValue::Boolean(
                self.relational_lt(&right, &left)?.unwrap_or(false),
            )),
            BinaryOp::Lte => Ok(JsValue::Boolean(
                self.relational_lt(&right, &left)?.map(|b| !b).unwrap_or(false),
            )),
            BinaryOp::Gte => Ok(JsValue::Boolean(
                self.relational_lt(&left, &right)?.map(|b| !b).unwrap_or(false),
            )),
            BinaryOp::In => {
                let JsValue::Object(oid) = right else {
                    return Err(JsError::type_err(
                        "Cannot use 'in' operator to search in non-object",
                    ));
                };
                let key = self.to_property_key(&left)?;
                Ok(JsValue::Boolean(self.has_property(oid, &key)?))
            }
            BinaryOp::InstanceOf => Ok(JsValue::Boolean(self.instanceof(&left, &right)?)),
        }
    }

    fn numeric_binop<F, B>(
        &mut self,
        left: JsValue,
        right: JsValue,
        fop: F,
        bop: B,
    ) -> Result<JsValue, JsError>
    where
        F: Fn(f64, f64) -> f64,
        B: Fn(i128, i128) -> Result<i128, JsError>,
    {
        let lp = self.to_primitive(&left, "number")?;
        let rp = self.to_primitive(&right, "number")?;
        if let (JsValue::BigInt(a), JsValue::BigInt(b)) = (&lp, &rp) {
            return Ok(JsValue::BigInt(bop(*a, *b)?));
        }
        if matches!(lp, JsValue::BigInt(_)) || matches!(rp, JsValue::BigInt(_)) {
            return Err(JsError::type_err("Cannot mix BigInt and other types"));
        }
        let a = self.to_number(&lp)?;
        let b = self.to_number(&rp)?;
        Ok(JsValue::Number(fop(a, b)))
    }

    fn bitwise_binop<F, B>(
        &mut self,
        left: JsValue,
        right: JsValue,
        iop: F,
        bop: B,
    ) -> Result<JsValue, JsError>
    where
        F: Fn(i32, i32) -> f64,
        B: Fn(i128, i128) -> i128,
    {
        let lp = self.to_primitive(&left, "number")?;
        let rp = self.to_primitive(&right, "number")?;
        if let (JsValue::BigInt(a), JsValue::BigInt(b)) = (&lp, &rp) {
            return Ok(JsValue::BigInt(bop(*a, *b)));
        }
        if matches!(lp, JsValue::BigInt(_)) || matches!(rp, JsValue::BigInt(_)) {
            return Err(JsError::type_err("Cannot mix BigInt and other types"));
        }
        let a = to_int32(self.to_number(&lp)?);
        let b = to_int32(self.to_number(&rp)?);
        Ok(JsValue::Number(iop(a, b)))
    }
}

fn get_const_str(chunk: &Chunk, idx: u32) -> &str {
    match &chunk.consts[idx as usize] {
        ConstValue::String(s) => s.as_str(),
        _ => "",
    }
}

trait OptJsValExt {
    fn copied_or_ref(self) -> JsValue;
}

impl OptJsValExt for Option<&JsValue> {
    fn copied_or_ref(self) -> JsValue {
        self.cloned().unwrap_or(JsValue::Undefined)
    }
}
