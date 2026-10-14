# Milestone E1 — Chess move generation + perft (DONE, validated)

**Language slice used:** structs, fixed arrays `[T; N]` + indexing (read/write), nested
place assignment (`c.mail[i] = x`), `while` loops, recursion, const array lengths, `static`
globals, function passing/returning structs by value.

**Engine (`engine/e1.ter`):** 10x10 mailbox board, all six pieces, castling, en-passant,
promotion, check legality (via `attacked()` + post-move king safety), copy-make make
(`make(pos, mv) -> Position`), and copy-make perft (`gperft`).

**Validated perft (Tessera -> C -> clang -> native, `tesserac run`):**

| Position | depth 1 | 2 | 3 | 4 | 5 | vs reference |
|---|---|---|---|---|---|---|
| startpos | 20 | 400 | 8902 | 197281 | 4865609 | exact |
| Kiwipete  | 48 | 2039 | 97862 | 4085603 | — | exact |
| position 3| 14 | 191 | 2812 | 43238 | 674624 | exact |

M.K. positions per the standard perft suite; promotions and en-passant exercised by pos3.

## What this proves (measured)
The compiler compiles a non-trivial ~230-line chess program to correct native code; move
generation is legal-move-correct to the tested depths across castling/promotion/e.p.

## Known limitations / not done yet (honest)
- Positions are built **programmatically**; **FEN parsing needs string/char values + pointer
  support** (next language addition, E1c).
- **10x10 mailbox generator, NOT magic bitboards.** Speed not measured yet; a fast magic/pext
  generator (classic magics on Tier A/B, pext on Tier C) comes next, behind this correctness
  baseline.
- **copy-make** (copy the board each node): correct first; an in-place make/unmake (and later
  refs) is a performance step.
- Perft speed / nodes-per-second not yet benchmarked.

## Reproduce
```
cd compiler
cargo build
target\debug\tesserac.exe run ..\engine\e1.ter
```
