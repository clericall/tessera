//! C-backend tests for casts and array code generation.
//!
//! Arrays are the sharp edge here: C wants `int16_t v3[8]` (brackets after the
//! name), rejects `v3 = (T[N]){...}` outright, and will not initialise a narrow
//! array from wide expressions. Each of those used to emit broken C.

use tessera_ast::Module;
use tessera_backend_c::emit_c;
use tessera_check::check;

fn gen(src: &str) -> String {
    let (m, perrs) = tessera_parser::parse(src);
    assert!(perrs.is_empty(), "parse errors: {perrs:?}");
    let (prog, cerrs) = check(&m);
    assert!(cerrs.is_empty(), "type errors: {cerrs:?}");
    emit_c(&prog)
}

fn module_ok(src: &str) -> Module {
    let (m, perrs) = tessera_parser::parse(src);
    assert!(perrs.is_empty(), "parse errors: {perrs:?}");
    m
}

#[test]
fn cast_emits_a_c_cast() {
    let c = gen("fn main() -> i64 { let a: i64 = 5; let b: i16 = a as i16; return b as i64; }");
    assert!(c.contains("(int16_t)"), "no C cast emitted:\n{c}");
}

#[test]
fn cast_target_drives_the_width() {
    let c = gen("fn main() -> i64 { let a: i64 = 5; return a as u8 as i64; }");
    assert!(c.contains("(uint8_t)"), "u8 cast missing:\n{c}");
}

#[test]
fn array_local_declares_brackets_after_the_name() {
    let c = gen("fn main() -> i64 { let a: [i16; 4] = [1, 2, 3, 4]; return a[0] as i64; }");
    assert!(c.contains("int16_t v0[4]"), "bad array declaration:\n{c}");
    assert!(!c.contains("[4] v0"), "brackets before the name are invalid C:\n{c}");
}

#[test]
fn array_literal_casts_each_element_to_the_element_type() {
    let c = gen("fn main() -> i64 { let a: [i16; 3] = [-1, 2, 300]; return a[0] as i64; }");
    assert!(c.contains("(int16_t)"), "narrow array elements are not cast:\n{c}");
}

#[test]
fn array_assignment_goes_through_memcpy() {
    let c = gen(
        r#"
        fn main() -> i64 {
            let a: [i64; 3] = [1, 2, 3];
            let b: [i64; 3] = [0, 0, 0];
            b = a;
            return b[1];
        }
        "#,
    );
    assert!(c.contains("tr_arr_assign"), "array assignment needs memcpy:\n{c}");
    assert!(!c.contains("v1 = (int64_t[3])"), "raw array assignment emitted:\n{c}");
}

#[test]
fn array_parameters_declare_as_arrays() {
    let c = gen("fn sum(a: [i64; 3]) -> i64 { return a[0]; }\nfn main() -> i64 { let v: [i64; 3] = [1, 2, 3]; return sum(v); }");
    assert!(c.contains("int64_t v0[3]"), "array parameter signature is wrong:\n{c}");
}

#[test]
fn array_in_struct_initialises_with_braces() {
    let c = gen("struct P { a: i64, b: [i16; 2] }\nfn main() -> i64 { let p: P = P { a: 1, b: [2, 3] }; return p.b[1] as i64; }");
    assert!(c.contains(".b = {"), "struct array member needs brace init:\n{c}");
}

#[test]
fn scalar_let_still_uses_a_plain_assignment() {
    let c = gen("fn main() -> i64 { var n: i64 = 7; return n; }");
    // The helper is always declared in the runtime; nothing here should *call* it.
    assert!(!c.contains("tr_arr_assign(&"), "scalar init should not memcpy:\n{c}");
    assert!(c.contains("v0 = ((int64_t)7LL)"), "scalar init changed:\n{c}");
}

#[test]
fn runtime_has_the_array_assign_helper() {
    let c = gen("fn main() -> i64 { return 0; }");
    assert!(c.contains("static void tr_arr_assign"), "helper missing from runtime:\n{c}");
}

#[test]
fn check_rejects_array_return_types() {
    let m = module_ok("fn mk() -> [i64; 2] { let a: [i64; 2] = [1, 2]; return a; }");
    let (_prog, errors) = check(&m);
    assert!(errors.iter().any(|e| e.message.contains("cannot return an array")), "{errors:?}");
}

// ---------------------------------------------------------------------------
// Floats
// ---------------------------------------------------------------------------

#[test]
fn float_literal_emits_cast() {
    let c = gen("fn main() -> f64 { let x: f64 = 1.5; return x; }");
    assert!(c.contains("(double)1.5"), "float literal missing:\n{c}");
}

#[test]
fn f32_literal_emits_f_suffix() {
    let c = gen("fn main() -> f32 { let x: f32 = 1.5; return x; }");
    assert!(c.contains("(float)1.5f"), "f32 literal missing f suffix:\n{c}");
}

#[test]
fn float_binary_emits_c_operators() {
    let c = gen("fn main() -> f64 { let a: f64 = 1.5; let b: f64 = 2.5; return a + b - a * b / 2.0; }");
    assert!(c.contains(" + "), "float add missing:\n{c}");
    assert!(c.contains(" - "), "float sub missing:\n{c}");
    assert!(c.contains(" * "), "float mul missing:\n{c}");
    assert!(c.contains(" / "), "float div missing:\n{c}");
}

#[test]
fn f32_widened_to_f64_in_binary() {
    let c = gen("fn main() -> f64 { let a: f32 = 1.5; let b: f64 = 2.5; return a + b; }");
    assert!(c.contains("(double)"), "f32 should be widened:\n{c}");
}

#[test]
fn int_to_float_cast_emits_c_cast() {
    let c = gen("fn main() -> f64 { let a: i64 = 5; return a as f64; }");
    assert!(c.contains("(double)"), "int->float cast missing:\n{c}");
}

#[test]
fn float_to_int_cast_emits_c_cast() {
    let c = gen("fn main() -> i64 { let a: f64 = 5.5; return a as i64; }");
    assert!(c.contains("(int64_t)"), "float->int cast missing:\n{c}");
}

// ---------------------------------------------------------------------------
// Strings
// ---------------------------------------------------------------------------

#[test]
fn len_builtin_emits_field_access() {
    let c = gen("fn main() -> i64 { let s: str = \"hi\"; return len(s); }");
    assert!(c.contains("tr_sl"), "str literal missing:\n{c}");
    assert!(c.contains(".n)"), "len should read .n:\n{c}");
}

#[test]
fn concat_builtin_emits_helper() {
    let c = gen("fn main() -> str { return concat(\"a\", \"b\"); }");
    assert!(c.contains("tr_str_concat"), "concat missing helper:\n{c}");
}