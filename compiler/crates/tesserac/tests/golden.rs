// End-to-end golden tests that exercise the lexer the way `tesserac lex` does,
// including line/column mapping via tessera_diag.

use tessera_diag::SourceFile;
use tessera_lexer::{tokenize, Tok};

fn kinds(src: &str) -> Vec<Tok> {
    let (toks, errs) = tokenize(src);
    assert!(errs.is_empty(), "errors: {errs:?}");
    toks.into_iter().filter(|t| t.tok != Tok::Eof).map(|t| t.tok).collect()
}

#[test]
fn line_col_first_line() {
    let sf = SourceFile::new("t.ter", "let x = 1;\n");
    assert_eq!(sf.line_col(0), (1, 1)); // 'l'
    assert_eq!(sf.line_col(4), (1, 5)); // 'x'
    assert_eq!(sf.line_col(8), (1, 9)); // '1'
}

#[test]
fn line_col_second_line() {
    let src = "a;\n  bb;";
    let sf = SourceFile::new("t.ter", src);
    assert_eq!(sf.line_col(0), (1, 1));
    assert_eq!(sf.line_col(3), (2, 1)); // first space on line 2
    assert_eq!(sf.line_col(5), (2, 3)); // 'b'
}

#[test]
fn error_report_points_at_the_right_column() {
    let src = "line one\nlet y = 1 @ 2;\n";
    let sf = SourceFile::new("t.ter", src);
    let (_, errs) = tokenize(src);
    assert_eq!(errs.len(), 1);
    let (line, col) = sf.line_col(errs[0].span.lo);
    assert_eq!(line, 2);
    assert_eq!(col, 11); // '@' on line 2
    let rendered = sf.render("E0001", &errs[0].message, (errs[0].span.lo, errs[0].span.hi), "", "");
    assert!(rendered.contains("2:11"), "rendered:\n{rendered}");
}

#[test]
fn sample_of_language_surface() {
    let src = r#"
        type BitBoard = u64;
        struct Position { pieces: [u64; 6], stm: i32 }
        const fn build() -> [u64; 64] { let t = 0; return t; }
        fn perft(pos: &Position, depth: i32) -> u64 {
            var n: u64 = 0;
            for i in 0..64 { n += 0x1 & 0b1010 ^ ~3; }
            if depth == 0 { return 1; } else { return n << 2 >> 1; }
        }
    "#;
    let ks = kinds(src);
    // Presence checks for representative tokens.
    assert!(ks.contains(&Tok::LBrace));
    assert!(ks.contains(&Tok::LBracket));
    assert!(ks.contains(&Tok::Arrow));
    assert!(ks.contains(&Tok::Amp));
    assert!(ks.contains(&Tok::Shl));
    assert!(ks.contains(&Tok::Shr));
    assert!(ks.contains(&Tok::PlusEq));
    assert!(ks.contains(&Tok::DotDot));
    assert!(ks.contains(&Tok::Tilde));
    assert!(ks.contains(&Tok::Caret));
}
