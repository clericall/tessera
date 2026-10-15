# Milestone M12 — Self-play data generation (NNUE phase 2)

The inference path (M11) can load a net but has no way to learn one. This milestone builds the
pipeline that produces training positions: self-play games in, a compact binary training set out.

## Pipeline

```
fastchess (Tessera vs Tessera, node-limited)  ->  selfplay.pgn
        tools/gen_data.py --pgn selfplay.pgn --out selfplay.bin
                -> 40-byte records, one per sampled position
        [phase 3] tools/train.py  ->  engine/nnue-test.tnn
```

`compiler/config-selfplay.json` runs both colours with different node budgets (30k vs 22k) so games
are decisive instead of mirrored draws, from the existing `engine/book.epd` openings.

## Record format (40 bytes, little-endian)

| offset | size | meaning |
|---|---|---|
| 0 | 32 | board: 4 bits per square, square 0 = a1 (the engine's ordering), codes 1-6 = P N B R Q K, 7-12 = black |
| 32 | 1 | side to move (0 = white) |
| 33 | 1 | castling mask, WK=1 WQ=2 BK=4 BQ=8 |
| 34 | 1 | en-passant square, 255 = none |
| 35 | 1 | reserved |
| 36 | 4 | float32 result from the side to move's point of view (1.0 / 0.5 / 0.0) |

Filtering: positions before ply 8 are dropped, and games that ended by time forfeit or abort are
skipped by default (they label the result by clock failure, not by chess).

## Feature convention is cross-validated

The trainer and the engine must agree on which feature a square maps to, or training is worthless.
`gen_data.py --features "<fen>"` prints the active feature indices using the engine's exact
convention (own piece -> block `pt-1`, square `sq`; enemy piece -> block `6+(pt-1)`, square
`63-sq`). The engine has an `fidx` command that dumps the same list. They were compared on the
start position, Kiwipete, an en-passant position, a promotion position and a missing-queen
position: **identical feature sets every time** (64/64, 64/64, 64/64, 56/56, 6/6).

## Engine work this needed

- **`go nodes N`** support. Self-play at a fixed time control produced time forfeits (both engines
  overran their clocks mid-iteration), which poisons labels. A node budget makes games clock-free
  and reproducible: `NODELIMIT` is checked every 1024 nodes alongside the time deadline.
  A first attempt failed silently because `opt_value` expects a `value` keyword (that is only
  valid for `setoption`), so `go nodes 30000` ran to depth 14 and 48M nodes; a `word_num` helper
  fixed it.

## First data set

40 self-play games, 4,094 positions, 163,760 bytes, in about 70 seconds of match time:

```
wins (stm) : 1357
draws      : 1393
losses     : 1344
```

## Verification

- `gen_data.py --selftest` round-trips four positions through the encoder and back, and pins the
  feature convention against hard-coded values (white king a1 / black king h1 -> 320, 327, 760,
  767; the e2 pawn -> 12; the e7 pawn -> 52).
- That self-test caught two real encoder bugs: the side-to-move byte was written inverted, and the
  castling mask was derived from the king's file instead of the FEN field, so kingside rights were
  lost and stored as queenside.
- `gen_data.py --dump N` decodes records back to FENs for eyeballing.
- Engine unchanged: perft still exact on all six reference positions; 40 compiler tests green.

## Honest expectations

4k positions is a smoke test, not a training set — a real run needs 10^6-10^7 positions, which at
~100 positions/second here is hours of self-play. This milestone only proves the extraction,
labelling, encoding and cross-validation are correct.
