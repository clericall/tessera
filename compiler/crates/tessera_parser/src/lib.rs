//! Tessera parser: recursive-descent + precedence climbing over the lexer's
//! token stream. Produces a [`Module`] plus a list of recoverable
//! [`ParseError`]s (never panics; best-effort recovery to the next item).

use tessera_ast::*;
use tessera_lexer::{tokenize, Span, Tok, Token};

pub use tessera_ast::{AssignOp, BinOp, UnaryOp};

#[derive(Debug, Clone)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
    pub help: Option<String>,
}

struct Parser<'a> {
    toks: &'a [Token],
    src: &'a str,
    pos: usize,
    last_hi: usize,
    errors: Vec<ParseError>,
}

enum Infix {
    Bin(BinOp),
    Assign(AssignOp),
}

impl<'a> Parser<'a> {
    fn new(toks: &'a [Token], src: &'a str) -> Self {
        Parser { toks, src, pos: 0, last_hi: 0, errors: Vec::new() }
    }

    // -- token cursor ------------------------------------------------------
    fn tok(&self) -> Tok {
        self.toks[self.pos].tok
    }
    fn span(&self) -> Span {
        self.toks[self.pos].span
    }
    fn text(&self, sp: Span) -> &'a str {
        &self.src[sp.lo..sp.hi]
    }
    fn peek_text(&self) -> &'a str {
        self.text(self.span())
    }
    fn at(&self, t: Tok) -> bool {
        self.toks[self.pos].tok == t
    }
    fn at_kw(&self, s: &str) -> bool {
        self.toks[self.pos].tok == Tok::Keyword && self.peek_text() == s
    }
    fn tok2(&self) -> Tok {
        self.toks.get(self.pos + 1).map(|t| t.tok).unwrap_or(Tok::Eof)
    }
    fn tok3(&self) -> Tok {
        self.toks.get(self.pos + 2).map(|t| t.tok).unwrap_or(Tok::Eof)
    }
    fn eat(&mut self, t: Tok) -> bool {
        if self.at(t) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn bump(&mut self) -> Span {
        let sp = self.toks[self.pos].span;
        self.last_hi = sp.hi;
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        sp
    }

    fn err(&self, msg: &str, span: Span) -> ParseError {
        ParseError { span, message: msg.to_string(), help: None }
    }
    fn describe(&self) -> String {
        let t = self.tok();
        let s = self.peek_text();
        match t {
            Tok::Eof => "end of file".into(),
            Tok::Keyword => format!("keyword `{s}`"),
            Tok::Ident => format!("identifier `{s}`"),
            Tok::Int => format!("integer `{s}`"),
            Tok::Float => format!("float `{s}`"),
            Tok::Str => "string literal".into(),
            _ => format!("`{s}`"),
        }
    }
    fn expect(&mut self, t: Tok, what: &str) -> Result<Span, ParseError> {
        if self.at(t) {
            Ok(self.bump())
        } else {
            Err(self.err(&format!("expected {what}, found {}", self.describe()), self.span()))
        }
    }
    fn expect_kw(&mut self, s: &str, what: &str) -> Result<(), ParseError> {
        if self.at_kw(s) {
            self.bump();
            Ok(())
        } else {
            Err(self.err(&format!("expected {what}, found {}", self.describe()), self.span()))
        }
    }
    fn expect_ident(&mut self) -> Result<String, ParseError> {
        if self.at(Tok::Ident) {
            let sp = self.span();
            self.bump();
            Ok(self.text(sp).to_string())
        } else {
            Err(self.err(&format!("expected identifier, found {}", self.describe()), self.span()))
        }
    }
    fn fin(&self, kind: ExprKind, start: Span) -> Expr {
        Expr { kind, span: Span::new(start.lo, self.last_hi) }
    }

    fn decode_char(&self, raw: &str) -> i64 {
        let b = raw.as_bytes();
        if b.len() < 3 {
            return 0;
        }
        let inner = &raw[1..raw.len() - 1];
        if inner.as_bytes()[0] == b'\\' {
            let e = if inner.len() >= 2 { inner.as_bytes()[1] } else { 0 };
            return match e {
                b'n' => 10,
                b't' => 9,
                b'r' => 13,
                b'0' => 0,
                b'\\' => 92,
                b'\'' => 39,
                _ => e as i64,
            };
        }
        inner.bytes().next().unwrap_or(0) as i64
    }

    fn peek_is_ctrl(&self) -> bool {
        self.at_kw("if") || self.at_kw("while") || self.at_kw("for") || self.at_kw("loop") || self.at(Tok::LBrace)
    }

    // -- module ------------------------------------------------------------
    fn parse_module(&mut self) -> Module {
        let mut items = Vec::new();
        loop {
            while self.eat(Tok::Semi) {}
            if self.at(Tok::Eof) {
                break;
            }
            match self.parse_item() {
                Ok(it) => items.push(it),
                Err(e) => {
                    self.errors.push(e);
                    self.recover_item();
                }
            }
        }
        Module { items }
    }

    fn recover_item(&mut self) {
        if !self.at(Tok::Eof) {
            self.bump();
        }
        while !self.at(Tok::Eof) {
            if self.at(Tok::RBrace) {
                break;
            }
            if self.at(Tok::Keyword) && is_item_start(self.peek_text()) {
                break;
            }
            self.bump();
        }
    }

    fn parse_item(&mut self) -> Result<Item, ParseError> {
        let start = self.span();
        if self.at_kw("const") {
            self.bump();
            if self.at_kw("fn") {
                return self.parse_fn(start, true);
            }
            let name = self.expect_ident()?;
            let ty = if self.eat(Tok::Colon) { Some(self.parse_type()?) } else { None };
            self.expect(Tok::Eq, "`=`")?;
            let value = self.parse_expr(0)?;
            self.expect(Tok::Semi, "`;`")?;
            return Ok(Item { kind: ItemKind::Const { name, ty, value }, span: Span::new(start.lo, self.last_hi) });
        }
        if self.at_kw("static") {
            self.bump();
            let name = self.expect_ident()?;
            self.expect(Tok::Colon, "`:`")?;
            let ty = self.parse_type()?;
            self.expect(Tok::Eq, "`=`")?;
            let value = self.parse_expr(0)?;
            self.expect(Tok::Semi, "`;`")?;
            return Ok(Item { kind: ItemKind::Static { name, ty, value }, span: Span::new(start.lo, self.last_hi) });
        }
        if self.at_kw("fn") {
            return self.parse_fn(start, false);
        }
        if self.at_kw("struct") {
            return self.parse_struct(start);
        }
        if self.at_kw("enum") {
            return self.parse_enum(start);
        }
        if self.at_kw("type") {
            self.bump();
            let name = self.expect_ident()?;
            self.expect(Tok::Eq, "`=`")?;
            let ty = self.parse_type()?;
            self.expect(Tok::Semi, "`;`")?;
            return Ok(Item { kind: ItemKind::TypeAlias { name, ty }, span: Span::new(start.lo, self.last_hi) });
        }
        if self.at_kw("import") {
            self.bump();
            let sp = self.span();
            if !self.at(Tok::Str) {
                return Err(self.err("expected a string path after `import`", sp));
            }
            self.bump();
            // The string token's text still carries its quotes; the module path
            // stored in the AST is the bare path.
            let path = self.text(sp).trim_matches('"').to_string();
            self.expect(Tok::Semi, "`;`")?;
            return Ok(Item { kind: ItemKind::Import { path }, span: Span::new(start.lo, self.last_hi) });
        }
        Err(self.err("expected an item (`fn`, `struct`, `enum`, `const`, `static`, `type`, `import`)", start))
    }

    fn parse_fn(&mut self, start: Span, is_const: bool) -> Result<Item, ParseError> {
        self.bump(); // fn
        let name = self.expect_ident()?;
        self.expect(Tok::LParen, "`(`")?;
        let params = self.parse_params()?;
        let ret = if self.eat(Tok::Arrow) { Some(self.parse_type()?) } else { None };
        let body = self.parse_block()?;
        Ok(Item {
            kind: ItemKind::Fn(FnDef { name, params, ret, body, is_const }),
            span: Span::new(start.lo, self.last_hi),
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut v = Vec::new();
        if self.eat(Tok::RParen) {
            return Ok(v);
        }
        loop {
            let name = self.expect_ident()?;
            self.expect(Tok::Colon, "`:`")?;
            let ty = self.parse_type()?;
            v.push(Param { name, ty });
            if self.eat(Tok::Comma) {
                if self.at(Tok::RParen) {
                    self.bump();
                    break;
                }
                continue;
            }
            self.expect(Tok::RParen, "`)`")?;
            break;
        }
        Ok(v)
    }

    fn parse_struct(&mut self, start: Span) -> Result<Item, ParseError> {
        self.bump(); // struct
        let name = self.expect_ident()?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut fields = Vec::new();
        loop {
            if self.eat(Tok::RBrace) {
                break;
            }
            let fs = self.span();
            let fname = self.expect_ident()?;
            self.expect(Tok::Colon, "`:`")?;
            let ty = self.parse_type()?;
            fields.push(Field { name: fname, ty, span: Span::new(fs.lo, self.last_hi) });
            if self.eat(Tok::Comma) {
                continue;
            }
            if self.at(Tok::RBrace) {
                self.bump();
                break;
            }
            return Err(self.err("expected `,` or `}` in struct", self.span()));
        }
        Ok(Item { kind: ItemKind::Struct(StructDef { name, fields }), span: Span::new(start.lo, self.last_hi) })
    }

    fn parse_enum(&mut self, start: Span) -> Result<Item, ParseError> {
        self.bump(); // enum
        let name = self.expect_ident()?;
        self.expect(Tok::LBrace, "`{`")?;
        let mut variants = Vec::new();
        loop {
            if self.eat(Tok::RBrace) {
                break;
            }
            let vs = self.span();
            let vname = self.expect_ident()?;
            let value = if self.eat(Tok::Eq) { Some(self.parse_expr(0)?) } else { None };
            variants.push(Variant { name: vname, value, span: Span::new(vs.lo, self.last_hi) });
            if self.eat(Tok::Comma) {
                continue;
            }
            if self.at(Tok::RBrace) {
                self.bump();
                break;
            }
            return Err(self.err("expected `,` or `}` in enum", self.span()));
        }
        Ok(Item { kind: ItemKind::Enum(EnumDef { name, variants }), span: Span::new(start.lo, self.last_hi) })
    }

    // -- types -------------------------------------------------------------
    fn parse_type(&mut self) -> Result<Ty, ParseError> {
        let start = self.span();
        match self.tok() {
            Tok::Amp => {
                self.bump();
                let ty = self.parse_type()?;
                Ok(Ty::Ref { ty: Box::new(ty), span: Span::new(start.lo, self.last_hi) })
            }
            Tok::Star => {
                self.bump();
                let ty = self.parse_type()?;
                Ok(Ty::Ptr { ty: Box::new(ty), span: Span::new(start.lo, self.last_hi) })
            }
            Tok::LBracket => {
                self.bump();
                if self.eat(Tok::RBracket) {
                    let elem = self.parse_type()?;
                    Ok(Ty::Slice { elem: Box::new(elem), span: Span::new(start.lo, self.last_hi) })
                } else {
                    let elem = self.parse_type()?;
                    self.expect(Tok::Semi, "`;` in array type `[T; N]`")?;
                    let len = self.parse_expr(0)?;
                    self.expect(Tok::RBracket, "`]`")?;
                    Ok(Ty::Array { elem: Box::new(elem), len: Box::new(len), span: Span::new(start.lo, self.last_hi) })
                }
            }
            Tok::Ident => {
                let segs = self.parse_path_segs()?;
                Ok(Ty::Named { segs, span: Span::new(start.lo, self.last_hi) })
            }
            _ => Err(self.err(&format!("expected a type, found {}", self.describe()), start)),
        }
    }

    fn parse_path_segs(&mut self) -> Result<Vec<String>, ParseError> {
        let mut segs = vec![self.expect_ident()?];
        while self.at(Tok::ColonColon) {
            self.bump();
            segs.push(self.expect_ident()?);
        }
        Ok(segs)
    }

    // -- expressions -------------------------------------------------------
    pub fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        let start = self.span();
        let mut lhs = self.parse_prefix()?;
        loop {
            let (lbp, rbp, infix) = match infix_bp(self.tok()) {
                Some(x) => x,
                None => break,
            };
            if lbp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_expr(rbp)?;
            lhs = match infix {
                Infix::Bin(b) => self.fin(
                    ExprKind::Binary { op: b, lhs: Box::new(lhs), rhs: Box::new(rhs) },
                    start,
                ),
                Infix::Assign(a) => self.fin(
                    ExprKind::Assign { op: a, lhs: Box::new(lhs), rhs: Box::new(rhs) },
                    start,
                ),
            };
        }
        Ok(lhs)
    }

    fn parse_prefix(&mut self) -> Result<Expr, ParseError> {
        let start = self.span();
        let op = match self.tok() {
            Tok::Minus => Some(UnaryOp::Neg),
            Tok::Bang => Some(UnaryOp::Not),
            Tok::Tilde => Some(UnaryOp::BitNot),
            Tok::Star => Some(UnaryOp::Deref),
            Tok::Amp => Some(UnaryOp::Ref),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.parse_prefix()?;
            return Ok(self.fin(ExprKind::Unary { op, expr: Box::new(e) }, start));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_atom()?;
        loop {
            let start = e.span;
            match self.tok() {
                Tok::LParen => {
                    self.bump();
                    let args = self.parse_call_args()?;
                    e = self.fin(ExprKind::Call { callee: Box::new(e), args }, start);
                }
                Tok::LBracket => {
                    self.bump();
                    let idx = self.parse_expr(0)?;
                    self.expect(Tok::RBracket, "`]`")?;
                    e = self.fin(ExprKind::Index { base: Box::new(e), index: Box::new(idx) }, start);
                }
                Tok::Dot => {
                    self.bump();
                    let name = self.expect_ident()?;
                    e = self.fin(ExprKind::Field { base: Box::new(e), name }, start);
                }
                Tok::Keyword if self.peek_text() == "as" => {
                    self.bump();
                    let ty = self.parse_type()?;
                    e = self.fin(ExprKind::Cast { expr: Box::new(e), ty }, start);
                }
                Tok::Question => {
                    return Err(self.err("the `?` operator is not supported yet", self.span()));
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut v = Vec::new();
        if self.eat(Tok::RParen) {
            return Ok(v);
        }
        loop {
            v.push(self.parse_expr(0)?);
            if self.eat(Tok::Comma) {
                if self.at(Tok::RParen) {
                    self.bump();
                    break;
                }
                continue;
            }
            self.expect(Tok::RParen, "`)`")?;
            break;
        }
        Ok(v)
    }

    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        let start = self.span();
        match self.tok() {
            Tok::Int => {
                let sp = self.span();
                self.bump();
                Ok(self.fin(ExprKind::Lit(Lit::Int(self.text(sp).to_string())), start))
            }
            Tok::Float => {
                let sp = self.span();
                self.bump();
                Ok(self.fin(ExprKind::Lit(Lit::Float(self.text(sp).to_string())), start))
            }
            Tok::Str => {
                let sp = self.span();
                self.bump();
                Ok(self.fin(ExprKind::Lit(Lit::Str(self.text(sp).to_string())), start))
            }
            Tok::Char => {
                let sp = self.span();
                self.bump();
                Ok(self.fin(ExprKind::Lit(Lit::Char(self.decode_char(self.text(sp)))), start))
            }
            Tok::Keyword if self.at_kw("true") => {
                self.bump();
                Ok(self.fin(ExprKind::Lit(Lit::Bool(true)), start))
            }
            Tok::Keyword if self.at_kw("false") => {
                self.bump();
                Ok(self.fin(ExprKind::Lit(Lit::Bool(false)), start))
            }
            Tok::Ident => {
                let mut segs = vec![self.expect_ident()?];
                while self.at(Tok::ColonColon) {
                    self.bump();
                    segs.push(self.expect_ident()?);
                }
                // `Ident { ... }` is a struct literal only when the brace is
                // followed by `}` (empty) or `Ident :` (a field). Otherwise the
                // `{` begins a block (e.g. `if cond { ... }`), not a struct lit.
                let is_struct_lit = self.at(Tok::LBrace)
                    && (self.tok2() == Tok::RBrace
                        || (self.tok2() == Tok::Ident && self.tok3() == Tok::Colon));
                if is_struct_lit {
                    self.bump(); // consume `{`
                    let mut fields = Vec::new();
                    loop {
                        if self.eat(Tok::RBrace) {
                            break;
                        }
                        let fname = self.expect_ident()?;
                        self.expect(Tok::Colon, "`:`")?;
                        let fe = self.parse_expr(0)?;
                        fields.push((fname, fe));
                        if self.eat(Tok::Comma) {
                            continue;
                        }
                        if self.at(Tok::RBrace) {
                            self.bump();
                            break;
                        }
                        return Err(self.err("expected `,` or `}` in struct literal", self.span()));
                    }
                    return Ok(self.fin(ExprKind::StructLit { path: segs, fields }, start));
                }
                let kind = if segs.len() == 1 {
                    ExprKind::Ident(segs.pop().unwrap())
                } else {
                    ExprKind::Path(segs)
                };
                Ok(self.fin(kind, start))
            }
            Tok::Keyword if self.at_kw("if") => self.parse_if(),
            Tok::Keyword if self.at_kw("while") => self.parse_while(),
            Tok::Keyword if self.at_kw("loop") => self.parse_loop(),
            Tok::Keyword if self.at_kw("for") => self.parse_for(),
            Tok::LBrace => {
                let b = self.parse_block()?;
                Ok(self.fin(ExprKind::Block(b), start))
            }
            Tok::LParen => {
                self.bump();
                if self.eat(Tok::RParen) {
                    return Ok(self.fin(ExprKind::Unit, start));
                }
                let e = self.parse_expr(0)?;
                self.expect(Tok::RParen, "`)`")?;
                Ok(e)
            }
            Tok::LBracket => {
                // Array literal: [e0, e1, ...] OR repetition: [value; N]
                self.bump();
                if self.eat(Tok::RBracket) {
                    return Ok(self.fin(ExprKind::ArrayLit(Vec::new()), start));
                }
                let first = self.parse_expr(0)?;
                if self.eat(Tok::Semi) {
                    let len = self.parse_expr(0)?;
                    self.expect(Tok::RBracket, "`]`")?;
                    return Ok(self.fin(ExprKind::ArrayRepeat { value: Box::new(first), len: Box::new(len) }, start));
                }
                let mut items = vec![first];
                loop {
                    if self.eat(Tok::Comma) {
                        if self.at(Tok::RBracket) {
                            self.bump();
                            break;
                        }
                        items.push(self.parse_expr(0)?);
                        continue;
                    }
                    if self.at(Tok::RBracket) {
                        self.bump();
                        break;
                    }
                    return Err(self.err("expected `,` or `]` in array literal", self.span()));
                }
                Ok(self.fin(ExprKind::ArrayLit(items), start))
            }
            _ => Err(self.err(&format!("expected expression, found {}", self.describe()), start)),
        }
    }

    fn parse_if(&mut self) -> Result<Expr, ParseError> {
        let start = self.span();
        self.bump(); // if
        let cond = Box::new(self.parse_expr(0)?);
        let then = self.parse_block()?;
        let els = if self.at_kw("else") {
            self.bump();
            if self.at_kw("if") {
                Some(Box::new(self.parse_if()?))
            } else {
                let b = self.parse_block()?;
                let sp = b.span;
                Some(Box::new(Expr { kind: ExprKind::Block(b), span: sp }))
            }
        } else {
            None
        };
        Ok(self.fin(ExprKind::If { cond, then, els }, start))
    }

    fn parse_while(&mut self) -> Result<Expr, ParseError> {
        let start = self.span();
        self.bump();
        let cond = Box::new(self.parse_expr(0)?);
        let body = self.parse_block()?;
        Ok(self.fin(ExprKind::While { cond, body }, start))
    }

    fn parse_loop(&mut self) -> Result<Expr, ParseError> {
        let start = self.span();
        self.bump();
        let body = self.parse_block()?;
        Ok(self.fin(ExprKind::Loop(body), start))
    }

    fn parse_for(&mut self) -> Result<Expr, ParseError> {
        let start = self.span();
        self.bump();
        let name = self.expect_ident()?;
        self.expect_kw("in", "`in`")?;
        let iter = Box::new(self.parse_expr(0)?);
        let body = self.parse_block()?;
        Ok(self.fin(ExprKind::For { name, iter, body }, start))
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let start = self.span();
        self.expect(Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        let mut tail = None;
        loop {
            if self.at(Tok::RBrace) {
                break;
            }
            if self.at(Tok::Eof) {
                return Err(self.err("unclosed block; expected `}`", Span::new(start.lo, self.last_hi)));
            }
            while self.eat(Tok::Semi) {}
            if self.at(Tok::RBrace) {
                break;
            }
            let ss = self.span();
            if self.at_kw("let") || self.at_kw("var") {
                stmts.push(self.parse_let()?);
                self.expect(Tok::Semi, "`;`")?;
                continue;
            }
            if self.at_kw("return") {
                stmts.push(self.parse_return()?);
                self.eat(Tok::Semi);
                continue;
            }
            if self.at_kw("break") {
                self.bump();
                stmts.push(Stmt { kind: StmtKind::Break, span: Span::new(ss.lo, self.last_hi) });
                self.eat(Tok::Semi);
                continue;
            }
            if self.at_kw("continue") {
                self.bump();
                stmts.push(Stmt { kind: StmtKind::Continue, span: Span::new(ss.lo, self.last_hi) });
                self.eat(Tok::Semi);
                continue;
            }
            let blocklike = self.peek_is_ctrl();
            let e = self.parse_expr(0)?;
            if self.eat(Tok::Semi) {
                stmts.push(Stmt { kind: StmtKind::Expr(e), span: Span::new(ss.lo, self.last_hi) });
            } else if self.at(Tok::RBrace) {
                tail = Some(Box::new(e));
            } else if blocklike {
                stmts.push(Stmt { kind: StmtKind::Expr(e), span: Span::new(ss.lo, self.last_hi) });
            } else {
                return Err(self.err("expected `;` or `}`", self.span()));
            }
        }
        let rb = self.expect(Tok::RBrace, "`}`")?;
        Ok(Block { stmts, tail, span: Span::new(start.lo, rb.hi) })
    }

    fn parse_let(&mut self) -> Result<Stmt, ParseError> {
        let start = self.span();
        let mutable = if self.at_kw("var") { self.bump(); true } else { self.expect_kw("let", "`let`")?; false };
        let name = self.expect_ident()?;
        let ty = if self.eat(Tok::Colon) { Some(self.parse_type()?) } else { None };
        let init = if self.eat(Tok::Eq) { Some(self.parse_expr(0)?) } else { None };
        Ok(Stmt { kind: StmtKind::Let { name, ty, init, mutable }, span: Span::new(start.lo, self.last_hi) })
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        let start = self.span();
        self.bump();
        let init = if self.at(Tok::Semi) || self.at(Tok::RBrace) || self.at(Tok::Eof) {
            None
        } else {
            Some(self.parse_expr(0)?)
        };
        Ok(Stmt { kind: StmtKind::Return(init), span: Span::new(start.lo, self.last_hi) })
    }
}

fn is_item_start(s: &str) -> bool {
    matches!(s, "const" | "static" | "fn" | "struct" | "enum" | "type" | "import")
}

fn infix_bp(t: Tok) -> Option<(u8, u8, Infix)> {
    use Infix::{Assign, Bin};
    Some(match t {
        Tok::Star => (14, 14 + 1, Bin(BinOp::Mul)),
        Tok::Slash => (14, 14 + 1, Bin(BinOp::Div)),
        Tok::Percent => (14, 14 + 1, Bin(BinOp::Rem)),
        Tok::Plus => (13, 13 + 1, Bin(BinOp::Add)),
        Tok::Minus => (13, 13 + 1, Bin(BinOp::Sub)),
        Tok::Shl => (12, 12 + 1, Bin(BinOp::Shl)),
        Tok::Shr => (12, 12 + 1, Bin(BinOp::Shr)),
        Tok::Amp => (11, 11 + 1, Bin(BinOp::BitAnd)),
        Tok::Caret => (10, 10 + 1, Bin(BinOp::BitXor)),
        Tok::Pipe => (9, 9 + 1, Bin(BinOp::BitOr)),
        Tok::EqEq => (7, 7 + 1, Bin(BinOp::Eq)),
        Tok::Ne => (7, 7 + 1, Bin(BinOp::Ne)),
        Tok::Lt => (8, 8 + 1, Bin(BinOp::Lt)),
        Tok::Le => (8, 8 + 1, Bin(BinOp::Le)),
        Tok::Gt => (8, 8 + 1, Bin(BinOp::Gt)),
        Tok::Ge => (8, 8 + 1, Bin(BinOp::Ge)),
        Tok::AndAnd => (6, 6 + 1, Bin(BinOp::And)),
        Tok::OrOr => (5, 5 + 1, Bin(BinOp::Or)),
        Tok::DotDot => (4, 4 + 1, Bin(BinOp::Range)),
        Tok::Eq => (2, 1, Assign(AssignOp::Set)),
        Tok::PlusEq => (2, 1, Assign(AssignOp::Add)),
        Tok::MinusEq => (2, 1, Assign(AssignOp::Sub)),
        Tok::StarEq => (2, 1, Assign(AssignOp::Mul)),
        Tok::SlashEq => (2, 1, Assign(AssignOp::Div)),
        Tok::PercentEq => (2, 1, Assign(AssignOp::Rem)),
        Tok::AmpEq => (2, 1, Assign(AssignOp::BitAnd)),
        Tok::PipeEq => (2, 1, Assign(AssignOp::BitOr)),
        Tok::CaretEq => (2, 1, Assign(AssignOp::BitXor)),
        Tok::ShlEq => (2, 1, Assign(AssignOp::Shl)),
        Tok::ShrEq => (2, 1, Assign(AssignOp::Shr)),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Modules: `import "file.ter";`
//
// Imports are a textual include with include-once semantics: the imported file
// is parsed in its own file context (so its diagnostics point at the right line)
// and every file is spliced in exactly once, which also breaks import cycles.
// ---------------------------------------------------------------------------

/// A diagnostic that knows the file (and source text) it came from, so an error
/// inside an imported module is still reported at the right line.
#[derive(Debug, Clone)]
pub struct FileDiagnostic {
    pub path: String,
    pub src: String,
    pub err: ParseError,
}

/// Parse `path` and splice in everything it imports, recursively.
///
/// Returns the merged module, the diagnostics for `path` itself (spans refer to
/// its own source, so existing renderers keep working) and the diagnostics of
/// every imported file.
pub fn parse_file(path: &std::path::Path) -> (Module, Vec<ParseError>, Vec<FileDiagnostic>) {
    let mut loaded: Vec<std::path::PathBuf> = Vec::new();
    let mut imported: Vec<FileDiagnostic> = Vec::new();
    let items = load_items(path, &mut loaded, &mut imported, None);
    (Module { items }, Vec::new(), imported)
}

fn load_items(
    path: &std::path::Path,
    loaded: &mut Vec<std::path::PathBuf>,
    imported: &mut Vec<FileDiagnostic>,
    via: Option<(&Item, &str, &str)>,
) -> Vec<Item> {
    let display = path.display().to_string();
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            // Report a failed read against the `import` line of the file that
            // asked for the module -- pointing at the missing file helps nobody.
            let (err_path, err_src, span) = match via {
                Some((item, parent_path, parent_src)) => {
                    (parent_path.to_string(), parent_src.to_string(), item.span)
                }
                None => (display.clone(), String::new(), Span::new(0, 0)),
            };
            let message = match via {
                Some(_) => format!("cannot read imported module `{display}`: {e}"),
                None => format!("cannot read `{display}`: {e}"),
            };
            imported.push(FileDiagnostic {
                path: err_path,
                src: err_src,
                err: ParseError {
                    span,
                    message,
                    help: Some("import paths are relative to the importing file".into()),
                },
            });
            return Vec::new();
        }
    };

    // Include-once: a second import of the same file contributes nothing, and a
    // cycle terminates instead of recursing forever.
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if loaded.iter().any(|p| p == &key) {
        return Vec::new();
    }
    loaded.push(key);

    let (module, mut errs) = parse(&src);
    for err in errs.drain(..) {
        imported.push(FileDiagnostic { path: display.clone(), src: src.clone(), err });
    }

    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut out = Vec::new();
    for item in module.items {
        match &item.kind {
            ItemKind::Import { path: rel } => {
                let target = if dir.as_os_str().is_empty() {
                    std::path::PathBuf::from(rel)
                } else {
                    dir.join(rel)
                };
                let sub = load_items(&target, loaded, imported, Some((&item, &display, &src)));
                out.extend(sub);
            }
            _ => out.push(item),
        }
    }
    out
}

/// Parse a whole source file into a module, collecting recoverable errors.
pub fn parse(src: &str) -> (Module, Vec<ParseError>) {
    let (toks, _lex) = tokenize(src);
    let mut p = Parser::new(&toks, src);
    let module = p.parse_module();
    (module, p.errors)
}

/// Convenience for tests: parse a single expression and flag trailing tokens.
pub fn parse_expr_str(src: &str) -> (Option<Expr>, Vec<ParseError>) {
    let (toks, _lex) = tokenize(src);
    let mut p = Parser::new(&toks, src);
    match p.parse_expr(0) {
        Ok(e) => {
            let mut errs = Vec::new();
            if !p.at(Tok::Eof) {
                errs.push(ParseError { span: p.span(), message: "unexpected trailing tokens".into(), help: None });
            }
            (Some(e), errs)
        }
        Err(e) => {
            p.errors.push(e);
            (None, p.errors)
        }
    }
}
