# Milestone M10 — File I/O builtins (NNUE prerequisite, phase 0)

The chess engine could not read a file, so no network could ever be loaded. This milestone adds the
minimum binary-read surface needed to stream a `.nnue` weight file at startup.

## New builtins

| builtin | signature | behaviour |
|---|---|---|
| `fopen` | `fopen(path: str) -> i64` | handle, or `-1` on failure |
| `fread_u8` | `fread_u8(h: i64) -> i64` | `0..255`, `-1` at EOF |
| `fread_i16` | `fread_i16(h: i64) -> i64` | sign-extended little-endian `i16` |
| `fread_i32` | `fread_i32(h: i64) -> i64` | sign-extended little-endian `i32` |
| `feof` | `feof(h: i64) -> bool` | true when exhausted, or on a bad handle |
| `fclose` | `fclose(h: i64) -> i64` | `0` on success |

`i16` is the width Stockfish-style networks actually use, and Tessera already had `i8`/`i16`/`u16`
array types, so no fixed-point emulation is needed. EOF is `-1` *and* `feof`, because `-1` is a
legitimate `i16`/`i32` value.

Implementation: C helpers in the backend `RUNTIME` block (FILE* is passed through an `int64_t`
handle), `emit_call` mappings, and typed arms in the checker.

## Two compiler bugs found while testing this

1. **Wrong-arity builtin calls panicked the compiler.** Every builtin arm did
   `self.expect_arity(&name, args, 1); self.check_expr(&args[0], ...)`, so `feof()` produced
   `index out of bounds: the len is 0 but the index is 0` — a crash, not a diagnostic. `expect_arity`
   now returns `bool` and all 11 call sites early-return `unit_expr` after reporting.
2. **`tesserac check` never reported type errors.** `run_check` discarded the checker's error list
   (`let (prog, _) = check(&module)`) and unconditionally printed "typechecked OK", so arity errors
   were invisible from the CLI. It now renders `E0002`/`E0003` diagnostics and exits non-zero like
   `build` does.

## Tests

- `compiler/crates/tessera_check/tests/builtins.rs` (new, 5 tests): correct usage type-checks;
  `fopen(7)`, `fread_i16(1, 2)` and `feof()` are rejected; `feof` works in a boolean context.
  This required adding `tessera_parser` as a dev-dependency of `tessera_check`.
- `compiler/samples/binio.ter` + `compiler/samples/binio_fixture.bin`: end-to-end read of a 32-byte
  little-endian fixture covering `i16` extremes (`32767`, `-32768`) and `i32` values:

  ```
  loaded=8
  300 -300 0 32767 -32768 1234 -1234 7 sum=6
  123456 -123456 0 65535 bsum=65535
  ```
  A missing path returns `loaded=-1` and the program degrades gracefully.

## Status

Full suite green (37 tests), engine still perft-exact at depth 4 on all six reference positions.
This milestone is a prerequisite only — no NNUE inference exists yet.
