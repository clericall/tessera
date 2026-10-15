//! Tessera type checker + typed IR.
//!
//! Supports the engine slice: functions, integer/bool arithmetic/comparisons/logic,
//! assignments to arbitrary *places* (`x`, `s.f`, `a[i]`, `s.p[i]`), `let`/`var`,
//! control flow, calls, the bit builtins `popcount/ctz/clz`, `print_*`, **structs**,
//! **fixed arrays + indexing**, struct/array literals, and functions that take and
//! return structs by value. Still outstanding (clear diagnostics): pointers/refs,
//! slices, floats, strings-as-values, static globals, and `const fn` evaluation.

use std::collections::{HashMap, HashSet};

use tessera_ast as a;
use tessera_lexer::Span;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Type {
    Int { bits: u16, signed: bool },
    Bool,
    Unit,
    Array { elem: Box<Type>, len: usize },
    Struct(String),
    Str,
}

impl Type {
    pub fn i64() -> Type {
        Type::Int { bits: 64, signed: true }
    }
    pub fn u64() -> Type {
        Type::Int { bits: 64, signed: false }
    }
    pub fn i32() -> Type {
        Type::Int { bits: 32, signed: true }
    }
    pub fn is_int(&self) -> bool {
        matches!(self, Type::Int { .. })
    }
    pub fn is_bool(&self) -> bool {
        matches!(self, Type::Bool)
    }
    pub fn is_numeric(&self) -> bool {
        self.is_int()
    }
    pub fn display(&self) -> String {
        match self {
            Type::Int { bits, signed } => format!("{}{}", if *signed { "i" } else { "u" }, bits),
            Type::Bool => "bool".into(),
            Type::Str => "str".into(),
            Type::Unit => "()".into(),
            Type::Array { elem, len } => format!("[{}; {}]", elem.display(), len),
            Type::Struct(n) => n.clone(),
        }
    }
    pub fn c_ty(&self) -> String {
        match self {
            Type::Int { bits: 8, signed: true } => "int8_t".into(),
            Type::Int { bits: 16, signed: true } => "int16_t".into(),
            Type::Int { bits: 32, signed: true } => "int32_t".into(),
            Type::Int { bits: 64, signed: true } => "int64_t".into(),
            Type::Int { bits: 8, signed: false } => "uint8_t".into(),
            Type::Int { bits: 16, signed: false } => "uint16_t".into(),
            Type::Int { bits: 32, signed: false } => "uint32_t".into(),
            Type::Int { bits: 64, signed: false } => "uint64_t".into(),
            Type::Int { .. } => "int64_t".into(),
            Type::Bool => "bool".into(),
            Type::Str => "Tr_str".into(),
            Type::Unit => "void".into(),
            Type::Array { elem, len } => format!("{}[{}]", elem.c_ty(), len),
            Type::Struct(n) => format!("Tr_{n}"),
        }
    }
}

fn unify_int(x: &Type, y: &Type) -> Option<Type> {
    match (x, y) {
        (Type::Int { bits: b1, signed: s1 }, Type::Int { bits: b2, signed: s2 }) => {
            Some(Type::Int { bits: (*b1).max(*b2), signed: *s1 && *s2 })
        }
        _ => None,
    }
}

fn builtin_type(name: &str) -> Option<Type> {
    Some(match name {
        "i8" => Type::Int { bits: 8, signed: true },
        "i16" => Type::Int { bits: 16, signed: true },
        "i32" => Type::Int { bits: 32, signed: true },
        "i64" | "isize" => Type::Int { bits: 64, signed: true },
        "u8" => Type::Int { bits: 8, signed: false },
        "u16" => Type::Int { bits: 16, signed: false },
        "u32" => Type::Int { bits: 32, signed: false },
        "u64" | "usize" => Type::Int { bits: 64, signed: false },
        "bool" => Type::Bool,
        "str" => Type::Str,
        _ => return None,
    })
}

fn const_usize(e: &a::Expr) -> Option<usize> {
    if let a::ExprKind::Lit(a::Lit::Int(t)) = &e.kind {
        parse_int_lit(t).and_then(|v| v.try_into().ok())
    } else {
        None
    }
}

/// Evaluate an expression to a compile-time array length: an int literal or a
/// previously-declared integer constant.
fn const_int(consts: &HashMap<String, (i128, Type)>, e: &a::Expr) -> Option<usize> {
    match &e.kind {
        a::ExprKind::Lit(a::Lit::Int(t)) => parse_int_lit(t).and_then(|v| usize::try_from(v).ok()),
        a::ExprKind::Ident(n) => consts.get(n).and_then(|(v, _)| usize::try_from(*v).ok()),
        _ => None,
    }
}

/// Render a compile-time constant initializer as C (scalars + const arrays of
/// int/bool literals). Returns None if `e` isn't a compile-time constant yet.
fn const_init(ty: &Type, e: &a::Expr) -> Option<String> {
    match (ty, &e.kind) {
        (Type::Int { .. }, a::ExprKind::Lit(a::Lit::Int(text))) => {
            let v = parse_int_lit(text)?;
            if let Type::Int { signed: false, .. } = ty {
                Some(format!("(({}){}ULL)", ty.c_ty(), v))
            } else {
                Some(format!("(({}){}LL)", ty.c_ty(), v))
            }
        }
        (Type::Bool, a::ExprKind::Lit(a::Lit::Bool(b))) => Some(if *b { "true".into() } else { "false".into() }),
        (Type::Array { elem, .. }, a::ExprKind::ArrayLit(items)) => {
            let mut parts = Vec::new();
            for it in items {
                parts.push(const_init(elem, it)?);
            }
            Some(format!("{{ {} }}", parts.join(", ")))
        }
        (Type::Array { elem, .. }, a::ExprKind::ArrayRepeat { value, len }) => {
            let n = const_usize(len)?;
            // `[0; N]` for an integer array -> C `{0}` (C zero-fills the rest);
            // avoids emitting N literal zeros into the generated C.
            if let (Type::Int { .. }, a::ExprKind::Lit(a::Lit::Int(t))) = (&**elem, &value.kind) {
                if parse_int_lit(t) == Some(0) {
                    return Some("{0}".to_string());
                }
            }
            let one = const_init(elem, value)?;
            let parts: Vec<String> = std::iter::repeat(one).take(n).collect();
            Some(format!("{{ {} }}", parts.join(", ")))
        }
        _ => None,
    }
}

fn resolve_ty_ctx(ty: &a::Ty, names: &HashSet<String>, consts: &HashMap<String, (i128, Type)>) -> Type {
    match ty {
        a::Ty::Named { segs, .. } if segs.len() == 1 => {
            builtin_type(&segs[0])
                .or_else(|| names.get(&segs[0]).map(|_| Type::Struct(segs[0].clone())))
                .unwrap_or(Type::i64())
        }
        a::Ty::Array { elem, len, .. } => Type::Array {
            elem: Box::new(resolve_ty_ctx(elem, names, consts)),
            len: const_int(consts, len).unwrap_or(0),
        },
        _ => Type::Unit,
    }
}

fn parse_int_lit(s: &str) -> Option<i128> {
    let s: String = s.chars().filter(|&c| c != '_').collect();
    let (radix, digits) = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (16, h)
    } else if let Some(b) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        (2, b)
    } else {
        (10, s.as_str())
    };
    i128::from_str_radix(digits, radix).ok()
}

fn decode_str(raw: &str) -> String {
    let b = raw.as_bytes();
    if b.len() < 2 {
        return String::new();
    }
    let inner = &raw[1..raw.len() - 1];
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('0') => out.push('\0'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Typed IR (TIR)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub funcs: Vec<Func>,
    pub structs: Vec<StructDef>,
    pub globals: Vec<(String, Type, String)>, // (name, type, C initializer)
}

#[derive(Debug, Clone)]
pub struct Func {
    pub name: String,
    pub ret: Type,
    pub arity: usize,
    pub local_tys: Vec<Type>,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Let { id: usize, init: Option<Expr> },
    Expr(Expr),
    Return(Option<Expr>),
    Break,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HBin {
    Add, Sub, Mul, Div, Rem, Shl, Shr, BitAnd, BitOr, BitXor,
    Eq, Ne, Lt, Le, Gt, Ge, And, Or,
}
impl HBin {
    pub fn c(&self) -> &'static str {
        match self {
            HBin::Add => "+", HBin::Sub => "-", HBin::Mul => "*", HBin::Div => "/", HBin::Rem => "%",
            HBin::Shl => "<<", HBin::Shr => ">>",
            HBin::BitAnd => "&", HBin::BitOr => "|", HBin::BitXor => "^",
            HBin::Eq => "==", HBin::Ne => "!=", HBin::Lt => "<", HBin::Le => "<=",
            HBin::Gt => ">", HBin::Ge => ">=", HBin::And => "&&", HBin::Or => "||",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HUn { Neg, Not, BitNot }

/// An assignable location.
#[derive(Debug, Clone)]
pub enum Place {
    Var(usize),
    Global(String),
    Field(Box<Place>, String),
    Index(Box<Place>, Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    IntLit(i128),
    BoolLit(bool),
    Str(String),
    Var(usize),
    Binary { op: HBin, lhs: Box<Expr>, rhs: Box<Expr> },
    Unary { op: HUn, expr: Box<Expr> },
    Assign { place: Box<Place>, value: Box<Expr> },
    Call { func: String, args: Vec<Expr> },
    Field { base: Box<Expr>, name: String },
    Index { base: Box<Expr>, index: Box<Expr> },
    ArrayInit(Vec<Expr>),
    StructInit { name: String, fields: Vec<(String, Expr)> },
    If { cond: Box<Expr>, then: Block, els: Option<Block> },
    While { cond: Box<Expr>, body: Block },
    Loop(Block),
    For { id: usize, lo: Box<Expr>, hi: Box<Expr>, body: Block },
    Block(Block),
    Unit,
    Global(String),
}

#[derive(Debug, Clone)]
pub struct CheckError {
    pub span: Span,
    pub message: String,
    pub help: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn unit_expr(sp: Span) -> Expr {
    Expr { kind: ExprKind::Unit, ty: Type::Unit, span: sp }
}
trait AsTyped {
    fn typed(self, ty: Type, span: Span) -> Expr;
}
impl AsTyped for ExprKind {
    fn typed(self, ty: Type, span: Span) -> Expr {
        Expr { kind: self, ty, span }
    }
}
fn coerce(e: Expr, t: &Type) -> Expr {
    if e.ty == *t || (e.ty.is_int() && t.is_int()) || matches!(t, Type::Unit) {
        Expr { ty: t.clone(), ..e }
    } else {
        e
    }
}
fn ast_binop(o: a::BinOp) -> Option<HBin> {
    Some(match o {
        a::BinOp::Add => HBin::Add, a::BinOp::Sub => HBin::Sub, a::BinOp::Mul => HBin::Mul,
        a::BinOp::Div => HBin::Div, a::BinOp::Rem => HBin::Rem, a::BinOp::Shl => HBin::Shl,
        a::BinOp::Shr => HBin::Shr, a::BinOp::BitAnd => HBin::BitAnd, a::BinOp::BitOr => HBin::BitOr,
        a::BinOp::BitXor => HBin::BitXor, a::BinOp::Eq => HBin::Eq, a::BinOp::Ne => HBin::Ne,
        a::BinOp::Lt => HBin::Lt, a::BinOp::Le => HBin::Le, a::BinOp::Gt => HBin::Gt, a::BinOp::Ge => HBin::Ge,
        a::BinOp::And => HBin::And, a::BinOp::Or => HBin::Or,
        a::BinOp::Range => return None,
    })
}
fn ast_assignop(o: a::AssignOp) -> HBin {
    match o {
        a::AssignOp::Set => unreachable!(),
        a::AssignOp::Add => HBin::Add, a::AssignOp::Sub => HBin::Sub, a::AssignOp::Mul => HBin::Mul,
        a::AssignOp::Div => HBin::Div, a::AssignOp::Rem => HBin::Rem, a::AssignOp::BitAnd => HBin::BitAnd,
        a::AssignOp::BitOr => HBin::BitOr, a::AssignOp::BitXor => HBin::BitXor,
        a::AssignOp::Shl => HBin::Shl, a::AssignOp::Shr => HBin::Shr,
    }
}

// ---------------------------------------------------------------------------
// Checker
// ---------------------------------------------------------------------------

struct Checker {
    funcs: HashMap<String, (Vec<Type>, Type)>,
    consts: HashMap<String, (i128, Type)>,
    structs: HashMap<String, Vec<(String, Type)>>,
    globals: HashMap<String, Type>,
    cur_ret: Type,
    local_tys: Vec<Type>,
    scopes: Vec<HashMap<String, usize>>,
    errors: Vec<CheckError>,
}

impl Checker {
    fn err(&mut self, span: Span, message: impl Into<String>, help: Option<&str>) {
        self.errors.push(CheckError { span, message: message.into(), help: help.map(Into::into) });
    }
    fn no_value(&mut self, sp: Span, expect: Option<&Type>) {
        if let Some(t) = expect {
            if *t != Type::Unit {
                self.err(sp, "control-flow (if/while/loop/for) produces no value", None);
            }
        }
    }
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    fn new_local(&mut self, name: &str, ty: Type) -> usize {
        let id = self.local_tys.len();
        self.local_tys.push(ty);
        self.scopes.last_mut().unwrap().insert(name.to_string(), id);
        id
    }
    fn lookup(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rev().find_map(|m| m.get(name).copied())
    }
    fn field_ty(&self, sname: &str, field: &str) -> Option<Type> {
        self.structs
            .get(sname)
            .and_then(|fs| fs.iter().find(|(n, _)| n == field).map(|(_, t)| t.clone()))
    }

    fn resolve_type(&mut self, ty: &a::Ty) -> Option<Type> {
        match ty {
            a::Ty::Named { segs, span } => {
                if segs.len() != 1 {
                    self.err(*span, "paths are not supported as types yet", None);
                    return None;
                }
                if let Some(t) = builtin_type(&segs[0]) {
                    return Some(t);
                }
                if self.structs.contains_key(&segs[0]) {
                    return Some(Type::Struct(segs[0].clone()));
                }
                self.err(*span, format!("unknown type `{}`", segs[0]), None);
                None
            }
            a::Ty::Array { elem, len, .. } => {
                let e = self.resolve_type(elem)?;
                match const_int(&self.consts, len) {
                    Some(n) => Some(Type::Array { elem: Box::new(e), len: n }),
                    None => {
                        self.err(len.span, "array length must be a compile-time integer in this slice", None);
                        None
                    }
                }
            }
            a::Ty::Ref { span, .. } | a::Ty::Ptr { span, .. } => {
                self.err(*span, "references/pointers are not supported yet", None);
                None
            }
            a::Ty::Slice { span, .. } => {
                self.err(*span, "slices are not supported yet", None);
                None
            }
        }
    }

    fn check_expr(&mut self, e: &a::Expr, expect: Option<&Type>) -> Expr {
        let sp = e.span;
        match &e.kind {
            a::ExprKind::Lit(a::Lit::Int(text)) => {
                let val = match parse_int_lit(text) {
                    Some(v) => v,
                    None => {
                        self.err(sp, format!("integer literal `{text}` out of range"), None);
                        0
                    }
                };
                let ty = expect.cloned().filter(|t| t.is_int()).unwrap_or_else(Type::i64);
                ExprKind::IntLit(val).typed(ty, sp)
            }
            a::ExprKind::Lit(a::Lit::Bool(b)) => ExprKind::BoolLit(*b).typed(Type::Bool, sp),
            a::ExprKind::Lit(a::Lit::Char(v)) => ExprKind::IntLit(*v as i128).typed(Type::i64(), sp),
            a::ExprKind::Lit(a::Lit::Str(s)) => ExprKind::Str(decode_str(s)).typed(Type::Str, sp),
            a::ExprKind::Lit(a::Lit::Float(_)) => {
                self.err(sp, "floating-point is not supported yet", None);
                unit_expr(sp)
            }
            a::ExprKind::Unit => ExprKind::Unit.typed(Type::Unit, sp),
            a::ExprKind::Ident(name) => {
                if let Some((v, ty)) = self.consts.get(name) {
                    ExprKind::IntLit(*v).typed(ty.clone(), sp)
                } else if let Some(id) = self.lookup(name) {
                    ExprKind::Var(id).typed(self.local_tys[id].clone(), sp)
                } else if let Some(ty) = self.globals.get(name) {
                    ExprKind::Global(name.clone()).typed(ty.clone(), sp)
                } else {
                    self.err(sp, format!("unknown name `{name}`"), None);
                    unit_expr(sp)
                }
            }
            a::ExprKind::Path(segs) => {
                self.err(sp, format!("path `{}` is not supported yet", segs.join("::")), None);
                unit_expr(sp)
            }
            a::ExprKind::StructLit { path, fields } => self.check_struct_lit(path, fields, sp),
            a::ExprKind::ArrayLit(items) => self.check_array_lit(items, expect, sp),
            a::ExprKind::ArrayRepeat { value, len } => {
                match const_int(&self.consts, len) {
                    Some(n) => {
                        let elem_ty = match expect {
                            Some(Type::Array { elem, len: l }) if *l == n => (**elem).clone(),
                            Some(Type::Array { elem, .. }) => (**elem).clone(),
                            _ => self.check_expr(value, None).ty,
                        };
                        if let Some(Type::Array { len: l, .. }) = expect {
                            if *l != n {
                                self.err(sp, format!("array repeat length {n} != expected {l}"), None);
                            }
                        }
                        let ve = coerce(self.check_expr(value, Some(&elem_ty)), &elem_ty);
                        let items: Vec<Expr> = (0..n).map(|_| ve.clone()).collect();
                        ExprKind::ArrayInit(items).typed(Type::Array { elem: Box::new(elem_ty), len: n }, sp)
                    }
                    None => {
                        self.err(len.span, "array repeat length must be a compile-time integer", None);
                        unit_expr(sp)
                    }
                }
            }
            a::ExprKind::Unary { op, expr } => {
                let inner = self.check_expr(expr, None);
                match op {
                    a::UnaryOp::Neg => {
                        if !inner.ty.is_int() {
                            self.err(sp, format!("unary `-` requires an integer, found {}", inner.ty.display()), None);
                        }
                        ExprKind::Unary { op: HUn::Neg, expr: Box::new(inner.clone()) }.typed(inner.ty.clone(), sp)
                    }
                    a::UnaryOp::Not => {
                        if !inner.ty.is_bool() {
                            self.err(sp, format!("unary `!` requires bool, found {}", inner.ty.display()), None);
                        }
                        ExprKind::Unary { op: HUn::Not, expr: Box::new(inner.clone()) }.typed(Type::Bool, sp)
                    }
                    a::UnaryOp::BitNot => {
                        if !inner.ty.is_int() {
                            self.err(sp, format!("unary `~` requires an integer, found {}", inner.ty.display()), None);
                        }
                        ExprKind::Unary { op: HUn::BitNot, expr: Box::new(inner.clone()) }.typed(inner.ty.clone(), sp)
                    }
                    a::UnaryOp::Deref | a::UnaryOp::Ref => {
                        self.err(sp, "references/pointers are not supported yet", None);
                        unit_expr(sp)
                    }
                }
            }
            a::ExprKind::Binary { op, lhs, rhs } => self.check_binary(*op, lhs, rhs, sp),
            a::ExprKind::Assign { op, lhs, rhs } => self.check_assign(*op, lhs, rhs, sp),
            a::ExprKind::Call { callee, args } => self.check_call(callee, args),
            a::ExprKind::Field { base, name } => {
                let b = self.check_expr(base, None);
                match &b.ty {
                    Type::Struct(sn) => match self.field_ty(sn, name) {
                        Some(fty) => ExprKind::Field { base: Box::new(b), name: name.clone() }.typed(fty, sp),
                        None => {
                            self.err(sp, format!("struct `{sn}` has no field `{name}`"), None);
                            unit_expr(sp)
                        }
                    },
                    other => {
                        self.err(sp, format!("field access on `{}`, which is not a struct", other.display()), None);
                        unit_expr(sp)
                    }
                }
            }
            a::ExprKind::Index { base, index } => {
                let b = self.check_expr(base, None);
                let ix = self.check_expr(index, Some(&Type::i64()));
                let elem_ty = match &b.ty {
                    Type::Array { elem, .. } => Some((**elem).clone()),
                    Type::Str => Some(Type::i64()),
                    _ => None,
                };
                match elem_ty {
                    Some(elem) => ExprKind::Index { base: Box::new(b), index: Box::new(coerce(ix, &Type::i64())) }.typed(elem, sp),                    None => {
                        self.err(sp, format!("indexing `{}`, which is not an array or string", b.ty.display()), None);
                        unit_expr(sp)
                    }
                }
            }
            a::ExprKind::If { cond, then, els } => {
                self.no_value(sp, expect);
                let c = self.check_expr(cond, Some(&Type::Bool));
                if !c.ty.is_bool() {
                    self.err(sp, format!("`if` condition must be bool, found {}", c.ty.display()), None);
                }
                ExprKind::If { cond: Box::new(c), then: self.check_block(then), els: els.as_ref().map(|el| self.check_block_or_expr(el)) }.typed(Type::Unit, sp)
            }
            a::ExprKind::Block(b) => ExprKind::Block(self.check_block(b)).typed(Type::Unit, sp),
            a::ExprKind::While { cond, body } => {
                self.no_value(sp, expect);
                let c = self.check_expr(cond, Some(&Type::Bool));
                if !c.ty.is_bool() {
                    self.err(sp, format!("`while` condition must be bool, found {}", c.ty.display()), None);
                }
                ExprKind::While { cond: Box::new(c), body: self.check_block(body) }.typed(Type::Unit, sp)
            }
            a::ExprKind::Loop(body) => ExprKind::Loop(self.check_block(body)).typed(Type::Unit, sp),
            a::ExprKind::For { name, iter, body } => {
                self.no_value(sp, expect);
                let (lo, hi) = match &iter.kind {
                    a::ExprKind::Binary { op: a::BinOp::Range, lhs, rhs } => (lhs.as_ref(), rhs.as_ref()),
                    _ => {
                        self.err(iter.span, "`for` requires a range `lo..hi`", None);
                        (iter.as_ref(), iter.as_ref())
                    }
                };
                let lo_e = coerce(self.check_expr(lo, Some(&Type::i64())), &Type::i64());
                let hi_e = coerce(self.check_expr(hi, Some(&Type::i64())), &Type::i64());
                self.push_scope();
                let id = self.new_local(name, Type::i64());
                let body_b = self.check_block(body);
                self.pop_scope();
                ExprKind::For { id, lo: Box::new(lo_e), hi: Box::new(hi_e), body: body_b }.typed(Type::Unit, sp)
            }
        }
    }

    fn check_struct_lit(&mut self, path: &[String], fields: &[(String, a::Expr)], sp: Span) -> Expr {
        let name = if path.len() == 1 { path[0].clone() } else { path.join("::") };
        let def = match self.structs.get(&name).cloned() {
            Some(d) => d,
            None => {
                self.err(sp, format!("unknown struct `{name}`"), None);
                return unit_expr(sp);
            }
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut checked = Vec::new();
        for (fname, fexpr) in fields {
            match self.field_ty(&name, fname) {
                Some(fty) => {
                    let ce = self.check_expr(fexpr, Some(&fty));
                    checked.push((fname.clone(), coerce(ce, &fty)));
                    seen.insert(fname.clone());
                }
                None => self.err(fexpr.span, format!("struct `{name}` has no field `{fname}`"), None),
            }
        }
        for (fname, _) in &def {
            if !seen.contains(fname) {
                self.err(sp, format!("missing field `{fname}` in `{name}` literal"), None);
            }
        }
        let sty = ty_of_struct(&name);
        ExprKind::StructInit { name, fields: checked }.typed(sty, sp)
    }

    fn check_array_lit(&mut self, items: &[a::Expr], expect: Option<&Type>, sp: Span) -> Expr {
        let want = match expect {
            Some(Type::Array { elem, len }) => {
                if items.len() != *len {
                    self.err(sp, format!("array literal has {} element(s), expected {}", items.len(), len), None);
                }
                Some((**elem).clone())
            }
            _ => None,
        };
        let elem_ty = want.clone().unwrap_or_else(|| {
            if items.is_empty() {
                Type::i64()
            } else {
                self.check_expr(&items[0], None).ty
            }
        });
        let mut checked = Vec::with_capacity(items.len());
        for it in items {
            let ce = self.check_expr(it, Some(&elem_ty));
            checked.push(coerce(ce, &elem_ty));
        }
        let n = checked.len();
        ExprKind::ArrayInit(checked).typed(Type::Array { elem: Box::new(elem_ty), len: n }, sp)
    }

    fn check_binary(&mut self, op: a::BinOp, lhs: &a::Expr, rhs: &a::Expr, sp: Span) -> Expr {
        let hb = match ast_binop(op) {
            Some(h) => h,
            None => {
                self.err(sp, "a range `..` is only valid as the iterable of a `for` loop", None);
                return unit_expr(sp);
            }
        };
        let want: Option<Type> = match hb {
            HBin::And | HBin::Or => Some(Type::Bool),
            _ => None,
        };
        let l = self.check_expr(lhs, want.as_ref());
        let r = self.check_expr(rhs, want.as_ref());
        match hb {
            HBin::Eq | HBin::Ne | HBin::Lt | HBin::Le | HBin::Gt | HBin::Ge => {
                let rt = match unify_int(&l.ty, &r.ty) {
                    Some(t) => t,
                    None => {
                        self.err(sp, format!("cannot compare {} with {}", l.ty.display(), r.ty.display()), None);
                        Type::i64()
                    }
                };
                ExprKind::Binary { op: hb, lhs: Box::new(coerce(l, &rt)), rhs: Box::new(coerce(r, &rt)) }.typed(Type::Bool, sp)
            }
            HBin::And | HBin::Or => {
                if !l.ty.is_bool() || !r.ty.is_bool() {
                    self.err(sp, format!("`{}` requires bool operands", hb.c()), None);
                }
                ExprKind::Binary { op: hb, lhs: Box::new(l), rhs: Box::new(r) }.typed(Type::Bool, sp)
            }
            _ => {
                let rt = match unify_int(&l.ty, &r.ty) {
                    Some(t) => t,
                    None => {
                        self.err(sp, format!("invalid operands for `{}`: {} and {}", hb.c(), l.ty.display(), r.ty.display()), None);
                        Type::i64()
                    }
                };
                ExprKind::Binary { op: hb, lhs: Box::new(coerce(l, &rt)), rhs: Box::new(coerce(r, &rt)) }.typed(rt, sp)
            }
        }
    }

    fn check_assign(&mut self, op: a::AssignOp, lhs: &a::Expr, rhs: &a::Expr, sp: Span) -> Expr {
        let (place, lty) = match self.check_place(lhs) {
            Some(x) => x,
            None => return unit_expr(sp),
        };
        if op == a::AssignOp::Set {
            let v = coerce(self.check_expr(rhs, Some(&lty)), &lty);
            ExprKind::Assign { place: Box::new(place), value: Box::new(v) }.typed(lty, sp)
        } else {
            let hb = ast_assignop(op);
            let re = coerce(self.check_expr(rhs, Some(&lty)), &lty);
            let read = self.read_of_place(&place, sp);
            let combined = ExprKind::Binary { op: hb, lhs: Box::new(read), rhs: Box::new(re) }.typed(lty.clone(), sp);
            ExprKind::Assign { place: Box::new(place), value: Box::new(combined) }.typed(lty, sp)
        }
    }

    fn check_place(&mut self, e: &a::Expr) -> Option<(Place, Type)> {
        match &e.kind {
            a::ExprKind::Ident(name) => {
                if self.consts.contains_key(name) {
                    self.err(e.span, format!("cannot assign to constant `{name}`"), None);
                    None
                } else if let Some(id) = self.lookup(name) {
                    Some((Place::Var(id), self.local_tys[id].clone()))
                } else if let Some(ty) = self.globals.get(name) {
                    Some((Place::Global(name.clone()), ty.clone()))
                } else {
                    self.err(e.span, format!("cannot assign to unknown name `{name}`"), None);
                    None
                }
            }
            a::ExprKind::Field { base, name } => {
                let (bp, bty) = self.check_place(base)?;
                match &bty {
                    Type::Struct(sn) => match self.field_ty(sn, name) {
                        Some(fty) => Some((Place::Field(Box::new(bp), name.clone()), fty)),
                        None => {
                            self.err(e.span, format!("struct `{sn}` has no field `{name}`"), None);
                            None
                        }
                    },
                    other => {
                        self.err(e.span, format!("cannot assign a field of `{}`", other.display()), None);
                        None
                    }
                }
            }
            a::ExprKind::Index { base, index } => {
                let (bp, bty) = self.check_place(base)?;
                let ix = self.check_expr(index, Some(&Type::i64()));
                match &bty {
                    Type::Array { elem, .. } => {
                        Some((Place::Index(Box::new(bp), Box::new(coerce(ix, &Type::i64()))), (**elem).clone()))
                    }
                    other => {
                        self.err(e.span, format!("cannot index-assign `{}`", other.display()), None);
                        None
                    }
                }
            }
            _ => {
                self.err(e.span, "invalid assignment target (expected a variable, field, or index)", None);
                None
            }
        }
    }

    fn read_of_place(&self, p: &Place, sp: Span) -> Expr {
        match p {
            Place::Var(id) => ExprKind::Var(*id).typed(self.local_tys[*id].clone(), sp),
            Place::Global(name) => {
                ExprKind::Global(name.clone()).typed(self.globals.get(name).cloned().unwrap_or(Type::i64()), sp)
            }
            Place::Field(inner, name) => {
                let base = self.read_of_place(inner, sp);
                let fty = match &base.ty {
                    Type::Struct(sn) => self.field_ty(sn, name).unwrap_or(Type::Unit),
                    _ => Type::Unit,
                };
                ExprKind::Field { base: Box::new(base), name: name.clone() }.typed(fty, sp)
            }
            Place::Index(inner, idx) => {
                let base = self.read_of_place(inner, sp);
                let ety = match &base.ty {
                    Type::Array { elem, .. } => (**elem).clone(),
                    _ => Type::Unit,
                };
                ExprKind::Index { base: Box::new(base), index: idx.clone() }.typed(ety, sp)
            }
        }
    }

    fn check_call(&mut self, callee: &a::Expr, args: &[a::Expr]) -> Expr {
        let sp = callee.span;
        let name = match &callee.kind {
            a::ExprKind::Ident(n) => n.clone(),
            a::ExprKind::Path(s) => {
                self.err(sp, format!("associated call `{}::(...)` is not supported yet", s.join("::")), None);
                return unit_expr(sp);
            }
            _ => {
                self.err(sp, "expression is not callable", None);
                return unit_expr(sp);
            }
        };
        match name.as_str() {
            "popcount" | "ctz" | "clz" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = coerce(self.check_expr(&args[0], Some(&Type::u64())), &Type::u64());
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::i32(), sp)
            }
            "print_i64" | "print_uint" | "print_bool" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let want = if name == "print_bool" { Type::Bool } else { Type::i64() };
                let a = self.check_expr(&args[0], Some(&want));
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::Unit, sp)
            }
            "print_str" => {
                if args.len() == 1 {
                    if let a::ExprKind::Lit(a::Lit::Str(s)) = &args[0].kind {
                        return ExprKind::Call {
                            func: name,
                            args: vec![Expr { kind: ExprKind::Str(decode_str(s)), ty: Type::Unit, span: args[0].span }],
                        }
                        .typed(Type::Unit, sp);
                    }
                }
                self.err(args.get(0).map(|x| x.span).unwrap_or(sp), "`print_str` takes a single string literal", None);
                unit_expr(sp)
            }
            "putstr" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = self.check_expr(&args[0], Some(&Type::Str));
                if a.ty != Type::Str { self.err(args[0].span, "`putstr` expects a string", None); }
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::Unit, sp)
            }
            "putc" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = self.check_expr(&args[0], Some(&Type::i64()));
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::Unit, sp)
            }
            "putint" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = self.check_expr(&args[0], Some(&Type::i64()));
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::Unit, sp)
            }
            "read_line" => {
                if !args.is_empty() { self.err(sp, "`read_line` takes no arguments", None); }
                ExprKind::Call { func: name, args: vec![] }.typed(Type::Str, sp)
            }
            "now_ms" => {
                if !args.is_empty() { self.err(sp, "`now_ms` takes no arguments", None); }
                ExprKind::Call { func: name, args: vec![] }.typed(Type::i64(), sp)
            }
            "fopen" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = self.check_expr(&args[0], Some(&Type::Str));
                if a.ty != Type::Str { self.err(args[0].span, "`fopen` expects a string path", None); }
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::i64(), sp)
            }
            "substr" => {
                if !self.expect_arity(&name, args, 3) { return unit_expr(sp); }
                let s = self.check_expr(&args[0], Some(&Type::Str));
                if s.ty != Type::Str { self.err(args[0].span, "`substr` expects a string", None); }
                let a = self.check_expr(&args[1], Some(&Type::i64()));
                let n = self.check_expr(&args[2], Some(&Type::i64()));
                ExprKind::Call { func: name, args: vec![s, a, n] }.typed(Type::Str, sp)
            }
            "fread_u8" | "fread_i16" | "fread_i32" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = self.check_expr(&args[0], Some(&Type::i64()));
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::i64(), sp)
            }
            "feof" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = self.check_expr(&args[0], Some(&Type::i64()));
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::Bool, sp)
            }
            "fclose" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = self.check_expr(&args[0], Some(&Type::i64()));
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::i64(), sp)
            }
            "flush" => {
                if !args.is_empty() { self.err(sp, "`flush` takes no arguments", None); }
                ExprKind::Call { func: name, args: vec![] }.typed(Type::Unit, sp)
            }
            "btable_index" => {
                if !self.expect_arity(&name, args, 3) { return unit_expr(sp); }
                let o = self.check_expr(&args[0], Some(&Type::u64()));
                let m = self.check_expr(&args[1], Some(&Type::u64()));
                let s = self.check_expr(&args[2], Some(&Type::i64()));
                ExprKind::Call { func: name, args: vec![o, m, s] }.typed(Type::i64(), sp)
            }
            "len" => {
                if !self.expect_arity(&name, args, 1) { return unit_expr(sp); }
                let a = self.check_expr(&args[0], Some(&Type::Str));
                if a.ty != Type::Str {
                    self.err(args[0].span, "`len` expects a string", None);
                }
                ExprKind::Call { func: name, args: vec![a] }.typed(Type::i64(), sp)
            }
            _ => match self.funcs.get(&name).cloned() {
                Some((params, ret)) => {
                    if params.len() != args.len() {
                        self.err(sp, format!("function `{name}` expects {} argument(s), found {}", params.len(), args.len()), None);
                    }
                    let mut checked = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        let ce = self.check_expr(arg, params.get(i));
                        let ce = match params.get(i) {
                            Some(t) => coerce(ce, t),
                            None => ce,
                        };
                        checked.push(ce);
                    }
                    ExprKind::Call { func: name, args: checked }.typed(ret, sp)
                }
                None => {
                    self.err(sp, format!("unknown function `{name}`"), None);
                    unit_expr(sp)
                }
            },
        }
    }

    /// Reports an arity mismatch and returns false so callers can bail out
    /// instead of indexing past the end of `args` (a wrong-arity builtin call
    /// must be a diagnostic, never a compiler panic).
    fn expect_arity(&mut self, name: &str, args: &[a::Expr], n: usize) -> bool {
        if args.len() != n {
            let sp = args.get(0).map(|x| x.span).unwrap_or(Span::new(0, 0));
            self.err(sp, format!("`{name}` takes {n} argument(s)"), None);
            return false;
        }
        true
    }

    fn check_block(&mut self, b: &a::Block) -> Block {
        self.push_scope();
        let mut stmts = Vec::new();
        for s in &b.stmts {
            stmts.push(self.check_stmt(s));
        }
        let tail = b.tail.as_ref().map(|t| Box::new(self.check_expr(t, None)));
        self.pop_scope();
        Block { stmts, tail }
    }

    fn check_block_or_expr(&mut self, e: &a::Expr) -> Block {
        let ce = self.check_expr(e, None);
        Block { stmts: vec![Stmt { kind: StmtKind::Expr(ce), span: e.span }], tail: None }
    }

    fn check_stmt(&mut self, s: &a::Stmt) -> Stmt {
        let kind = match &s.kind {
            a::StmtKind::Let { name, ty, init, .. } => {
                let declared = ty.as_ref().and_then(|t| self.resolve_type(t));
                if declared.is_none() && init.is_none() {
                    self.err(s.span, "cannot infer the type of this binding (add `: type` or an initializer)", None);
                }
                let init_e = match init {
                    Some(ie) => {
                        let ce = self.check_expr(ie, declared.as_ref());
                        Some(match &declared {
                            Some(t) => coerce(ce, t),
                            None => ce,
                        })
                    }
                    None => None,
                };
                let final_ty = declared
                    .clone()
                    .unwrap_or_else(|| init_e.as_ref().map(|e| e.ty.clone()).unwrap_or_else(Type::i64));
                let id = self.new_local(name, final_ty);
                StmtKind::Let { id, init: init_e }
            }
            a::StmtKind::Expr(e) => StmtKind::Expr(self.check_expr(e, None)),
            a::StmtKind::Return(e) => {
                let want = self.cur_ret.clone();
                let inner = e.as_ref().map(|ie| coerce(self.check_expr(ie, Some(&want)), &want));
                if want != Type::Unit && e.is_none() {
                    self.err(s.span, format!("expected a `{}` value from this function", want.display()), None);
                }
                StmtKind::Return(inner)
            }
            a::StmtKind::Break => StmtKind::Break,
            a::StmtKind::Continue => StmtKind::Continue,
        };
        Stmt { kind, span: s.span }
    }
}

fn ty_of_struct(name: &str) -> Type {
    Type::Struct(name.to_string())
}

pub fn check(module: &a::Module) -> (Program, Vec<CheckError>) {
    let mut errors: Vec<CheckError> = Vec::new();

    // 0) Struct names.
    let mut raw_structs: Vec<(String, Vec<(String, a::Ty)>)> = Vec::new();
    for item in &module.items {
        if let a::ItemKind::Struct(s) = &item.kind {
            raw_structs.push((
                s.name.clone(),
                s.fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect(),
            ));
        }
    }
    let struct_names: HashSet<String> = raw_structs.iter().map(|(n, _)| n.clone()).collect();

    // 1) Integer constants first (needed for array lengths + expression folding).
    let mut consts: HashMap<String, (i128, Type)> = HashMap::new();
    for item in &module.items {
        if let a::ItemKind::Const { name, ty, value } = &item.kind {
            let t = match ty {
                Some(t) => resolve_ty_ctx(t, &struct_names, &consts),
                None => Type::i64(),
            };
            match &value.kind {
                a::ExprKind::Lit(a::Lit::Int(text)) => match parse_int_lit(text) {
                    Some(v) => {
                        consts.insert(name.clone(), (v, t));
                    }
                    None => errors.push(CheckError {
                        span: item.span,
                        message: format!("constant `{name}` is out of range"),
                        help: None,
                    }),
                },
                _ => errors.push(CheckError {
                    span: item.span,
                    message: "only integer-literal constants are supported in this slice (const-fn evaluation comes later)".into(),
                    help: None,
                }),
            }
        }
    }

    // 2) Struct definitions (resolve field types, incl. const array lengths).
    let mut structs: HashMap<String, Vec<(String, Type)>> = HashMap::new();
    let mut struct_order: Vec<StructDef> = Vec::new();
    for (name, fields) in &raw_structs {
        let resolved = fields
            .iter()
            .map(|(fn_, ty)| (fn_.clone(), resolve_ty_ctx(ty, &struct_names, &consts)))
            .collect::<Vec<_>>();
        structs.insert(name.clone(), resolved.clone());
        struct_order.push(StructDef { name: name.clone(), fields: resolved });
    }

    // 3) Function signatures + static globals.
    let mut funcs: HashMap<String, (Vec<Type>, Type)> = HashMap::new();
    let mut globals: HashMap<String, Type> = HashMap::new();
    let mut global_inits: Vec<(String, Type, String)> = Vec::new();
    for item in &module.items {
        match &item.kind {
            a::ItemKind::Fn(f) => {
                let params = f
                    .params
                    .iter()
                    .map(|p| resolve_ty_ctx(&p.ty, &struct_names, &consts))
                    .collect::<Vec<_>>();
                let ret = f.ret.as_ref().map(|t| resolve_ty_ctx(t, &struct_names, &consts)).unwrap_or(Type::Unit);
                funcs.insert(f.name.clone(), (params, ret));
            }
            a::ItemKind::Static { name, ty, value } => {
                let t = resolve_ty_ctx(ty, &struct_names, &consts);
                match const_init(&t, value) {
                    Some(init) => {
                        globals.insert(name.clone(), t.clone());
                        global_inits.push((name.clone(), t, init));
                    }
                    None => errors.push(CheckError {
                        span: item.span,
                        message: "`static` initializer must be a compile-time constant (const-fn evaluation comes later)".into(),
                        help: None,
                    }),
                }
            }
            a::ItemKind::TypeAlias { .. } => {
                errors.push(CheckError { span: item.span, message: "type aliases are not supported yet".into(), help: None });
            }
            a::ItemKind::Import { .. } => {
                errors.push(CheckError { span: item.span, message: "`import` is not supported yet".into(), help: None });
            }
            a::ItemKind::Struct(_) | a::ItemKind::Enum(_) | a::ItemKind::Const { .. } => {}
        }
    }

    // 4) Check bodies.
    let mut out_funcs = Vec::new();
    for item in &module.items {
        if let a::ItemKind::Fn(f) = &item.kind {
            let (params, ret) = funcs.get(&f.name).cloned().unwrap();
            let mut c = Checker {
                funcs: funcs.clone(),
                consts: consts.clone(),
                structs: structs.clone(),
                globals: globals.clone(),
                cur_ret: ret.clone(),
                local_tys: Vec::new(),
                scopes: vec![HashMap::new()],
                errors: Vec::new(),
            };
            let arity = params.len();
            for (i, p) in params.iter().enumerate() {
                let id = c.local_tys.len();
                c.local_tys.push(p.clone());
                c.scopes.last_mut().unwrap().insert(f.params[i].name.clone(), id);
            }
            let body = c.check_block(&f.body);
            out_funcs.push(Func { name: f.name.clone(), ret, arity, local_tys: c.local_tys.clone(), body });
            errors.extend(c.errors);
        }
    }

    (Program { funcs: out_funcs, structs: struct_order, globals: global_inits }, errors)
}
