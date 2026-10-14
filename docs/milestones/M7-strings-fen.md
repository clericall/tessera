# Milestone M7 — Strings, char literals, FEN parsing (DONE)

**Language additions (all compiled via the C backend):**
- `str` value type (C `Tr_str { const char* p; int64_t n; }`) — passed by value, zero-copy,
  no allocation; string literals lower to `tr_sl("...", len)`.
- **char literals** `'K'`, `'\n'`, etc. → i64 byte.
- `len(s) -> i64`.
- **string indexing** `s[i] -> i64` (byte), e.g. `if s[0] == 'H'`.

**Engine:** `from_fen(fen: str) -> Position` (board, side to move, castling rights, en-passant
square), driving the same perft.

**Validated (Tessera -> C -> clang -> native, perft **from FEN strings**):**

| FEN | depth 1 | 2 | 3 | 4 | vs reference |
|---|---|---|---|---|---|
| startpos `.../RNBQKBNR w KQkq - 0 1` | 20 | 400 | 8902 | 197281 | exact |
| Kiwipete `r3k2r/... w KQkq - 0 1` | 48 | 2039 | 97862 | 4085603 | exact |
| pos3 `8/2p5/... w - - 0 1` | 14 | 191 | 2812 | 43238 | exact |

Matching perft-from-FEN means FEN parsing AND move generation are mutually consistent and correct
on castling, promotion, and en-passant (pos3). Compiler suite still 29 green.

## Honest status
- Still the **10x10 structural generator + copy-make** (correct, not fast). No magic bitboards,
  no speed measurement yet. Strings are read-only borrowed slices — no string building/mutation
  (fine for FEN; a mutable string/arena comes later if needed).

## Reproduce
```
cd compiler
target\debug\tesserac.exe run ..\engine\e1.ter
```
