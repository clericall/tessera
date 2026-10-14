# Milestone M8 — Playable UCI engine (DONE, first cut)

**Language additions:** `read_line() -> str`, `putstr(str)`, `putc(i64)`, `putint(i64)` (stdin +
newline-free output) — enough to implement the UCI wire protocol.

**Engine (`engine/uci.ter`):** built on the perft-validated movegen + FEN. Adds
- UCI protocol: `uci` (id/uciok), `isready` (readyok), `ucinewgame`,
  `position startpos|fen F [moves ...]`, `go depth N` -> `info ...` + `bestmove ...`, `quit`.
- **Search:** negamax + alpha-beta (copy-make), fixed depth from `go depth N`.
- **Eval:** material (P100 N320 B330 R500 Q900) + a tiny minor-piece centralization bonus,
  side-to-move perspective.

**Verified (scripted UCI session via stdin):**
```
> uci                -> id name Tessera 0.1 / id author / uciok
> isready            -> readyok
> position startpos
> go depth 4         -> info depth 4 score cp 4 nodes 8775 ; bestmove b1c3
> position startpos moves e2e4 e7e5
> go depth 4         -> info ... nodes 87120 ; bestmove d1h5
```
Sign check (material): Queen-up position -> `score cp 900`; same position Black to move ->
`score cp -900`. **Score/eval sign is correct.**

## Update: quiescence added (verified)
Added `quiesce()` (stand-pat + captures/promotions, alpha-beta) at the search horizon. Effect
measured: after `1.e4 e5` the depth-5 score went from the horizon-blunder `-93` to a material-equal
`+6`, and it now returns developing moves (`b1c3`); queen-up still `+900` (sign correct).

## Update 2: iterative deepening + move ordering + time control
- **Iterative deepening** (`search_root`): searches depth 1..N, prints `info depth D score cp X
  nodes N` per iteration, reuses the previous best move (prepended first).
- **MVV-LVA move ordering** (`order_moves`) in negamax and quiescence (captures/promotions first).
- **Time management**: `go movetime X`, `go wtime/btime [winc/binc]` (alloc clock via new
  `now_ms()` builtin), with a stop between completed depths. `go depth N` also honored.
- Verified: `go depth 5` on startpos -> per-depth info up to 127616 nodes, `bestmove b1c3`;
  `go movetime 300` returns within budget; queen-up still `+900`.

## Update 3: transposition table (zobrist)
- Zobrist keys recomputed per node from a splitmix-style LCG RNG (`rng_init`/`compute_key`) — recomputing avoids incremental-key bugs.
- Fixed 2^18-entry TT (parallel arrays, `{0}` zero-init) with exact/lower/upper bound flags; TT move used
  as an ordering hint. Mate scores are not stored (guard), so TT stays sound.
- Verified: back-rank mate-in-1 `6k1/5ppp/8/8/8/8/8/4R1K1 w` -> `bestmove e1e8`, `score cp 99999`;
  win-the-queen `4k3/8/8/3q4/4B3/8/8/4K3 w` -> `bestmove e4d5`, `score cp 336` (queen was Black's,
  so White ends up only up a bishop — the value is correct, not a bug);
  startpos depth-5 nodes dropped 127616 -> 66354 (TT working).

## Honest status — what this is NOT yet
- **Material(+minor centralization) eval; no PST, no repetition/50-move draw detection**, and the
  time check is only between iterations (no mid-iteration stop).
- Still the **10×10 copy-make generator** (correct, not fast); no incremental zobrist (keys recomputed
  per node). TT ~10MB.
- Not yet benchmarked against Stockfish or the C++ reference; no NNUE.


## Reproduce
```
cd compiler
target\debug\tesserac.exe run ..\engine\e1.ter        # perft sanity
# UCI:
samples\uci.exe     (then type: uci / isready / position startpos / go depth 5 / quit)
```
