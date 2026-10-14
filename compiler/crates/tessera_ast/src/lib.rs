//! Tessera AST. Pure data — no parsing logic. Kept small for v1.

use tessera_lexer::Span;

#[derive(Debug, Clone)]
pub struct Module {
    pub items: Vec<Item>,
}

// ---------------------------------------------------------------------------
// Items (top-level)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    Const { name: String, ty: Option<Ty>, value: Expr },
    Static { name: String, ty: Ty, value: Expr },
    Fn(FnDef),
    Struct(StructDef),
    Enum(EnumDef),
    TypeAlias { name: String, ty: Ty },
    Import { path: String },
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<Ty>,
    pub body: Block,
    pub is_const: bool,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Ty,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub value: Option<Expr>,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Ty {
    /// Builtin or user type / path: `u64`, `i32`, `Color`, `MyStruct`.
    Named { segs: Vec<String>, span: Span },
    /// `&T`
    Ref { ty: Box<Ty>, span: Span },
    /// `*T`
    Ptr { ty: Box<Ty>, span: Span },
    /// `[T; N]`
    Array { elem: Box<Ty>, len: Box<Expr>, span: Span },
    /// `[]T`
    Slice { elem: Box<Ty>, span: Span },
}

// ---------------------------------------------------------------------------
// Expressions and statements
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Lit(Lit),
    Ident(String),
    Path(Vec<String>), // 2+ segments; single segment stays `Ident`
    StructLit { path: Vec<String>, fields: Vec<(String, Expr)> },
    Unary { op: UnaryOp, expr: Box<Expr> },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Assign { op: AssignOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Expr> },
    Field { base: Box<Expr>, name: String },
    Index { base: Box<Expr>, index: Box<Expr> },
    If { cond: Box<Expr>, then: Block, els: Option<Box<Expr>> },
    Block(Block),
    While { cond: Box<Expr>, body: Block },
    Loop(Block),
    For { name: String, iter: Box<Expr>, body: Block },
    Unit,
    ArrayLit(Vec<Expr>),
    ArrayRepeat { value: Box<Expr>, len: Box<Expr> },
}

#[derive(Debug, Clone)]
pub enum Lit {
    Int(String),
    Float(String),
    Str(String),
    Char(i64),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Rem, Shl, Shr,
    BitAnd, BitOr, BitXor,
    And, Or,
    Eq, Ne, Lt, Le, Gt, Ge,
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp { Neg, Not, BitNot, Deref, Ref }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Add, Sub, Mul, Div, Rem,
    BitAnd, BitOr, BitXor,
    Shl, Shr,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Let { name: String, ty: Option<Ty>, init: Option<Expr>, mutable: bool },
    Return(Option<Expr>),
    Break,
    Continue,
    Expr(Expr),
}
