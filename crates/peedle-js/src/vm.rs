use crate::bytecode::{CallFrame, Chunk, FuncTemplate};
use crate::compiler::Compiler;
use crate::error::{ErrorKind, JsError};
use crate::heap::*;
use crate::parser::Parser;
use crate::regexp::CompiledRegExp;
use crate::value::*;
use std::collections::HashMap;
use std::rc::Rc;

pub type ModuleLoader = Box<dyn Fn(&str) -> Result<String, JsError>>;

#[derive(Clone, Copy)]
pub struct WellKnownSymbols {
    pub iterator: u64,
    pub async_iterator: u64,
    pub to_string_tag: u64,
    pub to_primitive: u64,
    pub has_instance: u64,
    pub is_concat_spreadable: u64,
    pub species: u64,
    pub match_sym: u64,
    pub match_all: u64,
    pub replace: u64,
    pub search: u64,
    pub split: u64,
    pub unscopables: u64,
}

#[derive(Clone)]
pub struct Intrinsics {
    pub object_proto: ObjectId,
    pub function_proto: ObjectId,
    pub array_proto: ObjectId,
    pub string_proto: ObjectId,
    pub number_proto: ObjectId,
    pub boolean_proto: ObjectId,
    pub symbol_proto: ObjectId,
    pub bigint_proto: ObjectId,
    pub regexp_proto: ObjectId,
    pub date_proto: ObjectId,
    pub error_proto: ObjectId,
    pub type_error_proto: ObjectId,
    pub reference_error_proto: ObjectId,
    pub syntax_error_proto: ObjectId,
    pub range_error_proto: ObjectId,
    pub uri_error_proto: ObjectId,
    pub eval_error_proto: ObjectId,
    pub aggregate_error_proto: ObjectId,
    pub map_proto: ObjectId,
    pub set_proto: ObjectId,
    pub weak_map_proto: ObjectId,
    pub weak_set_proto: ObjectId,
    pub promise_proto: ObjectId,
    pub array_buffer_proto: ObjectId,
    pub data_view_proto: ObjectId,
    pub typed_array_proto: ObjectId,
    pub typed_array_protos: [ObjectId; 11],
    pub iterator_proto: ObjectId,
    pub generator_proto: ObjectId,
    pub async_generator_proto: ObjectId,
    pub eval_fn: ObjectId,
    pub object_ctor: ObjectId,
    pub array_ctor: ObjectId,
    pub promise_ctor: ObjectId,
    pub regexp_ctor: ObjectId,
}

pub enum Microtask {
    Callback {
        func: JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    },
    Native(Box<dyn FnOnce(&mut JsRuntime)>),
}

pub struct JsRuntime {
    pub heap: Heap,
    pub global_object: ObjectId,
    pub global_env: EnvId,
    pub intrinsics: Intrinsics,
    pub symbols: WellKnownSymbols,
    pub microtasks: Vec<Microtask>,
    pub console_lines: Vec<String>,
    pub active_frames: Vec<EnvId>,
    pub module_cache: HashMap<String, ObjectId>,
    pub current_module_exports: Vec<ObjectId>,
    pub module_loader: Option<ModuleLoader>,
    pub call_depth: usize,
    pub max_call_depth: usize,
    pub max_steps: Option<usize>,
    pub step_count: usize,
}

impl JsRuntime {
    pub fn new() -> Self {
        let mut heap = Heap::new();
        let symbols = WellKnownSymbols {
            iterator: heap.create_symbol(Some("Symbol.iterator".into())),
            async_iterator: heap.create_symbol(Some("Symbol.asyncIterator".into())),
            to_string_tag: heap.create_symbol(Some("Symbol.toStringTag".into())),
            to_primitive: heap.create_symbol(Some("Symbol.toPrimitive".into())),
            has_instance: heap.create_symbol(Some("Symbol.hasInstance".into())),
            is_concat_spreadable: heap.create_symbol(Some("Symbol.isConcatSpreadable".into())),
            species: heap.create_symbol(Some("Symbol.species".into())),
            match_sym: heap.create_symbol(Some("Symbol.match".into())),
            match_all: heap.create_symbol(Some("Symbol.matchAll".into())),
            replace: heap.create_symbol(Some("Symbol.replace".into())),
            search: heap.create_symbol(Some("Symbol.search".into())),
            split: heap.create_symbol(Some("Symbol.split".into())),
            unscopables: heap.create_symbol(Some("Symbol.unscopables".into())),
        };

        let object_proto = heap.alloc_object(JsObject::new(None, ObjectKind::Ordinary));
        let function_proto = heap.alloc_object(JsObject::new(
            Some(object_proto),
            ObjectKind::NativeFunction(NativeFn {
                name: String::new(),
                length: 0,
                callback: Rc::new(|_, _, _| Ok(JsValue::Undefined)),
                is_constructor: false,
            }),
        ));
        let array_proto = heap.alloc_object(JsObject::new(
            Some(object_proto),
            ObjectKind::Array(ArrayData::from_vec(Vec::new())),
        ));
        let string_proto = heap.alloc_object(JsObject::new(
            Some(object_proto),
            ObjectKind::StringObj(String::new()),
        ));
        let number_proto = heap.alloc_object(JsObject::new(
            Some(object_proto),
            ObjectKind::NumberObj(0.0),
        ));
        let boolean_proto = heap.alloc_object(JsObject::new(
            Some(object_proto),
            ObjectKind::BooleanObj(false),
        ));
        let symbol_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let bigint_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let regexp_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let date_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let error_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let type_error_proto =
            heap.alloc_object(JsObject::new(Some(error_proto), ObjectKind::Ordinary));
        let reference_error_proto =
            heap.alloc_object(JsObject::new(Some(error_proto), ObjectKind::Ordinary));
        let syntax_error_proto =
            heap.alloc_object(JsObject::new(Some(error_proto), ObjectKind::Ordinary));
        let range_error_proto =
            heap.alloc_object(JsObject::new(Some(error_proto), ObjectKind::Ordinary));
        let uri_error_proto =
            heap.alloc_object(JsObject::new(Some(error_proto), ObjectKind::Ordinary));
        let eval_error_proto =
            heap.alloc_object(JsObject::new(Some(error_proto), ObjectKind::Ordinary));
        let aggregate_error_proto =
            heap.alloc_object(JsObject::new(Some(error_proto), ObjectKind::Ordinary));
        let map_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let set_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let weak_map_proto =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let weak_set_proto =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let promise_proto =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let array_buffer_proto =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let data_view_proto =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let typed_array_proto =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let mut typed_array_protos = [typed_array_proto; 11];
        for slot in &mut typed_array_protos {
            *slot = heap.alloc_object(JsObject::new(Some(typed_array_proto), ObjectKind::Ordinary));
        }
        let iterator_proto =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let generator_proto =
            heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
        let async_generator_proto =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));

        let global_object =
            heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
        let mut global_env_rec = EnvRecord::new(None, true, false);
        global_env_rec.object_env = Some(global_object);
        global_env_rec.this_binding = Some(JsValue::Object(global_object));
        let global_env = heap.alloc_env(global_env_rec);

        let intrinsics = Intrinsics {
            object_proto,
            function_proto,
            array_proto,
            string_proto,
            number_proto,
            boolean_proto,
            symbol_proto,
            bigint_proto,
            regexp_proto,
            date_proto,
            error_proto,
            type_error_proto,
            reference_error_proto,
            syntax_error_proto,
            range_error_proto,
            uri_error_proto,
            eval_error_proto,
            aggregate_error_proto,
            map_proto,
            set_proto,
            weak_map_proto,
            weak_set_proto,
            promise_proto,
            array_buffer_proto,
            data_view_proto,
            typed_array_proto,
            typed_array_protos,
            iterator_proto,
            generator_proto,
            async_generator_proto,
            eval_fn: function_proto,
            object_ctor: function_proto,
            array_ctor: function_proto,
            promise_ctor: function_proto,
            regexp_ctor: function_proto,
        };

        let mut rt = JsRuntime {
            heap,
            global_object,
            global_env,
            intrinsics,
            symbols,
            microtasks: Vec::new(),
            console_lines: Vec::new(),
            active_frames: Vec::new(),
            module_cache: HashMap::new(),
            current_module_exports: Vec::new(),
            module_loader: None,
            call_depth: 0,
            max_call_depth: 256,
            max_steps: None,
            step_count: 0,
        };

        crate::builtins::init_builtins(&mut rt);
        rt
    }

    pub fn set_module_loader<F>(&mut self, loader: F)
    where
        F: Fn(&str) -> Result<String, JsError> + 'static,
    {
        self.module_loader = Some(Box::new(loader));
    }

    pub fn global_object(&self) -> ObjectId {
        self.global_object
    }

    pub fn console_output(&self) -> &[String] {
        &self.console_lines
    }

    pub fn eval(&mut self, src: &str) -> Result<JsValue, JsError> {
        let program = Parser::parse_script(src)?;
        let chunk = Compiler::compile_program(&program)?;
        self.step_count = 0;
        let res = self.exec_script_chunk(chunk, self.global_env, program.strict)?;
        self.drain_microtasks();
        Ok(res)
    }

    pub fn eval_in_env(&mut self, src: &str, env: EnvId, strict: bool) -> Result<JsValue, JsError> {
        let program = Parser::parse_script_with_strict(src, strict)?;
        let chunk = Compiler::compile_program(&program)?;
        self.exec_script_chunk(chunk, env, program.strict)
    }

    pub fn eval_module(&mut self, specifier: &str, src: &str) -> Result<JsValue, JsError> {
        let ns = self.eval_module_namespace(specifier, src)?;
        self.drain_microtasks();
        Ok(JsValue::Object(ns))
    }

    pub fn eval_module_namespace(
        &mut self,
        specifier: &str,
        src: &str,
    ) -> Result<ObjectId, JsError> {
        if let Some(&cached) = self.module_cache.get(specifier) {
            return Ok(cached);
        }
        let program = Parser::parse_module(src)?;
        let chunk = Compiler::compile_program(&program)?;
        let ns = self.heap.alloc_object(JsObject::new(
            None,
            ObjectKind::ModuleNamespace(ModuleNamespaceData {
                exports: Vec::new(),
            }),
        ));
        self.module_cache.insert(specifier.to_string(), ns);
        let mut mod_env = EnvRecord::new(Some(self.global_env), true, true);
        mod_env.this_binding = Some(JsValue::Undefined);
        let env_id = self.heap.alloc_env(mod_env);
        self.current_module_exports.push(ns);
        let res = self.exec_script_chunk(chunk, env_id, true);
        self.current_module_exports.pop();
        res?;
        Ok(ns)
    }

    pub fn load_module(&mut self, specifier: &str) -> Result<ObjectId, JsError> {
        if let Some(&cached) = self.module_cache.get(specifier) {
            return Ok(cached);
        }
        let src = if let Some(ref loader) = self.module_loader {
            loader(specifier)?
        } else {
            return Err(JsError::error(format!(
                "Cannot load module '{}': no module loader configured",
                specifier
            )));
        };
        self.eval_module_namespace(specifier, &src)
    }

    pub fn create_object(&mut self) -> ObjectId {
        self.heap.alloc_object(JsObject::new(
            Some(self.intrinsics.object_proto),
            ObjectKind::Ordinary,
        ))
    }

    pub fn create_object_with_proto(&mut self, proto: Option<ObjectId>) -> ObjectId {
        self.heap.alloc_object(JsObject::new(proto, ObjectKind::Ordinary))
    }

    pub fn create_array(&mut self, elements: Vec<JsValue>) -> ObjectId {
        self.heap.alloc_object(JsObject::new(
            Some(self.intrinsics.array_proto),
            ObjectKind::Array(ArrayData::from_vec(elements)),
        ))
    }

    pub fn create_native_function(
        &mut self,
        name: &str,
        length: usize,
        callback: NativeCallback,
    ) -> ObjectId {
        let obj = self.heap.alloc_object(JsObject::new(
            Some(self.intrinsics.function_proto),
            ObjectKind::NativeFunction(NativeFn {
                name: name.to_string(),
                length,
                callback,
                is_constructor: false,
            }),
        ));
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("name"),
            PropEntry::data(JsValue::String(name.to_string()), false, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("length"),
            PropEntry::data(JsValue::Number(length as f64), false, false, true),
        );
        obj
    }

    pub fn create_native_constructor(
        &mut self,
        name: &str,
        length: usize,
        proto: ObjectId,
        callback: NativeCallback,
    ) -> ObjectId {
        let obj = self.heap.alloc_object(JsObject::new(
            Some(self.intrinsics.function_proto),
            ObjectKind::NativeFunction(NativeFn {
                name: name.to_string(),
                length,
                callback,
                is_constructor: true,
            }),
        ));
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("name"),
            PropEntry::data(JsValue::String(name.to_string()), false, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("length"),
            PropEntry::data(JsValue::Number(length as f64), false, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("prototype"),
            PropEntry::data(JsValue::Object(proto), false, false, false),
        );
        self.heap.get_obj_mut(proto).set_own_prop(
            PropertyKey::from("constructor"),
            PropEntry::data(JsValue::Object(obj), true, false, true),
        );
        obj
    }

    pub fn set_host_data<T: 'static>(&mut self, obj: ObjectId, data: T) {
        self.heap.get_obj_mut(obj).host_data = Some(Box::new(data));
    }

    pub fn get_host_data<T: 'static>(&self, obj: ObjectId) -> Option<&T> {
        self.heap
            .get_obj(obj)
            .host_data
            .as_ref()
            .and_then(|b| b.downcast_ref::<T>())
    }

    pub fn get_host_data_mut<T: 'static>(&mut self, obj: ObjectId) -> Option<&mut T> {
        self.heap
            .get_obj_mut(obj)
            .host_data
            .as_mut()
            .and_then(|b| b.downcast_mut::<T>())
    }

    pub fn create_error_object(&mut self, kind: ErrorKind, message: &str) -> ObjectId {
        let proto = match kind {
            ErrorKind::Error | ErrorKind::InternalError => self.intrinsics.error_proto,
            ErrorKind::TypeError => self.intrinsics.type_error_proto,
            ErrorKind::ReferenceError => self.intrinsics.reference_error_proto,
            ErrorKind::SyntaxError => self.intrinsics.syntax_error_proto,
            ErrorKind::RangeError => self.intrinsics.range_error_proto,
            ErrorKind::UriError => self.intrinsics.uri_error_proto,
            ErrorKind::EvalError => self.intrinsics.eval_error_proto,
            ErrorKind::AggregateError => self.intrinsics.aggregate_error_proto,
        };
        let stack = format!("{}: {}", kind.name(), message);
        let obj = self.heap.alloc_object(JsObject::new(
            Some(proto),
            ObjectKind::Error(ErrorData {
                kind,
                message: message.to_string(),
                stack: stack.clone(),
            }),
        ));
        if !message.is_empty() {
            self.heap.get_obj_mut(obj).set_own_prop(
                PropertyKey::from("message"),
                PropEntry::data(JsValue::String(message.to_string()), true, false, true),
            );
        }
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("stack"),
            PropEntry::data(JsValue::String(stack), true, false, true),
        );
        obj
    }

    pub fn error_to_js_value(&mut self, err: JsError) -> JsValue {
        match err {
            JsError::Thrown(v) => v,
            JsError::Native {
                kind,
                message,
                object,
                ..
            } => {
                if let Some(oid) = object {
                    JsValue::Object(oid)
                } else {
                    JsValue::Object(self.create_error_object(kind, &message))
                }
            }
        }
    }

    pub fn create_regexp_object(&mut self, pattern: &str, flags: &str) -> Result<ObjectId, JsError> {
        let compiled = Rc::new(CompiledRegExp::compile(pattern, flags)?);
        let obj = self.heap.alloc_object(JsObject::new(
            Some(self.intrinsics.regexp_proto),
            ObjectKind::RegExp(RegExpData {
                compiled,
                original_source: pattern.to_string(),
                last_index: 0,
            }),
        ));
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("lastIndex"),
            PropEntry::data(JsValue::Number(0.0), true, false, false),
        );
        Ok(obj)
    }

    pub fn create_js_function(&mut self, tmpl: &Rc<FuncTemplate>, env: EnvId) -> ObjectId {
        let name = tmpl.name.clone().unwrap_or_default();
        let param_len = compute_func_length(&tmpl.params);
        let proto = self.intrinsics.function_proto;
        let obj = self.heap.alloc_object(JsObject::new(
            Some(proto),
            ObjectKind::Function(JsFunction {
                name: name.clone(),
                chunk: tmpl.chunk.clone(),
                params: tmpl.params.clone(),
                env,
                is_async: tmpl.is_async,
                is_generator: tmpl.is_generator,
                is_arrow: tmpl.is_arrow,
                is_class_ctor: false,
                is_derived_ctor: false,
                strict: tmpl.strict,
                home_object: None,
                super_ctor: None,
                bound_this: None,
                bound_args: Vec::new(),
                bound_target: None,
                source: tmpl.source.clone(),
                class_brand: None,
                field_inits: Rc::new(Vec::new()),
            }),
        ));
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("name"),
            PropEntry::data(JsValue::String(name), false, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("length"),
            PropEntry::data(JsValue::Number(param_len as f64), false, false, true),
        );
        if !tmpl.is_arrow && !tmpl.is_async {
            let fn_proto = if tmpl.is_generator {
                self.create_object_with_proto(Some(self.intrinsics.generator_proto))
            } else {
                let p = self.create_object();
                self.heap.get_obj_mut(p).set_own_prop(
                    PropertyKey::from("constructor"),
                    PropEntry::data(JsValue::Object(obj), true, false, true),
                );
                p
            };
            self.heap.get_obj_mut(obj).set_own_prop(
                PropertyKey::from("prototype"),
                PropEntry::data(JsValue::Object(fn_proto), true, false, false),
            );
        }
        obj
    }

    pub fn queue_microtask(&mut self, cb: JsValue) {
        self.microtasks.push(Microtask::Callback {
            func: cb,
            this: JsValue::Undefined,
            args: Vec::new(),
        });
    }

    pub fn queue_native_microtask<F>(&mut self, f: F)
    where
        F: FnOnce(&mut JsRuntime) + 'static,
    {
        self.microtasks.push(Microtask::Native(Box::new(f)));
    }

    pub fn drain_microtasks(&mut self) {
        let mut guard = 0usize;
        while !self.microtasks.is_empty() && guard < 100_000 {
            guard += 1;
            let batch = std::mem::take(&mut self.microtasks);
            for task in batch {
                match task {
                    Microtask::Callback { func, this, args } => {
                        let _ = self.call_function(func, this, &args);
                    }
                    Microtask::Native(f) => f(self),
                }
            }
        }
    }

    pub fn collect_garbage(&mut self) {
        let mut root_objs = vec![
            self.global_object,
            self.intrinsics.object_proto,
            self.intrinsics.function_proto,
            self.intrinsics.array_proto,
            self.intrinsics.string_proto,
            self.intrinsics.number_proto,
            self.intrinsics.boolean_proto,
            self.intrinsics.symbol_proto,
            self.intrinsics.bigint_proto,
            self.intrinsics.regexp_proto,
            self.intrinsics.date_proto,
            self.intrinsics.error_proto,
            self.intrinsics.type_error_proto,
            self.intrinsics.reference_error_proto,
            self.intrinsics.syntax_error_proto,
            self.intrinsics.range_error_proto,
            self.intrinsics.uri_error_proto,
            self.intrinsics.eval_error_proto,
            self.intrinsics.aggregate_error_proto,
            self.intrinsics.map_proto,
            self.intrinsics.set_proto,
            self.intrinsics.weak_map_proto,
            self.intrinsics.weak_set_proto,
            self.intrinsics.promise_proto,
            self.intrinsics.array_buffer_proto,
            self.intrinsics.data_view_proto,
            self.intrinsics.typed_array_proto,
            self.intrinsics.iterator_proto,
            self.intrinsics.generator_proto,
            self.intrinsics.async_generator_proto,
            self.intrinsics.eval_fn,
            self.intrinsics.object_ctor,
            self.intrinsics.array_ctor,
            self.intrinsics.promise_ctor,
            self.intrinsics.regexp_ctor,
        ];
        root_objs.extend_from_slice(&self.intrinsics.typed_array_protos);
        for &m in self.module_cache.values() {
            root_objs.push(m);
        }
        root_objs.extend_from_slice(&self.current_module_exports);
        for mt in &self.microtasks {
            if let Microtask::Callback { func, this, args } = mt {
                trace_value(func, &mut root_objs);
                trace_value(this, &mut root_objs);
                for a in args {
                    trace_value(a, &mut root_objs);
                }
            }
        }
        let mut root_envs = vec![self.global_env];
        root_envs.extend_from_slice(&self.active_frames);
        self.heap.collect_garbage(&root_objs, &root_envs);
    }

    pub fn to_boolean(&self, val: &JsValue) -> bool {
        val.to_boolean()
    }

    pub fn to_string(&mut self, val: &JsValue) -> Result<String, JsError> {
        match val {
            JsValue::Undefined => Ok("undefined".to_string()),
            JsValue::Null => Ok("null".to_string()),
            JsValue::Boolean(b) => Ok(if *b { "true" } else { "false" }.to_string()),
            JsValue::Number(n) => Ok(number_to_string(*n)),
            JsValue::BigInt(n) => Ok(n.to_string()),
            JsValue::String(s) => Ok(s.clone()),
            JsValue::Symbol(_) => Err(JsError::type_err("Cannot convert a Symbol value to a string")),
            JsValue::Object(_) => {
                let prim = self.to_primitive(val, "string")?;
                self.to_string(&prim)
            }
        }
    }

    pub fn to_display_string(&mut self, val: &JsValue) -> String {
        match val {
            JsValue::Symbol(id) => {
                let desc = self.heap.symbol_description(*id).unwrap_or("");
                format!("Symbol({})", desc)
            }
            other => self.to_string(other).unwrap_or_else(|_| "[object Object]".into()),
        }
    }

    pub fn to_number(&mut self, val: &JsValue) -> Result<f64, JsError> {
        match val {
            JsValue::Undefined => Ok(f64::NAN),
            JsValue::Null => Ok(0.0),
            JsValue::Boolean(b) => Ok(if *b { 1.0 } else { 0.0 }),
            JsValue::Number(n) => Ok(*n),
            JsValue::BigInt(_) => Err(JsError::type_err("Cannot convert a BigInt value to a number")),
            JsValue::String(s) => Ok(string_to_number(s)),
            JsValue::Symbol(_) => Err(JsError::type_err("Cannot convert a Symbol value to a number")),
            JsValue::Object(_) => {
                let prim = self.to_primitive(val, "number")?;
                self.to_number(&prim)
            }
        }
    }

    pub fn to_bigint(&mut self, val: &JsValue) -> Result<i128, JsError> {
        let prim = self.to_primitive(val, "number")?;
        match prim {
            JsValue::BigInt(n) => Ok(n),
            JsValue::Boolean(b) => Ok(if b { 1 } else { 0 }),
            JsValue::String(s) => {
                let t = trim_js_whitespace(&s);
                if t.is_empty() {
                    return Ok(0);
                }
                if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                    i128::from_str_radix(hex, 16)
                } else if let Some(oct) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
                    i128::from_str_radix(oct, 8)
                } else if let Some(bin) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
                    i128::from_str_radix(bin, 2)
                } else {
                    t.parse::<i128>()
                }
                .map_err(|_| JsError::syntax(format!("Cannot convert {} to a BigInt", s)))
            }
            _ => Err(JsError::type_err("Cannot convert value to a BigInt")),
        }
    }

    pub fn to_primitive(&mut self, val: &JsValue, hint: &str) -> Result<JsValue, JsError> {
        let JsValue::Object(oid) = val else {
            return Ok(val.clone());
        };
        let to_prim_key = PropertyKey::Symbol(self.symbols.to_primitive);
        let exotic = self.get_property_key(*oid, &to_prim_key)?;
        if !exotic.is_nullish() {
            if !self.is_callable(&exotic) {
                return Err(JsError::type_err("@@toPrimitive is not callable"));
            }
            let res = self.call_function(
                exotic,
                JsValue::Object(*oid),
                &[JsValue::String(hint.to_string())],
            )?;
            if res.is_object() {
                return Err(JsError::type_err("Cannot convert object to primitive value"));
            }
            return Ok(res);
        }
        let order = if hint == "string" {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for name in order {
            let method = self.get_property(*oid, name)?;
            if self.is_callable(&method) {
                let res = self.call_function(method, JsValue::Object(*oid), &[])?;
                if !res.is_object() {
                    return Ok(res);
                }
            }
        }
        Err(JsError::type_err("Cannot convert object to primitive value"))
    }

    pub fn to_property_key(&mut self, val: &JsValue) -> Result<PropertyKey, JsError> {
        let prim = self.to_primitive(val, "string")?;
        match prim {
            JsValue::Symbol(id) => Ok(PropertyKey::Symbol(id)),
            other => Ok(PropertyKey::String(self.to_string(&other)?)),
        }
    }

    pub fn to_object(&mut self, val: &JsValue) -> Result<ObjectId, JsError> {
        match val {
            JsValue::Object(id) => Ok(*id),
            JsValue::Undefined | JsValue::Null => {
                Err(JsError::type_err("Cannot convert undefined or null to object"))
            }
            JsValue::Boolean(b) => Ok(self.heap.alloc_object(JsObject::new(
                Some(self.intrinsics.boolean_proto),
                ObjectKind::BooleanObj(*b),
            ))),
            JsValue::Number(n) => Ok(self.heap.alloc_object(JsObject::new(
                Some(self.intrinsics.number_proto),
                ObjectKind::NumberObj(*n),
            ))),
            JsValue::String(s) => Ok(self.heap.alloc_object(JsObject::new(
                Some(self.intrinsics.string_proto),
                ObjectKind::StringObj(s.clone()),
            ))),
            JsValue::Symbol(id) => Ok(self.heap.alloc_object(JsObject::new(
                Some(self.intrinsics.symbol_proto),
                ObjectKind::SymbolObj(*id),
            ))),
            JsValue::BigInt(n) => Ok(self.heap.alloc_object(JsObject::new(
                Some(self.intrinsics.bigint_proto),
                ObjectKind::BigIntObj(*n),
            ))),
        }
    }

    pub fn is_callable(&self, val: &JsValue) -> bool {
        let JsValue::Object(id) = val else {
            return false;
        };
        let obj = self.heap.get_obj(*id);
        match &obj.kind {
            ObjectKind::Function(_) | ObjectKind::NativeFunction(_) => true,
            ObjectKind::Proxy(p) => !p.revoked && self.is_callable(&JsValue::Object(p.target)),
            _ => false,
        }
    }

    pub fn is_constructor(&self, val: &JsValue) -> bool {
        let JsValue::Object(id) = val else {
            return false;
        };
        match &self.heap.get_obj(*id).kind {
            ObjectKind::Function(f) => !f.is_arrow && !f.is_async && !f.is_generator,
            ObjectKind::NativeFunction(nf) => nf.is_constructor,
            ObjectKind::Proxy(p) => !p.revoked && self.is_constructor(&JsValue::Object(p.target)),
            _ => false,
        }
    }

    pub fn get_property(&mut self, obj: ObjectId, key: &str) -> Result<JsValue, JsError> {
        self.get_property_key_receiver(obj, &PropertyKey::String(key.to_string()), JsValue::Object(obj))
    }

    pub fn get_property_key(&mut self, obj: ObjectId, key: &PropertyKey) -> Result<JsValue, JsError> {
        self.get_property_key_receiver(obj, key, JsValue::Object(obj))
    }

    pub fn get_property_key_receiver(
        &mut self,
        obj: ObjectId,
        key: &PropertyKey,
        receiver: JsValue,
    ) -> Result<JsValue, JsError> {
        let mut current = Some(obj);
        let mut depth = 0usize;
        while let Some(cur_id) = current {
            depth += 1;
            if depth > 256 {
                return Err(JsError::range("Prototype chain too deep"));
            }
            let proxy_info = match &self.heap.get_obj(cur_id).kind {
                ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
                _ => None,
            };
            if let Some((target, handler, revoked)) = proxy_info {
                if revoked {
                    return Err(JsError::type_err("Cannot perform 'get' on a revoked Proxy"));
                }
                let trap = self.get_property(handler, "get")?;
                if !trap.is_nullish() {
                    return self.call_function(
                        trap,
                        JsValue::Object(handler),
                        &[JsValue::Object(target), key.to_value(), receiver],
                    );
                }
                return self.get_property_key_receiver(target, key, receiver);
            }

            if let Some(val) = self.get_exotic_own_prop(cur_id, key)? {
                return Ok(val);
            }

            let entry_opt = self.heap.get_obj(cur_id).get_own_prop(key).cloned();
            if let Some(entry) = entry_opt {
                return match entry.value {
                    PropValue::Data(v) => Ok(v),
                    PropValue::Accessor { get, .. } => {
                        if let Some(getter) = get {
                            if !getter.is_undefined() {
                                return self.call_function(getter, receiver, &[]);
                            }
                        }
                        Ok(JsValue::Undefined)
                    }
                };
            }
            current = self.heap.get_obj(cur_id).prototype;
        }
        Ok(JsValue::Undefined)
    }

    fn get_exotic_own_prop(
        &mut self,
        obj: ObjectId,
        key: &PropertyKey,
    ) -> Result<Option<JsValue>, JsError> {
        let PropertyKey::String(ref k) = key else {
            return Ok(None);
        };
        let o = self.heap.get_obj(obj);
        match &o.kind {
            ObjectKind::Array(arr) => {
                if k == "length" {
                    return Ok(Some(JsValue::Number(arr.length as f64)));
                }
                if let Some(idx) = parse_array_index(k) {
                    if !arr.has_custom_index_props || o.get_own_prop(key).is_none() {
                        if let Some(v) = arr.get(idx as usize) {
                            return Ok(Some(v.clone()));
                        }
                    }
                }
            }
            ObjectKind::StringObj(s) => {
                if k == "length" {
                    return Ok(Some(JsValue::Number(s.encode_utf16().count() as f64)));
                }
                if let Some(idx) = parse_array_index(k) {
                    if let Some(u) = s.encode_utf16().nth(idx as usize) {
                        let ch = String::from_utf16_lossy(&[u]);
                        return Ok(Some(JsValue::String(ch)));
                    }
                }
            }
            ObjectKind::Arguments(args) => {
                if let Some(idx) = parse_array_index(k) {
                    if o.get_own_prop(key).is_none() {
                        if let Some(v) = args.values.get(idx as usize) {
                            return Ok(Some(v.clone()));
                        }
                    }
                }
            }
            ObjectKind::TypedArray(ta) => {
                if let Some(idx) = parse_array_index(k) {
                    let ta_clone = ta.clone();
                    if (idx as usize) < ta_clone.length {
                        return Ok(Some(self.typed_array_get(&ta_clone, idx as usize)?));
                    }
                    return Ok(Some(JsValue::Undefined));
                }
            }
            ObjectKind::ModuleNamespace(m) => {
                for (name, val) in &m.exports {
                    if name == k {
                        return Ok(Some(val.clone()));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    pub fn set_property(&mut self, obj: ObjectId, key: &str, val: JsValue) -> Result<bool, JsError> {
        self.set_property_key_strict(obj, PropertyKey::String(key.to_string()), val, false)
    }

    pub fn set_property_key_strict(
        &mut self,
        obj: ObjectId,
        key: PropertyKey,
        val: JsValue,
        strict: bool,
    ) -> Result<bool, JsError> {
        let ok = self.set_property_key_receiver(obj, key.clone(), val, JsValue::Object(obj))?;
        if !ok && strict {
            return Err(JsError::type_err(format!(
                "Cannot assign to read only property '{}'",
                key
            )));
        }
        Ok(ok)
    }

    pub fn set_property_key_receiver(
        &mut self,
        obj: ObjectId,
        key: PropertyKey,
        val: JsValue,
        receiver: JsValue,
    ) -> Result<bool, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err("Cannot perform 'set' on a revoked Proxy"));
            }
            let trap = self.get_property(handler, "set")?;
            if !trap.is_nullish() {
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), key.to_value(), val, receiver],
                )?;
                return Ok(res.to_boolean());
            }
            return self.set_property_key_receiver(target, key, val, receiver);
        }

        if let PropertyKey::String(ref k) = key {
            let kind_tag = match &self.heap.get_obj(obj).kind {
                ObjectKind::Array(_) => 1,
                ObjectKind::TypedArray(ta) => {
                    let ta_clone = ta.clone();
                    if let Some(idx) = parse_array_index(k) {
                        self.typed_array_set(&ta_clone, idx as usize, &val)?;
                        return Ok(true);
                    }
                    0
                }
                ObjectKind::Arguments(_) => 2,
                _ => 0,
            };
            if kind_tag == 1 {
                if k == "length" {
                    let n = self.to_number(&val)?;
                    let new_len = to_uint32(n);
                    if new_len as f64 != n {
                        return Err(JsError::range("Invalid array length"));
                    }
                    if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(obj).kind {
                        arr.truncate(new_len as usize);
                    }
                    return Ok(true);
                }
                if let Some(idx) = parse_array_index(k) {
                    let has_custom = match &self.heap.get_obj(obj).kind {
                        ObjectKind::Array(a) => a.has_custom_index_props,
                        _ => false,
                    };
                    let idx_u = idx as usize;
                    if (!has_custom || self.heap.get_obj(obj).get_own_prop(&key).is_none())
                        && idx_u < 65_536
                    {
                        if !self.heap.get_obj(obj).extensible {
                            let exists = match &self.heap.get_obj(obj).kind {
                                ObjectKind::Array(a) => a.get(idx_u).is_some(),
                                _ => false,
                            };
                            if !exists {
                                return Ok(false);
                            }
                        }
                        if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(obj).kind {
                            arr.set(idx_u, val);
                        }
                        return Ok(true);
                    }
                    if idx_u >= 65_536 {
                        if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(obj).kind {
                            arr.has_custom_index_props = true;
                            if idx_u >= arr.length {
                                arr.length = idx_u + 1;
                            }
                        }
                    }
                }
            }
            if kind_tag == 2 {
                if let Some(idx) = parse_array_index(k) {
                    let idx_u = idx as usize;
                    if let ObjectKind::Arguments(ref mut a) = self.heap.get_obj_mut(obj).kind {
                        if idx_u < a.values.len() {
                            a.values[idx_u] = val;
                            return Ok(true);
                        }
                    }
                }
            }
        }

        let own = self.heap.get_obj(obj).get_own_prop(&key).cloned();
        if let Some(entry) = own {
            match entry.value {
                PropValue::Data(_) => {
                    if !entry.writable {
                        return Ok(false);
                    }
                    if let JsValue::Object(recv_id) = receiver {
                        if recv_id == obj {
                            if let Some(e) = self.heap.get_obj_mut(obj).get_own_prop_mut(&key) {
                                e.value = PropValue::Data(val);
                            }
                            return Ok(true);
                        }
                        return self.set_on_receiver(recv_id, key, val);
                    }
                    return Ok(false);
                }
                PropValue::Accessor { set, .. } => {
                    if let Some(setter) = set {
                        if !setter.is_undefined() {
                            self.call_function(setter, receiver, &[val])?;
                            return Ok(true);
                        }
                    }
                    return Ok(false);
                }
            }
        }

        let mut proto = self.heap.get_obj(obj).prototype;
        while let Some(pid) = proto {
            let p_entry = self.heap.get_obj(pid).get_own_prop(&key).cloned();
            if let Some(entry) = p_entry {
                match entry.value {
                    PropValue::Accessor { set, .. } => {
                        if let Some(setter) = set {
                            if !setter.is_undefined() {
                                self.call_function(setter, receiver, &[val])?;
                                return Ok(true);
                            }
                        }
                        return Ok(false);
                    }
                    PropValue::Data(_) => {
                        if !entry.writable {
                            return Ok(false);
                        }
                        break;
                    }
                }
            }
            proto = self.heap.get_obj(pid).prototype;
        }

        if let JsValue::Object(recv_id) = receiver {
            self.set_on_receiver(recv_id, key, val)
        } else {
            Ok(false)
        }
    }

    fn set_on_receiver(
        &mut self,
        recv_id: ObjectId,
        key: PropertyKey,
        val: JsValue,
    ) -> Result<bool, JsError> {
        if let Some(existing) = self.get_own_property_descriptor(recv_id, &key)? {
            if existing.is_accessor() || existing.writable == Some(false) {
                return Ok(false);
            }
            let mut upd = PropertyDescriptor::empty();
            upd.value = Some(val);
            return self.define_property(recv_id, key, upd);
        }
        self.define_property(recv_id, key, PropertyDescriptor::data(val, true, true, true))
    }

    pub fn define_property(
        &mut self,
        obj: ObjectId,
        key: PropertyKey,
        desc: PropertyDescriptor,
    ) -> Result<bool, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err(
                    "Cannot perform 'defineProperty' on a revoked Proxy",
                ));
            }
            let trap = self.get_property(handler, "defineProperty")?;
            if !trap.is_nullish() {
                let desc_obj = self.from_property_descriptor(&desc);
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), key.to_value(), desc_obj],
                )?;
                return Ok(res.to_boolean());
            }
            return self.define_property(target, key, desc);
        }

        if let PropertyKey::String(ref k) = key {
            if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(obj).kind {
                if k == "length" {
                    if let Some(v) = desc.value {
                        let n = self.to_number(&v)?;
                        let new_len = to_uint32(n);
                        if new_len as f64 != n {
                            return Err(JsError::range("Invalid array length"));
                        }
                        if let ObjectKind::Array(ref mut a) = self.heap.get_obj_mut(obj).kind {
                            a.truncate(new_len as usize);
                        }
                    }
                    return Ok(true);
                }
                if let Some(idx) = parse_array_index(k) {
                    let is_plain_data = !desc.is_accessor()
                        && desc.writable.unwrap_or(true)
                        && desc.enumerable.unwrap_or(true)
                        && desc.configurable.unwrap_or(true)
                        && !arr.has_custom_index_props;
                    if is_plain_data {
                        let val = desc.value.unwrap_or(JsValue::Undefined);
                        arr.set(idx as usize, val);
                        return Ok(true);
                    }
                    arr.has_custom_index_props = true;
                    if (idx as usize) >= arr.length {
                        arr.length = (idx as usize) + 1;
                    }
                    let existing_val = arr.get(idx as usize).cloned();
                    if let Some(ev) = existing_val {
                        let o = self.heap.get_obj_mut(obj);
                        if o.get_own_prop(&key).is_none() {
                            o.set_own_prop(key.clone(), PropEntry::data(ev, true, true, true));
                        }
                    }
                }
            }
        }

        let current = self.heap.get_obj(obj).get_own_prop(&key).cloned();
        let extensible = self.heap.get_obj(obj).extensible;

        let Some(cur) = current else {
            if !extensible {
                return Ok(false);
            }
            let entry = if desc.is_accessor() {
                PropEntry::accessor(
                    desc.get,
                    desc.set,
                    desc.enumerable.unwrap_or(false),
                    desc.configurable.unwrap_or(false),
                )
            } else {
                PropEntry::data(
                    desc.value.unwrap_or(JsValue::Undefined),
                    desc.writable.unwrap_or(false),
                    desc.enumerable.unwrap_or(false),
                    desc.configurable.unwrap_or(false),
                )
            };
            self.heap.get_obj_mut(obj).set_own_prop(key, entry);
            return Ok(true);
        };

        if !cur.configurable {
            if desc.configurable == Some(true) {
                return Ok(false);
            }
            if let Some(e) = desc.enumerable {
                if e != cur.enumerable {
                    return Ok(false);
                }
            }
            if !desc.is_generic() {
                let cur_is_acc = matches!(cur.value, PropValue::Accessor { .. });
                if cur_is_acc != desc.is_accessor() {
                    return Ok(false);
                }
                if !cur_is_acc && !cur.writable {
                    if desc.writable == Some(true) {
                        return Ok(false);
                    }
                    if let (Some(ref nv), PropValue::Data(ref cv)) = (&desc.value, &cur.value) {
                        if !nv.same_value(cv) {
                            return Ok(false);
                        }
                    }
                }
            }
        }

        let mut next = cur.clone();
        if let Some(e) = desc.enumerable {
            next.enumerable = e;
        }
        if let Some(c) = desc.configurable {
            next.configurable = c;
        }
        if desc.is_accessor() {
            let (old_g, old_s) = match cur.value {
                PropValue::Accessor { get, set } => (get, set),
                PropValue::Data(_) => (None, None),
            };
            next.value = PropValue::Accessor {
                get: desc.get.or(old_g),
                set: desc.set.or(old_s),
            };
            next.writable = false;
        } else if desc.is_data() {
            let old_v = match cur.value {
                PropValue::Data(v) => v,
                PropValue::Accessor { .. } => JsValue::Undefined,
            };
            next.value = PropValue::Data(desc.value.unwrap_or(old_v));
            if let Some(w) = desc.writable {
                next.writable = w;
            }
        }
        self.heap.get_obj_mut(obj).set_own_prop(key, next);
        Ok(true)
    }

    pub fn delete_property(&mut self, obj: ObjectId, key: &PropertyKey) -> Result<bool, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err(
                    "Cannot perform 'deleteProperty' on a revoked Proxy",
                ));
            }
            let trap = self.get_property(handler, "deleteProperty")?;
            if !trap.is_nullish() {
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), key.to_value()],
                )?;
                return Ok(res.to_boolean());
            }
            return self.delete_property(target, key);
        }

        if let PropertyKey::String(ref k) = key {
            if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(obj).kind {
                if k == "length" {
                    return Ok(false);
                }
                if let Some(idx) = parse_array_index(k) {
                    arr.delete(idx as usize);
                    self.heap.get_obj_mut(obj).remove_own_prop(key);
                    return Ok(true);
                }
            }
        }

        if let Some(entry) = self.heap.get_obj(obj).get_own_prop(key) {
            if !entry.configurable {
                return Ok(false);
            }
            self.heap.get_obj_mut(obj).remove_own_prop(key);
        }
        Ok(true)
    }

    pub fn has_property(&mut self, obj: ObjectId, key: &PropertyKey) -> Result<bool, JsError> {
        let mut current = Some(obj);
        while let Some(cur_id) = current {
            let proxy_info = match &self.heap.get_obj(cur_id).kind {
                ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
                _ => None,
            };
            if let Some((target, handler, revoked)) = proxy_info {
                if revoked {
                    return Err(JsError::type_err("Cannot perform 'has' on a revoked Proxy"));
                }
                let trap = self.get_property(handler, "has")?;
                if !trap.is_nullish() {
                    let res = self.call_function(
                        trap,
                        JsValue::Object(handler),
                        &[JsValue::Object(target), key.to_value()],
                    )?;
                    return Ok(res.to_boolean());
                }
                return self.has_property(target, key);
            }
            if self.has_own_property(cur_id, key)? {
                return Ok(true);
            }
            current = self.heap.get_obj(cur_id).prototype;
        }
        Ok(false)
    }

    pub fn has_own_property(&mut self, obj: ObjectId, key: &PropertyKey) -> Result<bool, JsError> {
        Ok(self.get_own_property_descriptor(obj, key)?.is_some())
    }

    pub fn get_own_property_descriptor(
        &mut self,
        obj: ObjectId,
        key: &PropertyKey,
    ) -> Result<Option<PropertyDescriptor>, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err(
                    "Cannot perform 'getOwnPropertyDescriptor' on a revoked Proxy",
                ));
            }
            let trap = self.get_property(handler, "getOwnPropertyDescriptor")?;
            if !trap.is_nullish() {
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), key.to_value()],
                )?;
                if res.is_undefined() {
                    return Ok(None);
                }
                return Ok(Some(self.to_property_descriptor(&res)?));
            }
            return self.get_own_property_descriptor(target, key);
        }

        let o = self.heap.get_obj(obj);
        if let Some(entry) = o.get_own_prop(key) {
            return Ok(Some(entry.to_descriptor()));
        }
        if let PropertyKey::String(ref k) = key {
            match &o.kind {
                ObjectKind::Array(arr) => {
                    if k == "length" {
                        return Ok(Some(PropertyDescriptor::data(
                            JsValue::Number(arr.length as f64),
                            true,
                            false,
                            false,
                        )));
                    }
                    if let Some(idx) = parse_array_index(k) {
                        if let Some(v) = arr.get(idx as usize) {
                            return Ok(Some(PropertyDescriptor::data(v.clone(), true, true, true)));
                        }
                    }
                }
                ObjectKind::StringObj(s) => {
                    if k == "length" {
                        return Ok(Some(PropertyDescriptor::data(
                            JsValue::Number(s.encode_utf16().count() as f64),
                            false,
                            false,
                            false,
                        )));
                    }
                    if let Some(idx) = parse_array_index(k) {
                        if let Some(u) = s.encode_utf16().nth(idx as usize) {
                            return Ok(Some(PropertyDescriptor::data(
                                JsValue::String(String::from_utf16_lossy(&[u])),
                                false,
                                true,
                                false,
                            )));
                        }
                    }
                }
                ObjectKind::Arguments(args) => {
                    if let Some(idx) = parse_array_index(k) {
                        if let Some(v) = args.values.get(idx as usize) {
                            return Ok(Some(PropertyDescriptor::data(v.clone(), true, true, true)));
                        }
                    }
                }
                ObjectKind::TypedArray(ta) => {
                    if let Some(idx) = parse_array_index(k) {
                        let ta_clone = ta.clone();
                        if (idx as usize) < ta_clone.length {
                            let v = self.typed_array_get(&ta_clone, idx as usize)?;
                            return Ok(Some(PropertyDescriptor::data(v, true, true, true)));
                        }
                    }
                }
                ObjectKind::ModuleNamespace(m) => {
                    for (name, val) in &m.exports {
                        if name == k {
                            return Ok(Some(PropertyDescriptor::data(
                                val.clone(),
                                true,
                                true,
                                false,
                            )));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub fn from_property_descriptor(&mut self, desc: &PropertyDescriptor) -> JsValue {
        let obj = self.create_object();
        if let Some(ref v) = desc.value {
            let _ = self.set_property(obj, "value", v.clone());
        }
        if let Some(w) = desc.writable {
            let _ = self.set_property(obj, "writable", JsValue::Boolean(w));
        }
        if let Some(ref g) = desc.get {
            let _ = self.set_property(obj, "get", g.clone());
        }
        if let Some(ref s) = desc.set {
            let _ = self.set_property(obj, "set", s.clone());
        }
        if let Some(e) = desc.enumerable {
            let _ = self.set_property(obj, "enumerable", JsValue::Boolean(e));
        }
        if let Some(c) = desc.configurable {
            let _ = self.set_property(obj, "configurable", JsValue::Boolean(c));
        }
        JsValue::Object(obj)
    }

    pub fn to_property_descriptor(&mut self, val: &JsValue) -> Result<PropertyDescriptor, JsError> {
        let JsValue::Object(oid) = val else {
            return Err(JsError::type_err("Property description must be an object"));
        };
        let mut desc = PropertyDescriptor::empty();
        if self.has_property(*oid, &PropertyKey::from("enumerable"))? {
            let v = self.get_property(*oid, "enumerable")?;
            desc.enumerable = Some(v.to_boolean());
        }
        if self.has_property(*oid, &PropertyKey::from("configurable"))? {
            let v = self.get_property(*oid, "configurable")?;
            desc.configurable = Some(v.to_boolean());
        }
        if self.has_property(*oid, &PropertyKey::from("value"))? {
            desc.value = Some(self.get_property(*oid, "value")?);
        }
        if self.has_property(*oid, &PropertyKey::from("writable"))? {
            let v = self.get_property(*oid, "writable")?;
            desc.writable = Some(v.to_boolean());
        }
        if self.has_property(*oid, &PropertyKey::from("get"))? {
            let v = self.get_property(*oid, "get")?;
            if !v.is_undefined() && !self.is_callable(&v) {
                return Err(JsError::type_err("Getter must be a function"));
            }
            desc.get = Some(v);
        }
        if self.has_property(*oid, &PropertyKey::from("set"))? {
            let v = self.get_property(*oid, "set")?;
            if !v.is_undefined() && !self.is_callable(&v) {
                return Err(JsError::type_err("Setter must be a function"));
            }
            desc.set = Some(v);
        }
        if desc.is_accessor() && desc.is_data() {
            return Err(JsError::type_err(
                "Invalid property descriptor. Cannot both specify accessors and a value or writable attribute",
            ));
        }
        Ok(desc)
    }

    pub fn own_keys(&mut self, obj: ObjectId) -> Result<Vec<PropertyKey>, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err("Cannot perform 'ownKeys' on a revoked Proxy"));
            }
            let trap = self.get_property(handler, "ownKeys")?;
            if !trap.is_nullish() {
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target)],
                )?;
                let list_oid = self.to_object(&res)?;
                let len_val = self.get_property(list_oid, "length")?;
                let len = to_length(self.to_number(&len_val)?);
                let mut keys = Vec::with_capacity(len);
                for i in 0..len {
                    let elem = self.get_property(list_oid, &i.to_string())?;
                    match elem {
                        JsValue::String(s) => keys.push(PropertyKey::String(s)),
                        JsValue::Symbol(id) => keys.push(PropertyKey::Symbol(id)),
                        _ => {
                            return Err(JsError::type_err(
                                "Proxy ownKeys trap must return strings or symbols",
                            ));
                        }
                    }
                }
                return Ok(keys);
            }
            return self.own_keys(target);
        }
        Ok(self.heap.get_obj(obj).own_property_keys())
    }

    pub fn get_prototype_of(&mut self, obj: ObjectId) -> Result<Option<ObjectId>, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err(
                    "Cannot perform 'getPrototypeOf' on a revoked Proxy",
                ));
            }
            let trap = self.get_property(handler, "getPrototypeOf")?;
            if !trap.is_nullish() {
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target)],
                )?;
                return match res {
                    JsValue::Null => Ok(None),
                    JsValue::Object(id) => Ok(Some(id)),
                    _ => Err(JsError::type_err(
                        "Proxy getPrototypeOf trap must return an object or null",
                    )),
                };
            }
            return self.get_prototype_of(target);
        }
        Ok(self.heap.get_obj(obj).prototype)
    }

    pub fn set_prototype_of(
        &mut self,
        obj: ObjectId,
        proto: Option<ObjectId>,
    ) -> Result<bool, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err(
                    "Cannot perform 'setPrototypeOf' on a revoked Proxy",
                ));
            }
            let trap = self.get_property(handler, "setPrototypeOf")?;
            if !trap.is_nullish() {
                let pv = proto.map(JsValue::Object).unwrap_or(JsValue::Null);
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), pv],
                )?;
                return Ok(res.to_boolean());
            }
            return self.set_prototype_of(target, proto);
        }
        if self.heap.get_obj(obj).prototype == proto {
            return Ok(true);
        }
        if !self.heap.get_obj(obj).extensible {
            return Ok(false);
        }
        let mut p = proto;
        while let Some(pid) = p {
            if pid == obj {
                return Ok(false);
            }
            p = self.heap.get_obj(pid).prototype;
        }
        self.heap.get_obj_mut(obj).prototype = proto;
        Ok(true)
    }

    pub fn is_extensible(&mut self, obj: ObjectId) -> Result<bool, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err(
                    "Cannot perform 'isExtensible' on a revoked Proxy",
                ));
            }
            let trap = self.get_property(handler, "isExtensible")?;
            if !trap.is_nullish() {
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target)],
                )?;
                return Ok(res.to_boolean());
            }
            return self.is_extensible(target);
        }
        Ok(self.heap.get_obj(obj).extensible)
    }

    pub fn prevent_extensions(&mut self, obj: ObjectId) -> Result<bool, JsError> {
        let proxy_info = match &self.heap.get_obj(obj).kind {
            ObjectKind::Proxy(p) => Some((p.target, p.handler, p.revoked)),
            _ => None,
        };
        if let Some((target, handler, revoked)) = proxy_info {
            if revoked {
                return Err(JsError::type_err(
                    "Cannot perform 'preventExtensions' on a revoked Proxy",
                ));
            }
            let trap = self.get_property(handler, "preventExtensions")?;
            if !trap.is_nullish() {
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target)],
                )?;
                return Ok(res.to_boolean());
            }
            return self.prevent_extensions(target);
        }
        self.heap.get_obj_mut(obj).extensible = false;
        Ok(true)
    }

    pub fn abstract_eq(&mut self, a: &JsValue, b: &JsValue) -> Result<bool, JsError> {
        match (a, b) {
            (JsValue::Undefined, JsValue::Undefined)
            | (JsValue::Null, JsValue::Null)
            | (JsValue::Undefined, JsValue::Null)
            | (JsValue::Null, JsValue::Undefined) => Ok(true),
            (JsValue::Number(_), JsValue::Number(_))
            | (JsValue::String(_), JsValue::String(_))
            | (JsValue::Boolean(_), JsValue::Boolean(_))
            | (JsValue::Symbol(_), JsValue::Symbol(_))
            | (JsValue::BigInt(_), JsValue::BigInt(_))
            | (JsValue::Object(_), JsValue::Object(_)) => Ok(a.strict_eq(b)),
            (JsValue::Number(x), JsValue::String(s)) => Ok(*x == string_to_number(s)),
            (JsValue::String(s), JsValue::Number(y)) => Ok(string_to_number(s) == *y),
            (JsValue::Boolean(x), _) => {
                self.abstract_eq(&JsValue::Number(if *x { 1.0 } else { 0.0 }), b)
            }
            (_, JsValue::Boolean(y)) => {
                self.abstract_eq(a, &JsValue::Number(if *y { 1.0 } else { 0.0 }))
            }
            (JsValue::BigInt(n), JsValue::Number(f))
            | (JsValue::Number(f), JsValue::BigInt(n)) => {
                if !f.is_finite() || f.fract() != 0.0 {
                    Ok(false)
                } else {
                    Ok((*n as f64) == *f)
                }
            }
            (JsValue::BigInt(n), JsValue::String(s))
            | (JsValue::String(s), JsValue::BigInt(n)) => {
                match self.to_bigint(&JsValue::String(s.clone())) {
                    Ok(m) => Ok(*n == m),
                    Err(_) => Ok(false),
                }
            }
            (JsValue::Object(_), JsValue::String(_) | JsValue::Number(_) | JsValue::Symbol(_) | JsValue::BigInt(_)) => {
                let p = self.to_primitive(a, "default")?;
                self.abstract_eq(&p, b)
            }
            (JsValue::String(_) | JsValue::Number(_) | JsValue::Symbol(_) | JsValue::BigInt(_), JsValue::Object(_)) => {
                let p = self.to_primitive(b, "default")?;
                self.abstract_eq(a, &p)
            }
            _ => Ok(false),
        }
    }

    pub fn relational_lt(&mut self, a: &JsValue, b: &JsValue) -> Result<Option<bool>, JsError> {
        let px = self.to_primitive(a, "number")?;
        let py = self.to_primitive(b, "number")?;
        if let (JsValue::String(sx), JsValue::String(sy)) = (&px, &py) {
            let ux: Vec<u16> = sx.encode_utf16().collect();
            let uy: Vec<u16> = sy.encode_utf16().collect();
            return Ok(Some(ux < uy));
        }
        if let (JsValue::BigInt(x), JsValue::BigInt(y)) = (&px, &py) {
            return Ok(Some(x < y));
        }
        if let (JsValue::BigInt(x), JsValue::Number(y)) = (&px, &py) {
            if y.is_nan() {
                return Ok(None);
            }
            return Ok(Some((*x as f64) < *y));
        }
        if let (JsValue::Number(x), JsValue::BigInt(y)) = (&px, &py) {
            if x.is_nan() {
                return Ok(None);
            }
            return Ok(Some(*x < (*y as f64)));
        }
        let nx = self.to_number(&px)?;
        let ny = self.to_number(&py)?;
        if nx.is_nan() || ny.is_nan() {
            return Ok(None);
        }
        Ok(Some(nx < ny))
    }

    pub fn instanceof(&mut self, lhs: &JsValue, rhs: &JsValue) -> Result<bool, JsError> {
        let JsValue::Object(rhs_id) = rhs else {
            return Err(JsError::type_err("Right-hand side of 'instanceof' is not an object"));
        };
        let has_inst = PropertyKey::Symbol(self.symbols.has_instance);
        let handler = self.get_property_key(*rhs_id, &has_inst)?;
        if !handler.is_nullish() {
            let res = self.call_function(handler, JsValue::Object(*rhs_id), &[lhs.clone()])?;
            return Ok(res.to_boolean());
        }
        if !self.is_callable(rhs) {
            return Err(JsError::type_err("Right-hand side of 'instanceof' is not callable"));
        }
        let JsValue::Object(lhs_id) = lhs else {
            return Ok(false);
        };
        let proto_val = self.get_property(*rhs_id, "prototype")?;
        let JsValue::Object(target_proto) = proto_val else {
            return Err(JsError::type_err(
                "Function has non-object prototype in instanceof check",
            ));
        };
        let mut cur = self.get_prototype_of(*lhs_id)?;
        while let Some(pid) = cur {
            if pid == target_proto {
                return Ok(true);
            }
            cur = self.get_prototype_of(pid)?;
        }
        Ok(false)
    }

    pub fn typeof_value(&self, val: &JsValue) -> &'static str {
        match val {
            JsValue::Undefined => "undefined",
            JsValue::Null => "object",
            JsValue::Boolean(_) => "boolean",
            JsValue::Number(_) => "number",
            JsValue::BigInt(_) => "bigint",
            JsValue::String(_) => "string",
            JsValue::Symbol(_) => "symbol",
            JsValue::Object(_) => {
                if self.is_callable(val) {
                    "function"
                } else {
                    "object"
                }
            }
        }
    }

    pub fn typed_array_get(&self, ta: &TypedArrayData, index: usize) -> Result<JsValue, JsError> {
        let ObjectKind::ArrayBuffer(ref ab) = self.heap.get_obj(ta.buffer).kind else {
            return Err(JsError::type_err("Invalid ArrayBuffer"));
        };
        if ab.detached {
            return Ok(JsValue::Undefined);
        }
        let es = ta.kind.element_size();
        let off = ta.byte_offset + index * es;
        if off + es > ab.bytes.len() {
            return Ok(JsValue::Undefined);
        }
        let slice = &ab.bytes[off..off + es];
        Ok(match ta.kind {
            TypedArrayKind::Int8 => JsValue::Number(slice[0] as i8 as f64),
            TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped => {
                JsValue::Number(slice[0] as f64)
            }
            TypedArrayKind::Int16 => {
                JsValue::Number(i16::from_le_bytes([slice[0], slice[1]]) as f64)
            }
            TypedArrayKind::Uint16 => {
                JsValue::Number(u16::from_le_bytes([slice[0], slice[1]]) as f64)
            }
            TypedArrayKind::Int32 => {
                JsValue::Number(i32::from_le_bytes(slice.try_into().unwrap()) as f64)
            }
            TypedArrayKind::Uint32 => {
                JsValue::Number(u32::from_le_bytes(slice.try_into().unwrap()) as f64)
            }
            TypedArrayKind::Float32 => {
                JsValue::Number(f32::from_le_bytes(slice.try_into().unwrap()) as f64)
            }
            TypedArrayKind::Float64 => {
                JsValue::Number(f64::from_le_bytes(slice.try_into().unwrap()))
            }
            TypedArrayKind::BigInt64 => {
                JsValue::BigInt(i64::from_le_bytes(slice.try_into().unwrap()) as i128)
            }
            TypedArrayKind::BigUint64 => {
                JsValue::BigInt(u64::from_le_bytes(slice.try_into().unwrap()) as i128)
            }
        })
    }

    pub fn typed_array_set(
        &mut self,
        ta: &TypedArrayData,
        index: usize,
        val: &JsValue,
    ) -> Result<(), JsError> {
        let es = ta.kind.element_size();
        let off = ta.byte_offset + index * es;
        if ta.kind.is_bigint() {
            let bi = self.to_bigint(val)?;
            let ObjectKind::ArrayBuffer(ref mut ab) = self.heap.get_obj_mut(ta.buffer).kind else {
                return Err(JsError::type_err("Invalid ArrayBuffer"));
            };
            if ab.detached || index >= ta.length || off + es > ab.bytes.len() {
                return Ok(());
            }
            let bytes = match ta.kind {
                TypedArrayKind::BigInt64 => (bi as i64).to_le_bytes(),
                TypedArrayKind::BigUint64 => (bi as u64).to_le_bytes(),
                _ => unreachable!(),
            };
            ab.bytes[off..off + 8].copy_from_slice(&bytes);
            return Ok(());
        }
        let n = self.to_number(val)?;
        let ObjectKind::ArrayBuffer(ref mut ab) = self.heap.get_obj_mut(ta.buffer).kind else {
            return Err(JsError::type_err("Invalid ArrayBuffer"));
        };
        if ab.detached || index >= ta.length || off + es > ab.bytes.len() {
            return Ok(());
        }
        match ta.kind {
            TypedArrayKind::Int8 => ab.bytes[off] = to_int32(n) as i8 as u8,
            TypedArrayKind::Uint8 => ab.bytes[off] = to_uint32(n) as u8,
            TypedArrayKind::Uint8Clamped => {
                let clamped = if n.is_nan() || n <= 0.0 {
                    0u8
                } else if n >= 255.0 {
                    255u8
                } else {
                    let f = n.floor();
                    let frac = n - f;
                    if frac < 0.5 {
                        f as u8
                    } else if frac > 0.5 {
                        (f + 1.0) as u8
                    } else if (f as u32) % 2 == 1 {
                        (f + 1.0) as u8
                    } else {
                        f as u8
                    }
                };
                ab.bytes[off] = clamped;
            }
            TypedArrayKind::Int16 => {
                ab.bytes[off..off + 2].copy_from_slice(&(to_int32(n) as i16).to_le_bytes());
            }
            TypedArrayKind::Uint16 => {
                ab.bytes[off..off + 2].copy_from_slice(&(to_uint32(n) as u16).to_le_bytes());
            }
            TypedArrayKind::Int32 => {
                ab.bytes[off..off + 4].copy_from_slice(&to_int32(n).to_le_bytes());
            }
            TypedArrayKind::Uint32 => {
                ab.bytes[off..off + 4].copy_from_slice(&to_uint32(n).to_le_bytes());
            }
            TypedArrayKind::Float32 => {
                ab.bytes[off..off + 4].copy_from_slice(&(n as f32).to_le_bytes());
            }
            TypedArrayKind::Float64 => {
                ab.bytes[off..off + 8].copy_from_slice(&n.to_le_bytes());
            }
            _ => {}
        }
        Ok(())
    }

    pub fn get_iterator(&mut self, val: &JsValue, is_async: bool) -> Result<JsValue, JsError> {
        let oid = self.to_object(val)?;
        if is_async {
            let async_sym = PropertyKey::Symbol(self.symbols.async_iterator);
            let method = self.get_property_key(oid, &async_sym)?;
            if !method.is_nullish() {
                let iter = self.call_function(method, val.clone(), &[])?;
                if !iter.is_object() {
                    return Err(JsError::type_err("Async iterator must return an object"));
                }
                return Ok(iter);
            }
        }
        let sym = PropertyKey::Symbol(self.symbols.iterator);
        let method = self.get_property_key(oid, &sym)?;
        if method.is_nullish() {
            return Err(JsError::type_err("Value is not iterable"));
        }
        let iter = self.call_function(method, val.clone(), &[])?;
        if !iter.is_object() {
            return Err(JsError::type_err("Iterator must return an object"));
        }
        Ok(iter)
    }

    pub fn iterator_step(&mut self, iter: &JsValue) -> Result<Option<JsValue>, JsError> {
        let JsValue::Object(oid) = iter else {
            return Err(JsError::type_err("Iterator must be an object"));
        };
        let next_fn = self.get_property(*oid, "next")?;
        let res = self.call_function(next_fn, iter.clone(), &[])?;
        let JsValue::Object(res_id) = res else {
            return Err(JsError::type_err("Iterator result is not an object"));
        };
        let done = self.get_property(res_id, "done")?.to_boolean();
        if done {
            Ok(None)
        } else {
            Ok(Some(self.get_property(res_id, "value")?))
        }
    }

    pub fn iterator_close(&mut self, iter: &JsValue) {
        if let JsValue::Object(oid) = iter {
            if let Ok(ret_fn) = self.get_property(*oid, "return") {
                if self.is_callable(&ret_fn) {
                    let _ = self.call_function(ret_fn, iter.clone(), &[]);
                }
            }
        }
    }

    pub fn collect_iterable(&mut self, val: &JsValue) -> Result<Vec<JsValue>, JsError> {
        if let JsValue::Object(oid) = val {
            if let ObjectKind::Array(ref arr) = self.heap.get_obj(*oid).kind {
                let sym = PropertyKey::Symbol(self.symbols.iterator);
                let has_own_iter = self.heap.get_obj(*oid).get_own_prop(&sym).is_some();
                if !has_own_iter && !arr.has_custom_index_props {
                    let len = arr.length;
                    let mut out = Vec::with_capacity(len);
                    for i in 0..len {
                        out.push(arr.get(i).cloned().unwrap_or(JsValue::Undefined));
                    }
                    return Ok(out);
                }
            }
        }
        let iter = self.get_iterator(val, false)?;
        let mut out = Vec::new();
        while let Some(item) = self.iterator_step(&iter)? {
            out.push(item);
        }
        Ok(out)
    }

    pub fn create_promise(&mut self) -> ObjectId {
        self.heap.alloc_object(JsObject::new(
            Some(self.intrinsics.promise_proto),
            ObjectKind::Promise(PromiseData {
                state: PromiseState::Pending,
                reactions: Vec::new(),
                handled: false,
            }),
        ))
    }

    pub fn resolve_promise(&mut self, promise_id: ObjectId, value: JsValue) {
        let is_pending = matches!(
            self.heap.get_obj(promise_id).kind,
            ObjectKind::Promise(PromiseData {
                state: PromiseState::Pending,
                ..
            })
        );
        if !is_pending {
            return;
        }
        if value == JsValue::Object(promise_id) {
            let err = self.create_error_object(
                ErrorKind::TypeError,
                "Chaining cycle detected for promise",
            );
            self.reject_promise(promise_id, JsValue::Object(err));
            return;
        }
        if let JsValue::Object(thenable_id) = value {
            match self.get_property(thenable_id, "then") {
                Ok(then_fn) if self.is_callable(&then_fn) => {
                    let pid = promise_id;
                    let resolved_flag = Rc::new(std::cell::Cell::new(false));
                    let f1 = resolved_flag.clone();
                    let res_cb = self.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, args| {
                            if !f1.replace(true) {
                                let v = args.first().cloned().unwrap_or(JsValue::Undefined);
                                rt.resolve_promise(pid, v);
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    let f2 = resolved_flag;
                    let rej_cb = self.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, args| {
                            if !f2.replace(true) {
                                let r = args.first().cloned().unwrap_or(JsValue::Undefined);
                                rt.reject_promise(pid, r);
                            }
                            Ok(JsValue::Undefined)
                        }),
                    );
                    self.queue_native_microtask(move |rt| {
                        if let Err(e) = rt.call_function(
                            then_fn,
                            JsValue::Object(thenable_id),
                            &[JsValue::Object(res_cb), JsValue::Object(rej_cb)],
                        ) {
                            let ev = rt.error_to_js_value(e);
                            rt.reject_promise(pid, ev);
                        }
                    });
                    return;
                }
                Err(e) => {
                    let ev = self.error_to_js_value(e);
                    self.reject_promise(promise_id, ev);
                    return;
                }
                _ => {}
            }
        }

        let reactions = match &mut self.heap.get_obj_mut(promise_id).kind {
            ObjectKind::Promise(p) => {
                p.state = PromiseState::Fulfilled(value.clone());
                std::mem::take(&mut p.reactions)
            }
            _ => return,
        };
        for r in reactions {
            self.schedule_promise_reaction(r, true, value.clone());
        }
    }

    pub fn reject_promise(&mut self, promise_id: ObjectId, reason: JsValue) {
        let reactions = match &mut self.heap.get_obj_mut(promise_id).kind {
            ObjectKind::Promise(p) => {
                if !matches!(p.state, PromiseState::Pending) {
                    return;
                }
                p.state = PromiseState::Rejected(reason.clone());
                std::mem::take(&mut p.reactions)
            }
            _ => return,
        };
        for r in reactions {
            self.schedule_promise_reaction(r, false, reason.clone());
        }
    }

    pub fn schedule_promise_reaction(
        &mut self,
        reaction: PromiseReaction,
        fulfilled: bool,
        arg: JsValue,
    ) {
        self.queue_native_microtask(move |rt| {
            let handler = if fulfilled {
                reaction.on_fulfilled
            } else {
                reaction.on_rejected
            };
            match handler {
                Some(h) if rt.is_callable(&h) => match rt.call_function(h, JsValue::Undefined, &[arg]) {
                    Ok(v) => {
                        if let Some(target) = reaction.target {
                            rt.resolve_promise(target, v);
                        }
                    }
                    Err(e) => {
                        if let Some(target) = reaction.target {
                            let ev = rt.error_to_js_value(e);
                            rt.reject_promise(target, ev);
                        }
                    }
                },
                _ => {
                    if let Some(target) = reaction.target {
                        if fulfilled {
                            rt.resolve_promise(target, arg);
                        } else {
                            rt.reject_promise(target, arg);
                        }
                    }
                }
            }
        });
    }

    pub fn promise_then(
        &mut self,
        promise_id: ObjectId,
        on_fulfilled: Option<JsValue>,
        on_rejected: Option<JsValue>,
    ) -> ObjectId {
        let next_promise = self.create_promise();
        let reaction = PromiseReaction {
            on_fulfilled: on_fulfilled.filter(|v| self.is_callable(v)),
            on_rejected: on_rejected.filter(|v| self.is_callable(v)),
            target: Some(next_promise),
        };
        let state = match &mut self.heap.get_obj_mut(promise_id).kind {
            ObjectKind::Promise(p) => {
                p.handled = true;
                match &p.state {
                    PromiseState::Pending => {
                        p.reactions.push(reaction);
                        return next_promise;
                    }
                    PromiseState::Fulfilled(v) => Some((true, v.clone())),
                    PromiseState::Rejected(r) => Some((false, r.clone())),
                }
            }
            _ => None,
        };
        if let Some((fulfilled, val)) = state {
            self.schedule_promise_reaction(reaction, fulfilled, val);
        }
        next_promise
    }

    pub fn create_iter_result(&mut self, value: JsValue, done: bool) -> JsValue {
        let obj = self.create_object();
        let _ = self.set_property(obj, "value", value);
        let _ = self.set_property(obj, "done", JsValue::Boolean(done));
        JsValue::Object(obj)
    }

    pub fn create_builtin_iterator(&mut self, kind: BuiltinIteratorKind) -> ObjectId {
        self.heap.alloc_object(JsObject::new(
            Some(self.intrinsics.iterator_proto),
            ObjectKind::Iterator(IteratorData { kind, index: 0 }),
        ))
    }

    pub fn exec_script_chunk(
        &mut self,
        chunk: Rc<Chunk>,
        env: EnvId,
        strict: bool,
    ) -> Result<JsValue, JsError> {
        self.hoist_into_env(&chunk, env, strict)?;
        let mut frame = CallFrame::new(chunk, env, None);
        self.active_frames.push(env);
        let res = self.run_frame(&mut frame);
        self.active_frames.pop();
        match res? {
            crate::exec::FrameOutcome::Return(v) => Ok(v),
            _ => Ok(JsValue::Undefined),
        }
    }
}

impl Default for JsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn compute_func_length(params: &[crate::ast::Param]) -> usize {
    let mut len = 0;
    for p in params {
        if p.rest || matches!(p.pattern, crate::ast::Pattern::Assign { .. }) {
            break;
        }
        len += 1;
    }
    len
}
