//! Tessera C backend: typed IR (tessera_check) -> readable C.
//!
//! Structs become `typedef struct { ... } Tr_<Name>;`; user functions become
//! `tr_<name>`; a `main` wrapper is emitted. Arrays are C arrays embedded in
//! structs / compound literals. Bit builtins map to `__builtin_*`.

use tessera_check::{Expr, ExprKind, Func, HUn, Place, Program, Stmt, StmtKind, StructDef, Type};

const RUNTIME: &str = r#"
/* --- tessera runtime --- */
static void tr_print_i64(int64_t x){ printf("%lld\n", (long long)x); }
static void tr_print_u64(uint64_t x){ printf("%llu\n", (unsigned long long)x); }
static void tr_print_bool(bool b){ fputs(b ? "true\n" : "false\n", stdout); }
static void tr_print_str(const char* s){ fputs(s, stdout); fputc('\n', stdout); }
typedef struct { const char* p; int64_t n; } Tr_str;
static Tr_str tr_sl(const char* p, int64_t n){ Tr_str r; r.p=p; r.n=n; return r; }
static Tr_str tr_read_line(void){
    static char buf[4096];
    if (!fgets(buf, sizeof buf, stdin)) return tr_sl("", 0);
    int64_t L = 0; while (buf[L] != 0) L++;
    if (L > 0 && buf[L-1] == '\n') { buf[L-1] = 0; L--; }
    if (L > 0 && buf[L-1] == '\r') { buf[L-1] = 0; L--; }
    return tr_sl(buf, L);
}
static void tr_put_i64(int64_t x){ char b[32]; sprintf(b, "%lld", (long long)x); fputs(b, stdout); }
static int64_t tr_now_ms(void){ return (int64_t)clock(); } /* ms on CLOCKS_PER_SEC==1000 targets */
/* --- file I/O (sequential binary reads; NNUE weight files) --- */
static int64_t tr_fopen(Tr_str s){
    char buf[4096];
    int64_t n = s.n; if (n > 4095) n = 4095; if (n < 0) n = 0;
    for (int64_t i = 0; i < n; i++) buf[i] = s.p[i];
    buf[n] = 0;
    FILE* f = fopen(buf, "rb");
    return f ? (int64_t)(intptr_t)f : -1;
}
static Tr_str tr_substr(Tr_str s, int64_t from, int64_t n){
    Tr_str r;
    if (from < 0) from = 0; if (from > s.n) from = s.n;
    if (n < 0) n = 0; if (from + n > s.n) n = s.n - from;
    r.p = s.p + from; r.n = n;
    return r;
}
static int64_t tr_str_len(Tr_str s){ return s.n; }
static Tr_str tr_str_concat(Tr_str a, Tr_str b){
    char* buf = (char*)malloc((size_t)(a.n + b.n + 1));
    if (!buf) return tr_sl("", 0);
    for (int64_t i = 0; i < a.n; i++) buf[i] = a.p[i];
    for (int64_t i = 0; i < b.n; i++) buf[a.n + i] = b.p[i];
    buf[a.n + b.n] = 0;
    Tr_str r; r.p = buf; r.n = a.n + b.n; return r;
}
static int64_t tr_fread_u8(int64_t h){
    if (h < 0) return -1;
    unsigned char b;
    if (fread(&b, 1, 1, (FILE*)(intptr_t)h) != 1) return -1;
    return (int64_t)b;
}
static int64_t tr_fread_i16(int64_t h){
    if (h < 0) return -1;
    unsigned char b[2];
    if (fread(b, 1, 2, (FILE*)(intptr_t)h) != 2) return -1;
    uint16_t u = (uint16_t)((uint16_t)b[0] | ((uint16_t)b[1] << 8));
    return (int64_t)(int16_t)u;
}
static int64_t tr_fread_i32(int64_t h){
    if (h < 0) return -1;
    unsigned char b[4];
    if (fread(b, 1, 4, (FILE*)(intptr_t)h) != 4) return -1;
    uint32_t u = (uint32_t)b[0] | ((uint32_t)b[1] << 8) | ((uint32_t)b[2] << 16) | ((uint32_t)b[3] << 24);
    return (int64_t)(int32_t)u;
}
static bool tr_feof(int64_t h){
    if (h < 0) return true;
    return feof((FILE*)(intptr_t)h) != 0;
}
static void tr_arr_assign(void* dst, const void* src, int64_t bytes){
    __builtin_memcpy(dst, src, (unsigned long)bytes);
}

static int64_t tr_fclose(int64_t h){
    if (h < 0) return -1;
    return (int64_t)fclose((FILE*)(intptr_t)h);
}
/* Bitboard occupancy-index selection: hardware pext under -mbmi2 (Tier C), plain shift otherwise. */
#ifdef __BMI2__
static inline int64_t tr_bbidx(uint64_t occ, uint64_t mask, int64_t shift){ return (int64_t)__builtin_ia32_pext_di(occ, mask); }
#else
static inline int64_t tr_bbidx(uint64_t occ, uint64_t mask, int64_t shift){
    uint64_t m = mask; int64_t idx = 0, i = 0;
    while (m) { int b = __builtin_ctzll(m); m &= m - 1; if (occ & (1ULL << b)) idx |= (1LL << i); i++; }
    return idx;
}
#endif
"#;

fn is_control(k: &ExprKind) -> bool {
    matches!(
        k,
        ExprKind::If { .. } | ExprKind::While { .. } | ExprKind::Loop(_) | ExprKind::For { .. } | ExprKind::Block(_)
    )
}

fn c_escape(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            _ => o.push(c),
        }
    }
    o.push('"');
    o
}

fn zero_init_c(t: &Type) -> String {
    match t {
        Type::Bool => "false".into(),
        Type::Int { .. } => "0".into(),
        Type::F32 | Type::F64 => "0.0".into(),
        Type::Unit => "".into(),
        Type::Array { elem, len } => format!("({}[{}]){{0}}", elem.c_ty(), len),
        Type::Struct(n) => format!("(Tr_{n}){{0}}"),
        Type::Str => "tr_sl(\"\", 0)".into(),
    }
}

fn params_sig(f: &Func) -> String {
    (0..f.arity)
        .map(|i| {
            let (base, brackets) = decl_parts(&f.local_tys[i]);
            format!("{} v{}{}", base, i, brackets)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn ind(s: &mut String, n: usize) {
    for _ in 0..n {
        s.push_str("  ");
    }
}

fn emit_struct_def(s: &mut String, def: &StructDef) {
    s.push_str(&format!("typedef struct {{\n"));
    for (name, ty) in &def.fields {
        match ty {
            Type::Array { elem, len } => {
                s.push_str(&format!("  {} {}[{}];\n", elem.c_ty(), name, len));
            }
            _ => {
                s.push_str(&format!("  {} {};\n", ty.c_ty(), name));
            }
        }
    }
    s.push_str(&format!("}} Tr_{};\n", def.name));
}

fn emit_place(p: &Place) -> String {
    match p {
        Place::Var(id) => format!("v{id}"),
        Place::Global(name) => format!("tr_g_{name}"),
        Place::Field(inner, name) => format!("({}).{}", emit_place(inner), name),
        Place::Index(inner, idx) => format!("({})[{}]", emit_place(inner), emit_expr(idx)),
    }
}

fn emit_expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::IntLit(v) => {
            if let Type::Int { signed: false, .. } = e.ty {
                format!("(({}){}ULL)", e.ty.c_ty(), v)
            } else {
                format!("(({}){}LL)", e.ty.c_ty(), v)
            }
        }
        ExprKind::FloatLit(v) => {
            // Emit with enough precision to round-trip; `f` suffix for f32.
            match e.ty {
                Type::F32 => format!("({}){}f", e.ty.c_ty(), v),
                Type::F64 => format!("({}){}", e.ty.c_ty(), v),
                _ => format!("({}){}", e.ty.c_ty(), v),
            }
        }
        ExprKind::BoolLit(b) => if *b { "true".into() } else { "false".into() },
        ExprKind::Str(s) => format!("tr_sl({}, {})", c_escape(s), s.as_bytes().len()),
        ExprKind::Var(id) => format!("v{id}"),
        ExprKind::Global(name) => format!("tr_g_{name}"),
        ExprKind::Binary { op, lhs, rhs } => format!("({} {} {})", emit_expr(lhs), op.c(), emit_expr(rhs)),
        ExprKind::Unary { op, expr } => match op {
            HUn::Neg => format!("(-{})", emit_expr(expr)),
            HUn::Not => format!("(!{})", emit_expr(expr)),
            HUn::BitNot => format!("(~{})", emit_expr(expr)),
        },
        ExprKind::Assign { place, value } => {
            // C cannot assign arrays, so `a = b` / `a = [..]` needs a memcpy.
            if let Type::Array { elem, len } = &e.ty {
                format!(
                    "tr_arr_assign(&{}, {}, {} * sizeof({}))",
                    emit_place(place),
                    emit_expr(value),
                    len,
                    elem.c_ty()
                )
            } else {
                format!("({} = {})", emit_place(place), emit_expr(value))
            }
        }
        ExprKind::Field { base, name } => format!("({}).{}", emit_expr(base), name),
        ExprKind::Cast { expr, ty } => format!("(({})({}))", ty.c_ty(), emit_expr(expr)),
        ExprKind::Index { base, index } => {
            if let Type::Str = base.ty {
                format!("(({}).p[({})])", emit_expr(base), emit_expr(index))
            } else {
                format!("({})[{}]", emit_expr(base), emit_expr(index))
            }
        }
        ExprKind::ArrayInit(items) => {
            if let Type::Array { elem, len } = &e.ty {
                // Cast every element to the element type: a C compound literal of
                // int16_t cannot be initialised from int64_t expressions ("array
                // type is not assignable"), which is what `[i16; 3] = [1, 2, 3]`
                // produced.
                let cty = elem.c_ty();
                let elems = items
                    .iter()
                    .map(|it| format!("(({})({}))", cty, emit_expr(it)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({}[{}]){{ {} }}", cty, len, elems)
            } else {
                "{0}".into()
            }
        }
        ExprKind::StructInit { name, fields } => {
            let inits = fields
                .iter()
                .map(|(f, e)| {
                    // Array members must use brace-init `{ ... }`, not a compound
                    // literal, to be a valid C initializer.
                    let val = match &e.kind {
                        ExprKind::ArrayInit(items) => {
                            let elems = items
                                .iter()
                                .map(|it| format!("({})", emit_expr(it)))
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("{{ {} }}", elems)
                        }
                        _ => emit_expr(e),
                    };
                    format!(".{} = {}", f, val)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("(Tr_{}){{ {} }}", name, inits)
        }
        ExprKind::Call { func, args } => emit_call(func, args),
        ExprKind::Block(b) => {
            let mut sub = String::new();
            emit_block(&mut sub, b, 1);
            let tail = match &b.tail {
                Some(t) => format!("{};", emit_expr(t)),
                None => "0;".into(),
            };
            format!("({{\n{}{}\n}})", sub, tail)
        }
        _ => {
            let mut sub = String::new();
            emit_control(&mut sub, e, 1);
            format!("({{\n{}\n0;\n}})", sub)
        }
    }
}

fn emit_call(func: &str, args: &[Expr]) -> String {
    let a0 = || args.get(0).map(emit_expr).unwrap_or_else(|| "0".into());
    let a1 = || args.get(1).map(emit_expr).unwrap_or_else(|| "0".into());
    let a2 = || args.get(2).map(emit_expr).unwrap_or_else(|| "0".into());
    match func {
        "popcount" | "ctz" | "clz" => {
            let bi = match func {
                "popcount" => "popcountll",
                "ctz" => "ctzll",
                _ => "clzll",
            };
            format!("__builtin_{bi}((unsigned long long)({}))", a0())
        }
        "print_i64" => format!("(tr_print_i64((int64_t)({})))", a0()),
        "print_uint" => format!("(tr_print_u64((uint64_t)({})))", a0()),
        "print_bool" => format!("(tr_print_bool((bool)({})))", a0()),
        "print_str" => {
            let s = match args.get(0).map(|a| &a.kind) {
                Some(ExprKind::Str(s)) => c_escape(s),
                _ => "\"\"".into(),
            };
            format!("(tr_print_str({s}))")
        }
        "putstr" => format!("(fputs(({}).p, stdout))", a0()),
        "putc" => format!("(putchar((int)({})))", a0()),
        "putint" => format!("(tr_put_i64((int64_t)({})))", a0()),
        "now_ms" => "(tr_now_ms())".into(),
        "fopen" => format!("(tr_fopen({}))", a0()),
        "substr" => format!("(tr_substr({}, {}, {}))", a0(), a1(), a2()),
        "fread_u8" => format!("(tr_fread_u8((int64_t)({})))", a0()),
        "fread_i16" => format!("(tr_fread_i16((int64_t)({})))", a0()),
        "fread_i32" => format!("(tr_fread_i32((int64_t)({})))", a0()),
        "feof" => format!("(tr_feof((int64_t)({})))", a0()),
        "fclose" => format!("(tr_fclose((int64_t)({})))", a0()),
        "flush" => "(fflush(stdout))".into(),
        "btable_index" => format!("(tr_bbidx((uint64_t)({}), (uint64_t)({}), ({})))", a0(), a1(), a2()),
        "len" => format!("(({}).n)", a0()),
        "concat" => format!("(tr_str_concat({}, {}))", a0(), a1()),
        "read_line" => "(tr_read_line())".into(),
        _ => {
            let rendered: Vec<String> = args.iter().map(emit_expr).collect();
            format!("(tr_{func}({}))", rendered.join(", "))
        }
    }
}

fn emit_control(s: &mut String, e: &Expr, level: usize) {
    match &e.kind {
        ExprKind::If { cond, then, els } => {
            ind(s, level);
            s.push_str(&format!("if ({}) {{\n", emit_expr(cond)));
            emit_block(s, then, level + 1);
            if let Some(eb) = els {
                ind(s, level);
                s.push_str("} else {\n");
                emit_block(s, eb, level + 1);
            }
            ind(s, level);
            s.push_str("}\n");
        }
        ExprKind::While { cond, body } => {
            ind(s, level);
            s.push_str(&format!("while ({}) {{\n", emit_expr(cond)));
            emit_block(s, body, level + 1);
            ind(s, level);
            s.push_str("}\n");
        }
        ExprKind::Loop(body) => {
            ind(s, level);
            s.push_str("for (;;) {\n");
            emit_block(s, body, level + 1);
            ind(s, level);
            s.push_str("}\n");
        }
        ExprKind::For { id, lo, hi, body } => {
            ind(s, level);
            s.push_str(&format!(
                "for (int64_t v{0} = {1}; v{0} < ({2}); v{0} = v{0} + 1) {{\n",
                id,
                emit_expr(lo),
                emit_expr(hi)
            ));
            emit_block(s, body, level + 1);
            ind(s, level);
            s.push_str("}\n");
        }
        ExprKind::Block(b) => {
            ind(s, level);
            s.push_str("{\n");
            emit_block(s, b, level + 1);
            ind(s, level);
            s.push_str("}\n");
        }
        _ => {}
    }
}

fn emit_stmt(s: &mut String, st: &Stmt, level: usize) {
    match &st.kind {
        StmtKind::Let { id, init } => {
            if let Some(e) = init {
                ind(s, level);
                // Array initialisers need memcpy: `v0 = (T[N]){...}` is not legal C.
                match &e.ty {
                    Type::Array { elem, len } => s.push_str(&format!(
                        "tr_arr_assign(&v{}, {}, {} * sizeof({}));\n",
                        id,
                        emit_expr(e),
                        len,
                        elem.c_ty()
                    )),
                    _ => s.push_str(&format!("v{id} = {};\n", emit_expr(e))),
                }
            }
        }
        StmtKind::Expr(e) => {
            if is_control(&e.kind) {
                emit_control(s, e, level);
            } else {
                ind(s, level);
                s.push_str(&format!("{};\n", emit_expr(e)));
            }
        }
        StmtKind::Return(Some(e)) => {
            ind(s, level);
            s.push_str(&format!("return {};\n", emit_expr(e)));
        }
        StmtKind::Return(None) => {
            ind(s, level);
            s.push_str("return;\n");
        }
        StmtKind::Break => {
            ind(s, level);
            s.push_str("break;\n");
        }
        StmtKind::Continue => {
            ind(s, level);
            s.push_str("continue;\n");
        }
    }
}

fn emit_block(s: &mut String, b: &tessera_check::Block, level: usize) {
    for st in &b.stmts {
        emit_stmt(s, st, level);
    }
    if let Some(t) = &b.tail {
        if is_control(&t.kind) {
            emit_control(s, t, level);
        } else {
            ind(s, level);
            s.push_str(&format!("{};\n", emit_expr(t)));
        }
    }
}

/// Splits a type into the part before the variable name and the part after it.
/// C wants `int16_t v3[8]`, not `int16_t[8] v3` -- the latter does not compile.
fn decl_parts(ty: &Type) -> (String, String) {
    match ty {
        Type::Array { elem, len } => (elem.c_ty(), format!("[{}]", len)),
        other => (other.c_ty(), String::new()),
    }
}

fn emit_func(s: &mut String, f: &Func) {
    s.push_str(&format!("{} tr_{}({}) {{\n", f.ret.c_ty(), f.name, params_sig(f)));
    for id in f.arity..f.local_tys.len() {
        let ty = &f.local_tys[id];
        let (base, brackets) = decl_parts(ty);
        s.push_str(&format!("  {} v{}{} = {};\n", base, id, brackets, zero_init_c(ty)));
    }
    emit_block(s, &f.body, 1);
    if f.ret != Type::Unit {
        s.push_str(&format!("  return {};\n", zero_init_c(&f.ret)));
    }
    s.push_str("}\n\n");
}

pub fn emit_c(prog: &Program) -> String {
    let mut s = String::new();
    s.push_str("/* Generated by tesserac (Tessera -> C). Do not edit. */\n");
    s.push_str("#include <stdio.h>\n#include <stdint.h>\n#include <stdbool.h>\n#include <time.h>\n#include <stdlib.h>\n");
    s.push_str(RUNTIME);
    s.push('\n');
    for def in &prog.structs {
        emit_struct_def(&mut s, def);
    }
    for (name, ty, init) in &prog.globals {
        match ty {
            Type::Array { elem, len } => {
                s.push_str(&format!("static {} tr_g_{}[{}] = {};\n", elem.c_ty(), name, len, init));
            }
            _ => {
                s.push_str(&format!("static {} tr_g_{} = {};\n", ty.c_ty(), name, init));
            }
        }
    }
    s.push('\n');
    for f in &prog.funcs {
        s.push_str(&format!("{} tr_{}({});\n", f.ret.c_ty(), f.name, params_sig(f)));
    }
    s.push('\n');
    for f in &prog.funcs {
        emit_func(&mut s, f);
    }
    if prog.funcs.iter().any(|f| f.name == "main") {
        s.push_str("int main(int argc, char** argv){ (void)argc; (void)argv; return (int)tr_main(); }\n");
    } else {
        s.push_str("int main(int argc, char** argv){ (void)argc; (void)argv; return 0; }\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_program_has_main() {
        let out = emit_c(&Program { funcs: vec![], structs: vec![], globals: vec![] });
        assert!(out.contains("int main"));
    }
}
