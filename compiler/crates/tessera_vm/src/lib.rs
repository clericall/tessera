//! Tessera VM — tree-walking interpreter for the typed IR.
//!
//! Loads a [`tessera_check::Program`] and executes it directly, with no C
//! backend and no clang. Intended for `tesserac run --interpret`.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::time::SystemTime;

use tessera_check::{Block, Expr, ExprKind, Func, HBin, HUn, Place, Program, Stmt, StmtKind, Type};

/// A runtime value.
#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Vec<u8>),
    /// Tagged with the struct name so field lookups can resolve indices.
    Struct { name: String, fields: Vec<Value> },
    Array(Vec<Value>),
}

impl Value {
    pub fn as_i64(&self) -> Result<i64, VmError> {
        match self {
            Value::Int(i) => Ok(*i),
            Value::Bool(b) => Ok(if *b { 1 } else { 0 }),
            Value::Float(f) => Ok(*f as i64),
            _ => Err(VmError::type_error("integer", self)),
        }
    }

    pub fn as_f64(&self) -> Result<f64, VmError> {
        match self {
            Value::Float(f) => Ok(*f),
            Value::Int(i) => Ok(*i as f64),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            _ => Err(VmError::type_error("float", self)),
        }
    }

    pub fn as_bool(&self) -> Result<bool, VmError> {
        match self {
            Value::Bool(b) => Ok(*b),
            Value::Int(i) => Ok(*i != 0),
            Value::Float(f) => Ok(*f != 0.0),
            _ => Err(VmError::type_error("boolean", self)),
        }
    }

    pub fn as_bytes(&self) -> Result<&[u8], VmError> {
        match self {
            Value::Str(b) => Ok(b),
            _ => Err(VmError::type_error("string", self)),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Str(_) => "str",
            Value::Struct { .. } => "struct",
            Value::Array(_) => "array",
        }
    }
}

/// Control flow within a block.
#[derive(Debug, Clone)]
pub enum Control {
    Normal(Option<Value>),
    Return(Value),
    Break,
    Continue,
}

/// VM error.
#[derive(Debug)]
pub enum VmError {
    Message(String),
    TypeError { expected: String, got: String },
    UnknownFunction(String),
    UnknownField { struct_name: String, field: String },
    IndexOutOfBounds { index: i64, len: usize },
    DivisionByZero,
    ModuloByZero,
    InvalidFileHandle(i64),
    FileError(String),
    IoError(String),
    ReturnOutsideFunction,
    BreakOutsideLoop,
    ContinueOutsideLoop,
}

impl VmError {
    fn type_error(expected: &str, got: &Value) -> Self {
        VmError::TypeError { expected: expected.to_string(), got: got.type_name().to_string() }
    }

    fn message(s: impl Into<String>) -> Self {
        VmError::Message(s.into())
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::Message(s) => write!(f, "{s}"),
            VmError::TypeError { expected, got } => write!(f, "type error: expected {expected}, got {got}"),
            VmError::UnknownFunction(name) => write!(f, "unknown function `{name}`"),
            VmError::UnknownField { struct_name, field } => {
                write!(f, "unknown field `.{field}` on struct `{struct_name}`")
            }
            VmError::IndexOutOfBounds { index, len } => {
                write!(f, "index {index} out of bounds for array of length {len}")
            }
            VmError::DivisionByZero => write!(f, "division by zero"),
            VmError::ModuloByZero => write!(f, "modulo by zero"),
            VmError::InvalidFileHandle(h) => write!(f, "invalid file handle {h}"),
            VmError::FileError(s) => write!(f, "file error: {s}"),
            VmError::IoError(s) => write!(f, "IO error: {s}"),
            VmError::ReturnOutsideFunction => write!(f, "return outside function"),
            VmError::BreakOutsideLoop => write!(f, "break outside loop"),
            VmError::ContinueOutsideLoop => write!(f, "continue outside loop"),
        }
    }
}

impl std::error::Error for VmError {}

/// The virtual machine.
pub struct Vm {
    funcs: HashMap<String, Func>,
    structs: HashMap<String, Vec<(String, Type)>>,
    globals: HashMap<String, Value>,
    locals: Vec<Value>,
    local_tys: Vec<Type>,
    files: HashMap<i64, File>,
    next_handle: i64,
    stdout: String,
}

impl Vm {
    /// Create a new VM with no program loaded.
    pub fn new() -> Self {
        Vm {
            funcs: HashMap::new(),
            structs: HashMap::new(),
            globals: HashMap::new(),
            locals: Vec::new(),
            local_tys: Vec::new(),
            files: HashMap::new(),
            next_handle: 1,
            stdout: String::new(),
        }
    }

    /// Load a type-checked program: register structs, functions, and zero-init
    /// all globals.
    pub fn load_program(&mut self, prog: &Program) {
        for s in &prog.structs {
            self.structs.insert(s.name.clone(), s.fields.clone());
        }
        for f in &prog.funcs {
            self.funcs.insert(f.name.clone(), f.clone());
        }
        for (name, ty, _init) in &prog.globals {
            self.globals.insert(name.clone(), zero_value(ty, &self.structs));
        }
    }

    /// Run the program: find `main`, call it with no arguments, return its i64
    /// result.
    pub fn run_main(&mut self) -> Result<i64, VmError> {
        let main = self
            .funcs
            .get("main")
            .cloned()
            .ok_or_else(|| VmError::message("no `main` function found"))?;
        let result = self.call_user_func(&main, &[])?;
        match result {
            Value::Int(i) => Ok(i),
            Value::Bool(b) => Ok(if b { 1 } else { 0 }),
            Value::Float(f) => Ok(f as i64),
            _ => Ok(0),
        }
    }

    /// Capture stdout so far.
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    // -----------------------------------------------------------------------
    // Expression evaluation — returns Control so `return`/`break`/`continue`
    // inside expression blocks (if/while/loop/for/block) propagate correctly.
    // -----------------------------------------------------------------------

    pub fn eval_expr(&mut self, e: &Expr) -> Result<Control, VmError> {
        match &e.kind {
            ExprKind::IntLit(v) => Ok(Control::Normal(Some(Value::Int(*v as i64)))),
            ExprKind::FloatLit(v) => Ok(Control::Normal(Some(Value::Float(*v)))),
            ExprKind::BoolLit(b) => Ok(Control::Normal(Some(Value::Bool(*b)))),
            ExprKind::Str(s) => Ok(Control::Normal(Some(Value::Str(s.as_bytes().to_vec())))),
            ExprKind::Var(id) => Ok(Control::Normal(Some(self.locals[*id].clone()))),
            ExprKind::Global(name) => Ok(Control::Normal(Some(
                self.globals
                    .get(name)
                    .cloned()
                    .ok_or_else(|| VmError::message(format!("unknown global `{name}`")))?,
            ))),
            ExprKind::Binary { op, lhs, rhs } => {
                let l = match self.eval_expr(lhs)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                };
                let r = match self.eval_expr(rhs)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                };
                Ok(Control::Normal(Some(self.eval_binop(op, l, r)?)))
            }
            ExprKind::Unary { op, expr } => {
                let v = match self.eval_expr(expr)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                };
                Ok(Control::Normal(Some(self.eval_unary(op, v)?)))
            }
            ExprKind::Assign { place, value } => {
                let v = match self.eval_expr(value)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                };
                self.set_place(place, v.clone())?;
                Ok(Control::Normal(Some(v)))
            }
            ExprKind::Call { func, args } => {
                let mut values = Vec::with_capacity(args.len());
                for a in args {
                    let v = match self.eval_expr(a)? {
                        Control::Normal(Some(v)) => v,
                        Control::Normal(None) => Value::Int(0),
                        c @ Control::Return(_) => return Ok(c),
                        Control::Break => return Ok(Control::Break),
                        Control::Continue => return Ok(Control::Continue),
                    };
                    values.push(v);
                }
                let result = self.call(func, &values)?;
                Ok(Control::Normal(Some(result)))
            }
            ExprKind::Field { base, name } => {
                let base = match self.eval_expr(base)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                };
                match base {
                    Value::Struct { name: sname, fields } => {
                        let def = self
                            .structs
                            .get(&sname)
                            .ok_or_else(|| VmError::message(format!("unknown struct `{sname}`")))?;
                        let idx = def
                            .iter()
                            .position(|(n, _)| n == name)
                            .ok_or_else(|| VmError::UnknownField {
                                struct_name: sname,
                                field: name.clone(),
                            })?;
                        Ok(Control::Normal(Some(fields[idx].clone())))
                    }
                    other => Err(VmError::type_error("struct", &other)),
                }
            }
            ExprKind::Index { base, index } => {
                let base = match self.eval_expr(base)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                };
                let idx = match self.eval_expr(index)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                }
                .as_i64()?;
                match base {
                    Value::Array(elements) => {
                        let i = usize_val(idx, elements.len())?;
                        Ok(Control::Normal(Some(elements[i].clone())))
                    }
                    other => Err(VmError::type_error("array", &other)),
                }
            }
            ExprKind::Cast { expr, ty } => {
                let v = match self.eval_expr(expr)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                };
                Ok(Control::Normal(Some(cast_value(v, ty))))
            }
            ExprKind::ArrayInit(items) => {
                let mut values = Vec::with_capacity(items.len());
                for it in items {
                    let v = match self.eval_expr(it)? {
                        Control::Normal(Some(v)) => v,
                        Control::Normal(None) => Value::Int(0),
                        c @ Control::Return(_) => return Ok(c),
                        Control::Break => return Ok(Control::Break),
                        Control::Continue => return Ok(Control::Continue),
                    };
                    values.push(v);
                }
                Ok(Control::Normal(Some(Value::Array(values))))
            }
            ExprKind::StructInit { name, fields } => {
                let def = self
                    .structs
                    .get(name)
                    .ok_or_else(|| VmError::message(format!("unknown struct `{name}`")))?;
                let field_idx: Vec<(usize, String)> = def
                    .iter()
                    .enumerate()
                    .map(|(i, (n, _))| (i, n.clone()))
                    .collect();
                let mut values = vec![Value::Int(0); def.len()];
                for (fname, expr) in fields {
                    let val = match self.eval_expr(expr)? {
                        Control::Normal(Some(v)) => v,
                        Control::Normal(None) => Value::Int(0),
                        c @ Control::Return(_) => return Ok(c),
                        Control::Break => return Ok(Control::Break),
                        Control::Continue => return Ok(Control::Continue),
                    };
                    let idx = field_idx
                        .iter()
                        .find(|(_, n)| n == fname)
                        .map(|(i, _)| *i)
                        .ok_or_else(|| VmError::UnknownField {
                            struct_name: name.clone(),
                            field: fname.clone(),
                        })?;
                    values[idx] = val;
                }
                Ok(Control::Normal(Some(Value::Struct {
                    name: name.clone(),
                    fields: values,
                })))
            }
            ExprKind::If { cond, then, els } => {
                let cond = match self.eval_expr(cond)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                };
                if cond.as_bool()? {
                    self.eval_block(then)
                } else if let Some(eb) = els {
                    self.eval_block(eb)
                } else {
                    Ok(Control::Normal(Some(Value::Int(0))))
                }
            }
            ExprKind::While { cond, body } => {
                loop {
                    let cond = match self.eval_expr(cond)? {
                        Control::Normal(Some(v)) => v,
                        Control::Normal(None) => Value::Int(0),
                        c @ Control::Return(_) => return Ok(c),
                        Control::Break => return Ok(Control::Break),
                        Control::Continue => return Ok(Control::Continue),
                    };
                    if !cond.as_bool()? {
                        break;
                    }
                    match self.eval_block(body)? {
                        Control::Normal(_) => {}
                        Control::Break => break,
                        Control::Continue => {}
                        Control::Return(v) => return Ok(Control::Return(v)),
                    }
                }
                Ok(Control::Normal(Some(Value::Int(0))))
            }
            ExprKind::Loop(body) => {
                loop {
                    match self.eval_block(body)? {
                        Control::Normal(_) => {}
                        Control::Break => break,
                        Control::Continue => {}
                        Control::Return(v) => return Ok(Control::Return(v)),
                    }
                }
                Ok(Control::Normal(Some(Value::Int(0))))
            }
            ExprKind::For { id, lo, hi, body } => {
                let start = match self.eval_expr(lo)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                }
                .as_i64()?;
                let end = match self.eval_expr(hi)? {
                    Control::Normal(Some(v)) => v,
                    Control::Normal(None) => Value::Int(0),
                    c @ Control::Return(_) => return Ok(c),
                    Control::Break => return Ok(Control::Break),
                    Control::Continue => return Ok(Control::Continue),
                }
                .as_i64()?;
                let mut i = start;
                while i < end {
                    self.locals[*id] = Value::Int(i);
                    match self.eval_block(body)? {
                        Control::Break => break,
                        Control::Continue => {
                            i += 1;
                            continue;
                        }
                        Control::Return(v) => return Ok(Control::Return(v)),
                        Control::Normal(_) => {}
                    }
                    i += 1;
                }
                Ok(Control::Normal(Some(Value::Int(0))))
            }
            ExprKind::Block(block) => self.eval_block(block),
            ExprKind::Unit => Ok(Control::Normal(Some(Value::Int(0)))),
        }
    }

    // -----------------------------------------------------------------------
    // Expression evaluation — binary / unary operators
    // -----------------------------------------------------------------------

    /// Evaluate a binary operator.
    fn eval_binop(&self, op: &HBin, l: Value, r: Value) -> Result<Value, VmError> {
        use HBin::*;
        match op {
            Add => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(*b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                _ => Err(VmError::type_error("numeric", &l)),
            },
            Sub => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(*b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                _ => Err(VmError::type_error("numeric", &l)),
            },
            Mul => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(*b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                _ => Err(VmError::type_error("numeric", &l)),
            },
            Div => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => {
                    if *b == 0 {
                        return Err(VmError::DivisionByZero);
                    }
                    Ok(Value::Int(a.wrapping_div(*b)))
                }
                (Value::Float(a), Value::Float(b)) => {
                    if *b == 0.0 {
                        return Err(VmError::DivisionByZero);
                    }
                    Ok(Value::Float(a / b))
                }
                _ => Err(VmError::type_error("numeric", &l)),
            },
            Rem => {
                let a = l.as_i64()?;
                let b = r.as_i64()?;
                if b == 0 {
                    return Err(VmError::ModuloByZero);
                }
                Ok(Value::Int(a.wrapping_rem(b)))
            }
            Shl => {
                let a = l.as_i64()?;
                let b = r.as_i64()?;
                Ok(Value::Int(a.wrapping_shl(b as u32)))
            }
            Shr => {
                let a = l.as_i64()?;
                let b = r.as_i64()?;
                Ok(Value::Int(a.wrapping_shr(b as u32)))
            }
            BitAnd => {
                let a = l.as_i64()?;
                let b = r.as_i64()?;
                Ok(Value::Int(a & b))
            }
            BitOr => {
                let a = l.as_i64()?;
                let b = r.as_i64()?;
                Ok(Value::Int(a | b))
            }
            BitXor => {
                let a = l.as_i64()?;
                let b = r.as_i64()?;
                Ok(Value::Int(a ^ b))
            }
            Eq => Ok(Value::Bool(value_eq(&l, &r))),
            Ne => Ok(Value::Bool(!value_eq(&l, &r))),
            Lt => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
                _ => Err(VmError::type_error("comparable", &l)),
            },
            Le => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
                _ => Err(VmError::type_error("comparable", &l)),
            },
            Gt => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
                _ => Err(VmError::type_error("comparable", &l)),
            },
            Ge => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
                _ => Err(VmError::type_error("comparable", &l)),
            },
            And => Ok(Value::Bool(l.as_bool()? && r.as_bool()?)),
            Or => Ok(Value::Bool(l.as_bool()? || r.as_bool()?)),
        }
    }

    /// Evaluate a unary operator.
    fn eval_unary(&self, op: &HUn, v: Value) -> Result<Value, VmError> {
        use HUn::*;
        match op {
            Neg => match v {
                Value::Int(i) => Ok(Value::Int(i.wrapping_neg())),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err(VmError::type_error("numeric", &v)),
            },
            Not => Ok(Value::Bool(!v.as_bool()?)),
            BitNot => {
                let i = v.as_i64()?;
                Ok(Value::Int(!i))
            }
        }
    }

    // -----------------------------------------------------------------------
    // Place (lvalue) evaluation
    // -----------------------------------------------------------------------

    fn eval_place(&mut self, p: &Place) -> Result<Value, VmError> {
        match p {
            Place::Var(id) => Ok(self.locals[*id].clone()),
            Place::Global(name) => Ok(self
                .globals
                .get(name)
                .cloned()
                .ok_or_else(|| VmError::message(format!("unknown global `{name}`")))?),
            Place::Field(inner, name) => {
                let base = self.eval_place(inner)?;
                match base {
                    Value::Struct { name: sname, fields } => {
                        let def = self
                            .structs
                            .get(&sname)
                            .ok_or_else(|| VmError::message(format!("unknown struct `{sname}`")))?;
                        let idx = def
                            .iter()
                            .position(|(n, _)| n == name)
                            .ok_or_else(|| VmError::UnknownField {
                                struct_name: sname,
                                field: name.clone(),
                            })?;
                        Ok(fields[idx].clone())
                    }
                    other => Err(VmError::type_error("struct", &other)),
                }
            }
            Place::Index(inner, idx_expr) => {
                let idx = match self.eval_expr(idx_expr)? {
                    Control::Normal(Some(v)) => v,
                    _ => Value::Int(0),
                }
                .as_i64()?;
                let base = self.eval_place(inner)?;
                match base {
                    Value::Array(elements) => {
                        let i = usize_val(idx, elements.len())?;
                        Ok(elements[i].clone())
                    }
                    other => Err(VmError::type_error("array", &other)),
                }
            }
        }
    }

    fn set_place(&mut self, p: &Place, v: Value) -> Result<(), VmError> {
        match p {
            Place::Var(id) => {
                self.locals[*id] = v;
                Ok(())
            }
            Place::Global(name) => {
                self.globals.insert(name.clone(), v);
                Ok(())
            }
            Place::Field(inner, field_name) => {
                let base = self.eval_place(inner)?;
                let mut new_base = base;
                match &mut new_base {
                    Value::Struct { fields, .. } => {
                        let sname = self.place_struct_name(inner)?;
                        let def = self.structs.get(&sname).ok_or_else(|| {
                            VmError::message(format!("unknown struct `{sname}`"))
                        })?;
                        let idx = def
                            .iter()
                            .position(|(n, _)| n == field_name)
                            .ok_or_else(|| VmError::UnknownField {
                                struct_name: sname,
                                field: field_name.clone(),
                            })?;
                        fields[idx] = v;
                    }
                    other => return Err(VmError::type_error("struct", &other)),
                }
                self.set_place(inner, new_base)
            }
            Place::Index(inner, idx_expr) => {
                let base = self.eval_place(inner)?;
                let idx = match self.eval_expr(idx_expr)? {
                    Control::Normal(Some(v)) => v,
                    _ => Value::Int(0),
                }
                .as_i64()?;
                let mut new_base = base;
                match &mut new_base {
                    Value::Array(elements) => {
                        let i = usize_val(idx, elements.len())?;
                        elements[i] = v;
                    }
                    other => return Err(VmError::type_error("array", &other)),
                }
                self.set_place(inner, new_base)
            }
        }
    }

    /// Given a place, determine the struct name it resolves to (for Field
    /// mutation).
    fn place_struct_name(&self, p: &Place) -> Result<String, VmError> {
        match p {
            Place::Var(id) => match &self.locals[*id] {
                Value::Struct { name, .. } => Ok(name.clone()),
                other => Err(VmError::type_error("struct", other)),
            },
            Place::Global(name) => match &self.globals[name] {
                Value::Struct { name: sname, .. } => Ok(sname.clone()),
                other => Err(VmError::type_error("struct", other)),
            },
            Place::Field(inner, _) => self.place_struct_name(inner),
            Place::Index(inner, _) => self.place_struct_name(inner),
        }
    }

    // -----------------------------------------------------------------------
    // Statement / block evaluation
    // -----------------------------------------------------------------------

    fn eval_block(&mut self, block: &Block) -> Result<Control, VmError> {
        let mut tail: Option<Value> = None;
        for stmt in &block.stmts {
            match self.eval_stmt(stmt)? {
                Control::Normal(Some(v)) => tail = Some(v),
                Control::Normal(None) => {}
                c @ Control::Return(_) => return Ok(c),
                Control::Break => return Ok(Control::Break),
                Control::Continue => return Ok(Control::Continue),
            }
        }
        match &block.tail {
            Some(e) => match self.eval_expr(e)? {
                Control::Normal(Some(v)) => Ok(Control::Normal(Some(v))),
                Control::Normal(None) => Ok(Control::Normal(tail)),
                c @ Control::Return(_) => Ok(c),
                Control::Break => Ok(Control::Break),
                Control::Continue => Ok(Control::Continue),
            },
            None => Ok(Control::Normal(tail)),
        }
    }

    fn eval_stmt(&mut self, s: &Stmt) -> Result<Control, VmError> {
        match &s.kind {
            StmtKind::Let { id, init } => {
                let v = match init {
                    Some(e) => match self.eval_expr(e)? {
                        Control::Normal(Some(v)) => v,
                        Control::Normal(None) => Value::Int(0),
                        c @ Control::Return(_) => return Ok(c),
                        Control::Break => return Ok(Control::Break),
                        Control::Continue => return Ok(Control::Continue),
                    },
                    None => zero_value_for_type(&self.local_tys[*id], &self.structs),
                };
                self.locals[*id] = v;
                Ok(Control::Normal(None))
            }
            StmtKind::Expr(e) => self.eval_expr(e),
            StmtKind::Return(e) => {
                let v = match e {
                    Some(e) => match self.eval_expr(e)? {
                        Control::Normal(Some(v)) => v,
                        Control::Normal(None) => Value::Int(0),
                        c @ Control::Return(_) => return Ok(c),
                        Control::Break => return Ok(Control::Break),
                        Control::Continue => return Ok(Control::Continue),
                    },
                    None => Value::Int(0),
                };
                Ok(Control::Return(v))
            }
            StmtKind::Break => Ok(Control::Break),
            StmtKind::Continue => Ok(Control::Continue),
        }
    }

    // -----------------------------------------------------------------------
    // Function calls
    // -----------------------------------------------------------------------

    fn call(&mut self, func: &str, args: &[Value]) -> Result<Value, VmError> {
        // Try builtins first
        if let Some(v) = self.call_builtin(func, args)? {
            return Ok(v);
        }
        // Then user functions
        let f = self
            .funcs
            .get(func)
            .cloned()
            .ok_or_else(|| VmError::UnknownFunction(func.to_string()))?;
        self.call_user_func(&f, args)
    }

    fn call_user_func(&mut self, f: &Func, args: &[Value]) -> Result<Value, VmError> {
        if args.len() != f.arity {
            return Err(VmError::message(format!(
                "function `{}` expects {} argument(s), found {}",
                f.name,
                f.arity,
                args.len()
            )));
        }
        // Save current locals, set up new frame
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_local_tys = std::mem::take(&mut self.local_tys);
        // Parameters fill indices 0..arity, rest are zero-initialized
        let mut locals: Vec<Value> = args.to_vec();
        locals.resize(f.local_tys.len(), Value::Int(0));
        self.locals = locals;
        self.local_tys = f.local_tys.clone();

        // Zero-initialize locals that weren't parameters
        for i in f.arity..f.local_tys.len() {
            self.locals[i] = zero_value_for_type(&self.local_tys[i], &self.structs);
        }

        let result = self.eval_block(&f.body);

        // Restore locals
        self.locals = saved_locals;
        self.local_tys = saved_local_tys;

        match result? {
            Control::Normal(Some(v)) => Ok(v),
            Control::Normal(None) => Ok(Value::Int(0)),
            Control::Return(v) => Ok(v),
            Control::Break => Err(VmError::BreakOutsideLoop),
            Control::Continue => Err(VmError::ContinueOutsideLoop),
        }
    }

    // -----------------------------------------------------------------------
    // Builtins
    // -----------------------------------------------------------------------

    fn call_builtin(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>, VmError> {
        match name {
            "print_i64" | "print_uint" => {
                let a = args.get(0).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                self.stdout.push_str(&a.to_string());
                Ok(Some(Value::Int(0)))
            }
            "print_bool" => {
                let a = args.get(0).cloned().unwrap_or(Value::Bool(false)).as_bool()?;
                let s = if a { "true" } else { "false" };
                self.stdout.push_str(s);
                Ok(Some(Value::Int(0)))
            }
            "print_str" => {
                let s = args.get(0).cloned().unwrap_or(Value::Str(Vec::new()));
                let bytes = s.as_bytes()?;
                let s = String::from_utf8_lossy(bytes);
                self.stdout.push_str(&s);
                Ok(Some(Value::Int(0)))
            }
            "putstr" => {
                let s = args.get(0).cloned().unwrap_or(Value::Str(Vec::new()));
                let bytes = s.as_bytes()?;
                let s = String::from_utf8_lossy(bytes);
                self.stdout.push_str(&s);
                Ok(Some(Value::Int(0)))
            }
            "putc" => {
                let a = args.get(0).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                let ch = (a as u8) as char;
                self.stdout.push(ch);
                Ok(Some(Value::Int(0)))
            }
            "putint" => {
                let a = args.get(0).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                self.stdout.push_str(&a.to_string());
                Ok(Some(Value::Int(0)))
            }
            "flush" => {
                std::io::stdout().flush().map_err(|e| VmError::IoError(e.to_string()))?;
                Ok(Some(Value::Int(0)))
            }
            "now_ms" => {
                let ms = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;
                Ok(Some(Value::Int(ms)))
            }
            "fopen" => {
                let path_val = args.get(0).cloned().unwrap_or(Value::Str(Vec::new()));
                let bytes = path_val.as_bytes()?;
                let path = String::from_utf8_lossy(bytes);
                match File::open(path.as_ref()) {
                    Ok(f) => {
                        let h = self.next_handle;
                        self.next_handle += 1;
                        self.files.insert(h, f);
                        Ok(Some(Value::Int(h)))
                    }
                    Err(_) => Ok(Some(Value::Int(-1))),
                }
            }
            "fread_u8" => {
                let h = args.get(0).cloned().unwrap_or(Value::Int(-1)).as_i64()?;
                let f = self.files.get_mut(&h).ok_or_else(|| VmError::InvalidFileHandle(h))?;
                let mut buf = [0u8; 1];
                match f.read(&mut buf) {
                    Ok(1) => Ok(Some(Value::Int(buf[0] as i64))),
                    Ok(_) => Ok(Some(Value::Int(-1))),
                    Err(e) => Err(VmError::FileError(e.to_string())),
                }
            }
            "fread_i16" => {
                let h = args.get(0).cloned().unwrap_or(Value::Int(-1)).as_i64()?;
                let f = self.files.get_mut(&h).ok_or_else(|| VmError::InvalidFileHandle(h))?;
                let mut buf = [0u8; 2];
                match f.read(&mut buf) {
                    Ok(2) => {
                        let u = i16::from_le_bytes(buf) as i64;
                        Ok(Some(Value::Int(u)))
                    }
                    Ok(_) => Ok(Some(Value::Int(-1))),
                    Err(e) => Err(VmError::FileError(e.to_string())),
                }
            }
            "fread_i32" => {
                let h = args.get(0).cloned().unwrap_or(Value::Int(-1)).as_i64()?;
                let f = self.files.get_mut(&h).ok_or_else(|| VmError::InvalidFileHandle(h))?;
                let mut buf = [0u8; 4];
                match f.read(&mut buf) {
                    Ok(4) => {
                        let u = i32::from_le_bytes(buf) as i64;
                        Ok(Some(Value::Int(u)))
                    }
                    Ok(_) => Ok(Some(Value::Int(-1))),
                    Err(e) => Err(VmError::FileError(e.to_string())),
                }
            }
            "feof" => {
                let _h = args.get(0).cloned().unwrap_or(Value::Int(-1)).as_i64()?;
                Ok(Some(Value::Bool(self.files.contains_key(&_h))))
            }
            "fclose" => {
                let h = args.get(0).cloned().unwrap_or(Value::Int(-1)).as_i64()?;
                if self.files.remove(&h).is_some() {
                    Ok(Some(Value::Int(0)))
                } else {
                    Ok(Some(Value::Int(-1)))
                }
            }
            "len" => {
                let s = args.get(0).cloned().unwrap_or(Value::Str(Vec::new()));
                Ok(Some(Value::Int(s.as_bytes()?.len() as i64)))
            }
            "concat" => {
                let a = args.get(0).cloned().unwrap_or(Value::Str(Vec::new()));
                let b = args.get(1).cloned().unwrap_or(Value::Str(Vec::new()));
                let mut bytes = a.as_bytes()?.to_vec();
                bytes.extend_from_slice(b.as_bytes()?);
                Ok(Some(Value::Str(bytes)))
            }
            "substr" => {
                let s = args.get(0).cloned().unwrap_or(Value::Str(Vec::new()));
                let bytes = s.as_bytes()?;
                let from = args.get(1).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                let n = args.get(2).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                let from = from.clamp(0, bytes.len() as i64) as usize;
                let n = n.clamp(0, (bytes.len() - from) as i64) as usize;
                Ok(Some(Value::Str(bytes[from..from + n].to_vec())))
            }
            "read_line" => {
                let mut line = String::new();
                std::io::stdin()
                    .read_line(&mut line)
                    .map_err(|e| VmError::IoError(e.to_string()))?;
                if line.ends_with('\n') {
                    line.pop();
                }
                if line.ends_with('\r') {
                    line.pop();
                }
                Ok(Some(Value::Str(line.into_bytes())))
            }
            "popcount" => {
                let a = args.get(0).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                Ok(Some(Value::Int(a.count_ones() as i64)))
            }
            "ctz" => {
                let a = args.get(0).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                Ok(Some(Value::Int(if a == 0 {
                    64
                } else {
                    a.trailing_zeros() as i64
                })))
            }
            "clz" => {
                let a = args.get(0).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                Ok(Some(Value::Int(if a == 0 {
                    64
                } else {
                    a.leading_zeros() as i64
                })))
            }
            "btable_index" => {
                let occ = args.get(0).cloned().unwrap_or(Value::Int(0)).as_i64()? as u64;
                let mask = args.get(1).cloned().unwrap_or(Value::Int(0)).as_i64()? as u64;
                let shift = args.get(2).cloned().unwrap_or(Value::Int(0)).as_i64()?;
                let masked = occ & mask;
                let index = masked.count_ones() as i64;
                Ok(Some(Value::Int(index << shift)))
            }
            _ => Ok(None),
        }
    }
}

/// Zero-initialized value for a type.
fn zero_value(ty: &Type, structs: &HashMap<String, Vec<(String, Type)>>) -> Value {
    match ty {
        Type::Int { .. } => Value::Int(0),
        Type::F32 | Type::F64 => Value::Float(0.0),
        Type::Bool => Value::Bool(false),
        Type::Str => Value::Str(Vec::new()),
        Type::Unit => Value::Int(0),
        Type::Array { elem, len } => Value::Array(vec![zero_value(elem, structs); *len]),
        Type::Struct(name) => {
            let def = structs.get(name).cloned().unwrap_or_default();
            Value::Struct {
                name: name.clone(),
                fields: def.iter().map(|(_, t)| zero_value(t, structs)).collect(),
            }
        }
    }
}

/// Zero-initialized value for a local type (used when Let has no init).
fn zero_value_for_type(ty: &Type, structs: &HashMap<String, Vec<(String, Type)>>) -> Value {
    zero_value(ty, structs)
}

/// Cast a value to the target type.
fn cast_value(v: Value, target: &Type) -> Value {
    match (v, target) {
        (Value::Int(i), Type::F64) => Value::Float(i as f64),
        (Value::Int(i), Type::F32) => Value::Float((i as f32) as f64),
        (Value::Float(f), Type::Int { .. }) => Value::Int(f as i64),
        (Value::Float(f), Type::F32) => Value::Float((f as f32) as f64),
        (Value::Float(f), Type::F64) => Value::Float(f),
        (Value::Int(i), Type::Bool) => Value::Bool(i != 0),
        (Value::Bool(b), Type::Int { .. }) => Value::Int(if b { 1 } else { 0 }),
        (Value::Bool(b), Type::F64) => Value::Float(if b { 1.0 } else { 0.0 }),
        (Value::Int(i), Type::Int { bits, .. }) => match bits {
            8 => Value::Int(i as i8 as i64),
            16 => Value::Int(i as i16 as i64),
            32 => Value::Int(i as i32 as i64),
            _ => Value::Int(i),
        },
        (v, _) => v,
    }
}

/// Compare two values for equality.
fn value_eq(l: &Value, r: &Value) -> bool {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Str(a), Value::Str(b)) => a == b,
        _ => false,
    }
}

/// Convert an i64 index to a usize, bounds-checked.
fn usize_val(idx: i64, len: usize) -> Result<usize, VmError> {
    if idx < 0 || idx >= len as i64 {
        Err(VmError::IndexOutOfBounds {
            index: idx,
            len,
        })
    } else {
        Ok(idx as usize)
    }
}
