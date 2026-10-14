# Milestone C1 — Lexer (DONE)

**Goal:** source text → token stream with byte spans; readable line/col diagnostics.

## What works
- Full token set from the spec: literals (int dec/hex/bin, float, string), identifiers,
  keywords, all operators (longest-match: `<<=`, `>>=`, `==`, `->`, `::`, `..`, `+=`, …),
  and grouping/delimiters. Eof-terminated stream.
- Comments (`//` and non-nested `/* */`) as trivia; whitespace incl. newlines (not significant).
- `for … in` now uses keyword `in`.
- `tessera_diag::SourceFile`: line/col mapping + caret rendering (`--> file:line:col`).
- `tesserac lex <file>` CLI prints `line:col KIND 'text'`, or diagnostics + `exit 1`.

## Tests
- 11 lexer unit tests + 4 golden/integration tests = **15 passing** (`cargo test`).
- Covers: empty/EOF, idents-vs-keywords, longest-match operators, int/float, `1..8` vs `1.5`,
  comments, strings/escapes, unterminated string, unexpected char (span check),
  unterminated block comment, line/col across lines, error column, language-surface sample.

## What does NOT work yet (honest)
- No parser/AST/typechecker yet — lexer only.
- Attributes `#[...]` are not lexable (`#` is an unexpected char) — deferred, not needed for M1.
- `asm!(...)`, tuple types, and `Option`/`?` are recognized as out-of-scope for now.
- Big-int literals are kept as raw text; value parsing is deferred to the typechecker.
- No C/LLVM backend yet (needs clang — LLVM is being extracted).

## How to run
```
cd compiler
cargo build
cargo test
target\debug\tesserac.exe lex samples\ok.ter
target\debug\tesserac.exe lex samples\bad.ter
```
