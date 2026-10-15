# Milestone M11 — Tiny NNUE inference end to end

The engine can now load a weight file and evaluate with a real (fixed-point) neural network.
The weights are **not trained** — this milestone proves the inference path, not the strength.

## Architecture (format v1, `.tnn`)

```
input features : 768  = 12 (piece type x colour block) x 64 squares, side-to-move perspective
hidden layer   : NH = 32, i16 weights, i16-clipped accumulator
output         : clip_i16(sum(acc * w2) + sum(psqt)) * scale / 64
views          : two transformer tables (white view / black view), 768*NH i16 each
file           : magic "TNNU", version, hsize, scale, then sequential i16 blocks
```

Feature index: `idx = (own ? pt-1 : 6+(pt-1)) * 64 + (enemy ? 63-sq : sq)`.
The same set of pieces therefore produces *different* indices for the two views, which is why
`Pos` carries two accumulators (`acc[0..32]` white view, `acc[32..64]` black view) and `make()`
edits each view at its own indices.

## Engine changes

- `Pos` gained `acc: [i16; 64]`. Copy-make keeps both accumulators in sync for free.
- `make()` queues up to 8 feature edits (`feat`, sign, view) and applies them once: quiet moves,
  captures, en-passant (both colours), promotions/underpromotions, and **both castlings** (the
  rook is a second piece that moves, and its edit happens inside the castling branch).
- `from_fen_bb()` seeds the accumulators from `bias` plus one pass over the board.
- `nnue_eval()` is used by `eval_pos_bb()` when a net is loaded; the hand-crafted eval remains
  available as `hce_eval()` and can be switched back with `setoption name UseNNUE value false`.
- UCI: `option name EvalFile type string`, plus `Threads`/`Hash` stubs so GUIs stop warning.
  `ucinewgame` now clears the TT.

## Verification

`eval` prints the incremental score, a from-scratch recomputation of the same position, and whether
the 64 accumulator slots match. Across 12 scripted sequences — quiet moves, both castlings, rook
moves after castling, en passant for both colours, promotion, underpromotion, a 20-ply capture
chain, and both Kiwipete lines — **all report `accok=yes`**.

Two real bugs were caught this way:
1. Every queued edit was applied to *both* accumulators even though the two views use different
   feature indices — so any move corrupted the other view's accumulator. Fixed by tagging each
   edit with its view.
2. Castling updated the bitboards after the accumulator edit, so the rook's move was never
   recorded.

Perft is unchanged and exact (depth 5: startpos 4,865,609 / Kiwipete 193,690,690).

## Language/compiler work this needed

- `substr(s: str, start: i64, len: i64) -> str` — to lift a path out of a `setoption` line.
- `fopen` now takes a length-aware `str` (copies into a NUL-terminated buffer) instead of relying
  on a C string literal, so `fopen(substr(...))` works.
- Found while testing: `option` lines must be printed **before** `uciok`. fastchess stops reading
  engine output at `uciok`, so options printed afterwards were never registered (this is why
  Stockfish prints them first too).

## Status and honest expectations

- `nnue_init <path> [hsize] [seed]` writes a deterministic **placeholder** net (xorshift weights).
  A random net saturates the accumulator, so scores peg at the +-8000 clamp and the engine plays
  near-random moves. **No strength claim is possible from this.**
- Speed cost: `Pos` grew from 128 to 256 bytes, and copy-make dominates node time, so `bench 6`
  fell from 874k nps to ~700k nps. Recovering that needs either make/unmake with a global
  accumulator, or the `i8x16/i8x32 widen_madd` SIMD intrinsics the spec already plans.
- The next real milestone is training: self-play data generation (Phase 2) and an SGD trainer
  (Phase 3). Only then does this architecture have a chance at being stronger than the HCE.
