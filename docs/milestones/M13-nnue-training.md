# Milestone M13 — NNUE training loop (phase 3), end to end

`tools/train.py` trains a net with hand-derived backprop and exports the `.tnn` the engine already
loads. The whole loop is verified, and the honest result is that 37k self-play positions is nowhere
near enough to beat the hand-crafted eval.

## The loop

```
tools/gen_data.py  ->  selfplay.bin        (phase 2)
tools/train.py     ->  engine/nnue-smoke.tnn
engine  setoption name EvalFile value engine/nnue-smoke.tnn
```

`train.py` implements exactly the arithmetic in `engine/bb.ter`: i16 weights, i16-clipped
accumulator, `hidden = clip(Σ acc·w2)`, `value = clip(Σ psqt + hidden)`, `score = value·scale/64`.
Gradients are hand-derived (a 2-layer MLP needs no autodiff); the i16 clips use a
straight-through estimator so saturated units stop receiving gradient.

## Correctness: the engine and Python agree exactly

`train.py --check-fen "<fen>"` reads the exported file back and runs the same integer
computation. Across nine positions (start, Kiwipete, four perft positions, a promotion endgame, a
queen-up endgame, and a black-to-move case) the engine and Python produce **identical scores in 9/9
cases**, including the accumulator, psqt and scale stages. Synthetic probes isolate each stage:

| probe | python | engine |
|---|---|---|
| psqt = 1 everywhere (32 pieces) | 32 | 32 |
| psqt[0] = 100 (feature inactive) | 0 | 0 |
| w1[0][0] = 5, w2[0] = 3 (feature inactive) | 0 | 0 |
| bias[7] = 11, w2[7] = 2 | 22 | 22 |

Three real bugs were found and fixed on the way, all on the Python side (the engine was right):

1. **Gradient sign.** `w1` and `psqt` were updated with inverted gradients, so training loss went
   *up* and both quantized to zero.
2. **King colour decoding.** `code // 6` reads the white king (code 6) as black, corrupting the
   feature map for every position.
3. **Flat `w1` indexing.** `load_tnn` never reshaped the weight table to `(768, hidden)`, so
   `w1[feature]` silently returned a single weight instead of a row. Only visible with a real
   trained net — the synthetic probes had a single non-zero weight and hid it.

A fourth was a Python/C parity issue: `//` floors while C truncates toward zero, so negative
scores were off by one.

## What training actually achieved

400 self-play games → 37,366 positions (10,752 wins / 15,978 draws / 10,636 losses from the side to
move), 60 epochs, ~53 s on the i3-4130:

```
val MSE           58,243      (predicting 0 costs ~112,000)
val correlation   +0.600      (score vs actual game result, held out by position)
w1 [-2, 2]  psqt [-10, 10]  w2 [-6, 6]
```

The net learned the crudest material logic, symmetrically across colours:

| position | net score |
|---|---|
| white queen up, white to move | +141 |
| black queen up, black to move | +141 |
| white queen up, black to move | -144 |
| white rook up, white to move | -1 |

## Honest result: it is not competitive

```
fastchess, 5+0.05, 1 thread, 16 MB, 8 games
Tessera+NNUE vs Tessera+HCE  ->  0 wins, 1 draw, 7 losses  (6.25%)
```

A queen is worth +141 cp when it should be ~900, and a rook registers as worth nothing. The
hand-crafted evaluation knows kings, pawn structure, mobility and king safety; 37k positions teach
the net almost none of that. This is expected: Stockfish's first NNUE networks were trained on
millions of positions and initially played near hand-crafted strength.

Two measurement caveats on the numbers above:
- The validation split is **by position, not by game**, so positions from the same game appear in
  both train and validation. The +0.600 is therefore optimistic; a game-level split is the honest
  version.
- `val MSE 58,243` compares against a target distribution of ±400 cp; the "predict 0" baseline is
  roughly 112,000, so the net explains about half the variance of the label — on leaky data.

## What would be needed to actually win games

1. **Data volume**: 10^6-10^7 positions. At the measured rate (~4.5k positions/minute with 2-way
   concurrency) that is 4-40 hours per net, and quality needs stronger opponents than a clone of
   itself.
2. **Validation by game**, not by position.
3. **A better feature set** than (piece, square): Stockfish's HalfKP buckets and (later) MCTS
   features are what make a small net strong; 768 inputs is the toy version.
4. **Search quality**: the NNUE path currently costs ~20% nps versus the HCE path, and the engine
   still lacks PVS and aspiration windows.

None of those are blocked by missing tooling now — the pipeline runs end to end, and each step is
measurable.