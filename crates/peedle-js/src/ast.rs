use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct Program {
    pub body: Vec<Stmt>,
    pub is_module: bool,
    pub strict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
    Using,
    AwaitUsing,
}

#[derive(Debug, Clone)]
pub struct VarDeclarator {
    pub pattern: Pattern,
    pub init: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Block(Vec<Stmt>),
    VarDecl {
        kind: VarKind,
        decls: Vec<VarDeclarator>,
    },
    Expr(Expr),
    If {
        test: Expr,
        consequent: Box<Stmt>,
        alternate: Option<Box<Stmt>>,
    },
    While {
        test: Expr,
        body: Box<Stmt>,
    },
    DoWhile {
        body: Box<Stmt>,
        test: Expr,
    },
    For {
        init: Option<ForInit>,
        test: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },
    ForIn {
        left: ForInOfLeft,
        right: Expr,
        body: Box<Stmt>,
    },
    ForOf {
        left: ForInOfLeft,
        right: Expr,
        body: Box<Stmt>,
        is_await: bool,
    },
    Switch {
        discriminant: Expr,
        cases: Vec<SwitchCase>,
    },
    Try {
        block: Vec<Stmt>,
        handler: Option<CatchClause>,
        finalizer: Option<Vec<Stmt>>,
    },
    Throw(Expr),
    Return(Option<Expr>),
    Break(Option<String>),
    Continue(Option<String>),
    Labeled {
        label: String,
        body: Box<Stmt>,
    },
    With {
        object: Expr,
        body: Box<Stmt>,
    },
    Debugger,
    Empty,
    FuncDecl(Rc<FuncNode>),
    ClassDecl(Rc<ClassNode>),
    Import(ImportDecl),
    Export(ExportDecl),
}

#[derive(Debug, Clone)]
pub enum ForInit {
    VarDecl {
        kind: VarKind,
        decls: Vec<VarDeclarator>,
    },
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum ForInOfLeft {
    VarDecl { kind: VarKind, pattern: Pattern },
    Pattern(Pattern),
}

#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub test: Option<Expr>,
    pub consequent: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct CatchClause {
    pub param: Option<Pattern>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Ident(String),
    Array {
        elements: Vec<Option<Pattern>>,
        rest: Option<Box<Pattern>>,
    },
    Object {
        props: Vec<ObjectPatProp>,
        rest: Option<Box<Pattern>>,
    },
    Assign {
        left: Box<Pattern>,
        right: Box<Expr>,
    },
    Member(Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct ObjectPatProp {
    pub key: PropKey,
    pub value: Pattern,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub pattern: Pattern,
    pub rest: bool,
}

#[derive(Debug, Clone)]
pub struct FuncNode {
    pub name: Option<String>,
    pub params: Vec<Param>,
    pub body: FuncBody,
    pub is_async: bool,
    pub is_generator: bool,
    pub is_arrow: bool,
    pub is_decl: bool,
    pub strict: bool,
    pub source: String,
}

#[derive(Debug, Clone)]
pub enum FuncBody {
    Block(Vec<Stmt>),
    Expr(Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct ClassNode {
    pub name: Option<String>,
    pub super_class: Option<Box<Expr>>,
    pub elements: Vec<ClassElement>,
}

#[derive(Debug, Clone)]
pub enum ClassElement {
    Method {
        key: PropKey,
        kind: MethodKind,
        func: Rc<FuncNode>,
        is_static: bool,
        is_private: bool,
    },
    Field {
        key: PropKey,
        init: Option<Expr>,
        is_static: bool,
        is_private: bool,
    },
    StaticBlock(Vec<Stmt>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodKind {
    Constructor,
    Method,
    Get,
    Set,
}

#[derive(Debug, Clone)]
pub enum PropKey {
    Ident(String),
    String(String),
    Number(f64),
    Computed(Box<Expr>),
    Private(String),
}

#[derive(Debug, Clone)]
pub enum ArrayElem {
    Hole,
    Expr(Expr),
    Spread(Expr, bool),
}

#[derive(Debug, Clone)]
pub enum ObjectProp {
    Property {
        key: PropKey,
        value: Expr,
        shorthand: bool,
        is_cover_init: bool,
    },
    Method {
        key: PropKey,
        kind: MethodKind,
        func: Rc<FuncNode>,
    },
    Spread(Expr, bool),
}

#[derive(Debug, Clone)]
pub enum CallArg {
    Expr(Expr),
    Spread(Expr),
}

#[derive(Debug, Clone)]
pub enum MemberProp {
    Ident(String),
    Computed(Box<Expr>),
    Private(String),
}

#[derive(Debug, Clone)]
pub struct TemplateQuasi {
    pub raw: String,
    pub cooked: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AssignTarget {
    Ident(String),
    Member {
        object: Box<Expr>,
        property: MemberProp,
    },
    Pattern(Pattern),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Minus,
    Not,
    BitNot,
    TypeOf,
    Void,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Inc,
    Dec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Exp,
    Eq,
    Neq,
    StrictEq,
    StrictNeq,
    Lt,
    Lte,
    Gt,
    Gte,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitOr,
    BitXor,
    In,
    InstanceOf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
    Nullish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Exp,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitOr,
    BitXor,
    And,
    Or,
    Nullish,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    BigInt(String),
    String(String),
    RegExp {
        pattern: String,
        flags: String,
    },
    This,
    Super,
    Ident(String),
    PrivateIn {
        name: String,
        right: Box<Expr>,
    },
    Array(Vec<ArrayElem>),
    Object(Vec<ObjectProp>),
    Function(Rc<FuncNode>),
    Arrow(Rc<FuncNode>),
    Class(Rc<ClassNode>),
    Template {
        tag: Option<Box<Expr>>,
        quasis: Vec<TemplateQuasi>,
        exprs: Vec<Expr>,
    },
    Member {
        object: Box<Expr>,
        property: MemberProp,
        optional: bool,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<CallArg>,
        optional: bool,
    },
    OptionalChain(Box<Expr>),
    New {
        callee: Box<Expr>,
        args: Vec<CallArg>,
    },
    NewTarget,
    ImportMeta,
    DynamicImport {
        spec: Box<Expr>,
        options: Option<Box<Expr>>,
        phase: ImportPhase,
    },
    Unary {
        op: UnaryOp,
        arg: Box<Expr>,
    },
    Update {
        op: UpdateOp,
        arg: Box<Expr>,
        prefix: bool,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Logical {
        op: LogicalOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Conditional {
        test: Box<Expr>,
        consequent: Box<Expr>,
        alternate: Box<Expr>,
    },
    Assign {
        op: AssignOp,
        left: AssignTarget,
        right: Box<Expr>,
    },
    Sequence(Vec<Expr>),
    Yield {
        arg: Option<Box<Expr>>,
        delegate: bool,
    },
    Await(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportPhase {
    Evaluation,
    Defer,
    Source,
}

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub specifiers: Vec<ImportSpecifier>,
    pub source: String,
    pub phase: ImportPhase,
    pub attributes: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub enum ImportSpecifier {
    Default(String),
    Namespace(String),
    Named { imported: String, local: String },
}

#[derive(Debug, Clone)]
pub enum ExportDecl {
    DefaultExpr(Expr),
    DefaultFunc(Rc<FuncNode>),
    DefaultClass(Rc<ClassNode>),
    Decl(Box<Stmt>),
    Named {
        specifiers: Vec<ExportSpecifier>,
        source: Option<String>,
        attributes: Vec<(String, String)>,
    },
    All {
        exported: Option<String>,
        source: String,
        attributes: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone)]
pub struct ExportSpecifier {
    pub local: String,
    pub exported: String,
    pub local_is_string: bool,
}
