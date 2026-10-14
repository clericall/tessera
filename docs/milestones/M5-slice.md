# Milestone C5 — Slice expansion for the engine (DONE)

**Newly supported (all validated to native via clang):**
- **Structs** (`struct Name { ... }`), struct literals, field read, field assign.
- **Fixed arrays** `[T; N]` (as locals/fields), array literals, index read, index assign.
- **Nested place assignment**: `x = e`, `s.f = e`, `a[i] = e`, `s.p[i] = e` (LValue grammar).
- **Functions taking/returning structs by value** (C struct-by-value ABI; engine uses
  `make_move` returning a modified `Position`, i.e. functional make/unmake — no refs needed yet).
- **Static globals** `static NAME: T = const;` (scalars + const arrays), read + mutate
  (this is the home for zobrist keys / magic tables / TT later).

**Verified samples (via `tesserac run`):**
- `pop.ter` -> `8`; `fib.ter` -> `89`; `structs.ter` -> `19,13,2,44`; `statics.ter` -> `7,5,8,15,8`.
- Tests: **29 green**. Emitted C shows `typedef struct {...} Tr_Name;`, `uint64_t pieces[2];`,
  and correct nested lvalues like `((v0).pieces)[0] = ...`.

## What does NOT work yet (honest)
- **FEN parsing** needs string/char *values* + pointers (`&[u8]`); for E1 I will construct
  test positions programmatically first, then add string/char values + FEN.
- **Magic tables**: `static` initializers must be compile-time constants (literals) for now;
  real magic/attack tables will be **runtime-initialized** for E1 and moved to **compile-time
  `const fn`** generation in M6 (so Tier A/B use classic magics, Tier C pext).
- No pointers/refs (`&Position`), slices, floats, generics, LLVM backend, CRC, threading.
- No explicit 3-address TIR (typed IR is near-AST); redundant trailing `return` emitted.

## Status
The compiler can now express and run struct/array/static code natively — enough to begin the
chess engine. Next: **engine E1** (bitboard `Board`, make/unmake, magic movegen, perft vs
known node counts).
