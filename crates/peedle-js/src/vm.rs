use crate::bytecode::{CallFrame, Chunk, FuncTemplate, PendingCompletion};
use crate::compiler::Compiler;
use crate::error::{ErrorKind, JsError};
use crate::heap::*;
use crate::parser::Parser;
use crate::regexp::CompiledRegExp;
use crate::value::*;
use std::collections::HashMap;
use std::rc::Rc;

pub type ModuleLoader = Box<dyn Fn(&str) -> Result<String, JsError>>;
pub type ModuleBytesLoader = Box<dyn Fn(&str) -> Result<Vec<u8>, JsError>>;

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
    pub dispose: u64,
    pub async_dispose: u64,
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
    pub suppressed_error_proto: ObjectId,
    pub disposable_stack_proto: ObjectId,
    pub async_disposable_stack_proto: ObjectId,
    pub shadow_realm_proto: ObjectId,
    pub abstract_module_source_proto: ObjectId,
    pub abstract_module_source_ctor: ObjectId,
    pub map_proto: ObjectId,
    pub set_proto: ObjectId,
    pub weak_map_proto: ObjectId,
    pub weak_set_proto: ObjectId,
    pub weak_ref_proto: ObjectId,
    pub finalization_registry_proto: ObjectId,
    pub promise_proto: ObjectId,
    pub array_buffer_proto: ObjectId,
    pub shared_array_buffer_proto: ObjectId,
    pub data_view_proto: ObjectId,
    pub typed_array_proto: ObjectId,
    pub typed_array_protos: [ObjectId; 11],
    pub iterator_proto: ObjectId,
    pub iterator_ctor: ObjectId,
    pub iterator_helper_proto: ObjectId,
    pub wrap_for_valid_iterator_proto: ObjectId,
    pub async_iterator_proto: ObjectId,
    pub array_iterator_proto: ObjectId,
    pub string_iterator_proto: ObjectId,
    pub map_iterator_proto: ObjectId,
    pub set_iterator_proto: ObjectId,
    pub regexp_string_iterator_proto: ObjectId,
    pub generator_proto: ObjectId,
    pub async_generator_proto: ObjectId,
    pub generator_function_proto: ObjectId,
    pub generator_function_ctor: ObjectId,
    pub async_function_proto: ObjectId,
    pub async_function_ctor: ObjectId,
    pub async_generator_function_proto: ObjectId,
    pub async_generator_function_ctor: ObjectId,
    pub throw_type_error: ObjectId,
    pub eval_fn: ObjectId,
    pub function_ctor: ObjectId,
    pub object_ctor: ObjectId,
    pub array_ctor: ObjectId,
    pub promise_ctor: ObjectId,
    pub regexp_ctor: ObjectId,
    pub array_buffer_ctor: ObjectId,
    pub shared_array_buffer_ctor: ObjectId,
    pub typed_array_ctors: [ObjectId; 11],
}

#[derive(Clone)]
pub struct Realm {
    pub global_object: ObjectId,
    pub global_env: EnvId,
    pub intrinsics: Intrinsics,
}

pub enum Microtask {
    Callback {
        func: JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    },
    Native(Box<dyn FnOnce(&mut JsRuntime)>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ModuleEvalState {
    Linking,
    Linked,
    Evaluating,
    Evaluated,
}

#[derive(Clone)]
pub struct ModuleDep {
    pub key: String,
    pub is_deferred: bool,
}

#[derive(Clone)]
pub struct ModuleRecord {
    pub key: String,
    pub path: String,
    pub env: EnvId,
    pub ns: ObjectId,
    pub deferred_ns: Option<ObjectId>,
    pub chunk: Option<Rc<Chunk>>,
    pub deps: Vec<ModuleDep>,
    pub eval_state: ModuleEvalState,
    pub eval_promise: Option<ObjectId>,
    pub eval_error: Option<JsValue>,
    pub has_tla: bool,
}

pub struct JsRuntime {
    pub heap: Heap,
    pub global_object: ObjectId,
    pub global_env: EnvId,
    pub intrinsics: Intrinsics,
    pub realms: Vec<Realm>,
    pub current_realm_id: usize,
    pub symbols: WellKnownSymbols,
    pub microtasks: Vec<Microtask>,
    pub console_lines: Vec<String>,
    pub active_frames: Vec<EnvId>,
    pub module_cache: HashMap<String, ObjectId>,
    pub module_records: HashMap<String, ModuleRecord>,
    pub current_module_stack: Vec<String>,
    pub module_link_depth: usize,
    pub pending_link_stars: Vec<(ObjectId, Vec<ObjectId>)>,
    pub pending_link_checks: Vec<(ObjectId, String)>,
    pub pending_link_keys: Vec<String>,
    pub current_module_exports: Vec<ObjectId>,
    pub module_loader: Option<ModuleLoader>,
    pub module_bytes_loader: Option<ModuleBytesLoader>,
    pub call_depth: usize,
    pub max_call_depth: usize,
    pub max_steps: Option<usize>,
    pub step_count: usize,
    pub can_block: bool,
    pub native_new_target: Option<JsValue>,
    pub call_stack_callees: Vec<ObjectId>,
    pub template_cache: HashMap<(usize, u64), ObjectId>,
    pub host_root_tracers: Vec<Box<dyn Fn(&mut Vec<ObjectId>)>>,
    pub async_waiters: Vec<(
        std::sync::Arc<SharedBufferBacking>,
        usize,
        std::sync::Arc<WaiterEntry>,
        ObjectId,
        Option<std::time::Instant>,
    )>,
}

fn alloc_realm_parts(heap: &mut Heap) -> Realm {
    let object_proto = heap.alloc_object(JsObject::new(None, ObjectKind::Ordinary));
    let function_proto = heap.alloc_object(JsObject::new(
        Some(object_proto),
        ObjectKind::NativeFunction(NativeFn {
            name: String::new(),
            length: 0,
            callback: Rc::new(|_, _, _| Ok(JsValue::Undefined)),
            is_constructor: false,
            handles_new_target: false,
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
    let suppressed_error_proto =
        heap.alloc_object(JsObject::new(Some(error_proto), ObjectKind::Ordinary));
    let disposable_stack_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let async_disposable_stack_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let shadow_realm_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let abstract_module_source_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let map_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let set_proto = heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let weak_map_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let weak_set_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let weak_ref_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let finalization_registry_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let promise_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let array_buffer_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let shared_array_buffer_proto =
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
    let iterator_helper_proto =
        heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
    let wrap_for_valid_iterator_proto =
        heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
    let async_iterator_proto =
        heap.alloc_object(JsObject::new(Some(object_proto), ObjectKind::Ordinary));
    let array_iterator_proto =
        heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
    let string_iterator_proto =
        heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
    let map_iterator_proto =
        heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
    let set_iterator_proto =
        heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
    let regexp_string_iterator_proto =
        heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
    let generator_proto =
        heap.alloc_object(JsObject::new(Some(iterator_proto), ObjectKind::Ordinary));
    let async_generator_proto =
        heap.alloc_object(JsObject::new(Some(async_iterator_proto), ObjectKind::Ordinary));
    let generator_function_proto =
        heap.alloc_object(JsObject::new(Some(function_proto), ObjectKind::Ordinary));
    let async_function_proto =
        heap.alloc_object(JsObject::new(Some(function_proto), ObjectKind::Ordinary));
    let async_generator_function_proto =
        heap.alloc_object(JsObject::new(Some(function_proto), ObjectKind::Ordinary));
    let mut tte_obj = JsObject::new(
        Some(function_proto),
        ObjectKind::NativeFunction(NativeFn {
            name: String::new(),
            length: 0,
            callback: Rc::new(|_, _, _| {
                Err(JsError::type_err(
                    "'caller', 'callee', and 'arguments' properties may not be accessed on strict mode functions or the arguments objects for calls to them",
                ))
            }),
            is_constructor: false,
            handles_new_target: false,
        }),
    );
    tte_obj.set_own_prop(
        PropertyKey::from("length"),
        PropEntry::data(JsValue::Number(0.0), false, false, false),
    );
    tte_obj.set_own_prop(
        PropertyKey::from("name"),
        PropEntry::data(JsValue::String(String::new()), false, false, false),
    );
    tte_obj.extensible = false;
    let throw_type_error = heap.alloc_object(tte_obj);

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
        suppressed_error_proto,
        disposable_stack_proto,
        async_disposable_stack_proto,
        shadow_realm_proto,
        abstract_module_source_proto,
        abstract_module_source_ctor: function_proto,
        map_proto,
        set_proto,
        weak_map_proto,
        weak_set_proto,
        weak_ref_proto,
        finalization_registry_proto,
        promise_proto,
        array_buffer_proto,
        shared_array_buffer_proto,
        data_view_proto,
        typed_array_proto,
        typed_array_protos,
        iterator_proto,
        iterator_ctor: function_proto,
        iterator_helper_proto,
        wrap_for_valid_iterator_proto,
        async_iterator_proto,
        array_iterator_proto,
        string_iterator_proto,
        map_iterator_proto,
        set_iterator_proto,
        regexp_string_iterator_proto,
        generator_proto,
        async_generator_proto,
        generator_function_proto,
        generator_function_ctor: function_proto,
        async_function_proto,
        async_function_ctor: function_proto,
        async_generator_function_proto,
        async_generator_function_ctor: function_proto,
        throw_type_error,
        eval_fn: function_proto,
        function_ctor: function_proto,
        object_ctor: function_proto,
        array_ctor: function_proto,
        promise_ctor: function_proto,
        regexp_ctor: function_proto,
        array_buffer_ctor: function_proto,
        shared_array_buffer_ctor: function_proto,
        typed_array_ctors: [function_proto; 11],
    };

    Realm {
        global_object,
        global_env,
        intrinsics,
    }
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
            dispose: heap.create_symbol(Some("Symbol.dispose".into())),
            async_dispose: heap.create_symbol(Some("Symbol.asyncDispose".into())),
        };

        let realm0 = alloc_realm_parts(&mut heap);
        heap.object_realms.insert(realm0.intrinsics.function_proto, 0);

        let mut rt = JsRuntime {
            heap,
            global_object: realm0.global_object,
            global_env: realm0.global_env,
            intrinsics: realm0.intrinsics.clone(),
            realms: vec![realm0],
            current_realm_id: 0,
            symbols,
            microtasks: Vec::new(),
            console_lines: Vec::new(),
            active_frames: Vec::new(),
            module_cache: HashMap::new(),
            module_records: HashMap::new(),
            current_module_stack: Vec::new(),
            module_link_depth: 0,
            pending_link_stars: Vec::new(),
            pending_link_checks: Vec::new(),
            pending_link_keys: Vec::new(),
            current_module_exports: Vec::new(),
            module_loader: None,
            module_bytes_loader: None,
            call_depth: 0,
            max_call_depth: 256,
            max_steps: None,
            step_count: 0,
            native_new_target: None,
            call_stack_callees: Vec::new(),
            template_cache: HashMap::new(),
            host_root_tracers: Vec::new(),
            async_waiters: Vec::new(),
            can_block: true,
        };

        crate::builtins::init_builtins(&mut rt);
        rt.realms[0].intrinsics = rt.intrinsics.clone();
        rt
    }

    pub fn create_realm(&mut self) -> usize {
        let prev = self.current_realm_id;
        self.realms[prev].global_object = self.global_object;
        self.realms[prev].global_env = self.global_env;
        self.realms[prev].intrinsics = self.intrinsics.clone();

        let new_id = self.realms.len();
        let realm = alloc_realm_parts(&mut self.heap);
        self.heap.object_realms.insert(realm.intrinsics.function_proto, new_id);
        self.global_object = realm.global_object;
        self.global_env = realm.global_env;
        self.intrinsics = realm.intrinsics.clone();
        self.realms.push(realm);
        self.current_realm_id = new_id;

        crate::builtins::init_builtins(self);
        self.realms[new_id].intrinsics = self.intrinsics.clone();

        self.current_realm_id = prev;
        self.global_object = self.realms[prev].global_object;
        self.global_env = self.realms[prev].global_env;
        self.intrinsics = self.realms[prev].intrinsics.clone();
        new_id
    }

    pub fn switch_realm(&mut self, realm_id: usize) -> usize {
        let prev = self.current_realm_id;
        if prev == realm_id || realm_id >= self.realms.len() {
            return prev;
        }
        self.realms[prev].global_object = self.global_object;
        self.realms[prev].global_env = self.global_env;
        self.realms[prev].intrinsics = self.intrinsics.clone();
        self.current_realm_id = realm_id;
        self.global_object = self.realms[realm_id].global_object;
        self.global_env = self.realms[realm_id].global_env;
        self.intrinsics = self.realms[realm_id].intrinsics.clone();
        prev
    }

    pub fn eval_in_realm(&mut self, realm_id: usize, src: &str) -> Result<JsValue, JsError> {
        let prev = self.switch_realm(realm_id);
        let res = (|| {
            let program = Parser::parse_script(src)?;
            let chunk = Compiler::compile_program(&program)?;
            self.exec_script_chunk(chunk, self.global_env, program.strict)
        })();
        self.switch_realm(prev);
        res
    }

    pub fn get_function_realm(&mut self, mut obj: ObjectId) -> Result<usize, JsError> {
        for _ in 0..256 {
            match &self.heap.get_obj(obj).kind {
                ObjectKind::Proxy(p) => {
                    if p.revoked {
                        return Err(JsError::type_err(
                            "Cannot determine realm of a revoked Proxy",
                        ));
                    }
                    obj = p.target;
                }
                ObjectKind::Function(f) => {
                    if let Some(bt) = f.bound_target {
                        obj = bt;
                    } else {
                        return Ok(self
                            .heap
                            .object_realms
                            .get(&obj)
                            .copied()
                            .unwrap_or(self.current_realm_id));
                    }
                }
                ObjectKind::WrappedFunction { caller_realm, .. } => {
                    return Ok(*caller_realm);
                }
                _ => {
                    return Ok(self
                        .heap
                        .object_realms
                        .get(&obj)
                        .copied()
                        .unwrap_or(self.current_realm_id));
                }
            }
        }
        Ok(self.current_realm_id)
    }

    pub fn realm_proto_for(&self, realm_id: usize, default_proto: ObjectId) -> ObjectId {
        if realm_id == self.current_realm_id || realm_id >= self.realms.len() {
            return default_proto;
        }
        let cur = &self.intrinsics;
        let tgt = &self.realms[realm_id].intrinsics;
        if default_proto == cur.object_proto { return tgt.object_proto; }
        if default_proto == cur.function_proto { return tgt.function_proto; }
        if default_proto == cur.array_proto { return tgt.array_proto; }
        if default_proto == cur.string_proto { return tgt.string_proto; }
        if default_proto == cur.number_proto { return tgt.number_proto; }
        if default_proto == cur.boolean_proto { return tgt.boolean_proto; }
        if default_proto == cur.symbol_proto { return tgt.symbol_proto; }
        if default_proto == cur.bigint_proto { return tgt.bigint_proto; }
        if default_proto == cur.regexp_proto { return tgt.regexp_proto; }
        if default_proto == cur.date_proto { return tgt.date_proto; }
        if default_proto == cur.error_proto { return tgt.error_proto; }
        if default_proto == cur.type_error_proto { return tgt.type_error_proto; }
        if default_proto == cur.reference_error_proto { return tgt.reference_error_proto; }
        if default_proto == cur.syntax_error_proto { return tgt.syntax_error_proto; }
        if default_proto == cur.range_error_proto { return tgt.range_error_proto; }
        if default_proto == cur.uri_error_proto { return tgt.uri_error_proto; }
        if default_proto == cur.eval_error_proto { return tgt.eval_error_proto; }
        if default_proto == cur.aggregate_error_proto { return tgt.aggregate_error_proto; }
        if default_proto == cur.suppressed_error_proto { return tgt.suppressed_error_proto; }
        if default_proto == cur.disposable_stack_proto { return tgt.disposable_stack_proto; }
        if default_proto == cur.async_disposable_stack_proto { return tgt.async_disposable_stack_proto; }
        if default_proto == cur.shadow_realm_proto { return tgt.shadow_realm_proto; }
        if default_proto == cur.abstract_module_source_proto { return tgt.abstract_module_source_proto; }
        if default_proto == cur.map_proto { return tgt.map_proto; }
        if default_proto == cur.set_proto { return tgt.set_proto; }
        if default_proto == cur.weak_map_proto { return tgt.weak_map_proto; }
        if default_proto == cur.weak_set_proto { return tgt.weak_set_proto; }
        if default_proto == cur.weak_ref_proto { return tgt.weak_ref_proto; }
        if default_proto == cur.finalization_registry_proto { return tgt.finalization_registry_proto; }
        if default_proto == cur.promise_proto { return tgt.promise_proto; }
        if default_proto == cur.array_buffer_proto { return tgt.array_buffer_proto; }
        if default_proto == cur.shared_array_buffer_proto { return tgt.shared_array_buffer_proto; }
        if default_proto == cur.data_view_proto { return tgt.data_view_proto; }
        if default_proto == cur.typed_array_proto { return tgt.typed_array_proto; }
        if default_proto == cur.iterator_proto { return tgt.iterator_proto; }
        if default_proto == cur.async_iterator_proto { return tgt.async_iterator_proto; }
        if default_proto == cur.array_iterator_proto { return tgt.array_iterator_proto; }
        if default_proto == cur.string_iterator_proto { return tgt.string_iterator_proto; }
        if default_proto == cur.map_iterator_proto { return tgt.map_iterator_proto; }
        if default_proto == cur.set_iterator_proto { return tgt.set_iterator_proto; }
        if default_proto == cur.regexp_string_iterator_proto { return tgt.regexp_string_iterator_proto; }
        if default_proto == cur.generator_proto { return tgt.generator_proto; }
        if default_proto == cur.async_generator_proto { return tgt.async_generator_proto; }
        if default_proto == cur.generator_function_proto { return tgt.generator_function_proto; }
        if default_proto == cur.async_function_proto { return tgt.async_function_proto; }
        if default_proto == cur.async_generator_function_proto { return tgt.async_generator_function_proto; }
        for i in 0..11 {
            if default_proto == cur.typed_array_protos[i] {
                return tgt.typed_array_protos[i];
            }
        }
        default_proto
    }

    pub fn set_module_loader<F>(&mut self, loader: F)
    where
        F: Fn(&str) -> Result<String, JsError> + 'static,
    {
        self.module_loader = Some(Box::new(loader));
    }

    pub fn set_module_bytes_loader<F>(&mut self, loader: F)
    where
        F: Fn(&str) -> Result<Vec<u8>, JsError> + 'static,
    {
        self.module_bytes_loader = Some(Box::new(loader));
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
        let mut eval_ctx = crate::parser::EvalValidationCtx::default();
        let mut cur_e = Some(env);
        let mut found_this_env = false;
        while let Some(eid) = cur_e {
            let rec = self.heap.get_env(eid);
            if rec.allow_private {
                eval_ctx.allow_private = true;
            }
            for pn in rec.private_bindings.keys() {
                eval_ctx.declared_privates.insert(pn.clone());
                eval_ctx.allow_private = true;
            }
            if !found_this_env && rec.this_binding.is_some() {
                found_this_env = true;
                eval_ctx.allow_super_call = rec.allow_super_call;
                eval_ctx.allow_super_prop = rec.allow_super_prop;
                eval_ctx.allow_new_target = rec.allow_new_target;
                eval_ctx.forbid_arguments = rec.forbid_arguments;
            }
            cur_e = rec.parent;
        }
        let program = Parser::parse_eval(src, strict, eval_ctx)?;
        let chunk = Compiler::compile_program(&program)?;
        let eval_strict = strict || program.strict;
        if eval_strict {
            let eval_env = self.heap.alloc_env(EnvRecord::new(Some(env), true, true));
            return self.exec_script_chunk(chunk, eval_env, true);
        }

        let mut var_env = env;
        loop {
            let rec = self.heap.get_env(var_env);
            if rec.is_function_scope {
                break;
            }
            match rec.parent {
                Some(p) => var_env = p,
                None => break,
            }
        }

        let mut eval_var_names: Vec<&str> = Vec::new();
        for vn in &chunk.hoisted_vars {
            eval_var_names.push(vn.as_str());
        }
        for (fn_name, _) in &chunk.hoisted_funcs {
            eval_var_names.push(fn_name.as_str());
        }

        for &vn in &eval_var_names {
            let mut cur = env;
            loop {
                let rec = self.heap.get_env(cur);
                if cur == var_env {
                    if rec.lex_names.contains(vn) {
                        return Err(JsError::syntax(format!(
                            "Identifier '{}' has already been declared",
                            vn
                        )));
                    }
                    break;
                }
                if rec.object_env.is_none() && rec.bindings.contains_key(vn) {
                    return Err(JsError::syntax(format!(
                        "Identifier '{}' has already been declared",
                        vn
                    )));
                }
                match rec.parent {
                    Some(p) => cur = p,
                    None => break,
                }
            }
        }

        let is_global_var = self.heap.get_env(var_env).object_env == Some(self.global_object);
        if is_global_var {
            for (name, _) in &chunk.hoisted_funcs {
                let key = PropertyKey::String(name.clone());
                let existing = self.heap.get_obj(self.global_object).get_own_prop(&key).cloned();
                if let Some(ex) = existing {
                    if !ex.configurable
                        && (matches!(ex.value, PropValue::Accessor { .. })
                            || !ex.writable
                            || !ex.enumerable)
                    {
                        return Err(JsError::type_err("Cannot redefine global function"));
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
        }
        for name in &chunk.hoisted_vars {
            if is_global_var {
                let key = PropertyKey::String(name.clone());
                if !self.has_own_property(self.global_object, &key)? {
                    self.heap.get_obj_mut(self.global_object).set_own_prop(
                        key,
                        PropEntry::data(JsValue::Undefined, true, true, true),
                    );
                }
            } else {
                let e = self.heap.get_env_mut(var_env);
                e.bindings.entry(name.clone()).or_insert(Binding {
                    value: JsValue::Undefined,
                    mutable: true,
                    initialized: true,
                    deletable: true,
                    strict_immutable: false,
                });
            }
        }
        for (name, tmpl) in &chunk.hoisted_funcs {
            let fn_id = self.create_js_function(tmpl, var_env);
            if is_global_var {
                let key = PropertyKey::String(name.clone());
                let existing = self.heap.get_obj(self.global_object).get_own_prop(&key).cloned();
                if let Some(ex) = existing {
                    if !ex.configurable {
                        self.set_property(self.global_object, name, JsValue::Object(fn_id))?;
                    } else {
                        self.heap.get_obj_mut(self.global_object).set_own_prop(
                            key,
                            PropEntry::data(JsValue::Object(fn_id), true, true, true),
                        );
                    }
                } else {
                    self.heap.get_obj_mut(self.global_object).set_own_prop(
                        key,
                        PropEntry::data(JsValue::Object(fn_id), true, true, true),
                    );
                }
            } else {
                let e = self.heap.get_env_mut(var_env);
                if let Some(existing) = e.bindings.get_mut(name) {
                    existing.value = JsValue::Object(fn_id);
                    existing.initialized = true;
                } else {
                    e.bindings.insert(
                        name.clone(),
                        Binding {
                            value: JsValue::Object(fn_id),
                            mutable: true,
                            initialized: true,
                            deletable: true,
                            strict_immutable: false,
                        },
                    );
                }
            }
        }

        let eval_lex_env = self.heap.alloc_env(EnvRecord::new(Some(env), false, false));
        for (name, is_const) in &chunk.lex_names {
            let e = self.heap.get_env_mut(eval_lex_env);
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

        let mut frame = CallFrame::new(chunk, eval_lex_env, None);
        self.active_frames.push(eval_lex_env);
        let res = self.run_frame(&mut frame);
        self.active_frames.pop();
        match res? {
            crate::exec::FrameOutcome::Return(v) => Ok(v),
            _ => Ok(JsValue::Undefined),
        }
    }

    pub fn eval_module(&mut self, specifier: &str, src: &str) -> Result<JsValue, JsError> {
        let key = normalize_module_path(specifier);
        let ns = self.link_module_from_source(&key, &key, src, &[])?;
        let eval_p = self.evaluate_module(&key)?;
        self.drain_microtasks();
        if let Some(p) = eval_p {
            let state = match &self.heap.get_obj(p).kind {
                ObjectKind::Promise(pd) => pd.state.clone(),
                _ => PromiseState::Fulfilled(JsValue::Undefined),
            };
            if let PromiseState::Rejected(err) = state {
                return Err(JsError::Thrown(err));
            }
        } else if let Some(rec) = self.module_records.get(&key) {
            if let Some(ref err) = rec.eval_error {
                return Err(JsError::Thrown(err.clone()));
            }
        }
        Ok(JsValue::Object(ns))
    }

    pub fn eval_module_namespace(
        &mut self,
        specifier: &str,
        src: &str,
    ) -> Result<ObjectId, JsError> {
        let key = normalize_module_path(specifier);
        let ns = self.link_module_from_source(&key, &key, src, &[])?;
        let eval_p = self.evaluate_module(&key)?;
        self.drain_microtasks();
        if let Some(p) = eval_p {
            let state = match &self.heap.get_obj(p).kind {
                ObjectKind::Promise(pd) => pd.state.clone(),
                _ => PromiseState::Fulfilled(JsValue::Undefined),
            };
            if let PromiseState::Rejected(err) = state {
                return Err(JsError::Thrown(err));
            }
        }
        Ok(ns)
    }

    pub fn load_module(&mut self, specifier: &str) -> Result<ObjectId, JsError> {
        let (ns, eval_p) = self.load_module_async(specifier, &[])?;
        if let Some(p) = eval_p {
            self.drain_microtasks();
            let state = match &self.heap.get_obj(p).kind {
                ObjectKind::Promise(pd) => pd.state.clone(),
                _ => PromiseState::Fulfilled(JsValue::Undefined),
            };
            if let PromiseState::Rejected(err) = state {
                return Err(JsError::Thrown(err));
            }
        }
        Ok(ns)
    }

    pub fn abstract_module_source_ctor(&self) -> ObjectId {
        self.intrinsics.abstract_module_source_ctor
    }

    pub fn load_module_in_realm(
        &mut self,
        realm_id: usize,
        specifier: &str,
    ) -> Result<(ObjectId, Option<ObjectId>), JsError> {
        let (raw_key, path) = self.resolve_module_key_and_path(specifier, &[]);
        let key = format!("\0sr:{}:{}", realm_id, raw_key);
        let prev = self.switch_realm(realm_id);
        let res = (|| -> Result<(ObjectId, Option<ObjectId>), JsError> {
            let ns = self.link_module_by_key(&key, &path, &[])?;
            let eval_p = self.evaluate_module(&key)?;
            Ok((ns, eval_p))
        })();
        self.switch_realm(prev);
        res
    }

    pub fn load_module_async(
        &mut self,
        specifier: &str,
        attrs: &[(String, String)],
    ) -> Result<(ObjectId, Option<ObjectId>), JsError> {
        let (key, path) = self.resolve_module_key_and_path(specifier, attrs);
        let ns = self.link_module_by_key(&key, &path, attrs)?;
        let eval_p = self.evaluate_module(&key)?;
        Ok((ns, eval_p))
    }

    pub fn load_module_deferred(
        &mut self,
        specifier: &str,
        attrs: &[(String, String)],
    ) -> Result<ObjectId, JsError> {
        let (key, path) = self.resolve_module_key_and_path(specifier, attrs);
        self.link_module_by_key(&key, &path, attrs)?;
        Ok(self.get_or_create_deferred_ns(&key))
    }

    fn resolve_module_key_and_path(
        &self,
        specifier: &str,
        attrs: &[(String, String)],
    ) -> (String, String) {
        let resolved = if specifier.starts_with('/') {
            normalize_module_path(specifier)
        } else if let Some(referrer) = self.current_module_stack.last() {
            if let Some(parent) = std::path::Path::new(referrer).parent() {
                if !parent.as_os_str().is_empty() {
                    let joined = parent.join(specifier);
                    normalize_module_path(&joined.to_string_lossy())
                } else {
                    specifier.to_string()
                }
            } else {
                specifier.to_string()
            }
        } else {
            specifier.to_string()
        };
        let mut key = normalize_module_path(&resolved);
        if let Some((_, ty)) = attrs.iter().find(|(k, _)| k == "type") {
            key.push_str("\0type=");
            key.push_str(ty);
        }
        (key, resolved)
    }

    fn alloc_module_namespace(&mut self, is_deferred: bool, module_key: Option<String>) -> ObjectId {
        let mut obj = JsObject::new(
            None,
            ObjectKind::ModuleNamespace(ModuleNamespaceData {
                exports: Vec::new(),
                is_deferred,
                module_key,
            }),
        );
        obj.extensible = false;
        let tag = self.symbols.to_string_tag;
        let tag_str = if is_deferred {
            "Deferred Module"
        } else {
            "Module"
        };
        obj.set_own_prop(
            PropertyKey::Symbol(tag),
            PropEntry::data(JsValue::String(tag_str.into()), false, false, false),
        );
        self.heap.alloc_object(obj)
    }

    fn get_or_create_deferred_ns(&mut self, key: &str) -> ObjectId {
        if let Some(rec) = self.module_records.get(key) {
            if let Some(dns) = rec.deferred_ns {
                return dns;
            }
            let exports = match &self.heap.get_obj(rec.ns).kind {
                ObjectKind::ModuleNamespace(m) => m.exports.clone(),
                _ => Vec::new(),
            };
            let dns = self.alloc_module_namespace(true, Some(key.to_string()));
            if let ObjectKind::ModuleNamespace(ref mut m) = self.heap.get_obj_mut(dns).kind {
                m.exports = exports;
            }
            if let Some(rec_mut) = self.module_records.get_mut(key) {
                rec_mut.deferred_ns = Some(dns);
            }
            return dns;
        }
        self.alloc_module_namespace(true, Some(key.to_string()))
    }

    fn link_module_by_key(
        &mut self,
        key: &str,
        path: &str,
        attrs: &[(String, String)],
    ) -> Result<ObjectId, JsError> {
        if let Some(rec) = self.module_records.get(key) {
            return Ok(rec.ns);
        }
        if attrs.iter().any(|(k, v)| k == "type" && v == "bytes") {
            let bytes = if let Some(ref bl) = self.module_bytes_loader {
                bl(path)?
            } else if let Some(ref loader) = self.module_loader {
                loader(path)?.into_bytes()
            } else {
                return Err(JsError::error(format!(
                    "Cannot load module '{}': no module loader configured",
                    path
                )));
            };
            let len = bytes.len();
            let buf = self.heap.alloc_object(JsObject::new(
                Some(self.intrinsics.array_buffer_proto),
                ObjectKind::ArrayBuffer(ArrayBufferData {
                    bytes,
                    detached: false,
                }),
            ));
            self.heap.buffer_meta.insert(
                buf,
                BufferMeta {
                    max_byte_length: None,
                    is_shared: false,
                    immutable: true,
                    shared: None,
                },
            );
            let ta_proto = self.intrinsics.typed_array_protos[1];
            let ta = self.heap.alloc_object(JsObject::new(
                Some(ta_proto),
                ObjectKind::TypedArray(TypedArrayData {
                    kind: TypedArrayKind::Uint8,
                    buffer: buf,
                    byte_offset: 0,
                    length: len,
                }),
            ));
            let ns = self.alloc_module_namespace(false, Some(key.to_string()));
            if let ObjectKind::ModuleNamespace(ref mut m) = self.heap.get_obj_mut(ns).kind {
                m.exports
                    .push(("default".to_string(), ExportTarget::Value(JsValue::Object(ta))));
            }
            let mod_env = self
                .heap
                .alloc_env(EnvRecord::new(Some(self.global_env), true, true));
            self.module_cache.insert(key.to_string(), ns);
            self.module_records.insert(
                key.to_string(),
                ModuleRecord {
                    key: key.to_string(),
                    path: path.to_string(),
                    env: mod_env,
                    ns,
                    deferred_ns: None,
                    chunk: None,
                    deps: Vec::new(),
                    eval_state: ModuleEvalState::Evaluated,
                    eval_promise: None,
                    eval_error: None,
                    has_tla: false,
                },
            );
            return Ok(ns);
        }
        let src = if let Some(ref loader) = self.module_loader {
            loader(path)?
        } else {
            return Err(JsError::error(format!(
                "Cannot load module '{}': no module loader configured",
                path
            )));
        };
        self.link_module_from_source(key, path, &src, attrs)
    }

    fn link_module_from_source(
        &mut self,
        key: &str,
        path: &str,
        src: &str,
        attrs: &[(String, String)],
    ) -> Result<ObjectId, JsError> {
        if let Some(rec) = self.module_records.get(key) {
            return Ok(rec.ns);
        }
        if let Some((_, ty)) = attrs.iter().find(|(k, _)| k == "type") {
            let val = match ty.as_str() {
                "json" => crate::builtins::json::parse_json_str(self, src)?,
                "text" => JsValue::String(src.to_string()),
                "bytes" => {
                    let bytes = src.as_bytes().to_vec();
                    let len = bytes.len();
                    let buf = self.heap.alloc_object(JsObject::new(
                        Some(self.intrinsics.array_buffer_proto),
                        ObjectKind::ArrayBuffer(ArrayBufferData {
                            bytes,
                            detached: false,
                        }),
                    ));
                    self.heap.buffer_meta.insert(
                        buf,
                        BufferMeta {
                            max_byte_length: None,
                            is_shared: false,
                            immutable: true,
                            shared: None,
                        },
                    );
                    let ta_proto = self.intrinsics.typed_array_protos[1];
                    let ta = self.heap.alloc_object(JsObject::new(
                        Some(ta_proto),
                        ObjectKind::TypedArray(TypedArrayData {
                            kind: TypedArrayKind::Uint8,
                            buffer: buf,
                            byte_offset: 0,
                            length: len,
                        }),
                    ));
                    JsValue::Object(ta)
                }
                other => {
                    return Err(JsError::type_err(format!(
                        "Unsupported import attribute type '{}'",
                        other
                    )));
                }
            };
            let ns = self.alloc_module_namespace(false, Some(key.to_string()));
            if let ObjectKind::ModuleNamespace(ref mut m) = self.heap.get_obj_mut(ns).kind {
                m.exports.push(("default".to_string(), ExportTarget::Value(val)));
            }
            let mod_env = self
                .heap
                .alloc_env(EnvRecord::new(Some(self.global_env), true, true));
            self.module_cache.insert(key.to_string(), ns);
            self.module_records.insert(
                key.to_string(),
                ModuleRecord {
                    key: key.to_string(),
                    path: path.to_string(),
                    env: mod_env,
                    ns,
                    deferred_ns: None,
                    chunk: None,
                    deps: Vec::new(),
                    eval_state: ModuleEvalState::Evaluated,
                    eval_promise: None,
                    eval_error: None,
                    has_tla: false,
                },
            );
            return Ok(ns);
        }

        let program = Parser::parse_module(src)?;
        let chunk = Compiler::compile_program(&program)?;
        let has_tla = chunk.ops.iter().any(|op| matches!(op, crate::bytecode::Op::Await));

        let mut mod_env_rec = EnvRecord::new(Some(self.global_env), true, true);
        mod_env_rec.this_binding = Some(JsValue::Undefined);
        let mod_env = self.heap.alloc_env(mod_env_rec);
        let meta_obj = self.create_object_with_proto(None);
        self.heap.get_env_mut(mod_env).bindings.insert(
            "\0__import_meta".to_string(),
            Binding {
                value: JsValue::Object(meta_obj),
                mutable: false,
                initialized: true,
                deletable: false,
                strict_immutable: true,
            },
        );
        self.heap.get_env_mut(mod_env).bindings.insert(
            "\0__module_path".to_string(),
            Binding {
                value: JsValue::String(path.to_string()),
                mutable: false,
                initialized: true,
                deletable: false,
                strict_immutable: true,
            },
        );

        let ns = self.alloc_module_namespace(false, Some(key.to_string()));
        self.module_cache.insert(key.to_string(), ns);
        self.module_records.insert(
            key.to_string(),
            ModuleRecord {
                key: key.to_string(),
                path: path.to_string(),
                env: mod_env,
                ns,
                deferred_ns: None,
                chunk: Some(chunk.clone()),
                deps: Vec::new(),
                eval_state: ModuleEvalState::Linking,
                eval_promise: None,
                eval_error: None,
                has_tla,
            },
        );

        self.module_link_depth += 1;
        self.pending_link_keys.push(key.to_string());

        let link_res: Result<(), JsError> = (|| {
            self.hoist_into_env(&chunk, mod_env, true)?;

            let mut local_exports: Vec<(String, ExportTarget)> = Vec::new();
            for stmt in &program.body {
                if let crate::ast::Stmt::Export(exp) = stmt {
                    match exp {
                        crate::ast::ExportDecl::DefaultExpr(_) => {
                            local_exports.push((
                                "default".to_string(),
                                ExportTarget::Local {
                                    env: mod_env,
                                    name: "\0__default".to_string(),
                                },
                            ));
                        }
                        crate::ast::ExportDecl::DefaultFunc(f) => {
                            let local_name = f
                                .name
                                .clone()
                                .unwrap_or_else(|| "\0__default".to_string());
                            local_exports.push((
                                "default".to_string(),
                                ExportTarget::Local {
                                    env: mod_env,
                                    name: local_name,
                                },
                            ));
                        }
                        crate::ast::ExportDecl::DefaultClass(c) => {
                            let local_name = c
                                .name
                                .clone()
                                .unwrap_or_else(|| "\0__default".to_string());
                            local_exports.push((
                                "default".to_string(),
                                ExportTarget::Local {
                                    env: mod_env,
                                    name: local_name,
                                },
                            ));
                        }
                        crate::ast::ExportDecl::Decl(inner) => {
                            let mut names = Vec::new();
                            match inner.as_ref() {
                                crate::ast::Stmt::VarDecl { decls, .. } => {
                                    for d in decls {
                                        crate::compiler::collect_pattern_names(
                                            &d.pattern, &mut names, false,
                                        );
                                    }
                                }
                                crate::ast::Stmt::FuncDecl(f) => {
                                    if let Some(ref n) = f.name {
                                        names.push((n.clone(), false));
                                    }
                                }
                                crate::ast::Stmt::ClassDecl(c) => {
                                    if let Some(ref n) = c.name {
                                        names.push((n.clone(), false));
                                    }
                                }
                                _ => {}
                            }
                            for (n, _) in names {
                                local_exports.push((
                                    n.clone(),
                                    ExportTarget::Local {
                                        env: mod_env,
                                        name: n,
                                    },
                                ));
                            }
                        }
                        crate::ast::ExportDecl::Named {
                            specifiers,
                            source: None,
                            ..
                        } => {
                            for sp in specifiers {
                                local_exports.push((
                                    sp.exported.clone(),
                                    ExportTarget::Local {
                                        env: mod_env,
                                        name: sp.local.clone(),
                                    },
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let ObjectKind::ModuleNamespace(ref mut m) = self.heap.get_obj_mut(ns).kind {
                m.exports = local_exports;
            }

            let mut deps: Vec<ModuleDep> = Vec::new();
            let mut extra_exports: Vec<(String, ExportTarget)> = Vec::new();
            let mut star_sources: Vec<ObjectId> = Vec::new();
            let mut import_wires: Vec<(String, ObjectId, Option<String>, bool, String)> = Vec::new();

            self.current_module_stack.push(path.to_string());
            let body_res: Result<(), JsError> = (|| {
                for stmt in &program.body {
                    match stmt {
                        crate::ast::Stmt::Import(imp) => {
                            if imp.phase == crate::ast::ImportPhase::Source {
                                return Err(JsError::syntax(
                                    "Source phase import is not supported for source text modules",
                                ));
                            }
                            let (dep_key, dep_path) =
                                self.resolve_module_key_and_path(&imp.source, &imp.attributes);
                            let dep_ns =
                                self.link_module_by_key(&dep_key, &dep_path, &imp.attributes)?;
                            let is_def = imp.phase == crate::ast::ImportPhase::Defer;
                            deps.push(ModuleDep {
                                key: dep_key.clone(),
                                is_deferred: is_def,
                            });
                            for sp in &imp.specifiers {
                                match sp {
                                    crate::ast::ImportSpecifier::Default(local) => {
                                        self.pending_link_checks
                                            .push((dep_ns, "default".to_string()));
                                        import_wires.push((
                                            local.clone(),
                                            dep_ns,
                                            Some("default".to_string()),
                                            false,
                                            dep_key.clone(),
                                        ));
                                    }
                                    crate::ast::ImportSpecifier::Named { imported, local } => {
                                        self.pending_link_checks.push((dep_ns, imported.clone()));
                                        import_wires.push((
                                            local.clone(),
                                            dep_ns,
                                            Some(imported.clone()),
                                            false,
                                            dep_key.clone(),
                                        ));
                                    }
                                    crate::ast::ImportSpecifier::Namespace(local) => {
                                        import_wires.push((
                                            local.clone(),
                                            dep_ns,
                                            None,
                                            is_def,
                                            dep_key.clone(),
                                        ));
                                    }
                                }
                            }
                        }
                        crate::ast::Stmt::Export(exp) => match exp {
                            crate::ast::ExportDecl::Named {
                                specifiers,
                                source: Some(ref src_spec),
                                attributes,
                            } => {
                                let (dep_key, dep_path) =
                                    self.resolve_module_key_and_path(src_spec, attributes);
                                let dep_ns =
                                    self.link_module_by_key(&dep_key, &dep_path, attributes)?;
                                deps.push(ModuleDep {
                                    key: dep_key,
                                    is_deferred: false,
                                });
                                for sp in specifiers {
                                    self.pending_link_checks.push((dep_ns, sp.local.clone()));
                                    extra_exports.push((
                                        sp.exported.clone(),
                                        ExportTarget::Indirect {
                                            ns: dep_ns,
                                            export_name: sp.local.clone(),
                                        },
                                    ));
                                }
                            }
                            crate::ast::ExportDecl::All {
                                exported,
                                source,
                                attributes,
                            } => {
                                let (dep_key, dep_path) =
                                    self.resolve_module_key_and_path(source, attributes);
                                let dep_ns =
                                    self.link_module_by_key(&dep_key, &dep_path, attributes)?;
                                deps.push(ModuleDep {
                                    key: dep_key,
                                    is_deferred: false,
                                });
                                if let Some(ref name) = exported {
                                    extra_exports.push((
                                        name.clone(),
                                        ExportTarget::Value(JsValue::Object(dep_ns)),
                                    ));
                                } else {
                                    star_sources.push(dep_ns);
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
                Ok(())
            })();
            self.current_module_stack.pop();
            body_res?;

            for (local, dep_ns, maybe_imp, is_def, dep_key) in import_wires {
                if let Some(imp_name) = maybe_imp {
                    self.heap
                        .get_env_mut(mod_env)
                        .import_bindings
                        .insert(local, (dep_ns, imp_name));
                } else {
                    let target_ns = if is_def {
                        self.get_or_create_deferred_ns(&dep_key)
                    } else {
                        dep_ns
                    };
                    self.heap.get_env_mut(mod_env).bindings.insert(
                        local,
                        Binding {
                            value: JsValue::Object(target_ns),
                            mutable: false,
                            initialized: true,
                            deletable: false,
                            strict_immutable: true,
                        },
                    );
                }
            }

            if !extra_exports.is_empty() {
                if let ObjectKind::ModuleNamespace(ref mut m) = self.heap.get_obj_mut(ns).kind {
                    m.exports.extend(extra_exports);
                }
            }
            if !star_sources.is_empty() {
                self.pending_link_stars.push((ns, star_sources));
            }
            if let Some(rec) = self.module_records.get_mut(key) {
                rec.deps = deps;
                rec.eval_state = ModuleEvalState::Linked;
            }
            Ok(())
        })();

        self.module_link_depth -= 1;

        if let Err(e) = link_res {
            if self.module_link_depth == 0 {
                let keys = std::mem::take(&mut self.pending_link_keys);
                self.pending_link_stars.clear();
                self.pending_link_checks.clear();
                for k in keys {
                    self.module_records.remove(&k);
                    self.module_cache.remove(&k);
                }
            }
            return Err(e);
        }

        if self.module_link_depth == 0 {
            let stars = std::mem::take(&mut self.pending_link_stars);
            let checks = std::mem::take(&mut self.pending_link_checks);
            let keys = std::mem::take(&mut self.pending_link_keys);

            let rounds = stars.len() + 2;
            for _ in 0..rounds {
                for (target_ns, star_sources) in &stars {
                    let direct_names: std::collections::HashSet<String> =
                        match &self.heap.get_obj(*target_ns).kind {
                            ObjectKind::ModuleNamespace(m) => {
                                m.exports.iter().map(|(k, _)| k.clone()).collect()
                            }
                            _ => std::collections::HashSet::new(),
                        };
                    let mut star_map: HashMap<String, ExportTarget> = HashMap::new();
                    for src_ns in star_sources {
                        let mut collected = Vec::new();
                        let mut vis = std::collections::HashSet::new();
                        vis.insert(*target_ns);
                        self.collect_star_exports(*src_ns, &mut vis, &mut collected);
                        for (name, _raw_target) in collected {
                            if name == "default" || direct_names.contains(&name) {
                                continue;
                            }
                            let mut rvis = std::collections::HashSet::new();
                            rvis.insert((*target_ns, name.clone()));
                            let resolved = match self.resolve_export_canonical(*src_ns, &name, &mut rvis) {
                                ExportResolution::NotFound => continue,
                                ExportResolution::Ambiguous => ExportTarget::Ambiguous,
                                ExportResolution::Found(t) => t,
                            };
                            if let Some(prev) = star_map.get(&name) {
                                if !self.same_export_target(prev, &resolved) {
                                    star_map.insert(name, ExportTarget::Ambiguous);
                                }
                            } else {
                                star_map.insert(name, resolved);
                            }
                        }
                    }
                    if let ObjectKind::ModuleNamespace(ref mut m) =
                        self.heap.get_obj_mut(*target_ns).kind
                    {
                        for (name, target) in star_map {
                            if let Some(existing) = m.exports.iter_mut().find(|(k, _)| k == &name) {
                                existing.1 = target;
                            } else {
                                m.exports.push((name, target));
                            }
                        }
                    }
                }
            }

            for k in &keys {
                if let Some(rec) = self.module_records.get(k).cloned() {
                    if let ObjectKind::ModuleNamespace(ref mut m) =
                        self.heap.get_obj_mut(rec.ns).kind
                    {
                        m.exports
                            .sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
                    }
                    if let Some(dns) = rec.deferred_ns {
                        let exps = match &self.heap.get_obj(rec.ns).kind {
                            ObjectKind::ModuleNamespace(m) => m.exports.clone(),
                            _ => Vec::new(),
                        };
                        if let ObjectKind::ModuleNamespace(ref mut dm) =
                            self.heap.get_obj_mut(dns).kind
                        {
                            dm.exports = exps;
                        }
                    }
                }
            }

            for (dep_ns, imp_name) in checks {
                let mut vis = std::collections::HashSet::new();
                match self.resolve_export_canonical(dep_ns, &imp_name, &mut vis) {
                    ExportResolution::Found(_) => {}
                    ExportResolution::Ambiguous => {
                        for k in &keys {
                            self.module_records.remove(k);
                            self.module_cache.remove(k);
                        }
                        return Err(JsError::syntax(format!(
                            "Ambiguous indirect export '{}'",
                            imp_name
                        )));
                    }
                    ExportResolution::NotFound => {
                        for k in &keys {
                            self.module_records.remove(k);
                            self.module_cache.remove(k);
                        }
                        return Err(JsError::syntax(format!(
                            "The requested module does not provide an export named '{}'",
                            imp_name
                        )));
                    }
                }
            }
        }
        Ok(ns)
    }

    fn collect_star_exports(
        &self,
        ns: ObjectId,
        visited: &mut std::collections::HashSet<ObjectId>,
        out: &mut Vec<(String, ExportTarget)>,
    ) {
        if !visited.insert(ns) {
            return;
        }
        let exports = match &self.heap.get_obj(ns).kind {
            ObjectKind::ModuleNamespace(m) => m.exports.clone(),
            _ => return,
        };
        for (name, target) in exports {
            if name == "default" {
                continue;
            }
            out.push((name, target));
        }
    }

    fn same_export_target(&self, a: &ExportTarget, b: &ExportTarget) -> bool {
        let ca = self.canonicalize_export_target(a, 0);
        let cb = self.canonicalize_export_target(b, 0);
        match (ca, cb) {
            (
                ExportTarget::Local { env: e1, name: n1 },
                ExportTarget::Local { env: e2, name: n2 },
            ) => e1 == e2 && n1 == n2,
            (ExportTarget::Value(v1), ExportTarget::Value(v2)) => v1.same_value(&v2),
            _ => false,
        }
    }

    fn canonicalize_export_target(&self, target: &ExportTarget, depth: usize) -> ExportTarget {
        if depth > 64 {
            return ExportTarget::Ambiguous;
        }
        match target {
            ExportTarget::Local { env, name } => {
                let rec = self.heap.get_env(*env);
                if let Some((imp_ns, imp_name)) = rec.import_bindings.get(name) {
                    return self.canonicalize_export_target(
                        &ExportTarget::Indirect {
                            ns: *imp_ns,
                            export_name: imp_name.clone(),
                        },
                        depth + 1,
                    );
                }
                if let Some(b) = rec.bindings.get(name) {
                    if !b.mutable && b.initialized {
                        if let JsValue::Object(oid) = b.value {
                            if matches!(self.heap.get_obj(oid).kind, ObjectKind::ModuleNamespace(_)) {
                                return ExportTarget::Value(JsValue::Object(oid));
                            }
                        }
                    }
                }
                target.clone()
            }
            ExportTarget::Indirect { ns, export_name } => {
                let found = match &self.heap.get_obj(*ns).kind {
                    ObjectKind::ModuleNamespace(m) => m
                        .exports
                        .iter()
                        .find(|(k, _)| k == export_name)
                        .map(|(_, t)| t.clone()),
                    _ => None,
                };
                if let Some(inner) = found {
                    self.canonicalize_export_target(&inner, depth + 1)
                } else {
                    ExportTarget::Ambiguous
                }
            }
            other => other.clone(),
        }
    }

    fn resolve_export_canonical(
        &self,
        ns: ObjectId,
        name: &str,
        visited: &mut std::collections::HashSet<(ObjectId, String)>,
    ) -> ExportResolution {
        if !visited.insert((ns, name.to_string())) {
            return ExportResolution::NotFound;
        }
        let target = match &self.heap.get_obj(ns).kind {
            ObjectKind::ModuleNamespace(m) => m
                .exports
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, t)| t.clone()),
            _ => None,
        };
        match target {
            None => ExportResolution::NotFound,
            Some(ExportTarget::Ambiguous) => ExportResolution::Ambiguous,
            Some(ExportTarget::Indirect {
                ns: next_ns,
                export_name,
            }) => self.resolve_export_canonical(next_ns, &export_name, visited),
            Some(ExportTarget::Local { env, name: ref lname }) => {
                if let Some((imp_ns, imp_name)) = self.heap.get_env(env).import_bindings.get(lname) {
                    return self.resolve_export_canonical(*imp_ns, imp_name, visited);
                }
                ExportResolution::Found(ExportTarget::Local {
                    env,
                    name: lname.clone(),
                })
            }
            Some(ExportTarget::Value(v)) => ExportResolution::Found(ExportTarget::Value(v)),
        }
    }

    pub fn get_module_export_value(
        &mut self,
        ns: ObjectId,
        export_name: &str,
    ) -> Result<JsValue, JsError> {
        self.ensure_deferred_ns_evaluated(ns)?;
        let mut vis = std::collections::HashSet::new();
        match self.resolve_export_canonical(ns, export_name, &mut vis) {
            ExportResolution::Found(ExportTarget::Local { env, name }) => {
                let b = self.heap.get_env(env).bindings.get(&name).cloned();
                if let Some(binding) = b {
                    if !binding.initialized {
                        return Err(JsError::reference(format!(
                            "Cannot access '{}' before initialization",
                            export_name
                        )));
                    }
                    Ok(binding.value)
                } else {
                    Err(JsError::reference(format!(
                        "Cannot access '{}' before initialization",
                        export_name
                    )))
                }
            }
            ExportResolution::Found(ExportTarget::Value(v)) => Ok(v),
            _ => Ok(JsValue::Undefined),
        }
    }

    pub fn ensure_deferred_ns_evaluated(&mut self, ns: ObjectId) -> Result<(), JsError> {
        let mod_key = match &self.heap.get_obj(ns).kind {
            ObjectKind::ModuleNamespace(m) if m.is_deferred => m.module_key.clone(),
            _ => return Ok(()),
        };
        let Some(key) = mod_key else {
            return Ok(());
        };
        if let Some(rec) = self.module_records.get(&key) {
            if rec.eval_state == ModuleEvalState::Evaluated {
                if let Some(ref err) = rec.eval_error {
                    return Err(JsError::Thrown(err.clone()));
                }
                return Ok(());
            }
            if rec.eval_state == ModuleEvalState::Evaluating {
                return Err(JsError::type_err(
                    "Cannot synchronously evaluate a module that is currently evaluating",
                ));
            }
            if rec.has_tla {
                return Err(JsError::type_err(
                    "Cannot synchronously evaluate an async module via deferred namespace",
                ));
            }
        }
        let p = self.evaluate_module(&key)?;
        if let Some(ep) = p {
            let state = match &self.heap.get_obj(ep).kind {
                ObjectKind::Promise(pd) => pd.state.clone(),
                _ => PromiseState::Fulfilled(JsValue::Undefined),
            };
            if let PromiseState::Rejected(err) = state {
                return Err(JsError::Thrown(err));
            }
        }
        Ok(())
    }

    pub fn evaluate_module(&mut self, key: &str) -> Result<Option<ObjectId>, JsError> {
        let Some(rec) = self.module_records.get(key).cloned() else {
            return Ok(None);
        };
        match rec.eval_state {
            ModuleEvalState::Evaluated => {
                if let Some(err) = rec.eval_error {
                    return Err(JsError::Thrown(err));
                }
                return Ok(None);
            }
            ModuleEvalState::Evaluating => {
                return Ok(rec.eval_promise);
            }
            ModuleEvalState::Linking | ModuleEvalState::Linked => {}
        }
        if let Some(r) = self.module_records.get_mut(key) {
            r.eval_state = ModuleEvalState::Evaluating;
        }

        let mut pending_dep_promises: Vec<ObjectId> = Vec::new();
        for dep in &rec.deps {
            if dep.is_deferred {
                continue;
            }
            match self.evaluate_module(&dep.key) {
                Ok(Some(dp)) => pending_dep_promises.push(dp),
                Ok(None) => {}
                Err(e) => {
                    let ev = self.error_to_js_value(e.clone());
                    if let Some(r) = self.module_records.get_mut(key) {
                        r.eval_state = ModuleEvalState::Evaluated;
                        r.eval_error = Some(ev);
                    }
                    return Err(e);
                }
            }
        }

        if pending_dep_promises.is_empty() {
            return self.execute_module_body(key);
        }

        let mod_p = self.create_promise();
        if let Some(r) = self.module_records.get_mut(key) {
            r.eval_promise = Some(mod_p);
        }
        let key_owned = key.to_string();
        self.wait_dep_promises_and_exec(pending_dep_promises, 0, key_owned, mod_p);
        Ok(Some(mod_p))
    }

    fn wait_dep_promises_and_exec(
        &mut self,
        dep_promises: Vec<ObjectId>,
        idx: usize,
        key: String,
        mod_p: ObjectId,
    ) {
        if idx >= dep_promises.len() {
            match self.execute_module_body(&key) {
                Ok(None) => self.resolve_promise(mod_p, JsValue::Undefined),
                Ok(Some(inner_p)) => {
                    let key_ok = key.clone();
                    let on_ok = self.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, _| {
                            if let Some(r) = rt.module_records.get_mut(&key_ok) {
                                r.eval_state = ModuleEvalState::Evaluated;
                            }
                            rt.resolve_promise(mod_p, JsValue::Undefined);
                            Ok(JsValue::Undefined)
                        }),
                    );
                    let key_err = key.clone();
                    let on_err = self.create_native_function(
                        "",
                        1,
                        Rc::new(move |rt, _, args| {
                            let err = args.first().cloned().unwrap_or(JsValue::Undefined);
                            if let Some(r) = rt.module_records.get_mut(&key_err) {
                                r.eval_state = ModuleEvalState::Evaluated;
                                r.eval_error = Some(err.clone());
                            }
                            rt.reject_promise(mod_p, err);
                            Ok(JsValue::Undefined)
                        }),
                    );
                    self.promise_then(
                        inner_p,
                        Some(JsValue::Object(on_ok)),
                        Some(JsValue::Object(on_err)),
                    );
                }
                Err(e) => {
                    let ev = self.error_to_js_value(e);
                    if let Some(r) = self.module_records.get_mut(&key) {
                        r.eval_state = ModuleEvalState::Evaluated;
                        r.eval_error = Some(ev.clone());
                    }
                    self.reject_promise(mod_p, ev);
                }
            }
            return;
        }
        let dp = dep_promises[idx];
        let deps_next = dep_promises.clone();
        let key_ok = key.clone();
        let on_ok = self.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, _| {
                rt.wait_dep_promises_and_exec(deps_next.clone(), idx + 1, key_ok.clone(), mod_p);
                Ok(JsValue::Undefined)
            }),
        );
        let key_err = key.clone();
        let on_err = self.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, args| {
                let err = args.first().cloned().unwrap_or(JsValue::Undefined);
                if let Some(r) = rt.module_records.get_mut(&key_err) {
                    r.eval_state = ModuleEvalState::Evaluated;
                    r.eval_error = Some(err.clone());
                }
                rt.reject_promise(mod_p, err);
                Ok(JsValue::Undefined)
            }),
        );
        self.promise_then(dp, Some(JsValue::Object(on_ok)), Some(JsValue::Object(on_err)));
    }

    fn execute_module_body(&mut self, key: &str) -> Result<Option<ObjectId>, JsError> {
        let Some(rec) = self.module_records.get(key).cloned() else {
            return Ok(None);
        };
        let Some(chunk) = rec.chunk else {
            if let Some(r) = self.module_records.get_mut(key) {
                r.eval_state = ModuleEvalState::Evaluated;
            }
            return Ok(None);
        };
        let mut frame = CallFrame::new(chunk, rec.env, None);
        self.current_module_stack.push(rec.path.clone());
        self.current_module_exports.push(rec.ns);
        self.active_frames.push(rec.env);
        let res = self.run_frame(&mut frame);
        self.active_frames.pop();
        self.current_module_exports.pop();
        self.current_module_stack.pop();
        match res {
            Ok(crate::exec::FrameOutcome::Return(_)) => {
                if let Some(r) = self.module_records.get_mut(key) {
                    r.eval_state = ModuleEvalState::Evaluated;
                }
                Ok(None)
            }
            Ok(crate::exec::FrameOutcome::Await(awaited, _)) => {
                let mod_p = self.create_promise();
                if let Some(r) = self.module_records.get_mut(key) {
                    r.eval_promise = Some(mod_p);
                }
                let awaited_p = match self.promise_resolve_value_checked(awaited) {
                    Ok(p) => p,
                    Err(e) => {
                        let ev = self.error_to_js_value(e);
                        if let Some(r) = self.module_records.get_mut(key) {
                            r.eval_state = ModuleEvalState::Evaluated;
                            r.eval_error = Some(ev.clone());
                        }
                        self.reject_promise(mod_p, ev);
                        return Ok(Some(mod_p));
                    }
                };
                let holder = self.heap.alloc_object(JsObject::new(
                    None,
                    ObjectKind::Generator(GeneratorData {
                        state: GeneratorState::SuspendedYield,
                        frame: Some(Box::new(frame)),
                        is_async: true,
                        delegating: None,
                        result_promise: Some(mod_p),
                        queue: Vec::new(),
                    }),
                ));
                let key_ok = key.to_string();
                let key_err = key.to_string();
                let mod_inner_p = self.create_promise();
                let on_fin_ok = self.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, _| {
                        if let Some(r) = rt.module_records.get_mut(&key_ok) {
                            r.eval_state = ModuleEvalState::Evaluated;
                        }
                        rt.resolve_promise(mod_p, JsValue::Undefined);
                        Ok(JsValue::Undefined)
                    }),
                );
                let on_fin_err = self.create_native_function(
                    "",
                    1,
                    Rc::new(move |rt, _, args| {
                        let err = args.first().cloned().unwrap_or(JsValue::Undefined);
                        if let Some(r) = rt.module_records.get_mut(&key_err) {
                            r.eval_state = ModuleEvalState::Evaluated;
                            r.eval_error = Some(err.clone());
                        }
                        rt.reject_promise(mod_p, err);
                        Ok(JsValue::Undefined)
                    }),
                );
                self.promise_then(
                    mod_inner_p,
                    Some(JsValue::Object(on_fin_ok)),
                    Some(JsValue::Object(on_fin_err)),
                );
                let path_ok = rec.path.clone();
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
                            rt.current_module_stack.push(path_ok.clone());
                            rt.run_async_frame(*fr, mod_inner_p);
                            rt.current_module_stack.pop();
                        }
                        Ok(JsValue::Undefined)
                    }),
                );
                let path_err = rec.path.clone();
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
                                rt.reject_promise(mod_inner_p, ev);
                            } else {
                                rt.current_module_stack.push(path_err.clone());
                                rt.run_async_frame(*fr, mod_inner_p);
                                rt.current_module_stack.pop();
                            }
                        }
                        Ok(JsValue::Undefined)
                    }),
                );
                self.promise_then(
                    awaited_p,
                    Some(JsValue::Object(on_ok)),
                    Some(JsValue::Object(on_err)),
                );
                Ok(Some(mod_p))
            }
            Ok(_) => {
                if let Some(r) = self.module_records.get_mut(key) {
                    r.eval_state = ModuleEvalState::Evaluated;
                }
                Ok(None)
            }
            Err(e) => {
                let ev = self.error_to_js_value(e.clone());
                if let Some(r) = self.module_records.get_mut(key) {
                    r.eval_state = ModuleEvalState::Evaluated;
                    r.eval_error = Some(ev);
                }
                Err(e)
            }
        }
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
                handles_new_target: false,
            }),
        ));
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("length"),
            PropEntry::data(JsValue::Number(length as f64), false, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("name"),
            PropEntry::data(JsValue::String(name.to_string()), false, false, true),
        );
        self.heap.object_realms.insert(obj, self.current_realm_id);
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
                handles_new_target: false,
            }),
        ));
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("length"),
            PropEntry::data(JsValue::Number(length as f64), false, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("name"),
            PropEntry::data(JsValue::String(name.to_string()), false, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("prototype"),
            PropEntry::data(JsValue::Object(proto), false, false, false),
        );
        self.heap.get_obj_mut(proto).set_own_prop(
            PropertyKey::from("constructor"),
            PropEntry::data(JsValue::Object(obj), true, false, true),
        );
        self.heap.object_realms.insert(obj, self.current_realm_id);
        obj
    }

    pub fn create_spec_constructor(
        &mut self,
        name: &str,
        length: usize,
        proto: ObjectId,
        callback: NativeCallback,
    ) -> ObjectId {
        let obj = self.create_native_constructor(name, length, proto, callback);
        if let ObjectKind::NativeFunction(ref mut nf) = self.heap.get_obj_mut(obj).kind {
            nf.handles_new_target = true;
        }
        obj
    }

    pub fn require_new_target(&self, name: &str) -> Result<JsValue, JsError> {
        match &self.native_new_target {
            Some(nt) if !nt.is_undefined() => Ok(nt.clone()),
            _ => Err(JsError::type_err(format!(
                "Constructor {} requires 'new'",
                name
            ))),
        }
    }

    pub fn get_prototype_from_new_target(
        &mut self,
        default_proto: ObjectId,
    ) -> Result<ObjectId, JsError> {
        let nt = self.native_new_target.clone().unwrap_or(JsValue::Undefined);
        self.get_prototype_from_constructor(&nt, default_proto)
    }

    pub fn get_prototype_from_constructor(
        &mut self,
        ctor: &JsValue,
        default_proto: ObjectId,
    ) -> Result<ObjectId, JsError> {
        if let JsValue::Object(cid) = ctor {
            let proto_val = self.get_property(*cid, "prototype")?;
            if let JsValue::Object(pid) = proto_val {
                return Ok(pid);
            }
            let realm_id = self.get_function_realm(*cid)?;
            return Ok(self.realm_proto_for(realm_id, default_proto));
        }
        Ok(default_proto)
    }

    pub fn species_constructor(
        &mut self,
        oid: ObjectId,
        default_ctor: ObjectId,
    ) -> Result<JsValue, JsError> {
        let c = self.get_property(oid, "constructor")?;
        if c.is_undefined() {
            return Ok(JsValue::Object(default_ctor));
        }
        let JsValue::Object(cid) = c else {
            return Err(JsError::type_err("constructor property is not an object"));
        };
        let sym = PropertyKey::Symbol(self.symbols.species);
        let s = self.get_property_key(cid, &sym)?;
        if s.is_nullish() {
            return Ok(JsValue::Object(default_ctor));
        }
        if self.is_constructor(&s) {
            return Ok(s);
        }
        Err(JsError::type_err("Species constructor is not a constructor"))
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
            ErrorKind::SuppressedError => self.intrinsics.suppressed_error_proto,
        };
        self.create_error_object_with_proto(kind, message, !message.is_empty(), proto)
    }

    pub fn create_suppressed_error(&mut self, error: JsValue, suppressed: JsValue) -> JsValue {
        let proto = self.intrinsics.suppressed_error_proto;
        let obj = self.create_error_object_with_proto(
            ErrorKind::SuppressedError,
            "An error was suppressed during disposal",
            true,
            proto,
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("error"),
            PropEntry::data(error, true, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("suppressed"),
            PropEntry::data(suppressed, true, false, true),
        );
        JsValue::Object(obj)
    }

    pub fn create_error_object_with_proto(
        &mut self,
        kind: ErrorKind,
        message: &str,
        set_message_prop: bool,
        proto: ObjectId,
    ) -> ObjectId {
        let stack = if message.is_empty() {
            kind.name().to_string()
        } else {
            format!("{}: {}", kind.name(), message)
        };
        let obj = self.heap.alloc_object(JsObject::new(
            Some(proto),
            ObjectKind::Error(ErrorData {
                kind,
                message: message.to_string(),
                stack,
            }),
        ));
        if set_message_prop {
            self.heap.get_obj_mut(obj).set_own_prop(
                PropertyKey::from("message"),
                PropEntry::data(JsValue::String(message.to_string()), true, false, true),
            );
        }
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

    pub fn create_realm_error_object(
        &mut self,
        realm_id: usize,
        kind: ErrorKind,
        message: &str,
    ) -> ObjectId {
        let intr = if realm_id < self.realms.len() {
            &self.realms[realm_id].intrinsics
        } else {
            &self.intrinsics
        };
        let proto = match kind {
            ErrorKind::Error | ErrorKind::InternalError => intr.error_proto,
            ErrorKind::TypeError => intr.type_error_proto,
            ErrorKind::ReferenceError => intr.reference_error_proto,
            ErrorKind::SyntaxError => intr.syntax_error_proto,
            ErrorKind::RangeError => intr.range_error_proto,
            ErrorKind::UriError => intr.uri_error_proto,
            ErrorKind::EvalError => intr.eval_error_proto,
            ErrorKind::AggregateError => intr.aggregate_error_proto,
            ErrorKind::SuppressedError => intr.suppressed_error_proto,
        };
        self.create_error_object_with_proto(kind, message, !message.is_empty(), proto)
    }

    pub fn create_js_function(&mut self, tmpl: &Rc<FuncTemplate>, env: EnvId) -> ObjectId {
        let name = tmpl.name.clone().unwrap_or_default();
        let param_len = compute_func_length(&tmpl.params);
        let proto = if tmpl.is_async && tmpl.is_generator {
            self.intrinsics.async_generator_function_proto
        } else if tmpl.is_generator {
            self.intrinsics.generator_function_proto
        } else if tmpl.is_async {
            self.intrinsics.async_function_proto
        } else {
            self.intrinsics.function_proto
        };
        let obj = self.heap.alloc_object(JsObject::new(
            Some(proto),
            ObjectKind::Function(JsFunction {
                name: name.clone(),
                has_self_name: tmpl.has_self_name,
                chunk: tmpl.chunk.clone(),
                params: tmpl.params.clone(),
                env,
                is_async: tmpl.is_async,
                is_generator: tmpl.is_generator,
                is_arrow: tmpl.is_arrow,
                is_method: tmpl.is_method,
                is_decl: tmpl.is_decl,
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
            PropertyKey::from("length"),
            PropEntry::data(JsValue::Number(param_len as f64), false, false, true),
        );
        self.heap.get_obj_mut(obj).set_own_prop(
            PropertyKey::from("name"),
            PropEntry::data(JsValue::String(name), false, false, true),
        );
        if !tmpl.is_arrow
            && (!tmpl.is_method || tmpl.is_generator)
            && (!tmpl.is_async || tmpl.is_generator)
        {
            let fn_proto = if tmpl.is_async && tmpl.is_generator {
                self.create_object_with_proto(Some(self.intrinsics.async_generator_proto))
            } else if tmpl.is_generator {
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
        self.heap.object_realms.insert(obj, self.current_realm_id);
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

    pub fn add_gc_root_tracer<F: Fn(&mut Vec<ObjectId>) + 'static>(&mut self, tracer: F) {
        self.host_root_tracers.push(Box::new(tracer));
    }

    pub fn collect_garbage(&mut self) {
        self.collect_garbage_inner(None);
    }

    pub fn collect_garbage_with_frame(&mut self, frame: &CallFrame) {
        self.collect_garbage_inner(Some(frame));
    }

    fn collect_garbage_inner(&mut self, active_frame: Option<&CallFrame>) {
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
            self.intrinsics.suppressed_error_proto,
            self.intrinsics.disposable_stack_proto,
            self.intrinsics.async_disposable_stack_proto,
            self.intrinsics.shadow_realm_proto,
            self.intrinsics.abstract_module_source_proto,
            self.intrinsics.abstract_module_source_ctor,
            self.intrinsics.map_proto,
            self.intrinsics.set_proto,
            self.intrinsics.weak_map_proto,
            self.intrinsics.weak_set_proto,
            self.intrinsics.weak_ref_proto,
            self.intrinsics.finalization_registry_proto,
            self.intrinsics.promise_proto,
            self.intrinsics.array_buffer_proto,
            self.intrinsics.shared_array_buffer_proto,
            self.intrinsics.data_view_proto,
            self.intrinsics.typed_array_proto,
            self.intrinsics.iterator_proto,
            self.intrinsics.async_iterator_proto,
            self.intrinsics.array_iterator_proto,
            self.intrinsics.string_iterator_proto,
            self.intrinsics.map_iterator_proto,
            self.intrinsics.set_iterator_proto,
            self.intrinsics.regexp_string_iterator_proto,
            self.intrinsics.generator_proto,
            self.intrinsics.async_generator_proto,
            self.intrinsics.generator_function_proto,
            self.intrinsics.generator_function_ctor,
            self.intrinsics.async_function_proto,
            self.intrinsics.async_function_ctor,
            self.intrinsics.async_generator_function_proto,
            self.intrinsics.async_generator_function_ctor,
            self.intrinsics.throw_type_error,
            self.intrinsics.eval_fn,
            self.intrinsics.function_ctor,
            self.intrinsics.object_ctor,
            self.intrinsics.array_ctor,
            self.intrinsics.promise_ctor,
            self.intrinsics.regexp_ctor,
            self.intrinsics.array_buffer_ctor,
            self.intrinsics.shared_array_buffer_ctor,
        ];
        root_objs.extend_from_slice(&self.intrinsics.typed_array_protos);
        root_objs.extend_from_slice(&self.intrinsics.typed_array_ctors);
        for &m in self.module_cache.values() {
            root_objs.push(m);
        }
        for rec in self.module_records.values() {
            root_objs.push(rec.ns);
            if let Some(dns) = rec.deferred_ns {
                root_objs.push(dns);
            }
            if let Some(ep) = rec.eval_promise {
                root_objs.push(ep);
            }
            if let Some(ref ev) = rec.eval_error {
                trace_value(ev, &mut root_objs);
            }
        }
        for &t in self.template_cache.values() {
            root_objs.push(t);
        }
        for (_, _, _, p, _) in &self.async_waiters {
            root_objs.push(*p);
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
        for rec in self.module_records.values() {
            root_envs.push(rec.env);
        }
        for r in &self.realms {
            root_objs.push(r.global_object);
            root_objs.push(r.intrinsics.object_proto);
            root_objs.push(r.intrinsics.function_proto);
            root_objs.push(r.intrinsics.array_proto);
            root_objs.push(r.intrinsics.string_proto);
            root_objs.push(r.intrinsics.number_proto);
            root_objs.push(r.intrinsics.boolean_proto);
            root_objs.push(r.intrinsics.symbol_proto);
            root_objs.push(r.intrinsics.bigint_proto);
            root_objs.push(r.intrinsics.regexp_proto);
            root_objs.push(r.intrinsics.date_proto);
            root_objs.push(r.intrinsics.error_proto);
            root_objs.push(r.intrinsics.type_error_proto);
            root_objs.push(r.intrinsics.reference_error_proto);
            root_objs.push(r.intrinsics.syntax_error_proto);
            root_objs.push(r.intrinsics.range_error_proto);
            root_objs.push(r.intrinsics.uri_error_proto);
            root_objs.push(r.intrinsics.eval_error_proto);
            root_objs.push(r.intrinsics.aggregate_error_proto);
            root_objs.push(r.intrinsics.suppressed_error_proto);
            root_objs.push(r.intrinsics.disposable_stack_proto);
            root_objs.push(r.intrinsics.async_disposable_stack_proto);
            root_objs.push(r.intrinsics.shadow_realm_proto);
            root_objs.push(r.intrinsics.abstract_module_source_proto);
            root_objs.push(r.intrinsics.abstract_module_source_ctor);
            root_objs.push(r.intrinsics.map_proto);
            root_objs.push(r.intrinsics.set_proto);
            root_objs.push(r.intrinsics.weak_map_proto);
            root_objs.push(r.intrinsics.weak_set_proto);
            root_objs.push(r.intrinsics.weak_ref_proto);
            root_objs.push(r.intrinsics.finalization_registry_proto);
            root_objs.push(r.intrinsics.promise_proto);
            root_objs.push(r.intrinsics.array_buffer_proto);
            root_objs.push(r.intrinsics.shared_array_buffer_proto);
            root_objs.push(r.intrinsics.data_view_proto);
            root_objs.push(r.intrinsics.typed_array_proto);
            root_objs.push(r.intrinsics.iterator_proto);
            root_objs.push(r.intrinsics.async_iterator_proto);
            root_objs.push(r.intrinsics.array_iterator_proto);
            root_objs.push(r.intrinsics.string_iterator_proto);
            root_objs.push(r.intrinsics.map_iterator_proto);
            root_objs.push(r.intrinsics.set_iterator_proto);
            root_objs.push(r.intrinsics.regexp_string_iterator_proto);
            root_objs.push(r.intrinsics.generator_proto);
            root_objs.push(r.intrinsics.async_generator_proto);
            root_objs.push(r.intrinsics.generator_function_proto);
            root_objs.push(r.intrinsics.generator_function_ctor);
            root_objs.push(r.intrinsics.async_function_proto);
            root_objs.push(r.intrinsics.async_function_ctor);
            root_objs.push(r.intrinsics.async_generator_function_proto);
            root_objs.push(r.intrinsics.async_generator_function_ctor);
            root_objs.push(r.intrinsics.throw_type_error);
            root_objs.push(r.intrinsics.eval_fn);
            root_objs.push(r.intrinsics.function_ctor);
            root_objs.push(r.intrinsics.object_ctor);
            root_objs.push(r.intrinsics.array_ctor);
            root_objs.push(r.intrinsics.iterator_ctor);
            root_objs.push(r.intrinsics.iterator_helper_proto);
            root_objs.push(r.intrinsics.wrap_for_valid_iterator_proto);
            root_objs.push(r.intrinsics.promise_ctor);
            root_objs.push(r.intrinsics.regexp_ctor);
            root_objs.push(r.intrinsics.array_buffer_ctor);
            root_objs.push(r.intrinsics.shared_array_buffer_ctor);
            root_objs.extend_from_slice(&r.intrinsics.typed_array_protos);
            root_objs.extend_from_slice(&r.intrinsics.typed_array_ctors);
            root_envs.push(r.global_env);
        }
        root_envs.extend_from_slice(&self.active_frames);
        if let Some(frame) = active_frame {
            trace_frame(frame, &mut root_objs, &mut root_envs);
            trace_value(&frame.last_value, &mut root_objs);
            if let Some(t) = frame.this_for_ctor {
                root_objs.push(t);
            }
            if let Some(ap) = frame.async_promise {
                root_objs.push(ap);
            }
            for h in &frame.handlers {
                if let Some(ref sv) = h.saved_last_value {
                    trace_value(sv, &mut root_objs);
                }
                match &h.pending {
                    PendingCompletion::Return(v) => trace_value(v, &mut root_objs),
                    PendingCompletion::Throw(JsError::Thrown(v)) => trace_value(v, &mut root_objs),
                    _ => {}
                }
            }
        }
        root_objs.extend_from_slice(&self.call_stack_callees);
        if let Some(ref nt) = self.native_new_target {
            trace_value(nt, &mut root_objs);
        }
        for tracer in &self.host_root_tracers {
            tracer(&mut root_objs);
        }
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

    pub fn to_bigint(&mut self, val: &JsValue) -> Result<crate::bigint::JsBigInt, JsError> {
        let prim = self.to_primitive(val, "number")?;
        match prim {
            JsValue::BigInt(n) => Ok(n),
            JsValue::Boolean(b) => Ok(if b {
                crate::bigint::JsBigInt::one()
            } else {
                crate::bigint::JsBigInt::zero()
            }),
            JsValue::String(s) => crate::bigint::JsBigInt::parse_string(&s)
                .ok_or_else(|| JsError::syntax(format!("Cannot convert {} to a BigInt", s))),
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
                ObjectKind::BigIntObj(n.clone()),
            ))),
        }
    }

    pub fn is_callable(&self, val: &JsValue) -> bool {
        let JsValue::Object(id) = val else {
            return false;
        };
        let obj = self.heap.get_obj(*id);
        match &obj.kind {
            ObjectKind::Function(_)
            | ObjectKind::NativeFunction(_)
            | ObjectKind::WrappedFunction { .. } => true,
            ObjectKind::Proxy(p) => self.is_callable(&JsValue::Object(p.target)),
            _ => false,
        }
    }

    pub fn is_constructor(&self, val: &JsValue) -> bool {
        let JsValue::Object(id) = val else {
            return false;
        };
        match &self.heap.get_obj(*id).kind {
            ObjectKind::Function(f) => {
                if let Some(bt) = f.bound_target {
                    return self.is_constructor(&JsValue::Object(bt));
                }
                !f.is_arrow && !f.is_async && !f.is_generator && !f.is_method
            }
            ObjectKind::NativeFunction(nf) => nf.is_constructor,
            ObjectKind::Proxy(p) => self.is_constructor(&JsValue::Object(p.target)),
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
                    if !self.is_callable(&trap) {
                        return Err(JsError::type_err("Proxy get trap is not callable"));
                    }
                    let trap_result = self.call_function(
                        trap,
                        JsValue::Object(handler),
                        &[JsValue::Object(target), key.to_value(), receiver],
                    )?;
                    if let Some(td) = self.get_own_property_descriptor(target, key)? {
                        if td.configurable == Some(false) {
                            if !td.is_accessor() && td.writable == Some(false) {
                                let tv = td.value.unwrap_or(JsValue::Undefined);
                                if !trap_result.same_value(&tv) {
                                    return Err(JsError::type_err(
                                        "Proxy get trap violated non-writable non-configurable invariant",
                                    ));
                                }
                            } else if td.is_accessor()
                                && td.get.as_ref().is_none_or(|g| g.is_undefined())
                                && !trap_result.is_undefined()
                            {
                                return Err(JsError::type_err(
                                    "Proxy get trap violated non-configurable undefined getter invariant",
                                ));
                            }
                        }
                    }
                    return Ok(trap_result);
                }
                return self.get_property_key_receiver(target, key, receiver);
            }

            if cur_id == obj {
                if let PropertyKey::String(ref k) = key {
                    if k == "caller" || k == "arguments" {
                        let is_sloppy_plain = match &self.heap.get_obj(cur_id).kind {
                            ObjectKind::Function(f) => {
                                !f.strict
                                    && !f.is_arrow
                                    && !f.is_generator
                                    && !f.is_async
                                    && !f.is_class_ctor
                                    && f.bound_target.is_none()
                            }
                            _ => false,
                        };
                        if is_sloppy_plain && self.heap.get_obj(cur_id).get_own_prop(key).is_none() {
                            if k == "arguments" {
                                return Ok(JsValue::Null);
                            }
                            if let Some(pos) =
                                self.call_stack_callees.iter().rposition(|&id| id == cur_id)
                            {
                                if pos > 0 {
                                    let caller_id = self.call_stack_callees[pos - 1];
                                    let caller_restricted = match &self.heap.get_obj(caller_id).kind {
                                        ObjectKind::Function(f) => {
                                            f.strict
                                                || f.is_async
                                                || f.is_generator
                                                || f.is_class_ctor
                                        }
                                        _ => true,
                                    };
                                    if caller_restricted {
                                        return Err(JsError::type_err(
                                            "'caller' property cannot be accessed when caller is a strict function",
                                        ));
                                    }
                                    return Ok(JsValue::Object(caller_id));
                                }
                            }
                            return Ok(JsValue::Null);
                        }
                    }
                }
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
                    let idx_u = idx as usize;
                    if let Some(env_id) = args.mapped_env {
                        if let Some(Some(ref pname)) = args.mapped_params.get(idx_u) {
                            if let Some(b) = self.heap.get_env(env_id).bindings.get(pname) {
                                return Ok(Some(b.value.clone()));
                            }
                        }
                    }
                    let deleted = self
                        .heap
                        .deleted_arg_indices
                        .get(&obj)
                        .is_some_and(|s| s.contains(&idx_u));
                    if !deleted && o.get_own_prop(key).is_none() {
                        if let Some(v) = args.values.get(idx_u) {
                            return Ok(Some(v.clone()));
                        }
                    }
                }
            }
            ObjectKind::TypedArray(ta) => {
                if let Some(num_idx) = canonical_numeric_index_string(k) {
                    let ta_clone = ta.clone();
                    if let Some(eff_len) = self.typed_array_effective_length(obj) {
                        if let Some(idx) = valid_integer_index(num_idx, eff_len) {
                            return Ok(Some(self.typed_array_get(&ta_clone, idx)?));
                        }
                    }
                    return Ok(Some(JsValue::Undefined));
                }
            }
            ObjectKind::ModuleNamespace(m) => {
                if m.is_deferred && k == "then" {
                    return Ok(None);
                }
                let is_exported = m
                    .exports
                    .iter()
                    .any(|(name, t)| name == k && !matches!(t, ExportTarget::Ambiguous));
                if is_exported {
                    return Ok(Some(self.get_module_export_value(obj, k)?));
                }
                if m.is_deferred {
                    self.ensure_deferred_ns_evaluated(obj)?;
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
        if matches!(self.heap.get_obj(obj).kind, ObjectKind::ModuleNamespace(_)) {
            return Ok(false);
        }
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy set trap is not callable"));
                }
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), key.to_value(), val.clone(), receiver],
                )?;
                let bool_res = res.to_boolean();
                if !bool_res {
                    return Ok(false);
                }
                if let Some(td) = self.get_own_property_descriptor(target, &key)? {
                    if td.configurable == Some(false) {
                        if !td.is_accessor() && td.writable == Some(false) {
                            let tv = td.value.unwrap_or(JsValue::Undefined);
                            if !val.same_value(&tv) {
                                return Err(JsError::type_err(
                                    "Proxy set trap violated non-writable non-configurable invariant",
                                ));
                            }
                        } else if td.is_accessor()
                            && td.set.as_ref().is_none_or(|s| s.is_undefined())
                        {
                            return Err(JsError::type_err(
                                "Proxy set trap violated non-configurable undefined setter invariant",
                            ));
                        }
                    }
                }
                return Ok(true);
            }
            return self.set_property_key_receiver(target, key, val, receiver);
        }

        if let PropertyKey::String(ref k) = key {
            let kind_tag = match &self.heap.get_obj(obj).kind {
                ObjectKind::Array(_) => 1,
                ObjectKind::TypedArray(ta) => {
                    let ta_clone = ta.clone();
                    if let Some(num_idx) = canonical_numeric_index_string(k) {
                        let is_imm = self
                            .heap
                            .buffer_meta
                            .get(&ta_clone.buffer)
                            .is_some_and(|m| m.immutable);
                        if is_imm {
                            return Ok(false);
                        }
                        if receiver == JsValue::Object(obj) {
                            self.typed_array_set_checked(obj, &ta_clone, num_idx, &val)?;
                            return Ok(true);
                        }
                        let Some(eff_len) = self.typed_array_effective_length(obj) else {
                            return Ok(true);
                        };
                        if valid_integer_index(num_idx, eff_len).is_none() {
                            return Ok(true);
                        }
                        let JsValue::Object(recv_id) = receiver else {
                            return Ok(false);
                        };
                        if let Some(existing) = self.get_own_property_descriptor(recv_id, &key)? {
                            if existing.is_accessor() || existing.writable == Some(false) {
                                return Ok(false);
                            }
                            let mut value_desc = PropertyDescriptor::empty();
                            value_desc.value = Some(val);
                            return self.define_property(recv_id, key, value_desc);
                        }
                        return self.define_property(
                            recv_id,
                            key,
                            PropertyDescriptor::data(val, true, true, true),
                        );
                    }
                    0
                }
                ObjectKind::Arguments(_) => 2,
                _ => 0,
            };
            if kind_tag == 1 {
                if k == "length" {
                    if self.heap.non_writable_array_lengths.contains(&obj) {
                        return Ok(false);
                    }
                    if receiver != JsValue::Object(obj) {
                        let JsValue::Object(recv_id) = receiver else {
                            return Ok(false);
                        };
                        if let Some(existing) = self.get_own_property_descriptor(recv_id, &key)? {
                            if existing.is_accessor() || existing.writable == Some(false) {
                                return Ok(false);
                            }
                            let mut value_desc = PropertyDescriptor::empty();
                            value_desc.value = Some(val);
                            return self.define_property(recv_id, key, value_desc);
                        }
                        return self.define_property(
                            recv_id,
                            key,
                            PropertyDescriptor::data(val, true, true, true),
                        );
                    }
                    let mut desc = PropertyDescriptor::empty();
                    desc.value = Some(val);
                    return self.array_set_length(obj, desc);
                }
                if let Some(idx) = parse_array_index(k) {
                    let has_custom = match &self.heap.get_obj(obj).kind {
                        ObjectKind::Array(a) => a.has_custom_index_props,
                        _ => false,
                    };
                    let idx_u = idx as usize;
                    let old_len = match &self.heap.get_obj(obj).kind {
                        ObjectKind::Array(a) => a.length,
                        _ => 0,
                    };
                    if idx_u >= old_len && self.heap.non_writable_array_lengths.contains(&obj) {
                        return Ok(false);
                    }
                    if (!has_custom || self.heap.get_obj(obj).get_own_prop(&key).is_none())
                        && idx_u < 65_536
                        && receiver == JsValue::Object(obj)
                    {
                        let exists = match &self.heap.get_obj(obj).kind {
                            ObjectKind::Array(a) => a.get(idx_u).is_some(),
                            _ => false,
                        };
                        let proto_intercepts = if !exists {
                            let mut p = self.heap.get_obj(obj).prototype;
                            let mut hit = false;
                            while let Some(pid) = p {
                                let po = self.heap.get_obj(pid);
                                if matches!(po.kind, ObjectKind::Proxy(_) | ObjectKind::TypedArray(_))
                                    || po.get_own_prop(&key).is_some()
                                {
                                    hit = true;
                                    break;
                                }
                                p = po.prototype;
                            }
                            hit
                        } else {
                            false
                        };
                        if !proto_intercepts {
                            if !self.heap.get_obj(obj).extensible && !exists {
                                return Ok(false);
                            }
                            if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(obj).kind {
                                arr.set(idx_u, val);
                            }
                            return Ok(true);
                        }
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
                    let mapped_target = match &self.heap.get_obj(obj).kind {
                        ObjectKind::Arguments(a) => a.mapped_env.and_then(|eid| {
                            a.mapped_params
                                .get(idx_u)
                                .and_then(|p| p.clone())
                                .map(|pn| (eid, pn))
                        }),
                        _ => None,
                    };
                    if let Some((eid, pname)) = mapped_target {
                        if receiver == JsValue::Object(obj) {
                            let writable = self
                                .heap
                                .get_obj(obj)
                                .get_own_prop(&key)
                                .map(|e| e.writable)
                                .unwrap_or(true);
                            if !writable {
                                return Ok(false);
                            }
                            if let Some(b) = self.heap.get_env_mut(eid).bindings.get_mut(&pname) {
                                b.value = val.clone();
                            }
                            if let ObjectKind::Arguments(ref mut a) =
                                self.heap.get_obj_mut(obj).kind
                            {
                                if idx_u < a.values.len() {
                                    a.values[idx_u] = val.clone();
                                }
                            }
                            if let Some(e) = self.heap.get_obj_mut(obj).get_own_prop_mut(&key) {
                                e.value = PropValue::Data(val);
                            }
                            return Ok(true);
                        }
                    } else if self.heap.get_obj(obj).get_own_prop(&key).is_none()
                        && receiver == JsValue::Object(obj)
                    {
                        let deleted = self
                            .heap
                            .deleted_arg_indices
                            .get(&obj)
                            .is_some_and(|s| s.contains(&idx_u));
                        if !deleted {
                            if let ObjectKind::Arguments(ref mut a) =
                                self.heap.get_obj_mut(obj).kind
                            {
                                if idx_u < a.values.len() {
                                    a.values[idx_u] = val;
                                    return Ok(true);
                                }
                            }
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

        if let Some(pid) = self.heap.get_obj(obj).prototype {
            return self.set_property_key_receiver(pid, key, val, receiver);
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
        if matches!(self.heap.get_obj(obj).kind, ObjectKind::ModuleNamespace(_)) {
            if let PropertyKey::String(ref k) = key {
                let is_def_then = matches!(
                    &self.heap.get_obj(obj).kind,
                    ObjectKind::ModuleNamespace(m) if m.is_deferred && k == "then"
                );
                if is_def_then {
                    return self.ordinary_define_own_property(obj, key, desc);
                }
                self.ensure_deferred_ns_evaluated(obj)?;
                let is_exported = match &self.heap.get_obj(obj).kind {
                    ObjectKind::ModuleNamespace(m) => m
                        .exports
                        .iter()
                        .any(|(name, t)| name == k && !matches!(t, ExportTarget::Ambiguous)),
                    _ => false,
                };
                if !is_exported {
                    return Ok(false);
                }
                let cur_val = self.get_module_export_value(obj, k)?;
                if desc.is_accessor()
                    || desc.configurable == Some(true)
                    || desc.enumerable == Some(false)
                    || desc.writable == Some(false)
                {
                    return Ok(false);
                }
                if let Some(ref v) = desc.value {
                    if !v.same_value(&cur_val) {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
            return self.ordinary_define_own_property(obj, key, desc);
        }
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy defineProperty trap is not callable"));
                }
                let desc_obj = self.from_property_descriptor(&desc);
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), key.to_value(), desc_obj],
                )?;
                if !res.to_boolean() {
                    return Ok(false);
                }
                let target_desc = self.get_own_property_descriptor(target, &key)?;
                let extensible_target = self.is_extensible(target)?;
                let setting_config_false = desc.configurable == Some(false);
                if let Some(td) = target_desc {
                    if !is_compatible_property_descriptor(extensible_target, &desc, Some(&td)) {
                        return Err(JsError::type_err(
                            "Proxy defineProperty trap returned incompatible descriptor",
                        ));
                    }
                    if setting_config_false && td.configurable == Some(true) {
                        return Err(JsError::type_err(
                            "Proxy defineProperty trap cannot define non-configurable property for configurable target property",
                        ));
                    }
                    if !td.is_accessor()
                        && td.configurable == Some(false)
                        && td.writable == Some(true)
                        && desc.writable == Some(false)
                    {
                        return Err(JsError::type_err(
                            "Proxy defineProperty trap cannot define non-writable property for writable target property",
                        ));
                    }
                } else {
                    if !extensible_target {
                        return Err(JsError::type_err(
                            "Proxy defineProperty trap cannot add property to non-extensible target",
                        ));
                    }
                    if setting_config_false {
                        return Err(JsError::type_err(
                            "Proxy defineProperty trap cannot define non-configurable property not on target",
                        ));
                    }
                }
                return Ok(true);
            }
            return self.define_property(target, key, desc);
        }

        if let PropertyKey::String(ref k) = key {
            let is_array = matches!(self.heap.get_obj(obj).kind, ObjectKind::Array(_));
            if is_array {
                if k == "length" {
                    return self.array_set_length(obj, desc);
                }
                if let Some(idx) = parse_array_index(k) {
                    let old_len = match &self.heap.get_obj(obj).kind {
                        ObjectKind::Array(a) => a.length,
                        _ => 0,
                    };
                    if (idx as usize) >= old_len
                        && self.heap.non_writable_array_lengths.contains(&obj)
                    {
                        return Ok(false);
                    }
                    let obj_extensible = self.heap.get_obj(obj).extensible;
                    let (is_plain_data, existing_val) = match &mut self.heap.get_obj_mut(obj).kind {
                        ObjectKind::Array(ref mut arr) => {
                            let exists = arr.get(idx as usize).is_some();
                            if !obj_extensible && !exists {
                                return Ok(false);
                            }
                            let plain = !desc.is_accessor()
                                && !arr.has_custom_index_props
                                && (idx as usize) < 65_536
                                && if exists {
                                    desc.writable.unwrap_or(true)
                                        && desc.enumerable.unwrap_or(true)
                                        && desc.configurable.unwrap_or(true)
                                } else {
                                    desc.writable == Some(true)
                                        && desc.enumerable == Some(true)
                                        && desc.configurable == Some(true)
                                };
                            if plain {
                                let val = desc
                                    .value
                                    .clone()
                                    .or_else(|| arr.get(idx as usize).cloned())
                                    .unwrap_or(JsValue::Undefined);
                                arr.set(idx as usize, val);
                                (true, None)
                            } else {
                                arr.has_custom_index_props = true;
                                (false, arr.get(idx as usize).cloned())
                            }
                        }
                        _ => (false, None),
                    };
                    if is_plain_data {
                        return Ok(true);
                    }
                    if let Some(ev) = existing_val {
                        let o = self.heap.get_obj_mut(obj);
                        if o.get_own_prop(&key).is_none() {
                            o.set_own_prop(key.clone(), PropEntry::data(ev, true, true, true));
                        }
                    }
                    let ok = self.ordinary_define_own_property(obj, key, desc)?;
                    if ok {
                        if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(obj).kind {
                            if (idx as usize) >= arr.length {
                                arr.length = (idx as usize) + 1;
                            }
                        }
                    }
                    return Ok(ok);
                }
            }
            let ta_clone = match &self.heap.get_obj(obj).kind {
                ObjectKind::TypedArray(ta) => Some(ta.clone()),
                _ => None,
            };
            if let Some(ta) = ta_clone {
                if let Some(num_idx) = canonical_numeric_index_string(k) {
                    let Some(eff_len) = self.typed_array_effective_length(obj) else {
                        return Ok(false);
                    };
                    let Some(idx) = valid_integer_index(num_idx, eff_len) else {
                        return Ok(false);
                    };
                    if self
                        .heap
                        .buffer_meta
                        .get(&ta.buffer)
                        .is_some_and(|m| m.immutable)
                    {
                        if desc.is_accessor()
                            || desc.configurable == Some(true)
                            || desc.enumerable == Some(false)
                            || desc.writable == Some(true)
                        {
                            return Ok(false);
                        }
                        if let Some(ref v) = desc.value {
                            let cur = self.typed_array_get(&ta, idx)?;
                            if !v.same_value(&cur) {
                                return Ok(false);
                            }
                        }
                        return Ok(true);
                    }
                    if desc.configurable == Some(false)
                        || desc.is_accessor()
                        || desc.enumerable == Some(false)
                        || desc.writable == Some(false)
                    {
                        return Ok(false);
                    }
                    if let Some(ref v) = desc.value {
                        self.typed_array_set_checked(obj, &ta, num_idx, v)?;
                    }
                    return Ok(true);
                }
            }
            if matches!(self.heap.get_obj(obj).kind, ObjectKind::Arguments(_)) {
                if let Some(idx) = parse_array_index(k) {
                    let idx_u = idx as usize;
                    let (mapped_target, arg_val, deleted) = {
                        let del = self
                            .heap
                            .deleted_arg_indices
                            .get(&obj)
                            .is_some_and(|s| s.contains(&idx_u));
                        match &self.heap.get_obj(obj).kind {
                            ObjectKind::Arguments(a) => {
                                let mt = a.mapped_env.and_then(|eid| {
                                    a.mapped_params
                                        .get(idx_u)
                                        .and_then(|p| p.clone())
                                        .map(|pn| (eid, pn))
                                });
                                let av = a.values.get(idx_u).cloned();
                                (mt, av, del)
                            }
                            _ => (None, None, del),
                        }
                    };
                    let live_val = mapped_target.as_ref().and_then(|(eid, pn)| {
                        self.heap
                            .get_env(*eid)
                            .bindings
                            .get(pn)
                            .map(|b| b.value.clone())
                    });
                    if !deleted && self.heap.get_obj(obj).get_own_prop(&key).is_none() {
                        if let Some(v) = live_val.clone().or(arg_val) {
                            self.heap.get_obj_mut(obj).set_own_prop(
                                key.clone(),
                                PropEntry::data(v, true, true, true),
                            );
                        }
                    } else if let Some(ref lv) = live_val {
                        if let Some(e) = self.heap.get_obj_mut(obj).get_own_prop_mut(&key) {
                            if matches!(e.value, PropValue::Data(_)) {
                                e.value = PropValue::Data(lv.clone());
                            }
                        }
                    }
                    let mut new_arg_desc = desc.clone();
                    if mapped_target.is_some()
                        && !desc.is_accessor()
                        && desc.value.is_none()
                        && desc.writable == Some(false)
                    {
                        if let Some(lv) = live_val {
                            new_arg_desc.value = Some(lv);
                        }
                    }
                    let allowed = self.ordinary_define_own_property(obj, key.clone(), new_arg_desc)?;
                    if !allowed {
                        return Ok(false);
                    }
                    if let Some(set) = self.heap.deleted_arg_indices.get_mut(&obj) {
                        set.remove(&idx_u);
                    }
                    if let Some((eid, pname)) = mapped_target {
                        if desc.is_accessor() {
                            if let ObjectKind::Arguments(ref mut a) =
                                self.heap.get_obj_mut(obj).kind
                            {
                                if idx_u < a.mapped_params.len() {
                                    a.mapped_params[idx_u] = None;
                                }
                            }
                        } else {
                            if let Some(v) = desc.value {
                                if let Some(b) = self.heap.get_env_mut(eid).bindings.get_mut(&pname)
                                {
                                    b.value = v.clone();
                                }
                                if let ObjectKind::Arguments(ref mut a) =
                                    self.heap.get_obj_mut(obj).kind
                                {
                                    if idx_u < a.values.len() {
                                        a.values[idx_u] = v;
                                    }
                                }
                            }
                            if desc.writable == Some(false) {
                                if let ObjectKind::Arguments(ref mut a) =
                                    self.heap.get_obj_mut(obj).kind
                                {
                                    if idx_u < a.mapped_params.len() {
                                        a.mapped_params[idx_u] = None;
                                    }
                                }
                            }
                        }
                    }
                    return Ok(true);
                }
            }
            let str_cur = match &self.heap.get_obj(obj).kind {
                ObjectKind::StringObj(s) => {
                    if k == "length" {
                        Some(PropertyDescriptor::data(
                            JsValue::Number(s.encode_utf16().count() as f64),
                            false,
                            false,
                            false,
                        ))
                    } else if let Some(idx) = parse_array_index(k) {
                        s.encode_utf16().nth(idx as usize).map(|u| {
                            PropertyDescriptor::data(
                                JsValue::String(String::from_utf16_lossy(&[u])),
                                false,
                                true,
                                false,
                            )
                        })
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(cur_desc) = str_cur {
                return Ok(is_compatible_property_descriptor(
                    self.heap.get_obj(obj).extensible,
                    &desc,
                    Some(&cur_desc),
                ));
            }
        }

        self.ordinary_define_own_property(obj, key, desc)
    }

    fn ordinary_define_own_property(
        &mut self,
        obj: ObjectId,
        key: PropertyKey,
        desc: PropertyDescriptor,
    ) -> Result<bool, JsError> {
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
                if cur_is_acc {
                    if let PropValue::Accessor { ref get, ref set } = cur.value {
                        if let Some(ref ng) = desc.get {
                            let cg = get.as_ref().unwrap_or(&JsValue::Undefined);
                            if !ng.same_value(cg) {
                                return Ok(false);
                            }
                        }
                        if let Some(ref ns) = desc.set {
                            let cs = set.as_ref().unwrap_or(&JsValue::Undefined);
                            if !ns.same_value(cs) {
                                return Ok(false);
                            }
                        }
                    }
                } else if !cur.writable {
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

    fn array_set_length(&mut self, obj: ObjectId, desc: PropertyDescriptor) -> Result<bool, JsError> {
        let old_writable = !self.heap.non_writable_array_lengths.contains(&obj);
        let Some(ref v) = desc.value else {
            if desc.configurable == Some(true)
                || desc.enumerable == Some(true)
                || desc.is_accessor()
                || (!old_writable && desc.writable == Some(true))
            {
                return Ok(false);
            }
            if desc.writable == Some(false) {
                self.heap.non_writable_array_lengths.insert(obj);
            }
            return Ok(true);
        };
        let new_len_u32 = to_uint32(self.to_number(v)?);
        let num_len = self.to_number(v)?;
        if new_len_u32 as f64 != num_len {
            return Err(JsError::range("Invalid array length"));
        }
        let new_len = new_len_u32 as usize;
        let old_writable = !self.heap.non_writable_array_lengths.contains(&obj);
        let old_len = match &self.heap.get_obj(obj).kind {
            ObjectKind::Array(a) => a.length,
            _ => 0,
        };
        if desc.configurable == Some(true)
            || desc.enumerable == Some(true)
            || desc.is_accessor()
        {
            return Ok(false);
        }
        if !old_writable {
            if desc.writable == Some(true) || new_len != old_len {
                return Ok(false);
            }
            return Ok(true);
        }
        let new_writable = desc.writable.unwrap_or(true);
        if new_len >= old_len {
            if let ObjectKind::Array(ref mut a) = self.heap.get_obj_mut(obj).kind {
                a.length = new_len;
            }
            if !new_writable {
                self.heap.non_writable_array_lengths.insert(obj);
            }
            return Ok(true);
        }
        let mut custom_to_remove: Vec<(usize, String, bool)> = Vec::new();
        {
            let o = self.heap.get_obj(obj);
            for slot in o.prop_list.iter().flatten() {
                if let Some(idx) = parse_array_index(&slot.0) {
                    if (idx as usize) >= new_len {
                        custom_to_remove.push((idx as usize, slot.0.clone(), slot.1.configurable));
                    }
                }
            }
        }
        custom_to_remove.sort_by(|a, b| b.0.cmp(&a.0));
        let mut final_len = new_len;
        let mut succeeded = true;
        for (idx, key_name, configurable) in custom_to_remove {
            if !configurable {
                final_len = idx + 1;
                succeeded = false;
                break;
            }
            self.heap
                .get_obj_mut(obj)
                .remove_own_prop(&PropertyKey::String(key_name));
        }
        if let ObjectKind::Array(ref mut a) = self.heap.get_obj_mut(obj).kind {
            a.truncate(final_len);
        }
        if !new_writable {
            self.heap.non_writable_array_lengths.insert(obj);
        }
        Ok(succeeded)
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy deleteProperty trap is not callable"));
                }
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), key.to_value()],
                )?;
                if !res.to_boolean() {
                    return Ok(false);
                }
                if let Some(td) = self.get_own_property_descriptor(target, key)? {
                    if td.configurable == Some(false) {
                        return Err(JsError::type_err(
                            "Proxy deleteProperty trap cannot delete non-configurable property",
                        ));
                    }
                    if !self.is_extensible(target)? {
                        return Err(JsError::type_err(
                            "Proxy deleteProperty trap cannot delete property of non-extensible target",
                        ));
                    }
                }
                return Ok(true);
            }
            return self.delete_property(target, key);
        }

        if let PropertyKey::String(ref k) = key {
            match &mut self.heap.get_obj_mut(obj).kind {
                ObjectKind::Array(_) => {
                    if k == "length" {
                        return Ok(false);
                    }
                    if let Some(idx) = parse_array_index(k) {
                        if let Some(entry) = self.heap.get_obj(obj).get_own_prop(key) {
                            if !entry.configurable {
                                return Ok(false);
                            }
                        }
                        if let ObjectKind::Array(ref mut arr) = self.heap.get_obj_mut(obj).kind {
                            arr.delete(idx as usize);
                        }
                        self.heap.get_obj_mut(obj).remove_own_prop(key);
                        return Ok(true);
                    }
                }
                ObjectKind::StringObj(s) => {
                    if k == "length" {
                        return Ok(false);
                    }
                    if let Some(idx) = parse_array_index(k) {
                        if (idx as usize) < s.encode_utf16().count() {
                            return Ok(false);
                        }
                    }
                }
                ObjectKind::Arguments(args) => {
                    let arg_len = args.values.len();
                    if let Some(idx) = parse_array_index(k) {
                        let idx_u = idx as usize;
                        if idx_u < arg_len {
                            if let Some(entry) = self.heap.get_obj(obj).get_own_prop(key) {
                                if !entry.configurable {
                                    return Ok(false);
                                }
                            }
                            self.heap
                                .deleted_arg_indices
                                .entry(obj)
                                .or_default()
                                .insert(idx_u);
                            self.heap.get_obj_mut(obj).remove_own_prop(key);
                            if let ObjectKind::Arguments(ref mut a) =
                                self.heap.get_obj_mut(obj).kind
                            {
                                if idx_u < a.mapped_params.len() {
                                    a.mapped_params[idx_u] = None;
                                }
                            }
                            return Ok(true);
                        }
                    }
                }
                ObjectKind::TypedArray(_) => {
                    if let Some(num_idx) = canonical_numeric_index_string(k) {
                        if let Some(eff_len) = self.typed_array_effective_length(obj) {
                            if valid_integer_index(num_idx, eff_len).is_some() {
                                return Ok(false);
                            }
                        }
                        return Ok(true);
                    }
                }
                ObjectKind::ModuleNamespace(m) => {
                    if m.is_deferred && k == "then" {
                        return Ok(true);
                    }
                    let is_exported = m
                        .exports
                        .iter()
                        .any(|(name, t)| name == k && !matches!(t, ExportTarget::Ambiguous));
                    let is_def = m.is_deferred;
                    if is_def {
                        self.ensure_deferred_ns_evaluated(obj)?;
                    }
                    return Ok(!is_exported);
                }
                _ => {}
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
                    if !self.is_callable(&trap) {
                        return Err(JsError::type_err("Proxy has trap is not callable"));
                    }
                    let res = self.call_function(
                        trap,
                        JsValue::Object(handler),
                        &[JsValue::Object(target), key.to_value()],
                    )?;
                    let bool_res = res.to_boolean();
                    if !bool_res {
                        if let Some(td) = self.get_own_property_descriptor(target, key)? {
                            if td.configurable == Some(false) {
                                return Err(JsError::type_err(
                                    "Proxy has trap cannot hide non-configurable property",
                                ));
                            }
                            if !self.is_extensible(target)? {
                                return Err(JsError::type_err(
                                    "Proxy has trap cannot hide property of non-extensible target",
                                ));
                            }
                        }
                    }
                    return Ok(bool_res);
                }
                return self.has_property(target, key);
            }
            if let PropertyKey::String(ref k) = key {
                if matches!(self.heap.get_obj(cur_id).kind, ObjectKind::TypedArray(_)) {
                    if let Some(num_idx) = canonical_numeric_index_string(k) {
                        if let Some(eff_len) = self.typed_array_effective_length(cur_id) {
                            return Ok(valid_integer_index(num_idx, eff_len).is_some());
                        }
                        return Ok(false);
                    }
                }
            }
            if matches!(self.heap.get_obj(cur_id).kind, ObjectKind::ModuleNamespace(_)) {
                if let PropertyKey::String(ref k) = key {
                    let is_def_then = matches!(
                        &self.heap.get_obj(cur_id).kind,
                        ObjectKind::ModuleNamespace(m) if m.is_deferred && k == "then"
                    );
                    if is_def_then {
                        return Ok(false);
                    }
                    self.ensure_deferred_ns_evaluated(cur_id)?;
                    let is_exported = match &self.heap.get_obj(cur_id).kind {
                        ObjectKind::ModuleNamespace(m) => m
                            .exports
                            .iter()
                            .any(|(name, t)| name == k && !matches!(t, ExportTarget::Ambiguous)),
                        _ => false,
                    };
                    return Ok(is_exported);
                }
                return Ok(self.heap.get_obj(cur_id).get_own_prop(key).is_some());
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err(
                        "Proxy getOwnPropertyDescriptor trap is not callable",
                    ));
                }
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), key.to_value()],
                )?;
                if !res.is_undefined() && !res.is_object() {
                    return Err(JsError::type_err(
                        "Proxy getOwnPropertyDescriptor trap must return an object or undefined",
                    ));
                }
                let target_desc = self.get_own_property_descriptor(target, key)?;
                if res.is_undefined() {
                    if let Some(td) = target_desc {
                        if td.configurable == Some(false) {
                            return Err(JsError::type_err(
                                "Proxy getOwnPropertyDescriptor trap cannot return undefined for non-configurable property",
                            ));
                        }
                        if !self.is_extensible(target)? {
                            return Err(JsError::type_err(
                                "Proxy getOwnPropertyDescriptor trap cannot return undefined for property of non-extensible target",
                            ));
                        }
                    }
                    return Ok(None);
                }
                let extensible_target = self.is_extensible(target)?;
                let mut result_desc = self.to_property_descriptor(&res)?;
                complete_property_descriptor(&mut result_desc);
                if !is_compatible_property_descriptor(
                    extensible_target,
                    &result_desc,
                    target_desc.as_ref(),
                ) {
                    return Err(JsError::type_err(
                        "Proxy getOwnPropertyDescriptor trap returned incompatible descriptor",
                    ));
                }
                if result_desc.configurable == Some(false) {
                    match &target_desc {
                        None => {
                            return Err(JsError::type_err(
                                "Proxy getOwnPropertyDescriptor trap cannot report non-existent property as non-configurable",
                            ));
                        }
                        Some(td) if td.configurable == Some(true) => {
                            return Err(JsError::type_err(
                                "Proxy getOwnPropertyDescriptor trap cannot report configurable property as non-configurable",
                            ));
                        }
                        Some(td)
                            if !result_desc.is_accessor()
                                && result_desc.writable == Some(false)
                                && !td.is_accessor()
                                && td.writable == Some(true) =>
                        {
                            return Err(JsError::type_err(
                                "Proxy getOwnPropertyDescriptor trap cannot report writable property as non-writable",
                            ));
                        }
                        _ => {}
                    }
                }
                return Ok(Some(result_desc));
            }
            return self.get_own_property_descriptor(target, key);
        }

        let o = self.heap.get_obj(obj);
        if let Some(entry) = o.get_own_prop(key) {
            let mut desc = entry.to_descriptor();
            if let (PropertyKey::String(k), ObjectKind::Arguments(args)) = (key, &o.kind) {
                if let Some(idx) = parse_array_index(k) {
                    if let Some(env_id) = args.mapped_env {
                        if let Some(Some(ref pname)) = args.mapped_params.get(idx as usize) {
                            if let Some(b) = self.heap.get_env(env_id).bindings.get(pname) {
                                desc.value = Some(b.value.clone());
                            }
                        }
                    }
                }
            }
            return Ok(Some(desc));
        }
        if let PropertyKey::String(ref k) = key {
            match &o.kind {
                ObjectKind::Array(arr) => {
                    if k == "length" {
                        let writable = !self.heap.non_writable_array_lengths.contains(&obj);
                        return Ok(Some(PropertyDescriptor::data(
                            JsValue::Number(arr.length as f64),
                            writable,
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
                        let idx_u = idx as usize;
                        if let Some(env_id) = args.mapped_env {
                            if let Some(Some(ref pname)) = args.mapped_params.get(idx_u) {
                                if let Some(b) = self.heap.get_env(env_id).bindings.get(pname) {
                                    return Ok(Some(PropertyDescriptor::data(
                                        b.value.clone(),
                                        true,
                                        true,
                                        true,
                                    )));
                                }
                            }
                        }
                        let deleted = self
                            .heap
                            .deleted_arg_indices
                            .get(&obj)
                            .is_some_and(|s| s.contains(&idx_u));
                        if !deleted {
                            if let Some(v) = args.values.get(idx_u) {
                                return Ok(Some(PropertyDescriptor::data(v.clone(), true, true, true)));
                            }
                        }
                    }
                }
                ObjectKind::TypedArray(ta) => {
                    if let Some(num_idx) = canonical_numeric_index_string(k) {
                        let ta_clone = ta.clone();
                        if let Some(eff_len) = self.typed_array_effective_length(obj) {
                            if let Some(idx) = valid_integer_index(num_idx, eff_len) {
                                let v = self.typed_array_get(&ta_clone, idx)?;
                                let is_imm = self
                                    .heap
                                    .buffer_meta
                                    .get(&ta_clone.buffer)
                                    .is_some_and(|m| m.immutable);
                                return Ok(Some(PropertyDescriptor::data(v, !is_imm, true, !is_imm)));
                            }
                        }
                        return Ok(None);
                    }
                }
                ObjectKind::ModuleNamespace(m) => {
                    if m.is_deferred && k == "then" {
                        return Ok(None);
                    }
                    let is_exported = m
                        .exports
                        .iter()
                        .any(|(name, t)| name == k && !matches!(t, ExportTarget::Ambiguous));
                    let is_def = m.is_deferred;
                    if is_def {
                        self.ensure_deferred_ns_evaluated(obj)?;
                    }
                    if is_exported {
                        let val = self.get_module_export_value(obj, k)?;
                        return Ok(Some(PropertyDescriptor::data(val, true, true, false)));
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub fn from_property_descriptor(&mut self, desc: &PropertyDescriptor) -> JsValue {
        let obj = self.create_object();
        if desc.is_accessor() {
            let g = desc.get.clone().unwrap_or(JsValue::Undefined);
            let s = desc.set.clone().unwrap_or(JsValue::Undefined);
            let _ = self.set_property(obj, "get", g);
            let _ = self.set_property(obj, "set", s);
        } else {
            if let Some(ref v) = desc.value {
                let _ = self.set_property(obj, "value", v.clone());
            }
            if let Some(w) = desc.writable {
                let _ = self.set_property(obj, "writable", JsValue::Boolean(w));
            }
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
        if matches!(
            &self.heap.get_obj(obj).kind,
            ObjectKind::ModuleNamespace(m) if m.is_deferred
        ) {
            self.ensure_deferred_ns_evaluated(obj)?;
        }
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy ownKeys trap is not callable"));
                }
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target)],
                )?;
                let JsValue::Object(list_oid) = res else {
                    return Err(JsError::type_err("Proxy ownKeys trap must return an object"));
                };
                let len_val = self.get_property(list_oid, "length")?;
                let len = to_length(self.to_number(&len_val)?);
                let mut keys = Vec::with_capacity(len);
                let mut seen = std::collections::HashSet::new();
                for i in 0..len {
                    let elem = self.get_property(list_oid, &i.to_string())?;
                    let pk = match elem {
                        JsValue::String(s) => PropertyKey::String(s),
                        JsValue::Symbol(id) => PropertyKey::Symbol(id),
                        _ => {
                            return Err(JsError::type_err(
                                "Proxy ownKeys trap must return strings or symbols",
                            ));
                        }
                    };
                    if !seen.insert(pk.clone()) {
                        return Err(JsError::type_err(
                            "Proxy ownKeys trap returned duplicate key",
                        ));
                    }
                    keys.push(pk);
                }
                let extensible_target = self.is_extensible(target)?;
                let target_keys = self.own_keys(target)?;
                let mut target_nonconfig = Vec::new();
                let mut target_config = Vec::new();
                for tk in target_keys {
                    let desc = self.get_own_property_descriptor(target, &tk)?;
                    if let Some(d) = desc {
                        if d.configurable == Some(false) {
                            target_nonconfig.push(tk);
                        } else {
                            target_config.push(tk);
                        }
                    }
                }
                if extensible_target && target_nonconfig.is_empty() {
                    return Ok(keys);
                }
                let mut unchecked = seen;
                for k in &target_nonconfig {
                    if !unchecked.remove(k) {
                        return Err(JsError::type_err(
                            "Proxy ownKeys trap omitted non-configurable key",
                        ));
                    }
                }
                if extensible_target {
                    return Ok(keys);
                }
                for k in &target_config {
                    if !unchecked.remove(k) {
                        return Err(JsError::type_err(
                            "Proxy ownKeys trap omitted key of non-extensible target",
                        ));
                    }
                }
                if !unchecked.is_empty() {
                    return Err(JsError::type_err(
                        "Proxy ownKeys trap returned extra keys for non-extensible target",
                    ));
                }
                return Ok(keys);
            }
            return self.own_keys(target);
        }
        if matches!(self.heap.get_obj(obj).kind, ObjectKind::TypedArray(_)) {
            let eff_len = self.typed_array_effective_length(obj).unwrap_or(0);
            let mut keys = Vec::with_capacity(eff_len);
            for i in 0..eff_len {
                keys.push(PropertyKey::String(i.to_string()));
            }
            let o = self.heap.get_obj(obj);
            for item in o.prop_list.iter().flatten() {
                keys.push(PropertyKey::String(item.0.clone()));
            }
            for (sym, _) in &o.symbol_props {
                keys.push(PropertyKey::Symbol(*sym));
            }
            return Ok(keys);
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy getPrototypeOf trap is not callable"));
                }
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target)],
                )?;
                let handler_proto = match res {
                    JsValue::Null => None,
                    JsValue::Object(id) => Some(id),
                    _ => {
                        return Err(JsError::type_err(
                            "Proxy getPrototypeOf trap must return an object or null",
                        ));
                    }
                };
                if !self.is_extensible(target)? {
                    let target_proto = self.get_prototype_of(target)?;
                    if handler_proto != target_proto {
                        return Err(JsError::type_err(
                            "Proxy getPrototypeOf trap returned different prototype for non-extensible target",
                        ));
                    }
                }
                return Ok(handler_proto);
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
        if obj == self.intrinsics.object_proto
            || self.realms.iter().any(|r| r.intrinsics.object_proto == obj)
        {
            return Ok(proto.is_none());
        }
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy setPrototypeOf trap is not callable"));
                }
                let pv = proto.map(JsValue::Object).unwrap_or(JsValue::Null);
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target), pv],
                )?;
                if !res.to_boolean() {
                    return Ok(false);
                }
                if !self.is_extensible(target)? {
                    let target_proto = self.get_prototype_of(target)?;
                    if proto != target_proto {
                        return Err(JsError::type_err(
                            "Proxy setPrototypeOf trap returned true for non-extensible target",
                        ));
                    }
                }
                return Ok(true);
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err("Proxy isExtensible trap is not callable"));
                }
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target)],
                )?;
                let bool_res = res.to_boolean();
                let target_ext = self.is_extensible(target)?;
                if bool_res != target_ext {
                    return Err(JsError::type_err(
                        "Proxy isExtensible trap result does not match target extensibility",
                    ));
                }
                return Ok(bool_res);
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
                if !self.is_callable(&trap) {
                    return Err(JsError::type_err(
                        "Proxy preventExtensions trap is not callable",
                    ));
                }
                let res = self.call_function(
                    trap,
                    JsValue::Object(handler),
                    &[JsValue::Object(target)],
                )?;
                let bool_res = res.to_boolean();
                if bool_res && self.is_extensible(target)? {
                    return Err(JsError::type_err(
                        "Proxy preventExtensions trap returned true while target is extensible",
                    ));
                }
                return Ok(bool_res);
            }
            return self.prevent_extensions(target);
        }
        if let ObjectKind::TypedArray(ref ta) = self.heap.get_obj(obj).kind {
            let buf_id = ta.buffer;
            let is_auto = self.heap.auto_length_views.contains(&obj);
            let is_resizable_ab = self
                .heap
                .buffer_meta
                .get(&buf_id)
                .map(|m| m.max_byte_length.is_some() && !m.is_shared)
                .unwrap_or(false);
            if is_auto || is_resizable_ab {
                return Ok(false);
            }
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
                Ok(n.cmp_f64(*f) == Some(std::cmp::Ordering::Equal))
            }
            (JsValue::BigInt(n), JsValue::String(s))
            | (JsValue::String(s), JsValue::BigInt(n)) => {
                match crate::bigint::JsBigInt::parse_string(s) {
                    Some(m) => Ok(*n == m),
                    None => Ok(false),
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

    pub fn to_numeric(&mut self, val: &JsValue) -> Result<JsValue, JsError> {
        let prim = self.to_primitive(val, "number")?;
        if let JsValue::BigInt(n) = prim {
            Ok(JsValue::BigInt(n))
        } else {
            Ok(JsValue::Number(self.to_number(&prim)?))
        }
    }

    pub fn relational_lt(
        &mut self,
        a: &JsValue,
        b: &JsValue,
        left_first: bool,
    ) -> Result<Option<bool>, JsError> {
        let (px, py) = if left_first {
            let x = self.to_primitive(a, "number")?;
            let y = self.to_primitive(b, "number")?;
            (x, y)
        } else {
            let y = self.to_primitive(b, "number")?;
            let x = self.to_primitive(a, "number")?;
            (x, y)
        };
        if let (JsValue::String(sx), JsValue::String(sy)) = (&px, &py) {
            let ux: Vec<u16> = sx.encode_utf16().collect();
            let uy: Vec<u16> = sy.encode_utf16().collect();
            return Ok(Some(ux < uy));
        }
        if let (JsValue::BigInt(x), JsValue::String(sy)) = (&px, &py) {
            let Some(ny) = crate::bigint::JsBigInt::parse_string(sy) else {
                return Ok(None);
            };
            return Ok(Some(x < &ny));
        }
        if let (JsValue::String(sx), JsValue::BigInt(y)) = (&px, &py) {
            let Some(nx) = crate::bigint::JsBigInt::parse_string(sx) else {
                return Ok(None);
            };
            return Ok(Some(&nx < y));
        }
        let nx = self.to_numeric(&px)?;
        let ny = self.to_numeric(&py)?;
        match (&nx, &ny) {
            (JsValue::BigInt(x), JsValue::BigInt(y)) => Ok(Some(x < y)),
            (JsValue::Number(x), JsValue::Number(y)) => {
                if x.is_nan() || y.is_nan() {
                    Ok(None)
                } else {
                    Ok(Some(x < y))
                }
            }
            (JsValue::BigInt(x), JsValue::Number(y)) => Ok(x.cmp_f64(*y).map(|o| o.is_lt())),
            (JsValue::Number(x), JsValue::BigInt(y)) => Ok(y.cmp_f64(*x).map(|o| o.is_gt())),
            _ => Ok(None),
        }
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

    pub fn buffer_byte_len(&self, buf_id: ObjectId) -> Option<usize> {
        if let Some(sb) = self
            .heap
            .buffer_meta
            .get(&buf_id)
            .and_then(|m| m.shared.as_ref())
        {
            return Some(sb.data.lock().unwrap().len());
        }
        match &self.heap.get_obj(buf_id).kind {
            ObjectKind::ArrayBuffer(ab) if !ab.detached => Some(ab.bytes.len()),
            _ => None,
        }
    }

    pub fn shared_buffer_backing(
        &self,
        buf_id: ObjectId,
    ) -> Option<std::sync::Arc<SharedBufferBacking>> {
        self.heap
            .buffer_meta
            .get(&buf_id)
            .and_then(|m| m.shared.clone())
    }

    pub fn create_shared_array_buffer_from_backing(
        &mut self,
        backing: std::sync::Arc<SharedBufferBacking>,
    ) -> ObjectId {
        let max_len = backing.max_byte_length;
        let obj = self.heap.alloc_object(JsObject::new(
            Some(self.intrinsics.shared_array_buffer_proto),
            ObjectKind::ArrayBuffer(ArrayBufferData {
                bytes: Vec::new(),
                detached: false,
            }),
        ));
        self.heap.buffer_meta.insert(
            obj,
            BufferMeta {
                max_byte_length: max_len,
                is_shared: true,
                immutable: false,
                shared: Some(backing),
            },
        );
        obj
    }

    pub fn has_pending_async_waiters(&self) -> bool {
        !self.async_waiters.is_empty()
    }

    pub fn next_async_waiter_deadline(&self) -> Option<std::time::Instant> {
        self.async_waiters.iter().filter_map(|w| w.4).min()
    }

    pub fn tick_async_waiters(&mut self) -> bool {
        if self.async_waiters.is_empty() {
            return false;
        }
        let now = std::time::Instant::now();
        let mut remaining = Vec::with_capacity(self.async_waiters.len());
        let mut resolved: Vec<(ObjectId, &'static str)> = Vec::new();
        let current = std::mem::take(&mut self.async_waiters);
        for (backing, byte_idx, entry, promise, deadline) in current {
            let mut st = entry.state.lock().unwrap();
            match *st {
                WaiterState::Notified => {
                    drop(st);
                    resolved.push((promise, "ok"));
                }
                WaiterState::TimedOut => {
                    drop(st);
                    resolved.push((promise, "timed-out"));
                }
                WaiterState::Waiting => {
                    if deadline.is_some_and(|d| now >= d) {
                        *st = WaiterState::TimedOut;
                        drop(st);
                        let mut map = backing.waiters.lock().unwrap();
                        if let Some(q) = map.get_mut(&byte_idx) {
                            q.retain(|e| !std::sync::Arc::ptr_eq(e, &entry));
                        }
                        resolved.push((promise, "timed-out"));
                    } else {
                        drop(st);
                        remaining.push((backing, byte_idx, entry, promise, deadline));
                    }
                }
            }
        }
        self.async_waiters = remaining;
        if resolved.is_empty() {
            return false;
        }
        for (p, status) in resolved {
            self.resolve_promise(p, JsValue::String(status.into()));
        }
        self.drain_microtasks();
        true
    }

    pub fn typed_array_effective_length(&self, oid: ObjectId) -> Option<usize> {
        let ObjectKind::TypedArray(ref ta) = self.heap.get_obj(oid).kind else {
            return None;
        };
        let buf_len = self.buffer_byte_len(ta.buffer)?;
        let es = ta.kind.element_size();
        if self.heap.auto_length_views.contains(&oid) {
            if ta.byte_offset > buf_len {
                return None;
            }
            Some((buf_len - ta.byte_offset) / es)
        } else {
            let byte_end = ta.byte_offset.checked_add(ta.length.checked_mul(es)?)?;
            if byte_end > buf_len {
                return None;
            }
            Some(ta.length)
        }
    }

    pub fn data_view_effective_byte_length(&self, oid: ObjectId) -> Option<usize> {
        let ObjectKind::DataView(ref dv) = self.heap.get_obj(oid).kind else {
            return None;
        };
        let buf_len = self.buffer_byte_len(dv.buffer)?;
        if self.heap.auto_length_views.contains(&oid) {
            if dv.byte_offset > buf_len {
                return None;
            }
            Some(buf_len - dv.byte_offset)
        } else {
            let byte_end = dv.byte_offset.checked_add(dv.byte_length)?;
            if byte_end > buf_len {
                return None;
            }
            Some(dv.byte_length)
        }
    }

    pub fn typed_array_set_checked(
        &mut self,
        oid: ObjectId,
        ta: &TypedArrayData,
        num_idx: f64,
        val: &JsValue,
    ) -> Result<(), JsError> {
        let converted = if ta.kind.is_bigint() {
            JsValue::BigInt(self.to_bigint(val)?)
        } else {
            JsValue::Number(self.to_number(val)?)
        };
        if let Some(eff_len) = self.typed_array_effective_length(oid) {
            if let Some(idx) = valid_integer_index(num_idx, eff_len) {
                self.typed_array_set_raw(ta, idx, &converted)?;
            }
        }
        Ok(())
    }

    pub fn typed_array_set_raw(
        &mut self,
        ta: &TypedArrayData,
        index: usize,
        converted: &JsValue,
    ) -> Result<(), JsError> {
        if self
            .heap
            .buffer_meta
            .get(&ta.buffer)
            .is_some_and(|m| m.immutable)
        {
            return Err(JsError::type_err("TypedArray buffer is immutable"));
        }
        let es = ta.kind.element_size();
        let off = ta.byte_offset + index * es;
        let write_into = |buf: &mut [u8]| {
            if off + es > buf.len() {
                return;
            }
            match converted {
                JsValue::BigInt(bi) => {
                    let bytes = match ta.kind {
                        TypedArrayKind::BigInt64 => bi.to_i64_wrapping().to_le_bytes(),
                        TypedArrayKind::BigUint64 => bi.to_u64_wrapping().to_le_bytes(),
                        _ => return,
                    };
                    buf[off..off + 8].copy_from_slice(&bytes);
                }
                JsValue::Number(n) => {
                    let n = *n;
                    match ta.kind {
                        TypedArrayKind::Int8 => buf[off] = to_int32(n) as i8 as u8,
                        TypedArrayKind::Uint8 => buf[off] = to_uint32(n) as u8,
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
                            buf[off] = clamped;
                        }
                        TypedArrayKind::Int16 => {
                            buf[off..off + 2].copy_from_slice(&(to_int32(n) as i16).to_le_bytes());
                        }
                        TypedArrayKind::Uint16 => {
                            buf[off..off + 2].copy_from_slice(&(to_uint32(n) as u16).to_le_bytes());
                        }
                        TypedArrayKind::Int32 => {
                            buf[off..off + 4].copy_from_slice(&to_int32(n).to_le_bytes());
                        }
                        TypedArrayKind::Uint32 => {
                            buf[off..off + 4].copy_from_slice(&to_uint32(n).to_le_bytes());
                        }
                        TypedArrayKind::Float32 => {
                            buf[off..off + 4].copy_from_slice(&(n as f32).to_le_bytes());
                        }
                        TypedArrayKind::Float64 => {
                            buf[off..off + 8].copy_from_slice(&n.to_le_bytes());
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        };
        if let Some(sb) = self.shared_buffer_backing(ta.buffer) {
            let mut guard = sb.data.lock().unwrap();
            write_into(&mut guard);
            return Ok(());
        }
        let ObjectKind::ArrayBuffer(ref mut ab) = self.heap.get_obj_mut(ta.buffer).kind else {
            return Err(JsError::type_err("Invalid ArrayBuffer"));
        };
        if ab.detached {
            return Ok(());
        }
        write_into(&mut ab.bytes);
        Ok(())
    }

    pub fn typed_array_get(&self, ta: &TypedArrayData, index: usize) -> Result<JsValue, JsError> {
        let es = ta.kind.element_size();
        let off = ta.byte_offset + index * es;
        let mut raw = [0u8; 8];
        if let Some(sb) = self.shared_buffer_backing(ta.buffer) {
            let guard = sb.data.lock().unwrap();
            if off + es > guard.len() {
                return Ok(JsValue::Undefined);
            }
            raw[..es].copy_from_slice(&guard[off..off + es]);
        } else {
            let ObjectKind::ArrayBuffer(ref ab) = self.heap.get_obj(ta.buffer).kind else {
                return Err(JsError::type_err("Invalid ArrayBuffer"));
            };
            if ab.detached || off + es > ab.bytes.len() {
                return Ok(JsValue::Undefined);
            }
            raw[..es].copy_from_slice(&ab.bytes[off..off + es]);
        }
        let slice = &raw[..es];
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
            TypedArrayKind::BigInt64 => JsValue::BigInt(crate::bigint::JsBigInt::from_i64(
                i64::from_le_bytes(slice.try_into().unwrap()),
            )),
            TypedArrayKind::BigUint64 => JsValue::BigInt(crate::bigint::JsBigInt::from_u64(
                u64::from_le_bytes(slice.try_into().unwrap()),
            )),
        })
    }

    pub fn typed_array_set(
        &mut self,
        ta: &TypedArrayData,
        index: usize,
        val: &JsValue,
    ) -> Result<(), JsError> {
        let converted = if ta.kind.is_bigint() {
            JsValue::BigInt(self.to_bigint(val)?)
        } else {
            JsValue::Number(self.to_number(val)?)
        };
        self.typed_array_set_raw(ta, index, &converted)
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
        let JsValue::Object(iter_oid) = iter else {
            return Err(JsError::type_err("Iterator must return an object"));
        };
        if is_async {
            let next_fn = self.get_property(iter_oid, "next")?;
            return Ok(JsValue::Object(
                self.create_async_from_sync_iterator(iter_oid, next_fn),
            ));
        }
        Ok(iter)
    }

    fn async_from_sync_continuation(
        &mut self,
        sync_iter: ObjectId,
        result: JsValue,
        out_p: ObjectId,
    ) {
        let step: Result<(bool, JsValue), JsError> = (|| {
            let JsValue::Object(r_oid) = result else {
                return Err(JsError::type_err("Iterator result is not an object"));
            };
            let done = self.get_property(r_oid, "done")?.to_boolean();
            let val = self.get_property(r_oid, "value")?;
            Ok((done, val))
        })();
        let (done, val) = match step {
            Ok(pair) => pair,
            Err(e) => {
                let ev = self.error_to_js_value(e);
                self.reject_promise(out_p, ev);
                return;
            }
        };
        let value_p = match self.promise_resolve_checked(val) {
            Ok(p) => p,
            Err(e) => {
                if !done {
                    self.iterator_close(&JsValue::Object(sync_iter));
                }
                let ev = self.error_to_js_value(e);
                self.reject_promise(out_p, ev);
                return;
            }
        };
        let on_ok = self.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, a| {
                let v = a.first().cloned().unwrap_or(JsValue::Undefined);
                let ir = rt.create_iter_result(v, done);
                rt.resolve_promise(out_p, ir);
                Ok(JsValue::Undefined)
            }),
        );
        let on_err = self.create_native_function(
            "",
            1,
            Rc::new(move |rt, _, a| {
                let err = a.first().cloned().unwrap_or(JsValue::Undefined);
                if !done {
                    let _ = rt.iterator_close_checked(&JsValue::Object(sync_iter), true);
                }
                rt.reject_promise(out_p, err);
                Ok(JsValue::Undefined)
            }),
        );
        self.promise_then(
            value_p,
            Some(JsValue::Object(on_ok)),
            Some(JsValue::Object(on_err)),
        );
    }

    fn promise_resolve_checked(&mut self, value: JsValue) -> Result<ObjectId, JsError> {
        if let JsValue::Object(oid) = value {
            if matches!(self.heap.get_obj(oid).kind, ObjectKind::Promise(_)) {
                let ctor = self.get_property(oid, "constructor")?;
                if ctor == JsValue::Object(self.intrinsics.promise_ctor) {
                    return Ok(oid);
                }
            }
        }
        let p = self.create_promise();
        self.resolve_promise(p, value);
        Ok(p)
    }

    fn create_async_from_sync_iterator(&mut self, sync_iter: ObjectId, next_fn: JsValue) -> ObjectId {
        let obj = self.create_object_with_proto(Some(self.intrinsics.async_iterator_proto));
        let next_method = next_fn;
        let next_wrapper = self.create_native_function(
            "next",
            1,
            Rc::new(move |rt, _, args| {
                let out_p = rt.create_promise();
                let call_args: &[JsValue] = if args.is_empty() { &[] } else { &args[..1] };
                let res = rt.call_function(
                    next_method.clone(),
                    JsValue::Object(sync_iter),
                    call_args,
                );
                match res {
                    Ok(v) => rt.async_from_sync_continuation(sync_iter, v, out_p),
                    Err(e) => {
                        let ev = rt.error_to_js_value(e);
                        rt.reject_promise(out_p, ev);
                    }
                }
                Ok(JsValue::Object(out_p))
            }),
        );
        let return_wrapper = self.create_native_function(
            "return",
            1,
            Rc::new(move |rt, _, args| {
                let out_p = rt.create_promise();
                let ret_fn = match rt.get_property(sync_iter, "return") {
                    Ok(f) => f,
                    Err(e) => {
                        let ev = rt.error_to_js_value(e);
                        rt.reject_promise(out_p, ev);
                        return Ok(JsValue::Object(out_p));
                    }
                };
                if ret_fn.is_nullish() {
                    let v = args.first().cloned().unwrap_or(JsValue::Undefined);
                    let ir = rt.create_iter_result(v, true);
                    rt.resolve_promise(out_p, ir);
                    return Ok(JsValue::Object(out_p));
                }
                let call_args: &[JsValue] = if args.is_empty() { &[] } else { &args[..1] };
                let res = rt.call_function(ret_fn, JsValue::Object(sync_iter), call_args);
                match res {
                    Ok(v) => rt.async_from_sync_continuation(sync_iter, v, out_p),
                    Err(e) => {
                        let ev = rt.error_to_js_value(e);
                        rt.reject_promise(out_p, ev);
                    }
                }
                Ok(JsValue::Object(out_p))
            }),
        );
        let throw_wrapper = self.create_native_function(
            "throw",
            1,
            Rc::new(move |rt, _, args| {
                let out_p = rt.create_promise();
                let throw_fn = match rt.get_property(sync_iter, "throw") {
                    Ok(f) => f,
                    Err(e) => {
                        let ev = rt.error_to_js_value(e);
                        rt.reject_promise(out_p, ev);
                        return Ok(JsValue::Object(out_p));
                    }
                };
                if throw_fn.is_nullish() {
                    if let Err(e) = rt.iterator_close_checked(&JsValue::Object(sync_iter), false) {
                        let ev = rt.error_to_js_value(e);
                        rt.reject_promise(out_p, ev);
                        return Ok(JsValue::Object(out_p));
                    }
                    let ev = rt.error_to_js_value(JsError::type_err(
                        "The iterator does not provide a 'throw' method",
                    ));
                    rt.reject_promise(out_p, ev);
                    return Ok(JsValue::Object(out_p));
                }
                let call_args: &[JsValue] = if args.is_empty() { &[] } else { &args[..1] };
                let res = rt.call_function(throw_fn, JsValue::Object(sync_iter), call_args);
                match res {
                    Ok(v) => rt.async_from_sync_continuation(sync_iter, v, out_p),
                    Err(e) => {
                        let ev = rt.error_to_js_value(e);
                        rt.reject_promise(out_p, ev);
                    }
                }
                Ok(JsValue::Object(out_p))
            }),
        );
        let _ = self.set_property(obj, "next", JsValue::Object(next_wrapper));
        let _ = self.set_property(obj, "return", JsValue::Object(return_wrapper));
        let _ = self.set_property(obj, "throw", JsValue::Object(throw_wrapper));
        obj
    }

    pub fn iterator_step(&mut self, iter: &JsValue) -> Result<Option<JsValue>, JsError> {
        let JsValue::Object(oid) = iter else {
            return Err(JsError::type_err("Iterator must be an object"));
        };
        let cached = match &self.heap.get_obj(*oid).kind {
            ObjectKind::Iterator(it) => match &it.kind {
                BuiltinIteratorKind::CachedIter(rec) => {
                    Some((rec.iterator.clone(), rec.next_method.clone()))
                }
                _ => None,
            },
            _ => None,
        };
        let (target_iter, next_fn) = if let Some((ti, nf)) = cached {
            (ti, nf)
        } else {
            (iter.clone(), self.get_property(*oid, "next")?)
        };
        let res = self.call_function(next_fn, target_iter, &[])?;
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
        let _ = self.iterator_close_checked(iter, true);
    }

    pub fn iterator_close_checked(
        &mut self,
        iter: &JsValue,
        suppress_err: bool,
    ) -> Result<(), JsError> {
        let JsValue::Object(raw_oid) = iter else {
            return Ok(());
        };
        let real_iter = match &self.heap.get_obj(*raw_oid).kind {
            ObjectKind::Iterator(it) => match &it.kind {
                BuiltinIteratorKind::CachedIter(rec) => rec.iterator.clone(),
                _ => iter.clone(),
            },
            _ => iter.clone(),
        };
        let JsValue::Object(oid) = real_iter else {
            return Ok(());
        };
        let ret_fn = match self.get_property(oid, "return") {
            Ok(v) => v,
            Err(e) => {
                if suppress_err {
                    return Ok(());
                }
                return Err(e);
            }
        };
        if ret_fn.is_nullish() {
            return Ok(());
        }
        if !self.is_callable(&ret_fn) {
            if suppress_err {
                return Ok(());
            }
            return Err(JsError::type_err("Iterator return is not callable"));
        }
        let res = match self.call_function(ret_fn, real_iter, &[]) {
            Ok(v) => v,
            Err(e) => {
                if suppress_err {
                    return Ok(());
                }
                return Err(e);
            }
        };
        if !suppress_err && !res.is_object() {
            return Err(JsError::type_err("Iterator result is not an object"));
        }
        Ok(())
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
                            let _ = rt.call_function(
                                JsValue::Object(rej_cb),
                                JsValue::Undefined,
                                &[ev],
                            );
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
            let resolve_target = |rt: &mut JsRuntime, v: JsValue| {
                if let Some(ref res_fn) = reaction.resolve {
                    let _ = rt.call_function(res_fn.clone(), JsValue::Undefined, &[v]);
                } else if let Some(target) = reaction.target {
                    rt.resolve_promise(target, v);
                }
            };
            let reject_target = |rt: &mut JsRuntime, r: JsValue| {
                if let Some(ref rej_fn) = reaction.reject {
                    let _ = rt.call_function(rej_fn.clone(), JsValue::Undefined, &[r]);
                } else if let Some(target) = reaction.target {
                    rt.reject_promise(target, r);
                }
            };
            match handler {
                Some(h) if rt.is_callable(&h) => match rt.call_function(h, JsValue::Undefined, &[arg]) {
                    Ok(v) => resolve_target(rt, v),
                    Err(e) => {
                        let ev = rt.error_to_js_value(e);
                        reject_target(rt, ev);
                    }
                },
                _ => {
                    if fulfilled {
                        resolve_target(rt, arg);
                    } else {
                        reject_target(rt, arg);
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
            resolve: None,
            reject: None,
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

    pub fn promise_then_with_capability(
        &mut self,
        promise_id: ObjectId,
        on_fulfilled: Option<JsValue>,
        on_rejected: Option<JsValue>,
        resolve: JsValue,
        reject: JsValue,
    ) {
        let reaction = PromiseReaction {
            on_fulfilled: on_fulfilled.filter(|v| self.is_callable(v)),
            on_rejected: on_rejected.filter(|v| self.is_callable(v)),
            target: None,
            resolve: Some(resolve),
            reject: Some(reject),
        };
        let state = match &mut self.heap.get_obj_mut(promise_id).kind {
            ObjectKind::Promise(p) => {
                p.handled = true;
                match &p.state {
                    PromiseState::Pending => {
                        p.reactions.push(reaction);
                        return;
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
    }

    pub fn create_iter_result(&mut self, value: JsValue, done: bool) -> JsValue {
        let obj = self.create_object();
        let _ = self.set_property(obj, "value", value);
        let _ = self.set_property(obj, "done", JsValue::Boolean(done));
        JsValue::Object(obj)
    }

    pub fn create_builtin_iterator(&mut self, kind: BuiltinIteratorKind) -> ObjectId {
        let proto = match &kind {
            BuiltinIteratorKind::Array { .. } => self.intrinsics.array_iterator_proto,
            BuiltinIteratorKind::String { .. } => self.intrinsics.string_iterator_proto,
            BuiltinIteratorKind::Map { .. } => self.intrinsics.map_iterator_proto,
            BuiltinIteratorKind::Set { .. } => self.intrinsics.set_iterator_proto,
            BuiltinIteratorKind::RegExpMatchAll { .. } => {
                self.intrinsics.regexp_string_iterator_proto
            }
            BuiltinIteratorKind::Wrapped(_) => self.intrinsics.wrap_for_valid_iterator_proto,
            BuiltinIteratorKind::Helper(_) => self.intrinsics.iterator_helper_proto,
            BuiltinIteratorKind::Keys { .. } | BuiltinIteratorKind::CachedIter(_) => {
                self.intrinsics.iterator_proto
            }
        };
        self.heap.alloc_object(JsObject::new(
            Some(proto),
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

enum ExportResolution {
    Found(ExportTarget),
    NotFound,
    Ambiguous,
}

fn normalize_module_path(path: &str) -> String {
    let is_abs = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if let Some(&last) = parts.last() {
                if last != ".." {
                    parts.pop();
                    continue;
                }
            }
            if !is_abs {
                parts.push("..");
            }
        } else {
            parts.push(seg);
        }
    }
    if is_abs {
        format!("/{}", parts.join("/"))
    } else if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
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

pub fn canonical_numeric_index_string(s: &str) -> Option<f64> {
    if s == "-0" {
        return Some(-0.0);
    }
    if s.is_empty() {
        return None;
    }
    let n = string_to_number(s);
    if number_to_string(n) == s {
        Some(n)
    } else {
        None
    }
}

pub fn valid_integer_index(n: f64, len: usize) -> Option<usize> {
    if n.is_nan() || n.is_infinite() || (n == 0.0 && n.is_sign_negative()) {
        return None;
    }
    if n.trunc() != n || n < 0.0 || n >= len as f64 {
        return None;
    }
    Some(n as usize)
}

fn complete_property_descriptor(desc: &mut PropertyDescriptor) {
    if desc.is_accessor() {
        if desc.get.is_none() {
            desc.get = Some(JsValue::Undefined);
        }
        if desc.set.is_none() {
            desc.set = Some(JsValue::Undefined);
        }
    } else {
        if desc.value.is_none() {
            desc.value = Some(JsValue::Undefined);
        }
        if desc.writable.is_none() {
            desc.writable = Some(false);
        }
    }
    if desc.enumerable.is_none() {
        desc.enumerable = Some(false);
    }
    if desc.configurable.is_none() {
        desc.configurable = Some(false);
    }
}

fn is_compatible_property_descriptor(
    extensible: bool,
    desc: &PropertyDescriptor,
    current: Option<&PropertyDescriptor>,
) -> bool {
    let Some(cur) = current else {
        return extensible;
    };
    if desc.value.is_none()
        && desc.writable.is_none()
        && desc.get.is_none()
        && desc.set.is_none()
        && desc.enumerable.is_none()
        && desc.configurable.is_none()
    {
        return true;
    }
    if cur.configurable == Some(false) {
        if desc.configurable == Some(true) {
            return false;
        }
        if let Some(e) = desc.enumerable {
            if Some(e) != cur.enumerable {
                return false;
            }
        }
        if !desc.is_generic() {
            let cur_is_acc = cur.is_accessor();
            if cur_is_acc != desc.is_accessor() {
                return false;
            }
            if cur_is_acc {
                if let Some(ref ns) = desc.set {
                    let cs = cur.set.as_ref().unwrap_or(&JsValue::Undefined);
                    if !ns.same_value(cs) {
                        return false;
                    }
                }
                if let Some(ref ng) = desc.get {
                    let cg = cur.get.as_ref().unwrap_or(&JsValue::Undefined);
                    if !ng.same_value(cg) {
                        return false;
                    }
                }
            } else if cur.writable == Some(false) {
                if desc.writable == Some(true) {
                    return false;
                }
                if let Some(ref nv) = desc.value {
                    let cv = cur.value.as_ref().unwrap_or(&JsValue::Undefined);
                    if !nv.same_value(cv) {
                        return false;
                    }
                }
            }
        }
    }
    true
}
