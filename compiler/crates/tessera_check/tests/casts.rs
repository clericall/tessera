//! Type-checker tests for `as` casts, type aliases, and array typing.

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
// Casts
// ---------------------------------------------------------------------------

#[test]
fn casts_between_integer_widths_and_signedness_typecheck() {
    check_ok("fn main() -> i64 { let a: i64 = 5; let b: u64 = a as u64; return b as i64; }");
    check_ok("fn main() -> i64 { let a: u64 = 5; return a as i64; }");
    check_ok("fn main() -> i64 { let a: i64 = 300; let b: i16 = a as i16; return b as i64; }");
    check_ok("fn main() -> i64 { let a: i64 = 7; return (a as u8) as i64; }");
}

#[test]
fn casts_work_inside_expressions() {
    check_ok("fn main() -> i64 { return (3 as i64) + (4 as i64); }");
    check_ok(
        r#"
        fn main() -> i64 {
            let a: [i16; 3] = [-1, 2, 300];
            var s: i64 = 0;
            var i: i64 = 0;
            while i < 3 { s += a[i] as i64; i += 1; }
            return s;
        }
        "#,
    );
}

#[test]
fn cast_result_type_is_the_target_type() {
    let m = module("fn main() -> i64 { let a: i64 = 5; let b: i16 = a as i16; return 0; }");
    let (program, errors) = check(&m);
    assert!(errors.is_empty(), "{errors:?}");
    let main_fn = program.funcs.iter().find(|f| f.name == "main").expect("main");
    let var = main_fn
        .local_tys
        .iter()
        .find(|t| matches!(t, tessera_check::Type::Int { bits: 16, signed: true }))
        .expect("an i16 local");
    assert!(matches!(var, tessera_check::Type::Int { bits: 16, signed: true }));
}

#[test]
fn casting_a_struct_is_rejected() {
    let msg = check_err(
        "struct P { a: i64 }\nfn main() -> i64 { let p: P = P { a: 1 }; let x: i64 = p as i64; return x; }",
    );
    assert!(msg.contains("cannot cast"), "{msg}");
}

#[test]
fn casting_a_string_is_rejected() {
    let msg = check_err("fn main() -> i64 { let s: str = \"a\"; let x: i64 = s as i64; return x; }");
    assert!(msg.contains("cannot cast"), "{msg}");
}

#[test]
fn cast_to_an_unknown_type_is_rejected() {
    let msg = check_err("fn main() -> i64 { let x: i64 = 1 as Nope; return x; }");
    assert!(msg.contains("Nope"), "{msg}");
}

#[test]
fn casts_bind_tighter_than_arithmetic() {
    // `a as i64 + 1` must parse as `(a as i64) + 1`, not `a as (i64 + 1)`.
    check_ok("fn main() -> i64 { let a: i8 = 1; return a as i64 + 1; }");
}

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

#[test]
fn aliases_resolve_for_locals_params_and_returns() {
    check_ok("type Sq = i64;\nfn f(x: Sq) -> Sq { return x; }\nfn main() -> i64 { let v: Sq = f(3); return v; }");
}

#[test]
fn aliases_of_aliases_resolve() {
    check_ok("type Sq = i64;\ntype FileSq = Sq;\nfn main() -> i64 { let v: FileSq = 3; return v; }");
}

#[test]
fn aliases_work_in_struct_fields_and_array_elements() {
    check_ok(
        "type Sq = i64;\nstruct P { a: Sq, b: [Sq; 2] }\nfn main() -> i64 { let p: P = P { a: 1, b: [2, 3] }; return p.a; }",
    );
}

#[test]
fn alias_cycles_are_reported() {
    let msg = check_err("type A = B;\ntype B = A;\nfn main() -> i64 { return 0; }");
    assert!(msg.contains("alias") || msg.contains("cycle"), "{msg}");
}

#[test]
fn unknown_alias_target_is_reported() {
    let msg = check_err("type A = Missing;\nfn main() -> i64 { return 0; }");
    assert!(msg.contains("Missing"), "{msg}");
}

#[test]
fn arrays_of_narrow_types_typecheck() {
    check_ok(
        "fn main() -> i64 {\n\
         \x20   let a: [i8; 2] = [1, 2];\n\
         \x20   let b: [u32; 2] = [1 as u32, 2 as u32];\n\
         \x20   return a[0] as i64 + b[1] as i64;\n\
         \x20 }",
    );
}

#[test]
fn functions_cannot_return_arrays() {
    let msg = check_err("fn mk() -> [i64; 2] { let a: [i64; 2] = [1, 2]; return a; }");
    assert!(msg.contains("cannot return an array"), "{msg}");
}

#[test]
fn arrays_are_valid_parameters() {
    check_ok("fn sum(a: [i64; 3]) -> i64 { return a[0] + a[1] + a[2]; }\nfn main() -> i64 { let v: [i64; 3] = [1, 2, 3]; return sum(v); }");
}