use crate::ast::Param;
use crate::bytecode::{CallFrame, Chunk};
use crate::error::{ErrorKind, JsError};
use crate::regexp::CompiledRegExp;
use crate::value::{
    parse_array_index, EnvId, JsValue, ObjectId, PropertyDescriptor, PropertyKey, TypedArrayKind,
};
use crate::JsRuntime;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub type NativeCallback =
    Rc<dyn Fn(&mut JsRuntime, JsValue, &[JsValue]) -> Result<JsValue, JsError>>;

#[derive(Clone)]
pub struct NativeFn {
    pub name: String,
    pub length: usize,
    pub callback: NativeCallback,
    pub is_constructor: bool,
    pub handles_new_target: bool,
}

#[derive(Clone, Default)]
pub struct BufferMeta {
    pub max_byte_length: Option<usize>,
    pub is_shared: bool,
    pub immutable: bool,
}

#[derive(Clone)]
pub struct JsFunction {
    pub name: String,
    pub chunk: Rc<Chunk>,
    pub params: Rc<Vec<Param>>,
    pub env: EnvId,
    pub is_async: bool,
    pub is_generator: bool,
    pub is_arrow: bool,
    pub is_class_ctor: bool,
    pub is_derived_ctor: bool,
    pub strict: bool,
    pub home_object: Option<ObjectId>,
    pub super_ctor: Option<ObjectId>,
    pub bound_this: Option<JsValue>,
    pub bound_args: Vec<JsValue>,
    pub bound_target: Option<ObjectId>,
    pub source: String,
    pub class_brand: Option<usize>,
    pub field_inits: Rc<Vec<ClassFieldInit>>,
}

#[derive(Clone)]
pub struct ClassFieldInit {
    pub key: FieldKeySpec,
    pub init_chunk: Option<Rc<Chunk>>,
    pub is_private: bool,
}

#[derive(Clone)]
pub enum FieldKeySpec {
    Name(String),
    Computed(ObjectId),
    Private(String),
}

#[derive(Clone)]
pub struct ArrayData {
    pub elements: Vec<Option<JsValue>>,
    pub length: usize,
    pub has_custom_index_props: bool,
}

impl ArrayData {
    pub fn from_vec(vals: Vec<JsValue>) -> Self {
        let len = vals.len();
        ArrayData {
            elements: vals.into_iter().map(Some).collect(),
            length: len,
            has_custom_index_props: false,
        }
    }

    pub fn with_len(length: usize) -> Self {
        let alloc = length.min(1024);
        ArrayData {
            elements: vec![None; alloc],
            length,
            has_custom_index_props: false,
        }
    }

    pub fn get(&self, idx: usize) -> Option<&JsValue> {
        self.elements.get(idx).and_then(|o| o.as_ref())
    }

    pub fn set(&mut self, idx: usize, val: JsValue) {
        if idx >= self.elements.len() {
            if idx < 1_000_000 {
                self.elements.resize(idx + 1, None);
            } else {
                self.length = self.length.max(idx + 1);
                return;
            }
        }
        self.elements[idx] = Some(val);
        if idx >= self.length {
            self.length = idx + 1;
        }
    }

    pub fn delete(&mut self, idx: usize) {
        if idx < self.elements.len() {
            self.elements[idx] = None;
        }
    }

    pub fn truncate(&mut self, new_len: usize) {
        self.length = new_len;
        if self.elements.len() > new_len {
            self.elements.truncate(new_len);
        }
    }
}

#[derive(Clone)]
pub struct RegExpData {
    pub compiled: Rc<CompiledRegExp>,
    pub original_source: String,
    pub last_index: usize,
}

#[derive(Clone)]
pub struct ErrorData {
    pub kind: ErrorKind,
    pub message: String,
    pub stack: String,
}

#[derive(Clone, Default)]
pub struct MapData {
    pub entries: Vec<Option<(JsValue, JsValue)>>,
    pub count: usize,
}

impl MapData {
    pub fn get(&self, key: &JsValue) -> Option<&JsValue> {
        for entry in self.entries.iter().flatten() {
            if entry.0.same_value_zero(key) {
                return Some(&entry.1);
            }
        }
        None
    }

    pub fn has(&self, key: &JsValue) -> bool {
        self.get(key).is_some()
    }

    pub fn set(&mut self, key: JsValue, val: JsValue) {
        let norm_key = match key {
            JsValue::Number(n) if n == 0.0 => JsValue::Number(0.0),
            other => other,
        };
        for entry in self.entries.iter_mut().flatten() {
            if entry.0.same_value_zero(&norm_key) {
                entry.1 = val;
                return;
            }
        }
        self.entries.push(Some((norm_key, val)));
        self.count += 1;
    }

    pub fn delete(&mut self, key: &JsValue) -> bool {
        for slot in &mut self.entries {
            if let Some((k, _)) = slot {
                if k.same_value_zero(key) {
                    *slot = None;
                    self.count -= 1;
                    return true;
                }
            }
        }
        false
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.count = 0;
    }
}

#[derive(Clone, Default)]
pub struct SetData {
    pub entries: Vec<Option<JsValue>>,
    pub count: usize,
}

impl SetData {
    pub fn has(&self, val: &JsValue) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|v| v.same_value_zero(val))
    }

    pub fn add(&mut self, val: JsValue) {
        let norm = match val {
            JsValue::Number(n) if n == 0.0 => JsValue::Number(0.0),
            other => other,
        };
        if !self.has(&norm) {
            self.entries.push(Some(norm));
            self.count += 1;
        }
    }

    pub fn delete(&mut self, val: &JsValue) -> bool {
        for slot in &mut self.entries {
            if let Some(v) = slot {
                if v.same_value_zero(val) {
                    *slot = None;
                    self.count -= 1;
                    return true;
                }
            }
        }
        false
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.count = 0;
    }
}

#[derive(Clone)]
pub enum PromiseState {
    Pending,
    Fulfilled(JsValue),
    Rejected(JsValue),
}

#[derive(Clone)]
pub struct PromiseReaction {
    pub on_fulfilled: Option<JsValue>,
    pub on_rejected: Option<JsValue>,
    pub target: Option<ObjectId>,
}

#[derive(Clone)]
pub struct PromiseData {
    pub state: PromiseState,
    pub reactions: Vec<PromiseReaction>,
    pub handled: bool,
}

#[derive(Clone)]
pub struct ProxyData {
    pub target: ObjectId,
    pub handler: ObjectId,
    pub revoked: bool,
}

#[derive(Clone)]
pub struct ArrayBufferData {
    pub bytes: Vec<u8>,
    pub detached: bool,
}

#[derive(Clone)]
pub struct DataViewData {
    pub buffer: ObjectId,
    pub byte_offset: usize,
    pub byte_length: usize,
}

#[derive(Clone)]
pub struct TypedArrayData {
    pub kind: TypedArrayKind,
    pub buffer: ObjectId,
    pub byte_offset: usize,
    pub length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorState {
    SuspendedStart,
    SuspendedYield,
    Executing,
    Completed,
}

#[derive(Clone)]
pub struct GeneratorData {
    pub state: GeneratorState,
    pub frame: Option<Box<CallFrame>>,
    pub is_async: bool,
    pub delegating: Option<JsValue>,
    pub result_promise: Option<ObjectId>,
}

#[derive(Clone)]
pub struct ArgumentsData {
    pub values: Vec<JsValue>,
    pub strict: bool,
    pub mapped_env: Option<EnvId>,
    pub mapped_params: Vec<Option<String>>,
}

#[derive(Clone)]
pub struct ModuleNamespaceData {
    pub exports: Vec<(String, JsValue)>,
}

#[derive(Clone)]
pub struct IteratorData {
    pub kind: BuiltinIteratorKind,
    pub index: usize,
}

#[derive(Clone)]
pub enum BuiltinIteratorKind {
    Array { target: ObjectId, mode: IterMode },
    String { units: Vec<u16> },
    Map { target: ObjectId, mode: IterMode },
    Set { target: ObjectId, mode: IterMode },
    Keys(Vec<JsValue>),
    RegExpMatchAll {
        regexp: ObjectId,
        input: String,
        global: bool,
        done: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterMode {
    Values,
    Keys,
    Entries,
}

pub enum ObjectKind {
    Ordinary,
    Function(JsFunction),
    NativeFunction(NativeFn),
    Array(ArrayData),
    StringObj(String),
    NumberObj(f64),
    BooleanObj(bool),
    SymbolObj(u64),
    BigIntObj(i128),
    RegExp(RegExpData),
    Date(f64),
    Error(ErrorData),
    Map(MapData),
    Set(SetData),
    WeakMap(Vec<(ObjectId, JsValue)>),
    WeakSet(Vec<ObjectId>),
    Promise(PromiseData),
    Proxy(ProxyData),
    ArrayBuffer(ArrayBufferData),
    DataView(DataViewData),
    TypedArray(TypedArrayData),
    Generator(GeneratorData),
    Arguments(ArgumentsData),
    ModuleNamespace(ModuleNamespaceData),
    Iterator(IteratorData),
    HostObject(Box<dyn Any>),
}

#[derive(Clone)]
pub enum PropValue {
    Data(JsValue),
    Accessor {
        get: Option<JsValue>,
        set: Option<JsValue>,
    },
}

#[derive(Clone)]
pub struct PropEntry {
    pub value: PropValue,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}

impl PropEntry {
    pub fn data(val: JsValue, writable: bool, enumerable: bool, configurable: bool) -> Self {
        PropEntry {
            value: PropValue::Data(val),
            writable,
            enumerable,
            configurable,
        }
    }

    pub fn accessor(
        get: Option<JsValue>,
        set: Option<JsValue>,
        enumerable: bool,
        configurable: bool,
    ) -> Self {
        PropEntry {
            value: PropValue::Accessor { get, set },
            writable: false,
            enumerable,
            configurable,
        }
    }

    pub fn to_descriptor(&self) -> PropertyDescriptor {
        match &self.value {
            PropValue::Data(v) => PropertyDescriptor {
                value: Some(v.clone()),
                writable: Some(self.writable),
                get: None,
                set: None,
                enumerable: Some(self.enumerable),
                configurable: Some(self.configurable),
            },
            PropValue::Accessor { get, set } => PropertyDescriptor {
                value: None,
                writable: None,
                get: get.clone(),
                set: set.clone(),
                enumerable: Some(self.enumerable),
                configurable: Some(self.configurable),
            },
        }
    }
}

pub struct JsObject {
    pub prototype: Option<ObjectId>,
    pub extensible: bool,
    pub kind: ObjectKind,
    pub prop_list: Vec<Option<(String, PropEntry)>>,
    pub prop_index: HashMap<String, usize>,
    pub symbol_props: Vec<(u64, PropEntry)>,
    pub private_fields: HashMap<(usize, String), PropEntry>,
    pub class_brands: Vec<usize>,
    pub host_data: Option<Box<dyn Any>>,
}

impl JsObject {
    pub fn new(prototype: Option<ObjectId>, kind: ObjectKind) -> Self {
        JsObject {
            prototype,
            extensible: true,
            kind,
            prop_list: Vec::new(),
            prop_index: HashMap::new(),
            symbol_props: Vec::new(),
            private_fields: HashMap::new(),
            class_brands: Vec::new(),
            host_data: None,
        }
    }

    pub fn get_own_prop(&self, key: &PropertyKey) -> Option<&PropEntry> {
        match key {
            PropertyKey::String(s) => {
                let idx = *self.prop_index.get(s)?;
                self.prop_list.get(idx)?.as_ref().map(|(_, e)| e)
            }
            PropertyKey::Symbol(id) => self
                .symbol_props
                .iter()
                .find(|(k, _)| k == id)
                .map(|(_, e)| e),
        }
    }

    pub fn get_own_prop_mut(&mut self, key: &PropertyKey) -> Option<&mut PropEntry> {
        match key {
            PropertyKey::String(s) => {
                let idx = *self.prop_index.get(s)?;
                self.prop_list.get_mut(idx)?.as_mut().map(|(_, e)| e)
            }
            PropertyKey::Symbol(id) => self
                .symbol_props
                .iter_mut()
                .find(|(k, _)| k == id)
                .map(|(_, e)| e),
        }
    }

    pub fn set_own_prop(&mut self, key: PropertyKey, entry: PropEntry) {
        match key {
            PropertyKey::String(s) => {
                if let Some(&idx) = self.prop_index.get(&s) {
                    self.prop_list[idx] = Some((s, entry));
                } else {
                    let idx = self.prop_list.len();
                    self.prop_index.insert(s.clone(), idx);
                    self.prop_list.push(Some((s, entry)));
                }
            }
            PropertyKey::Symbol(id) => {
                if let Some(pair) = self.symbol_props.iter_mut().find(|(k, _)| *k == id) {
                    pair.1 = entry;
                } else {
                    self.symbol_props.push((id, entry));
                }
            }
        }
    }

    pub fn remove_own_prop(&mut self, key: &PropertyKey) -> bool {
        match key {
            PropertyKey::String(s) => {
                if let Some(idx) = self.prop_index.remove(s) {
                    self.prop_list[idx] = None;
                    true
                } else {
                    false
                }
            }
            PropertyKey::Symbol(id) => {
                if let Some(pos) = self.symbol_props.iter().position(|(k, _)| k == id) {
                    self.symbol_props.remove(pos);
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn own_property_keys(&self) -> Vec<PropertyKey> {
        let mut indices: Vec<u32> = Vec::new();
        let mut strings: Vec<String> = Vec::new();
        let mut seen_idx: HashSet<u32> = HashSet::new();

        match &self.kind {
            ObjectKind::Array(arr) => {
                for (i, slot) in arr.elements.iter().enumerate() {
                    if slot.is_some() {
                        indices.push(i as u32);
                        seen_idx.insert(i as u32);
                    }
                }
            }
            ObjectKind::StringObj(s) => {
                let len = s.encode_utf16().count();
                for i in 0..len {
                    indices.push(i as u32);
                    seen_idx.insert(i as u32);
                }
            }
            ObjectKind::TypedArray(ta) => {
                for i in 0..ta.length {
                    indices.push(i as u32);
                    seen_idx.insert(i as u32);
                }
            }
            ObjectKind::Arguments(args) => {
                for i in 0..args.values.len() {
                    indices.push(i as u32);
                    seen_idx.insert(i as u32);
                }
            }
            _ => {}
        }

        if matches!(self.kind, ObjectKind::Array(_) | ObjectKind::StringObj(_)) {
            strings.push("length".to_string());
        }
        for item in self.prop_list.iter().flatten() {
            if let Some(idx) = parse_array_index(&item.0) {
                if seen_idx.insert(idx) {
                    indices.push(idx);
                }
            } else if !(matches!(self.kind, ObjectKind::Array(_) | ObjectKind::StringObj(_))
                && item.0 == "length")
            {
                strings.push(item.0.clone());
            }
        }

        indices.sort_unstable();
        let mut keys = Vec::with_capacity(indices.len() + strings.len() + self.symbol_props.len());
        for i in indices {
            keys.push(PropertyKey::String(i.to_string()));
        }
        for s in strings {
            keys.push(PropertyKey::String(s));
        }
        for (sym, _) in &self.symbol_props {
            keys.push(PropertyKey::Symbol(*sym));
        }
        keys
    }

    pub fn is_callable(&self) -> bool {
        match &self.kind {
            ObjectKind::Function(_) | ObjectKind::NativeFunction(_) => true,
            ObjectKind::Proxy(_) => true,
            _ => false,
        }
    }
}

#[derive(Clone)]
pub struct Binding {
    pub value: JsValue,
    pub mutable: bool,
    pub initialized: bool,
    pub deletable: bool,
    pub strict_immutable: bool,
}

#[derive(Clone)]
pub struct EnvRecord {
    pub parent: Option<EnvId>,
    pub bindings: HashMap<String, Binding>,
    pub lex_names: HashSet<String>,
    pub var_names: HashSet<String>,
    pub object_env: Option<ObjectId>,
    pub this_binding: Option<JsValue>,
    pub new_target: Option<JsValue>,
    pub super_proto: Option<ObjectId>,
    pub is_function_scope: bool,
    pub is_param_env: bool,
    pub strict: bool,
    pub allow_super_call: bool,
    pub allow_super_prop: bool,
    pub allow_new_target: bool,
    pub allow_private: bool,
}

impl EnvRecord {
    pub fn new(parent: Option<EnvId>, is_function_scope: bool, strict: bool) -> Self {
        EnvRecord {
            parent,
            bindings: HashMap::new(),
            lex_names: HashSet::new(),
            var_names: HashSet::new(),
            object_env: None,
            this_binding: None,
            new_target: None,
            super_proto: None,
            is_function_scope,
            is_param_env: false,
            strict,
            allow_super_call: false,
            allow_super_prop: false,
            allow_new_target: false,
            allow_private: false,
        }
    }
}

pub struct Heap {
    objects: Vec<Option<JsObject>>,
    free_objects: Vec<usize>,
    envs: Vec<Option<EnvRecord>>,
    free_envs: Vec<usize>,
    next_symbol_id: u64,
    symbol_descriptions: HashMap<u64, Option<String>>,
    global_symbol_registry: HashMap<String, u64>,
    next_class_brand: usize,
    alloc_since_gc: usize,
    pub buffer_meta: HashMap<ObjectId, BufferMeta>,
    pub auto_length_views: HashSet<ObjectId>,
    pub raw_json_objects: HashSet<ObjectId>,
    pub weak_map_symbols: HashMap<ObjectId, Vec<(u64, JsValue)>>,
    pub weak_set_symbols: HashMap<ObjectId, Vec<u64>>,
    pub object_realms: HashMap<ObjectId, usize>,
    pub non_writable_array_lengths: HashSet<ObjectId>,
    pub deleted_arg_indices: HashMap<ObjectId, HashSet<usize>>,
}

impl Heap {
    pub fn new() -> Self {
        Heap {
            objects: Vec::with_capacity(1024),
            free_objects: Vec::new(),
            envs: Vec::with_capacity(256),
            free_envs: Vec::new(),
            next_symbol_id: 1,
            symbol_descriptions: HashMap::new(),
            global_symbol_registry: HashMap::new(),
            next_class_brand: 1,
            alloc_since_gc: 0,
            buffer_meta: HashMap::new(),
            auto_length_views: HashSet::new(),
            raw_json_objects: HashSet::new(),
            weak_map_symbols: HashMap::new(),
            weak_set_symbols: HashMap::new(),
            object_realms: HashMap::new(),
            non_writable_array_lengths: HashSet::new(),
            deleted_arg_indices: HashMap::new(),
        }
    }

    pub fn alloc_class_brand(&mut self) -> usize {
        let id = self.next_class_brand;
        self.next_class_brand += 1;
        id
    }

    pub fn alloc_object(&mut self, obj: JsObject) -> ObjectId {
        self.alloc_since_gc += 1;
        if let Some(idx) = self.free_objects.pop() {
            self.objects[idx] = Some(obj);
            ObjectId(idx)
        } else {
            let idx = self.objects.len();
            self.objects.push(Some(obj));
            ObjectId(idx)
        }
    }

    pub fn get_obj(&self, id: ObjectId) -> &JsObject {
        self.objects[id.0].as_ref().expect("dangling ObjectId")
    }

    pub fn get_obj_mut(&mut self, id: ObjectId) -> &mut JsObject {
        self.objects[id.0].as_mut().expect("dangling ObjectId")
    }

    pub fn try_get_obj(&self, id: ObjectId) -> Option<&JsObject> {
        self.objects.get(id.0).and_then(|o| o.as_ref())
    }

    pub fn alloc_env(&mut self, env: EnvRecord) -> EnvId {
        if let Some(idx) = self.free_envs.pop() {
            self.envs[idx] = Some(env);
            EnvId(idx)
        } else {
            let idx = self.envs.len();
            self.envs.push(Some(env));
            EnvId(idx)
        }
    }

    pub fn get_env(&self, id: EnvId) -> &EnvRecord {
        self.envs[id.0].as_ref().expect("dangling EnvId")
    }

    pub fn get_env_mut(&mut self, id: EnvId) -> &mut EnvRecord {
        self.envs[id.0].as_mut().expect("dangling EnvId")
    }

    pub fn create_symbol(&mut self, desc: Option<String>) -> u64 {
        let id = self.next_symbol_id;
        self.next_symbol_id += 1;
        self.symbol_descriptions.insert(id, desc);
        id
    }

    pub fn symbol_description(&self, id: u64) -> Option<&str> {
        self.symbol_descriptions
            .get(&id)
            .and_then(|o| o.as_deref())
    }

    pub fn symbol_for(&mut self, key: &str) -> u64 {
        if let Some(&id) = self.global_symbol_registry.get(key) {
            return id;
        }
        let id = self.create_symbol(Some(key.to_string()));
        self.global_symbol_registry.insert(key.to_string(), id);
        id
    }

    pub fn symbol_key_for(&self, id: u64) -> Option<String> {
        for (k, &v) in &self.global_symbol_registry {
            if v == id {
                return Some(k.clone());
            }
        }
        None
    }

    pub fn live_object_count(&self) -> usize {
        self.objects.len() - self.free_objects.len()
    }

    pub fn alloc_since_gc(&self) -> usize {
        self.alloc_since_gc
    }

    pub fn collect_garbage(&mut self, root_objects: &[ObjectId], root_envs: &[EnvId]) {
        self.alloc_since_gc = 0;
        let mut marked_objs = vec![false; self.objects.len()];
        let mut marked_envs = vec![false; self.envs.len()];
        let mut obj_stack: Vec<ObjectId> = root_objects.to_vec();
        let mut env_stack: Vec<EnvId> = root_envs.to_vec();

        loop {
            let mut progressed = false;
            while let Some(oid) = obj_stack.pop() {
                if oid.0 >= marked_objs.len() || marked_objs[oid.0] {
                    continue;
                }
                let Some(ref obj) = self.objects[oid.0] else {
                    continue;
                };
                marked_objs[oid.0] = true;
                progressed = true;

                if let Some(proto) = obj.prototype {
                    obj_stack.push(proto);
                }
                for item in obj.prop_list.iter().flatten() {
                    trace_prop_entry(&item.1, &mut obj_stack);
                }
                for (_, entry) in &obj.symbol_props {
                    trace_prop_entry(entry, &mut obj_stack);
                }
                for entry in obj.private_fields.values() {
                    trace_prop_entry(entry, &mut obj_stack);
                }
                match &obj.kind {
                    ObjectKind::Function(f) => {
                        env_stack.push(f.env);
                        if let Some(h) = f.home_object {
                            obj_stack.push(h);
                        }
                        if let Some(s) = f.super_ctor {
                            obj_stack.push(s);
                        }
                        if let Some(ref bt) = f.bound_this {
                            trace_value(bt, &mut obj_stack);
                        }
                        for a in &f.bound_args {
                            trace_value(a, &mut obj_stack);
                        }
                        if let Some(bt) = f.bound_target {
                            obj_stack.push(bt);
                        }
                        for fi in f.field_inits.iter() {
                            if let FieldKeySpec::Computed(id) = fi.key {
                                obj_stack.push(id);
                            }
                        }
                    }
                    ObjectKind::Array(arr) => {
                        for el in arr.elements.iter().flatten() {
                            trace_value(el, &mut obj_stack);
                        }
                    }
                    ObjectKind::Map(m) => {
                        for (k, v) in m.entries.iter().flatten() {
                            trace_value(k, &mut obj_stack);
                            trace_value(v, &mut obj_stack);
                        }
                    }
                    ObjectKind::Set(s) => {
                        for v in s.entries.iter().flatten() {
                            trace_value(v, &mut obj_stack);
                        }
                    }
                    ObjectKind::Promise(p) => {
                        match &p.state {
                            PromiseState::Fulfilled(v) | PromiseState::Rejected(v) => {
                                trace_value(v, &mut obj_stack);
                            }
                            PromiseState::Pending => {}
                        }
                        for r in &p.reactions {
                            if let Some(ref f) = r.on_fulfilled {
                                trace_value(f, &mut obj_stack);
                            }
                            if let Some(ref f) = r.on_rejected {
                                trace_value(f, &mut obj_stack);
                            }
                            if let Some(t) = r.target {
                                obj_stack.push(t);
                            }
                        }
                    }
                    ObjectKind::Proxy(p) => {
                        obj_stack.push(p.target);
                        obj_stack.push(p.handler);
                    }
                    ObjectKind::DataView(dv) => {
                        obj_stack.push(dv.buffer);
                    }
                    ObjectKind::TypedArray(ta) => {
                        obj_stack.push(ta.buffer);
                    }
                    ObjectKind::Generator(g) => {
                        if let Some(ref fr) = g.frame {
                            trace_frame(fr, &mut obj_stack, &mut env_stack);
                        }
                        if let Some(ref d) = g.delegating {
                            trace_value(d, &mut obj_stack);
                        }
                        if let Some(rp) = g.result_promise {
                            obj_stack.push(rp);
                        }
                    }
                    ObjectKind::Arguments(a) => {
                        for v in &a.values {
                            trace_value(v, &mut obj_stack);
                        }
                        if let Some(me) = a.mapped_env {
                            env_stack.push(me);
                        }
                    }
                    ObjectKind::ModuleNamespace(m) => {
                        for (_, v) in &m.exports {
                            trace_value(v, &mut obj_stack);
                        }
                    }
                    ObjectKind::Iterator(it) => match &it.kind {
                        BuiltinIteratorKind::Array { target, .. }
                        | BuiltinIteratorKind::Map { target, .. }
                        | BuiltinIteratorKind::Set { target, .. } => obj_stack.push(*target),
                        BuiltinIteratorKind::Keys(vs) => {
                            for v in vs {
                                trace_value(v, &mut obj_stack);
                            }
                        }
                        BuiltinIteratorKind::RegExpMatchAll { regexp, .. } => {
                            obj_stack.push(*regexp);
                        }
                        BuiltinIteratorKind::String { .. } => {}
                    },
                    _ => {}
                }
            }

            while let Some(eid) = env_stack.pop() {
                if eid.0 >= marked_envs.len() || marked_envs[eid.0] {
                    continue;
                }
                let Some(ref env) = self.envs[eid.0] else {
                    continue;
                };
                marked_envs[eid.0] = true;
                progressed = true;

                if let Some(p) = env.parent {
                    env_stack.push(p);
                }
                if let Some(o) = env.object_env {
                    obj_stack.push(o);
                }
                if let Some(ref t) = env.this_binding {
                    trace_value(t, &mut obj_stack);
                }
                if let Some(ref nt) = env.new_target {
                    trace_value(nt, &mut obj_stack);
                }
                if let Some(sp) = env.super_proto {
                    obj_stack.push(sp);
                }
                for b in env.bindings.values() {
                    trace_value(&b.value, &mut obj_stack);
                }
            }

            for (i, slot) in self.objects.iter().enumerate() {
                if !marked_objs[i] {
                    continue;
                }
                if let Some(obj) = slot {
                    if let ObjectKind::WeakMap(entries) = &obj.kind {
                        for (k, v) in entries {
                            if k.0 < marked_objs.len() && marked_objs[k.0] {
                                if let JsValue::Object(vo) = v {
                                    if vo.0 < marked_objs.len() && !marked_objs[vo.0] {
                                        obj_stack.push(*vo);
                                        progressed = true;
                                    }
                                }
                            }
                        }
                        if let Some(sym_entries) = self.weak_map_symbols.get(&ObjectId(i)) {
                            for (_, v) in sym_entries {
                                if let JsValue::Object(vo) = v {
                                    if vo.0 < marked_objs.len() && !marked_objs[vo.0] {
                                        obj_stack.push(*vo);
                                        progressed = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !progressed {
                break;
            }
        }

        for (i, slot) in self.objects.iter_mut().enumerate() {
            if !marked_objs[i] {
                if slot.is_some() {
                    *slot = None;
                    self.free_objects.push(i);
                    let oid = ObjectId(i);
                    self.buffer_meta.remove(&oid);
                    self.auto_length_views.remove(&oid);
                    self.raw_json_objects.remove(&oid);
                    self.weak_map_symbols.remove(&oid);
                    self.weak_set_symbols.remove(&oid);
                    self.object_realms.remove(&oid);
                    self.non_writable_array_lengths.remove(&oid);
                    self.deleted_arg_indices.remove(&oid);
                }
            } else if let Some(obj) = slot {
                match &mut obj.kind {
                    ObjectKind::WeakMap(entries) => {
                        entries.retain(|(k, _)| k.0 < marked_objs.len() && marked_objs[k.0]);
                    }
                    ObjectKind::WeakSet(entries) => {
                        entries.retain(|k| k.0 < marked_objs.len() && marked_objs[k.0]);
                    }
                    _ => {}
                }
            }
        }

        for (i, slot) in self.envs.iter_mut().enumerate() {
            if !marked_envs[i] && slot.is_some() {
                *slot = None;
                self.free_envs.push(i);
            }
        }
    }
}

pub fn trace_value(val: &JsValue, objs: &mut Vec<ObjectId>) {
    if let JsValue::Object(id) = val {
        objs.push(*id);
    }
}

fn trace_prop_entry(entry: &PropEntry, objs: &mut Vec<ObjectId>) {
    match &entry.value {
        PropValue::Data(v) => trace_value(v, objs),
        PropValue::Accessor { get, set } => {
            if let Some(g) = get {
                trace_value(g, objs);
            }
            if let Some(s) = set {
                trace_value(s, objs);
            }
        }
    }
}

pub fn trace_frame(frame: &CallFrame, objs: &mut Vec<ObjectId>, envs: &mut Vec<EnvId>) {
    envs.push(frame.env);
    for e in &frame.env_stack {
        envs.push(*e);
    }
    for v in &frame.stack {
        trace_value(v, objs);
    }
    if let Some(c) = frame.callee {
        objs.push(c);
    }
}
