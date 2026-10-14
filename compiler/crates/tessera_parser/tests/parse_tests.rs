use tessera_ast::{ExprKind, ItemKind, Lit};
use tessera_parser::{parse, parse_expr_str, AssignOp, BinOp, UnaryOp};

fn expr(src: &str) -> tessera_ast::Expr {
    let (e, errs) = parse_expr_str(src);
    assert!(errs.is_empty(), "unexpected errors for `{src}`: {errs:?}");
    e.expect("parsed an expression")
}

#[test]
fn int_plus_mul_precedence() {
    // 1 + 2 * 3  =>  Add(1, Mul(2, 3))
    let e = expr("1 + 2 * 3");
    match e.kind {
        ExprKind::Binary { op: BinOp::Add, lhs, rhs } => {
            assert!(matches!(lhs.kind, ExprKind::Lit(Lit::Int(_))));
            assert!(matches!(&rhs.kind, ExprKind::Binary { op: BinOp::Mul, .. }));
        }
        other => panic!("wrong shape: {other:?}"),
    }
}

#[test]
fn shifts_bind_tighter_than_comparison() {
    // a << 2 > b  =>  Gt( Shl(a, 2), b )
    let e = expr("a << 2 > b");
    match e.kind {
        ExprKind::Binary { op: BinOp::Gt, lhs, .. } => {
            assert!(matches!(&lhs.kind, ExprKind::Binary { op: BinOp::Shl, .. }));
        }
        other => panic!("wrong shape: {other:?}"),
    }
}

#[test]
fn assignment_is_right_assoc() {
    // a = b = c  =>  Assign(Set, a, Assign(Set, b, c))
    let e = expr("a = b = c");
    match e.kind {
        ExprKind::Assign { op: AssignOp::Set, rhs, .. } => {
            assert!(matches!(&rhs.kind, ExprKind::Assign { op: AssignOp::Set, .. }));
        }
        other => panic!("wrong shape: {other:?}"),
    }
}

#[test]
fn compound_assignment_ops() {
    assert!(matches!(expr("a += 1").kind, ExprKind::Assign { op: AssignOp::Add, .. }));
    assert!(matches!(expr("a <<= 1").kind, ExprKind::Assign { op: AssignOp::Shl, .. }));
}

#[test]
fn prefix_unary() {
    assert!(matches!(expr("-x").kind, ExprKind::Unary { op: UnaryOp::Neg, .. }));
    assert!(matches!(expr("&x").kind, ExprKind::Unary { op: UnaryOp::Ref, .. }));
    assert!(matches!(expr("*p").kind, ExprKind::Unary { op: UnaryOp::Deref, .. }));
    assert!(matches!(expr("!b").kind, ExprKind::Unary { op: UnaryOp::Not, .. }));
    assert!(matches!(expr("~m").kind, ExprKind::Unary { op: UnaryOp::BitNot, .. }));
}

#[test]
fn postfix_call_field_index() {
    // f(1).g[2] => Index( Field( Call(f,[1]), g ), 2 )
    let e = expr("f(1).g[2]");
    match e.kind {
        ExprKind::Index { base, index } => {
            assert!(matches!(&base.kind, ExprKind::Field { .. }));
            assert!(matches!(index.kind, ExprKind::Lit(Lit::Int(_))));
        }
        other => panic!("wrong shape: {other:?}"),
    }
}

#[test]
fn range_expression() {
    assert!(matches!(expr("0..64").kind, ExprKind::Binary { op: BinOp::Range, .. }));
}

#[test]
fn path_segments() {
    assert!(matches!(expr("i32x8::splat").kind, ExprKind::Path(_)));
    assert!(matches!(expr("plain").kind, ExprKind::Ident(_)));
}

#[test]
fn parse_sample_file() {
    let src = include_str!("../../../samples/ok.ter");
    let (m, errs) = parse(src);
    assert!(errs.is_empty(), "errors: {errs:?}");
    assert_eq!(m.items.len(), 5);
    assert!(matches!(m.items[0].kind, ItemKind::TypeAlias { .. }));
    assert!(matches!(m.items[1].kind, ItemKind::Enum { .. }));
    assert!(matches!(m.items[2].kind, ItemKind::Struct { .. }));
    assert!(matches!(&m.items[3].kind, ItemKind::Fn(f) if f.is_const));
    assert!(matches!(&m.items[4].kind, ItemKind::Fn(f) if !f.is_const));
}

#[test]
fn blocklike_statement_needs_no_semicolon_midblock() {
    let src = "fn f() -> i32 { if depth == 0 { return 1; } var n: i32 = 0; return n; }";
    let (m, errs) = parse(src);
    assert!(errs.is_empty(), "errors: {errs:?}");
    assert_eq!(m.items.len(), 1);
}

#[test]
fn struct_trailing_comma_and_empty() {
    let (_, e1) = parse("struct P { a: u64, b: u32, }");
    assert!(e1.is_empty());
    let (_, e2) = parse("struct E { }");
    assert!(e2.is_empty());
}

#[test]
fn error_on_missing_rvalue() {
    let (_m, errs) = parse("fn f() { let x = ; }");
    assert!(!errs.is_empty());
}

#[test]
fn error_recovers_and_continues() {
    // A broken item should not stop parsing subsequent items.
    let (m, errs) = parse("struct { broken\nfn ok() {}");
    assert!(!errs.is_empty());
    assert!(m.items.iter().any(|it| matches!(&it.kind, ItemKind::Fn(_))));
}

#[test]
fn binary_ops_are_left_associative() {
    // 10 - 3 - 2  =>  Sub( Sub(10, 3), 2)   (NOT 10 - (3 - 2))
    let e = expr("10 - 3 - 2");
    match e.kind {
        ExprKind::Binary { op: BinOp::Sub, lhs, rhs } => {
            assert!(matches!(lhs.kind, ExprKind::Binary { op: BinOp::Sub, .. }));
            assert!(matches!(rhs.kind, ExprKind::Lit(Lit::Int(_))));
        }
        other => panic!("wrong shape: {other:?}"),
    }
}

#[test]
fn subtraction_does_not_swallow_trailing_add() {
    // 0 - b + 1  =>  Add( Sub(0, b), 1)   (NOT Sub(0, Add(b, 1)))
    let e = expr("0 - b + 1");
    match e.kind {
        ExprKind::Binary { op: BinOp::Add, lhs, rhs } => {
            assert!(matches!(lhs.kind, ExprKind::Binary { op: BinOp::Sub, .. }));
            assert!(matches!(rhs.kind, ExprKind::Lit(Lit::Int(_))));
        }
        other => panic!("wrong shape: {other:?}"),
    }
}

#[test]
fn division_and_modulo_are_left_associative() {
    // 8 / 2 * 4  =>  Mul( Div(8, 2), 4)
    let e = expr("8 / 2 * 4");
    match e.kind {
        ExprKind::Binary { op: BinOp::Mul, lhs, .. } => {
            assert!(matches!(lhs.kind, ExprKind::Binary { op: BinOp::Div, .. }));
        }
        other => panic!("wrong shape: {other:?}"),
    }
}