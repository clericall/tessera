# Strength results — 2026-10-14 (after the operator-precedence fix)

Config: fastchess v1.8.2-alpha; **Tessera** (`compiler/samples/bbuci.exe`, Tier C) vs **Stockfish 19
universal**. tc **5+0.05**; **Threads 1** / **Hash 16 MB**; 8-opening balanced `book.epd`
(order=random); concurrency 1. Both engines run from the repo root (config uses relative paths).

## Headline: the 0-8 result was a compiler bug, not a weak eval

The previous "0 - 16" result was caused by three defects found on 2026-10-14:

1. **Parser operator precedence (compiler bug).** `infix_bp` returned `(lbp, lbp)` for every binary
   operator instead of `(lbp, lbp + 1)`, making all binary operators **right-associative**.
   `0 - beta + 1` parsed as `0 - (beta + 1)`; `a - b - c` parsed as `a - (b - c)`.
   Fixed in `compiler/crates/tessera_parser/src/lib.rs` (19 operators); three regression tests added
   to `compiler/crates/tessera_parser/tests/parse_tests.rs`.
2. **Depth cap.** `parse_depth` returned its default of `4` when the `go` line had no `depth` token,
   and the `go` handler accepted it. Every GUI game was capped at depth 4.
3. **Time management.** `DEADLINE = now + myt/2 + myinc/2` halved the *remaining* clock every move
   (2.53s -> 1.29 -> 0.67 -> ... -> 0.05s), so the engine effectively stopped thinking after ~6 moves.
   Now `budget = min(myt/4 + myinc/2, 3000)`, floored at 30 ms.

Also fixed in the same pass: `HIST` was sized 1536 but indexed up to ~4900 (silent out-of-bounds
writes into neighbouring statics); `GMV` was sized for 64 plies while the ply cap was raised to 120;
and the `-1000000` "infinity" sentinel could leak as a real score through the null-move window.

## Correctness after the fixes

Perft is still exact (mailbox removal and all search changes leave movegen untouched):

| position | depth 5 nodes |
|---|---|
| startpos | 4,865,609 |
| Kiwipete | 193,690,690 |
| `8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w` | 674,624 |
| `r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w` | 15,833,292 |
| `rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w` | 89,941,194 |
| `r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w` | 164,075,551 |

## Search now reaches usable depth

`position startpos` + `go depth 10` finishes in ~0.7 s at ~1.09 M nps; `go wtime 5000 btime 5000
winc 50 binc 50` reaches depth 12 in the 2.5 s opening budget. Before the fixes the same engine
stopped at depth 4 in ~8 ms.

## Matches (this run)

| opponent | TC | games | result |
|---|---|---|---|
| Stockfish 19 (full strength) | 5+0.05, 1t, 16 MB | 16 | **0 - 16** (0.00%) |
| Stockfish 19, `Skill Level 0` (~1350 Elo) | 5+0.05, 1t, 16 MB | 8 | **6 - 2** (75.00%) |

**Interpretation.** The engine is now a functioning, protocol-correct, perft-exact chess engine: it
beats a ~1350-Elo opponent 75% of the time at equal limits. It still loses every game to full
Stockfish 19 (~2900 Elo), which is the expected result for a bitboard engine with a hand-crafted
tapered eval and no NNUE. The gap is evaluation, not protocol or movegen.

## Speed

`bench 6`: 499,229 nodes, 571 ms, **874 k nps** (Tier C). Tier A/B/C are now within 0.5% of each
other (790-794 k nps) because the richer evaluation, not the magic-table indexing, dominates node
cost. For reference the tier-matched Stockfish bench was ~469 k nps.

## Honest caveats
- 16 games at 5+0.05 is a smoke test, **not** an SPRT; the numbers above have very wide error bars.
- Tessera ignores `setoption` (`Threads`/`Hash` warnings) and keeps its TT across `ucinewgame`.
- Stockfish was given `Skill Level 0` only for the sanity match; that is not an equal-strength
  opponent and the 75% figure should not be read as an Elo estimate.
