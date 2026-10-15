# Data scaling experiment — where the NNUE learning curve actually flattens

**Date:** 2026-10-14 · **Engine:** Tessera (bitboards, HCE) · **Data:** 5,500 self-play games
**Trainer:** `tools/train.py`, split by game · **Opponent for strength tests:** the same engine with
its hand-crafted evaluation (HCE)

## The headline

**Self-play data from clones has saturated.** Going from 142k to 518k positions (3.7×) bought
almost nothing, and neither did making the net wider. The binding constraint is the *information in
the games*, not data volume or model capacity.

| positions | games | val correlation (float) | vs HCE (8–12 games) |
|---|---|---|---|
| 4,094 | 40 | +0.890 (leaky split) | 0-0-7 → 6.25% |
| 37,366 | 400 | +0.600 (leaky split) | 0-1-7 → 6.25% |
| 141,656 | 1,500 | +0.502 (game split) | 2-0-10 → 16.67% |
| 517,736 | 5,500 | +0.517 (game split) | 1-2-9 → 16.67% |

The correlation moved +0.502 → +0.517 for 3.7× the data. The match score did not move at all, and
both are inside the error bars of 8–12 games anyway.

## Capacity sweep (same split, same schedule)

```
 hidden  schedule  val correlation
     16  constant             +0.516
     32  constant             +0.517
     32    cosine             +0.515
     64    cosine             +0.509
```

16 hidden units is as good as 64. The net is nowhere near capacity-limited — which is the opposite
of the usual assumption, and the reason "scale the net" was dropped from the plan.

## Draw filtering made it worse

Self-play between clones produces ~45% draws. Training only on decisive positions looked promising
in the trainer's own metric (+0.591 vs +0.517) — but that number was measured on a *different, easier
evaluation set*, since dropping draws also dropped them from validation. Measured on the same
held-out positions:

| model | correlation (all held-out) | correlation (decisive held-out) |
|---|---|---|
| trained on everything | **+0.357** | **+0.440** |
| trained without draws | +0.339 | +0.424 |

So draws are not noise to be filtered out — holding a draw is part of the game, and removing those
positions makes the net measurably worse.

## The real bug this experiment uncovered: i16 export precision

The trainer trains in float32 and exports i16 weights (the engine's format). Comparing the two
paths on identical held-out data:

```
float path      : +0.517
i16 path, naive : +0.357     <- 0.16 of correlation thrown away at export
i16 path, fixed : +0.415
```

Cause: the trained weights are small — `|w1|` around 1-2 and `|w2|` around 0.05 — so rounding them
straight to integers rounds almost everything to **zero**. The net the engine was loading was a
large fraction of the net that had been trained.

Fix: `w1`/bias and `w2`/psqt are scaled by independent factors `a` and `b`, which leaves the float
function unchanged up to a factor `a*b` (compensated in the engine's `scale` field) but buys integer
resolution. Because the right `(a, b)` depends on the trained magnitudes, the exporter **measures**
it: each candidate is run through the engine's own integer forward pass on the validation set and
ranked by correlation with the game result.

```
candidate w1 x1   w2 x1   -> corr +0.2494     <- naive rounding
candidate w1 x4   w2 x4   -> corr +0.3074     <- chosen
candidate w1 x64  w2 x8   -> corr +0.2903
...
quantisation: w1 x4, w2 x4 (int-path val correlation +0.415)
```

This is the one change from this experiment that is a clear, non-noisy win, and it applies to every
future net.

## Data quality of the runs themselves

Both runs are clean: no time forfeits (node-limited search makes forfeits impossible), no skipped
games, median 101 plies. Throughput was steady at ~3,600 positions/minute with 2-way concurrency, so
the 4,000-game run took 92 minutes.

## What this means for the plan

Ruled out by measurement, not assumption:
- **More self-play data of the same kind** — the curve is flat past ~150k positions.
- **A bigger network** — 16 and 64 hidden units are indistinguishable.
- **Filtering draws** — actively harmful.

What is left, in order:
1. **Different data, not more of it.** Games against a stronger opponent carry information the
   engine does not already have; self-play between clones mostly teaches the net what the eval
   already knew (material). This is the only change on the list that has a mechanism for progress.
2. **Better features.** 768 inputs of (piece, square) cannot express king safety or pawn structure at
   all. HalfKP-style buckets are a structural fix, not a scaling fix.
3. **Honest measurement.** 8-12 game matches cannot separate a 6% engine from a 17% engine. Before
   drawing any conclusion from the next comparison, run enough games to have overlapping error bars.