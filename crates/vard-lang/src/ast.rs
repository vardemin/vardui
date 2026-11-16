//! Arena-based AST. Expressions and UI nodes live in flat vectors and are referenced by
//! index, so the runtime can hold cheap `ExprId`s in reactive effects without lifetimes.

use crate::diag::Span;
use std::collections::HashMap;

pub type ExprId = u32;
pub type UiId = u32;

#[derive(Clone, Debug)]
pub enum StrPart {
    Lit(String),
    Expr(ExprId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    /// Postfix `x!` — force-unwrap an optional.
    Unwrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Coalesce,
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<String>,
    pub name_span: Span,
    pub value: ExprId,
}

#[derive(Clone, Debug)]
pub enum LambdaBody {
    Expr(ExprId),
    Block(Vec<Stmt>),
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Bool(bool),
    Nil,
    Str(Vec<StrPart>),
    Color(u32),
    Ident(String),
    /// `.name` — enum variant or design token, resolved from the expected type.
    Dot(String),
    List(Vec<ExprId>),
    /// `["key": value, ...]`, `[:]` = empty map. Keys are Strings.
    Map(Vec<(ExprId, ExprId)>),
    Unary(UnOp, ExprId),
    Binary(BinOp, ExprId, ExprId),
    Ternary(ExprId, ExprId, ExprId),
    Member(ExprId, String, Span),
    Index(ExprId, ExprId),
    Call { callee: ExprId, args: Vec<Arg> },
    Lambda { params: Vec<String>, body: LambdaBody },
    /// Placeholder produced by error recovery.
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add,
    Sub,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let { name: String, mutable: bool, ty: Option<TypeRef>, value: ExprId, span: Span },
    Assign { target: ExprId, op: AssignOp, value: ExprId, span: Span },
    Expr(ExprId),
    If { cond: ExprId, then: Vec<Stmt>, els: Vec<Stmt> },
    For { var: String, iter: ExprId, body: Vec<Stmt>, span: Span },
    Return(Option<ExprId>, Span),
}

#[derive(Clone, Debug)]
pub enum TypeRef {
    Named(String, Span),
    List(Box<TypeRef>),
    /// `[String: T]`
    Map(Box<TypeRef>),
    Opt(Box<TypeRef>),
    Fn(Vec<TypeRef>, Option<Box<TypeRef>>),
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: TypeRef,
    pub default: Option<ExprId>,
    pub bind: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeRef>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ExternDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeRef>,
    pub span: Span,
}

/// `extern event name(params)` — something the platform tells the app (push, back, lifecycle).
#[derive(Clone, Debug)]
pub struct EventDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<Param>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub value: ExprId,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ThemeDecl {
    pub dark: bool,
    pub entries: Vec<(String, ExprId, Span)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum UiNode {
    Element { name: String, name_span: Span, args: Vec<Arg>, children: Vec<UiId>, has_block: bool, span: Span },
    If { cond: ExprId, then: Vec<UiId>, els: Vec<UiId>, span: Span },
    For { var: String, index_var: Option<String>, iter: ExprId, key: Option<ExprId>, body: Vec<UiId>, span: Span },
    /// Insert a `slot` parameter's content.
    Slot { name: String, span: Span },
}

#[derive(Clone, Debug)]
pub enum ViewMember {
    State { name: String, ty: Option<TypeRef>, init: ExprId, span: Span },
    /// `pref name = init` — state persisted in the platform preferences, shared app-wide by name.
    Pref { name: String, ty: Option<TypeRef>, init: ExprId, span: Span },
    Let { name: String, ty: Option<TypeRef>, value: ExprId, span: Span },
    Fn(FnDecl),
    /// `on event(params) [if cond] { body }` — handler of a platform event while the view is alive.
    On { event: String, event_span: Span, params: Vec<String>, cond: Option<ExprId>, body: Vec<Stmt>, span: Span },
    Node(UiId),
}

#[derive(Clone, Debug)]
pub struct ViewDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub members: Vec<ViewMember>,
    pub span: Span,
}

/// How a `.name` expression was resolved by the checker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DotKind {
    Enum,
    Color,
}

#[derive(Default)]
pub struct Program {
    pub exprs: Vec<Expr>,
    pub expr_spans: Vec<Span>,
    pub nodes: Vec<UiNode>,
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub consts: Vec<ConstDecl>,
    pub fns: Vec<FnDecl>,
    pub externs: Vec<ExternDecl>,
    pub events: Vec<EventDecl>,
    pub views: Vec<ViewDecl>,
    pub themes: Vec<ThemeDecl>,
    // ---- filled by the checker ----
    pub dot_kinds: HashMap<ExprId, DotKind>,
    pub view_index: HashMap<String, usize>,
    pub fn_index: HashMap<String, usize>,
    pub struct_index: HashMap<String, usize>,
    pub const_index: HashMap<String, usize>,
    pub extern_index: HashMap<String, usize>,
    pub event_index: HashMap<String, usize>,
    /// User code mentions `safeArea` (prop or value): it manages system insets itself, so the
    /// runtime does not pad the root.
    pub uses_safe_area: bool,
}

impl Program {
    pub fn add_expr(&mut self, e: Expr, span: Span) -> ExprId {
        self.exprs.push(e);
        self.expr_spans.push(span);
        (self.exprs.len() - 1) as ExprId
    }
    pub fn add_node(&mut self, n: UiNode) -> UiId {
        self.nodes.push(n);
        (self.nodes.len() - 1) as UiId
    }
    pub fn expr(&self, id: ExprId) -> &Expr {
        &self.exprs[id as usize]
    }
    pub fn span(&self, id: ExprId) -> Span {
        self.expr_spans[id as usize]
    }
    pub fn node(&self, id: UiId) -> &UiNode {
        &self.nodes[id as usize]
    }
}
