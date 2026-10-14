# Milestone M9 — Compiler operator-precedence fix + real search depth

## The bug

`infix_bp` in `compiler/crates/tessera_parser/src/lib.rs` returned the same binding power for the
left and right side of every binary operator:

```rust
Tok::Plus  => (13, 13, Bin(BinOp::Add)),   // rbp must be lbp + 1
Tok::Minus => (13, 13, Bin(BinOp::Sub)),
```

With `lbp == rbp`, the recursive right-hand parse happily consumed operators of the *same*
precedence, so every binary operator became right-associative:

| source | parsed as | should be |
|---|---|---|
| `0 - beta + 1` | `Sub(0, Add(beta, 1))` | `Add(Sub(0, beta), 1)` |
| `a - b - c` | `a - (b - c)` | `(a - b) - c` |
| `8 / 2 * 4` | `8 / (2 * 4)` | `(8 / 2) * 4` |

Assignment was unaffected (it already used `(2, 1)`), which is why the bug went unnoticed in the
parser tests and only surfaced as "the engine is inexplicably bad at chess".

Fix: all 19 binary operators now return `(lbp, lbp + 1)`. Three regression tests added:
`binary_ops_are_left_associative`, `subtraction_does_not_swallow_trailing_add`,
`division_and_modulo_are_left_associative`.

## Why it mattered so much

The chess engine's evaluation and search are full of subtractions in scaled terms
(`0 - 100000 + ply`, `(E_MG * E_PH + E_EG * (24 - E_PH)) / 24`, `0 - beta + 1` for the null-move
window, `0 - 1000000` for the mate sentinel). With right associativity each of those silently
computed a different number, which:
- poisoned the null-move window (`beta = -beta - 1` instead of `-beta + 1`),
- let the `infinity` sentinel leak as a real score, inverting alpha/beta windows,
- made iterative deepening terminate after depth 4 with `localmv = -1`,
- and corrupted the tapered evaluation.

## Engine fixes found while chasing it

| area | bug | fix |
|---|---|---|
| dispatch | the `go` branch body was nested inside the `position` branch, so every `position` command also ran a depth-4 search and `go` did nothing | un-nested the if/else chain |
| arrays | `HIST` sized 1536, indexed to ~4900 | resized to 8192 |
| arrays | `GMV` sized for 64 plies, ply cap raised to 120 | resized to 32768 |
| search | `-1000000` sentinel could be returned as a score | sentinel moved to +-100000000, out of mate-score range |
| time | `parse_depth` returned its default `4` when `go` had no `depth` token | default changed to `0` (meaning "no depth limit") |
| time | `myt/2` halved the remaining clock every move | `min(myt/4 + myinc/2, 3000)`, floor 30 ms |
| time | a full depth iteration could overrun the deadline | `ABORT` flag checked every 1024 nodes; aborted iterations are discarded |
| uci | mate scores reported as `cp -99994` | reported as `score mate N` |

## Verification

- `cargo test`: all suites green, including the 3 new associativity tests.
- Perft unchanged and exact at depth 4 and 5 on all six reference positions.
- `position startpos` + `go depth 10`: depth 10 in ~0.7 s at ~1.09 M nps (was: depth 4 in 8 ms).
- fastchess vs Stockfish 19 `Skill Level 0`, 5+0.05, 1 thread: **6-2 (75%)** (was 0-8 by mate).

See `docs/results/strength-2026-10-14-after-precedence-fix.md` for full numbers.
