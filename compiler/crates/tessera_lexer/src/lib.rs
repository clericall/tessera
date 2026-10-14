//! Tessera lexer.
//!
//! Produces a flat token stream with byte [`Span`]s. Line/column mapping is done
//! by the consumer via [`tessera_diag::SourceFile`]. Newlines are not significant
//! (statements are `;`/`}`-terminated), so they are treated as whitespace.

/// Byte range `[lo, hi)` into the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub lo: usize,
    pub hi: usize,
}

impl Span {
    pub fn new(lo: usize, hi: usize) -> Self {
        Span { lo, hi }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    // Trivia-free tokens
    Ident,
    Keyword, // text is src[span]; matched against KEYWORDS
    Int,
    Float,
    Str,          // raw slice includes surrounding quotes
    Char,         // raw slice includes surrounding single quotes

    // Arithmetic / bitwise
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Bang,
    Shl,
    Shr,
    ShlEq,
    ShrEq,

    // Comparison / equality
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,

    // Assignment
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,

    // Punctuation
    Dot,
    DotDot,
    Colon,
    ColonColon,
    Semi,
    Comma,
    Arrow, // ->
    Question,

    // Grouping
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.span.lo..self.span.hi]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub span: Span,
    pub message: String,
    pub help: Option<String>,
}

pub const KEYWORDS: &[&str] = &[
    "as", "asm", "break", "const", "continue", "else", "enum", "false", "fn", "for", "if", "import",
    "in", "let", "loop", "return", "static", "struct", "true", "type", "var", "while",
];

fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    i: usize,
}

fn is_id_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}
fn is_id_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

/// UTF-8 sequence length from a lead byte (1 for ASCII/invalid, guaranteeing progress).
#[inline]
fn utf8_len(byte: u8) -> usize {
    if byte < 0x80 {
        1
    } else if byte >> 5 == 0b110 {
        2
    } else if byte >> 4 == 0b1110 {
        3
    } else if byte >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// If `pat` occurs at `i`, return the index just past it.
fn eat(b: &[u8], i: usize, pat: &[u8]) -> Option<usize> {
    if b.len() >= i + pat.len() && &b[i..i + pat.len()] == pat {
        Some(i + pat.len())
    } else {
        None
    }
}

fn scan_op(b: &[u8], i: usize) -> Option<(Tok, usize)> {
    use Tok::*;
    // 3-char
    if let Some(j) = eat(b, i, b"<<=") {
        return Some((ShlEq, j));
    }
    if let Some(j) = eat(b, i, b">>=") {
        return Some((ShrEq, j));
    }
    // 2-char
    if let Some(j) = eat(b, i, b"<<") {
        return Some((Shl, j));
    }
    if let Some(j) = eat(b, i, b">>") {
        return Some((Shr, j));
    }
    if let Some(j) = eat(b, i, b"==") {
        return Some((EqEq, j));
    }
    if let Some(j) = eat(b, i, b"!=") {
        return Some((Ne, j));
    }
    if let Some(j) = eat(b, i, b"<=") {
        return Some((Le, j));
    }
    if let Some(j) = eat(b, i, b">=") {
        return Some((Ge, j));
    }
    if let Some(j) = eat(b, i, b"&&") {
        return Some((AndAnd, j));
    }
    if let Some(j) = eat(b, i, b"||") {
        return Some((OrOr, j));
    }
    if let Some(j) = eat(b, i, b"+=") {
        return Some((PlusEq, j));
    }
    if let Some(j) = eat(b, i, b"-=") {
        return Some((MinusEq, j));
    }
    if let Some(j) = eat(b, i, b"*=") {
        return Some((StarEq, j));
    }
    if let Some(j) = eat(b, i, b"/=") {
        return Some((SlashEq, j));
    }
    if let Some(j) = eat(b, i, b"%=") {
        return Some((PercentEq, j));
    }
    if let Some(j) = eat(b, i, b"&=") {
        return Some((AmpEq, j));
    }
    if let Some(j) = eat(b, i, b"|=") {
        return Some((PipeEq, j));
    }
    if let Some(j) = eat(b, i, b"^=") {
        return Some((CaretEq, j));
    }
    if let Some(j) = eat(b, i, b"->") {
        return Some((Arrow, j));
    }
    if let Some(j) = eat(b, i, b"::") {
        return Some((ColonColon, j));
    }
    if let Some(j) = eat(b, i, b"..") {
        return Some((DotDot, j));
    }
    // 1-char
    let c = b[i];
    let single = match c {
        b'+' => Plus,
        b'-' => Minus,
        b'*' => Star,
        b'/' => Slash,
        b'%' => Percent,
        b'&' => Amp,
        b'|' => Pipe,
        b'^' => Caret,
        b'~' => Tilde,
        b'!' => Bang,
        b'<' => Lt,
        b'>' => Gt,
        b'=' => Eq,
        b'.' => Dot,
        b':' => Colon,
        b';' => Semi,
        b',' => Comma,
        b'(' => LParen,
        b')' => RParen,
        b'[' => LBracket,
        b']' => RBracket,
        b'{' => LBrace,
        b'}' => RBrace,
        b'?' => Question,
        _ => return None,
    };
    Some((single, i + 1))
}

impl<'a> Lexer<'a> {
    fn tok(&self, tok: Tok, lo: usize, hi: usize) -> Token {
        Token { tok, span: Span { lo, hi } }
    }

    fn skip_trivia(&mut self, errs: &mut Vec<LexError>) {
        loop {
            let len = self.b.len();
            while self.i < len
                && matches!(self.b[self.i], b' ' | b'\t' | b'\r' | b'\n')
            {
                self.i += 1;
            }
            // line comment
            if self.i + 1 < len && self.b[self.i] == b'/' && self.b[self.i + 1] == b'/' {
                self.i += 2;
                while self.i < len && self.b[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            // block comment (non-nested)
            if self.i + 1 < len && self.b[self.i] == b'/' && self.b[self.i + 1] == b'*' {
                let start = self.i;
                self.i += 2;
                let mut closed = false;
                while self.i < len {
                    if self.i + 1 < len && self.b[self.i] == b'*' && self.b[self.i + 1] == b'/' {
                        self.i += 2;
                        closed = true;
                        break;
                    }
                    self.i += 1;
                }
                if !closed {
                    errs.push(LexError {
                        span: Span { lo: start, hi: self.i },
                        message: "unterminated block comment".into(),
                        help: None,
                    });
                }
                continue;
            }
            break;
        }
    }

    fn scan_ident(&mut self, start: usize) -> Token {
        while self.i < self.b.len() && is_id_continue(self.b[self.i]) {
            self.i += 1;
        }
        let text = &self.src[start..self.i];
        let kind = if is_keyword(text) { Tok::Keyword } else { Tok::Ident };
        self.tok(kind, start, self.i)
    }

    fn scan_number(&mut self, start: usize) -> Token {
        let b = self.b;
        let len = b.len();
        let mut is_float = false;

        let lower = |x: u8| x | 0x20;
        if b[self.i] == b'0' && self.i + 1 < len && lower(b[self.i + 1]) == b'x' {
            self.i += 2;
            while self.i < len && (b[self.i].is_ascii_hexdigit() || b[self.i] == b'_') {
                self.i += 1;
            }
        } else if b[self.i] == b'0' && self.i + 1 < len && lower(b[self.i + 1]) == b'b' {
            self.i += 2;
            while self.i < len && (b[self.i] == b'0' || b[self.i] == b'1' || b[self.i] == b'_') {
                self.i += 1;
            }
        } else {
            while self.i < len && (b[self.i].is_ascii_digit() || b[self.i] == b'_') {
                self.i += 1;
            }
            // fractional part: '.' counts only if followed by a digit (so `1..8` stays a range)
            if self.i + 1 < len && b[self.i] == b'.' && b[self.i + 1].is_ascii_digit() {
                is_float = true;
                self.i += 1;
                while self.i < len && (b[self.i].is_ascii_digit() || b[self.i] == b'_') {
                    self.i += 1;
                }
            }
            // exponent
            if self.i < len && (b[self.i] == b'e' || b[self.i] == b'E') {
                let mut j = self.i + 1;
                if j < len && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < len && b[j].is_ascii_digit() {
                    is_float = true;
                    self.i = j + 1;
                    while self.i < len && (b[self.i].is_ascii_digit() || b[self.i] == b'_') {
                        self.i += 1;
                    }
                }
            }
        }
        self.tok(if is_float { Tok::Float } else { Tok::Int }, start, self.i)
    }

    fn scan_string(&mut self, start: usize, errs: &mut Vec<LexError>) -> Token {
        self.i += 1; // opening quote
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'\\' {
                self.i += 1;
                if self.i < self.b.len() {
                    self.i += utf8_len(self.b[self.i]);
                }
                continue;
            }
            if c == b'"' {
                self.i += 1;
                return self.tok(Tok::Str, start, self.i);
            }
            self.i += utf8_len(c);
        }
        errs.push(LexError {
            span: Span { lo: start, hi: self.i },
            message: "unterminated string literal".into(),
            help: Some("add a closing \"".into()),
        });
        self.tok(Tok::Str, start, self.i)
    }

    fn scan_char(&mut self, start: usize, errs: &mut Vec<LexError>) -> Token {
        self.i += 1; // opening quote
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c == b'\\' {
                self.i += 1;
                if self.i < self.b.len() {
                    self.i += utf8_len(self.b[self.i]);
                }
                continue;
            }
            if c == b'\'' {
                self.i += 1;
                return self.tok(Tok::Char, start, self.i);
            }
            self.i += utf8_len(c);
        }
        errs.push(LexError {
            span: Span { lo: start, hi: self.i },
            message: "unterminated char literal".into(),
            help: Some("add a closing '".into()),
        });
        self.tok(Tok::Char, start, self.i)
    }

    fn next_token(&mut self, errs: &mut Vec<LexError>) -> Token {
        loop {
            self.skip_trivia(errs);
            let start = self.i;
            if self.i >= self.b.len() {
                return self.tok(Tok::Eof, start, start);
            }
            let c = self.b[self.i];
            if is_id_start(c) {
                return self.scan_ident(start);
            }
            if c.is_ascii_digit() {
                return self.scan_number(start);
            }
            if c == b'"' {
                return self.scan_string(start, errs);
            }
            if c == b'\'' {
                return self.scan_char(start, errs);
            }
            if let Some((tok, j)) = scan_op(self.b, self.i) {
                self.i = j;
                return self.tok(tok, start, j);
            }
            // Unknown byte: record and skip a full UTF-8 char, then loop (guarantees progress).
            errs.push(LexError {
                span: Span { lo: self.i, hi: self.i + utf8_len(c) },
                message: format!("unexpected character `{}`", c as char),
                help: None,
            });
            self.i += utf8_len(c);
        }
    }
}

/// Tokenize `src` into a stream (terminated by [`Tok::Eof`]) plus any lexical errors.
pub fn tokenize(src: &str) -> (Vec<Token>, Vec<LexError>) {
    let mut lexer = Lexer { src, b: src.as_bytes(), i: 0 };
    let mut toks = Vec::new();
    let mut errs = Vec::new();
    loop {
        let t = lexer.next_token(&mut errs);
        let is_eof = t.tok == Tok::Eof;
        toks.push(t);
        if is_eof {
            break;
        }
    }
    (toks, errs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<Tok> {
        let (toks, errs) = tokenize(s);
        assert!(errs.is_empty(), "unexpected errors: {errs:?}");
        toks.into_iter().filter(|t| t.tok != Tok::Eof).map(|t| t.tok).collect()
    }

    #[test]
    fn empty_is_eof() {
        let (toks, errs) = tokenize("");
        assert!(errs.is_empty());
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].tok, Tok::Eof);
    }

    #[test]
    fn idents_and_keywords() {
        assert_eq!(kinds("let x = 1;"), vec![Tok::Keyword, Tok::Ident, Tok::Eq, Tok::Int, Tok::Semi]);
        assert_eq!(kinds("letx"), vec![Tok::Ident]); // keyword prefix is still an ident
        for kw in KEYWORDS {
            assert_eq!(kinds(kw), vec![Tok::Keyword], "keyword {kw} not recognized");
        }
    }

    #[test]
    fn operators_longest_match() {
        assert_eq!(kinds("<<="), vec![Tok::ShlEq]);
        assert_eq!(kinds(">>="), vec![Tok::ShrEq]);
        assert_eq!(kinds("<< >>"), vec![Tok::Shl, Tok::Shr]);
        assert_eq!(kinds("<<= <="), vec![Tok::ShlEq, Tok::Le]);
        assert_eq!(kinds("-> : ::"), vec![Tok::Arrow, Tok::Colon, Tok::ColonColon]);
        assert_eq!(kinds("a::b"), vec![Tok::Ident, Tok::ColonColon, Tok::Ident]);
        assert_eq!(kinds("+= -= *= /= %= &= |= ^="),
            vec![Tok::PlusEq, Tok::MinusEq, Tok::StarEq, Tok::SlashEq, Tok::PercentEq, Tok::AmpEq, Tok::PipeEq, Tok::CaretEq]);
        assert_eq!(kinds("&& || == != <= >="),
            vec![Tok::AndAnd, Tok::OrOr, Tok::EqEq, Tok::Ne, Tok::Le, Tok::Ge]);
    }

    #[test]
    fn integer_literals() {
        assert_eq!(kinds("123 0x1F 0b1010"), vec![Tok::Int, Tok::Int, Tok::Int]);
        assert_eq!(kinds("1_000"), vec![Tok::Int]);
        let (toks, _) = tokenize("0x1F");
        assert_eq!(toks[0].text("0x1F"), "0x1F");
    }

    #[test]
    fn float_literals() {
        assert_eq!(kinds("1.5 2e10 3.14e-2"), vec![Tok::Float, Tok::Float, Tok::Float]);
    }

    #[test]
    fn range_vs_float() {
        // `1..8` must be Int DotDot Int, not a float `1.`
        assert_eq!(kinds("1..8"), vec![Tok::Int, Tok::DotDot, Tok::Int]);
        assert_eq!(kinds("0..64"), vec![Tok::Int, Tok::DotDot, Tok::Int]);
    }

    #[test]
    fn comments_are_trivia() {
        assert_eq!(kinds("a // line comment\nb /* block */ c"),
            vec![Tok::Ident, Tok::Ident, Tok::Ident]);
    }

    #[test]
    fn strings_and_escapes() {
        let (toks, errs) = tokenize("\"he\\\"llo\"");
        assert!(errs.is_empty());
        assert_eq!(toks[0].tok, Tok::Str);
    }

    #[test]
    fn error_unterminated_string() {
        let (_, errs) = tokenize("\"oops");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("unterminated string"));
    }

    #[test]
    fn error_unexpected_char() {
        let (_, errs) = tokenize("1 @ 2");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("unexpected character"));
        assert_eq!(errs[0].span.lo, 2);
    }

    #[test]
    fn error_unterminated_block_comment() {
        let (_, errs) = tokenize("a /* never ends");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("unterminated block comment"));
    }
}
