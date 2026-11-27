use crate::ast::{BinaryOp, Param, UnaryOp};
use crate::error::JsError;
use crate::value::{EnvId, JsValue, ObjectId};
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct FuncTemplate {
    pub name: Option<String>,
    pub chunk: Rc<Chunk>,
    pub params: Rc<Vec<Param>>,
    pub is_async: bool,
    pub is_generator: bool,
    pub is_arrow: bool,
    pub is_method: bool,
    pub strict: bool,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct ClassFieldTemplate {
    pub name: Option<String>,
    pub is_computed: bool,
    pub is_static: bool,
    pub is_private: bool,
    pub init: Option<Rc<Chunk>>,
}

#[derive(Debug, Clone)]
pub struct ClassMethodTemplate {
    pub name: Option<String>,
    pub is_computed: bool,
    pub is_static: bool,
    pub is_private: bool,
    pub kind: crate::ast::MethodKind,
    pub func: Rc<FuncTemplate>,
}

#[derive(Debug, Clone)]
pub struct ClassTemplate {
    pub name: Option<String>,
    pub has_super: bool,
    pub constructor: Option<Rc<FuncTemplate>>,
    pub methods: Vec<ClassMethodTemplate>,
    pub fields: Vec<ClassFieldTemplate>,
    pub static_blocks: Vec<Rc<Chunk>>,
}

#[derive(Debug, Clone)]
pub enum ConstValue {
    Number(f64),
    BigInt(i128),
    String(String),
    RegExp(String, String),
    Func(Rc<FuncTemplate>),
    Class(Rc<ClassTemplate>),
    Names(Vec<String>),
    TemplateStrings {
        site_id: u64,
        raw: Vec<String>,
        cooked: Vec<Option<String>>,
    },
}

#[derive(Debug, Clone)]
pub enum Op {
    PushUndefined,
    PushNull,
    PushTrue,
    PushFalse,
    PushConst(u32),
    Pop,
    Dup,
    Dup2,
    Swap,
    Rot3,

    PushThis,
    PushNewTarget,
    PushImportMeta,

    EnterScope,
    ExitScope,
    EnterWith,
    CloneLoopEnv(u32),

    DeclareVar(u32),
    DeclareLet(u32),
    DeclareConst(u32),
    InitBinding(u32),
    GetVar(u32),
    GetVarTypeof(u32),
    SetVar(u32),
    DeleteVar(u32),

    GetProp(u32),
    SetProp(u32),
    DeleteProp(u32),
    GetElem,
    SetElem,
    DeleteElem,
    GetPrivate(u32),
    SetPrivate(u32),
    HasPrivate(u32),

    GetSuperProp(u32),
    SetSuperProp(u32),
    GetSuperElem,
    SetSuperElem,
    CallSuper(u32),
    CallSuperSpread,

    CreateObject,
    CreateArray(u32),
    ArrayPush,
    ArrayPushHole,
    ArraySpread,
    ObjectSetProp(u32),
    ObjectSetOwnProp(u32),
    ObjectSetComputed,
    ObjectSetGetter(u32),
    ObjectSetSetter(u32),
    ObjectSetGetterComputed,
    ObjectSetSetterComputed,
    ObjectSpread,
    ObjectRest(u32),

    CreateFunction(u32),
    CreateClass(u32),

    Unary(UnaryOp),
    Binary(BinaryOp),
    ToNumber,
    ToString,
    ToPropertyKey,
    ToPropertyKeyOnly,
    RequireObjectCoercible,
    Inc,
    Dec,

    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    JumpIfNullish(u32),
    JumpIfNotNullish(u32),
    JumpIfNotUndefined(u32),

    Call(u32),
    CallMethod(u32),
    CallSpread,
    CallMethodSpread,
    CallDirectEval(u32),
    CallDirectEvalSpread,
    New(u32),
    NewSpread,

    Return,
    Throw,
    TryPush {
        catch_ip: Option<u32>,
        finally_ip: Option<u32>,
    },
    TryPop,
    FinallyStart,
    FinallyEnd,
    Break {
        target: u32,
        env_depth: u32,
        stack_depth: u32,
        handler_depth: u32,
    },

    ForInInit,
    ForInNext(u32),
    GetIterator(bool),
    IteratorNext(u32),
    IteratorNextValue,
    IteratorNextValueAt(u32),
    IteratorRest,
    IteratorRestAt(u32),
    IteratorClose,

    Yield,
    YieldDelegate,
    Await,

    CreateTemplateObject(u32),
    DynamicImport,
    ExportBinding(u32),
    ImportModule(u32),
    ImportBinding(u32, u32),
    ImportNamespace(u32),

    SetLastValue,
    PushLastValue,
    Debugger,
    AddDisposable {
        stack_name: u32,
        is_async: bool,
    },
    DisposeNext {
        stack_name: u32,
        sync_ip: u32,
        end_ip: u32,
    },
    RecordExcludedKey,
    ObjectRestDyn(u32),
    DeleteSuper,
    ResolveBinding(u32),
    GetResolvedBinding(u32),
    SetResolvedBinding(u32),
    PopBindingRef,
    AsyncIteratorNextCall,
    IteratorResultUnpack(u32),
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub ops: Vec<Op>,
    pub consts: Vec<ConstValue>,
    pub hoisted_vars: Vec<String>,
    pub hoisted_funcs: Vec<(String, Rc<FuncTemplate>)>,
    pub lex_names: Vec<(String, bool)>,
    pub strict: bool,
    pub is_eval_or_script: bool,
}

impl Chunk {
    pub fn new(strict: bool, is_eval_or_script: bool) -> Self {
        Chunk {
            ops: Vec::new(),
            consts: Vec::new(),
            hoisted_vars: Vec::new(),
            hoisted_funcs: Vec::new(),
            lex_names: Vec::new(),
            strict,
            is_eval_or_script,
        }
    }

    pub fn add_const(&mut self, c: ConstValue) -> u32 {
        let idx = self.consts.len() as u32;
        self.consts.push(c);
        idx
    }

    pub fn add_string(&mut self, s: String) -> u32 {
        for (i, c) in self.consts.iter().enumerate() {
            if let ConstValue::String(ref existing) = c {
                if existing == &s {
                    return i as u32;
                }
            }
        }
        self.add_const(ConstValue::String(s))
    }

    pub fn emit(&mut self, op: Op) -> usize {
        let idx = self.ops.len();
        self.ops.push(op);
        idx
    }

    pub fn patch_jump(&mut self, op_idx: usize, target: u32) {
        match &mut self.ops[op_idx] {
            Op::Jump(t)
            | Op::JumpIfFalse(t)
            | Op::JumpIfTrue(t)
            | Op::JumpIfNullish(t)
            | Op::JumpIfNotNullish(t)
            | Op::JumpIfNotUndefined(t)
            | Op::ForInNext(t)
            | Op::IteratorNext(t)
            | Op::IteratorResultUnpack(t) => *t = target,
            Op::Break { target: t, .. } => *t = target,
            _ => panic!("patch_jump on non-jump op"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum PendingCompletion {
    Normal,
    Return(JsValue),
    Throw(JsError),
    Jump {
        target: u32,
        env_depth: u32,
        stack_depth: u32,
        handler_depth: u32,
    },
}

#[derive(Debug, Clone)]
pub struct TryHandler {
    pub catch_ip: Option<u32>,
    pub finally_ip: Option<u32>,
    pub stack_len: usize,
    pub env_depth: usize,
    pub ref_depth: usize,
    pub in_catch: bool,
    pub in_finally: bool,
    pub pending: PendingCompletion,
    pub saved_last_value: Option<JsValue>,
}

#[derive(Clone)]
pub struct CallFrame {
    pub chunk: Rc<Chunk>,
    pub ip: usize,
    pub stack: Vec<JsValue>,
    pub env: EnvId,
    pub env_stack: Vec<EnvId>,
    pub ref_stack: Vec<Option<EnvId>>,
    pub handlers: Vec<TryHandler>,
    pub callee: Option<ObjectId>,
    pub last_value: JsValue,
    pub is_constructor: bool,
    pub this_for_ctor: Option<ObjectId>,
    pub super_called: bool,
    pub is_derived_ctor: bool,
    pub is_generator: bool,
    pub is_async: bool,
    pub async_promise: Option<ObjectId>,
}

impl CallFrame {
    pub fn new(chunk: Rc<Chunk>, env: EnvId, callee: Option<ObjectId>) -> Self {
        CallFrame {
            chunk,
            ip: 0,
            stack: Vec::with_capacity(32),
            env,
            env_stack: Vec::new(),
            ref_stack: Vec::new(),
            handlers: Vec::new(),
            callee,
            last_value: JsValue::Undefined,
            is_constructor: false,
            this_for_ctor: None,
            super_called: false,
            is_derived_ctor: false,
            is_generator: false,
            is_async: false,
            async_promise: None,
        }
    }
}
