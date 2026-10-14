# Bitboard SEARCH engine (E2 integrated) — 2026-10-14

Wired the validated bitboard movegen into a full searching UCI engine (`engine/bb.ter`,
`id name Tessera 0.2 (bitboards)`): incremental zobrist `key` maintained in `make`, PST-lite eval,
MVV-LVA ordering (O(n) capture-partition), quiescence, negamax with a 2^18 TT, iterative deepening,
time control, UCI I/O.

## Measured (single thread)
- Correctness: mate-in-1 `6k1/5ppp/8/8/8/8/8/4R1K1 w` -> `bestmove e1e8`, `score cp 99999`;
  queen-up -> `+900`. Movegen unchanged from the perft-exact bb core.
- **Search nps (bench, fixed FEN set):**
  - Tier C (AVX2/BMI2, pext), bench 6: **~1,173,210 nps** (71.5M nodes / 61 s).
  - Tier A (SSE2, gather), bench 5: **~931,036 nps**; Tier B: **~953,107 nps** (88.4M nodes / ~94 s).
  => ~9x the old 10x10 mailbox search (125 k nps), and raw-faster than the tier-matched
  Stockfish 19's bench (~469 k nps) — though SF does far more work per node.

## Honest finding: speed is fixed; EVAL is now the bottleneck
- fastchess vs tier-matched Stockfish (5+0.05, 1t/16MB): **Tessera 0 - Stockfish 8** (0 points).
  Despite searching faster, it loses because its eval is **material + a tiny centralization term**,
  while Stockfish uses a mature NNUE. More nps at a fixed time buys depth but cannot close an
  evaluation gap this size.
- Breather data point: a key early bug was O(n^2) selection-sort ordering per node (search stuck
  at 142 k nps); the O(n) capture-partition took it to ~1.17M. So the profile was ordering, not the
  generator.

## Implication / next lever
Strength must come from **evaluation** (proper piece-square tables, then a small NNUE), not more
search speed. Bits: PST tuning, then an int8 NNUE via the language's SIMD types (network must be
small enough for L2/L3; report bytes touched/eval).

## Reproduce
```
cd compiler
target\debug\tesserac.exe build ..\engine\bb.ter -o samples/bbuci.exe --tier c
samples/bbuci.exe    # then: bench 6 / position startpos / go depth 6 / quit
```
