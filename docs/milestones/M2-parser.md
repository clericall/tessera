# Milestone C2 — Parser (DONE)

**Goal:** tokens -> AST via recursive descent + precedence climbing; recoverable, non-panicking errors.

## What works
- Items: `const` (incl. `const fn`), `static`, `fn`, `struct`, `enum`, `type` alias, `import`.
- Types: named/path (`u64`, `Color`, `MyStruct`), `&T`, `*T`, `[T; N]`, `[]T`.
- Expressions: literals (int/float/str/bool/unit `()`), idents, paths `a::b`,
  prefix unary `- ! ~ * &`, postfix call/field/index, full binary-operator ladder with
  correct precedence (assignment right-assoc, `..` low), compound assignment, ranges.
- Control flow as expressions: `if/else if/else`, `while`, `loop`, `for x in <expr>`.
- Statements: `let`/`var` (typed or inferred, optional init), `return`, `break`,
  `continue`, expression statements, nested blocks; block-like statements need no
  trailing `;` mid-block; last expression is the block tail.
- Error recovery: a broken item is reported and parsing continues with the next item.

## Tests
- 13 parser tests (in `crates/tessera_parser/tests/parse_tests.rs`) all pass; covers
  precedence, associativity, unary/postfix, ranges, paths, sample-file structure (5 items),
  blocklike statements, trailing commas, error cases + recovery.
- Total workspace tests green: **28** (11 lexer + 13 parser + 4 golden).

## What does NOT work yet (honest)
- No type checking/name resolution — accepts syntactically-valid-but-ill-typed programs.
- Block-like expressions are not captured as a block's *tail* value in all positions yet.
- Deferred: attributes `#[...]`, `asm!`, tuples, `Option`/`?` (lex these later).
- Error recovery is item-level; multi-error within a block aborts that item.
- No typed IR / backends yet (front-end only).

## How to run
```
cd compiler
cargo test
target\debug\tesserac.exe parse samples\ok.ter
```

## Status vs. the engine goal (honest)
We're 2 compiler milestones in (lexer, parser). Before the chess engine (Part 2) can run we
still need: type checking, typed IR, the C backend + clang wired in end-to-end, and only then
can `board/FEN/make-unmake` exist as Tessera code. The Stockfish-strength goal remains a
stretch goal behind all of that.
