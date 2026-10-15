//! Type-checker tests for the file-I/O builtins added in the NNUE prerequisite
//! pass (`fopen` / `fread_u8` / `fread_i16` / `fread_i32` / `feof` / `fclose`).

use tessera_ast::Module;
use tessera_check::check;

fn check_src(src: &str) -> usize {
    let module: Module = tessera_parser_parse(src);
    let (_program, errors) = check(&module);
    errors.len()
}

fn tessera_parser_parse(src: &str) -> Module {
    let (m, errs) = tessera_parser::parse(src);
    assert!(errs.is_empty(), "parse errors: {errs:?}");
    m
}

#[test]
fn file_builtins_accept_correct_arguments() {
    let src = r#"
        fn main() -> i64 {
            let p: str = "net.nnue";
            let h: i64 = fopen(p);
            if h < 0 { return 0 - 1; }
            let a: i64 = fread_i16(h);
            let b: i64 = fread_i32(h);
            let c: i64 = fread_u8(h);
            let e: bool = feof(h);
            let r: i64 = fclose(h);
            if e { return a + b + c + r; }
            return 0;
        }
    "#;
    assert_eq!(check_src(src), 0);
}

#[test]
fn fopen_requires_a_string_path() {
    let src = r#"
        fn main() -> i64 {
            let h: i64 = fopen(7);
            return h;
        }
    "#;
    assert!(check_src(src) > 0, "expected a type error for fopen(7)");
}

#[test]
fn fread_builtins_take_exactly_one_handle() {
    let src = r#"
        fn main() -> i64 {
            return fread_i16(1, 2);
        }
    "#;
    assert!(check_src(src) > 0, "expected an arity error for fread_i16(1, 2)");
}

#[test]
fn feof_is_usable_as_a_condition() {
    let src = r#"
        fn main() -> i64 {
            let h: i64 = fopen("x");
            var n: i64 = 0;
            if feof(h) { n = 1; } else { n = 2; }
            fclose(h);
            return n;
        }
    "#;
    assert_eq!(check_src(src), 0);
}

#[test]
fn feof_takes_one_argument() {
    let src = r#"
        fn main() -> i64 {
            if feof() { return 1; }
            return 0;
        }
    "#;
    assert!(check_src(src) > 0, "expected an arity error for feof()");
}