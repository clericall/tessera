//! Type-checker tests for `f32`/`f64` floating-point support.

use tessera_ast::Module;
use tessera_check::check;

fn module(src: &str) -> Module {
    let (m, errs) = tessera_parser::parse(src);
    assert!(errs.is_empty(), "parse errors: {errs:?}");
    m
}

fn check_src(src: &str) -> usize {
    let m = module(src);
    let (_program, errors) = check(&m);
    errors.len()
}

fn check_ok(src: &str) {
    let n = check_src(src);
    assert_eq!(n, 0, "expected `{src}` to type-check");
}

fn check_err(src: &str) -> String {
    let m = module(src);
    let (_program, errors) = check(&m);
    assert!(!errors.is_empty(), "expected `{src}` to fail type-checking");
    errors[0].message.clone()
}

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

#[test]
fn float_literal_defaults_to_f64() {
    check_ok("fn main() -> i64 { let x: f64 = 1.5; return 0; }");
    check_ok("fn main() -> i64 { let x = 1.5; return 0; }");
}

#[test]
fn f32_literal_typed() {
    check_ok("fn main() -> i64 { let x: f32 = 1.5; return 0; }");
}

#[test]
fn float_literal_in_expr() {
    check_ok("fn main() -> i64 { let x: f64 = 1.5 + 2.5; return 0; }");
}

// ---------------------------------------------------------------------------
// Binary ops
// ---------------------------------------------------------------------------

#[test]
fn float_arithmetic_typechecks() {
    check_ok("fn main() -> f64 { let a: f64 = 1.5; let b: f64 = 2.5; return a + b - a * b / 2.0; }");
}

#[test]
fn f32_arithmetic_stays_f32() {
    check_ok("fn main() -> f32 { let a: f32 = 1.5; let b: f32 = 2.5; return a * b; }");
}

#[test]
fn f32_mixed_with_f64_promotes_to_f64() {
    check_ok("fn main() -> f64 { let a: f32 = 1.5; let b: f64 = 2.5; return a + b; }");
}

#[test]
fn float_comparisons_return_bool() {
    check_ok("fn main() -> bool { let a: f64 = 1.5; let b: f64 = 2.5; return a < b && a == b; }");
}

#[test]
fn mixed_int_and_float_is_rejected() {
    let msg = check_err("fn main() -> f64 { let a: f64 = 1.5; return a + 1; }");
    assert!(msg.contains("invalid operands"), "{msg}");
    let msg = check_err("fn main() -> bool { let a: f64 = 1.5; return a < 1; }");
    assert!(msg.contains("cannot compare"), "{msg}");
}

#[test]
fn float_modulo_is_rejected() {
    let msg = check_err("fn main() -> f64 { let a: f64 = 1.5; return a % 1.0; }");
    assert!(msg.contains("%"), "{msg}");
}

#[test]
fn int_binary_ops_still_work() {
    check_ok("fn main() -> i64 { return 1 + 2 * 3 - 4 / 2; }");
}

// ---------------------------------------------------------------------------
// Casts
// ---------------------------------------------------------------------------

#[test]
fn int_to_float_cast() {
    check_ok("fn main() -> f64 { let a: i64 = 5; return a as f64; }");
}

#[test]
fn float_to_int_cast() {
    check_ok("fn main() -> i64 { let a: f64 = 5.5; return a as i64; }");
}

#[test]
fn f32_to_f64_cast() {
    check_ok("fn main() -> f64 { let a: f32 = 1.5; return a as f64; }");
}

#[test]
fn f64_to_bool_cast() {
    check_ok("fn main() -> bool { let a: f64 = 1.5; return a as bool; }");
}

#[test]
fn float_to_struct_is_rejected() {
    let msg = check_err("struct P { a: i64 }\nfn main() -> i64 { let p: P = P { a: 1 }; let x: f64 = p as f64; return 0; }");
    assert!(msg.contains("cannot cast"), "{msg}");
}

// ---------------------------------------------------------------------------
// Arrays / structs of floats
// ---------------------------------------------------------------------------

#[test]
fn float_array() {
    check_ok("fn main() -> f64 { let a: [f64; 3] = [1.0, 2.0, 3.0]; return a[1]; }");
}

#[test]
fn struct_with_float_field() {
    check_ok("struct P { x: f64, y: f64 }\nfn main() -> f64 { let p: P = P { x: 1.0, y: 2.0 }; return p.x + p.y; }");
}

// ---------------------------------------------------------------------------
// String builtins
// ---------------------------------------------------------------------------

#[test]
fn len_builtin() {
    check_ok("fn main() -> i64 { let s: str = \"hello\"; return len(s); }");
}

#[test]
fn concat_builtin() {
    check_ok("fn main() -> str { return concat(\"a\", \"b\"); }");
}

#[test]
fn len_on_non_string_is_rejected() {
    let msg = check_err("fn main() -> i64 { return len(7); }");
    assert!(msg.contains("expects a string"), "{msg}");
}

#[test]
fn concat_with_non_string_is_rejected() {
    let msg = check_err("fn main() -> str { return concat(\"a\", 1); }");
    assert!(msg.contains("expects a string"), "{msg}");
}