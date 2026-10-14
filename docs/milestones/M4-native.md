# Milestone C3+C4 — Type checker, C backend, first native compile (DONE)

**What works (measured this session):**
- `tesserac check` (type checker + typed IR, crate `crates/tessera_check`),
  `tesserac build`/`run` (C backend `crates/tessera_backend_c` + clang, driver
  `crates/tesserac/src/main.rs`).
- Full pipeline `source -> lex -> parse -> typecheck -> typed IR -> C -> clang -> exe -> run`.
- LLVM/clang 23.1.3 installed **without admin** (portable tarball extracted via Python `lzma`)
  at `%USERPROFILE%\tools\clang+llvm-23.1.3-x86_64-pc-windows-msvc`; `tesserac` auto-finds it
  (override with `TESSERA_CLANG`). clang links MSVC-target C out of the box here.
- Verified end-to-end: `tesserac run samples/pop.ter` -> `popcount(0xF0F0) =` / `8`;
  `tesserac run samples/fib.ter` -> `fib =` / `89`. Generated C is readable and correct.
- Tests: **29 green** (11 lexer, 13 parser, 4 golden, 1 backend). Bit builtins lower to
  `__builtin_popcountll/ctzll/clzll`.

## What does NOT work yet (honest)
- **Slice is limited:** functions + integer/bool arithmetic + loops + calls + `popcount/ctz/clz`
  + `print_*`. **No structs, enums, fixed-array indexing, slices, pointers/refs, static globals,
  floats, `const fn` evaluation, or string values.** Those are required before the chess board
  (bitboards are fine as `u64`, but we need arrays, static tables, and FEN string handling).
- Single C backend only; no LLVM (inkwell) backend yet. No explicit 3-address TIR (the typed IR is
  close to the AST — honest note). Emits a benign redundant trailing `return`.
- No tiering (SSE2/AVX2/BMI2) or NNUE SIMD yet.

## How to run
```
cd compiler
cargo build
target\debug\tesserac.exe run samples\pop.ter
# clang path optional: setx TESSERA_CLANG "...\bin\clang.exe"
```

## Honest status vs the engine
The compiler can now compile and run native code, but only a small language subset. The chess
engine (Part 2) cannot start until the slice grows to include structs/arrays/static/const-eval/
strings. Stockfish-strength remains a stretch goal far beyond this point.
