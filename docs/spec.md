# The Tessera Language — Specification (v0.1, draft for approval)

Tessera is a small, statically typed, AOT-compiled systems language with **no garbage collector,
no hidden heap allocation, and no hidden control flow.** Value types and flat memory layouts by
default, with first-class primitives for bit-heavy numeric code (bitboards) and small explicit SIMD
types for NNUE-style inference. This document is the contract; it is deliberately short.

> Scope note: this is **v1**. Generics, pattern matching, and modules-with-visibility are cut from v1
> to keep the spec tiny; they are additive later. The engine does not need them.

## 1. Compilation model
Pipeline: `lexer → parser → AST → typecheck → typed IR (3-address) → optimizer → backend`.
Backends: **(B1) C** — emit readable C, compile with clang/gcc `-O3` (debug-friendly);
**(B2) LLVM via inkwell** — after B1 passes all tests. Targets: x86-64 and aarch64. CPU tier selectable
(`--tier A|B|C` or explicit `--features`). Diagnostics carry spans + line/col (`codespan-reporting`).

Build tiers (for chess):
- **Tier A** — SSE2/SSSE3 only; **software** popcount/ctz fallbacks; classic magic bitboards.
- **Tier B** — SSE4.2 + POPCNT (+ optional AVX); classic magic bitboards.
- **Tier C** — AVX2 + BMI2 (pext/pdep magics); SIMD NNUE via AVX2.
Each tier exposes the NNUE SIMD path through the language's vector types.

## 2. Lexical structure
- Identifiers: `[A-Za-z_][A-Za-z0-9_]*`. Types are lowercase builtins (`u64`); user types CapWords.
- Keywords: `let var const fn const struct enum static as return if else while for loop break
  continue true false import asm`. (`const fn` is two keywords; context resolves.)
- Literals: integer (`123`, `0x1F`, `0b1010`, with optional `_` separators, and typed via suffix or
  inference), float (`1.5`, `1e-3`), bool, `()` unit. Integers are arbitrary-width at parse time; they
  must resolve to a concrete integer type by inference or annotation.
- Operators: `+ - * / % & | ^ ~ << >> == != < <= > >= && || ! = += -= *= &= |= ^= <<= >>=`. Deref `*p`
  (prefix), address-of `&x`, member `.`, index `a[i]`, range `a..b` (exclusive) used in `for`.
- Comments: `// line`, `/* block */`.

## 3. Types
Value types, flat layouts, known sizes:
| Type | Size | Notes |
|---|---|---|
| `i8 i16 i32 i64` | 1/2/4/8 | two's complement; arithmetic **wraps** by default |
| `u8 u16 u32 u64` | 1/2/4/8 | `u64` is the **bitboard** type |
| `usize isize` | ptr-sized | indexing / sizes |
| `f32 f64` | 4/8 | for tuning/analysis only; hot eval is integer |
| `bool` | 1 | |
| `()` | 0 | unit |

- **`BitBoard`** is a *type alias* `type BitBoard = u64;` (documentation; real ops are builtins on u64).
- **Arrays** `[T; N]`: value type, flat, **bounds-checked**; checks removable per-build (`bounds=false`)
  or per-item `#[bounds(off)]`.
- **Slices** `[]T`: (ptr,len) fat pointer; passing an array/slice does **not** copy.
- **Pointers** `*T` and **references** `&T`: explicit, no borrow checker (systems language).
- **Tuples** `(A, B, …)`: flat, value type. Unit `()` is the 0-tuple.
- **Enums**: C-like (`enum Color { White = 0, Black = 1 }`), `#[repr(u8/i32/...)]`-controllable in v0.
- **Option[T]**: minimal `Some(x)` / `None` (reserved; may be v0.x). `?`-postfix reserved.
- **Vectors** (see §9): `i8x16 i16x8 i32x4 i64x2` and `i8x32 i16x16 i32x8 i64x4`.

Numeric rules: no implicit conversions. Widening/narrowing is via explicit `as`. Integer division/`%`
follow two's complement; **wrap on overflow** (matches zobrist/hash expectations); checked variants
(`checked_add`, …) are builtins.

## 4. Memory model (no hidden allocations)
- Locals and value types live on the **stack**; struct assignment **copies** (documented cost; pass
  large structs by `&`).
- The **only heap** is the explicit **arena**: `let a = Arena(); let buf = a.alloc(u64, 256);` — bump
  allocation; reset to free. Search scratch uses this; zero per-node heap traffic.
- Globals (`static`): allowed; must be const-initialized or zero-initialized (TT, embedded weights).
- No closures, no boxing, no GC. The **only** implicit overhead is the bounds checks of §3, and those
  are compile-time removable.

## 5. Items
- `const NAME: T = <const-expr>;` — compile-time value (tables, magic constants).
- `static NAME: T = <value>;` — mutable global storage (RAM-resident).
- `fn name(a: T, b: U) -> R { … }` — function; `-> ()` optional for void.
- `const fn name(...) -> R { … }` — pure, evaluated at compile time (see §8).
- `struct Name { field: T, … }` — flat POD value type.
- `enum Name { A = 0, B }` — C-like.
- `import "path";` — multi-file composition (v1: minimal).

## 6. Statements & expressions
- Bindings: `let x = e;` (immutable), `var x = e;` (mutable). Type annotation optional: `let x: u64 = e;`.
- Control: `if c { } else if c2 { } else { }`; `while c { }`; `for x in 0..8 { }` and
  `for x in slice { }`; `loop { }`; `break;` `continue;`.
- Assignment & compound assignment (`+=` etc.). No prefix/postfix `++/--`.
- `as` casts. Field `.`, index `a[i]`, call `f(x)`. Blocks are expressions: `let y = { let t = a + b; t * 2 };`.

## 7. First-class numeric primitives (builtins, on integers)
`popcount(x) -> i32`, `ctz(x) -> i32`, `clz(x) -> i32`,
`pext(x, mask) -> T`, `pdep(x, mask) -> T`, `rotl(x, r) -> T`, `rotr(x, r) -> T`, `clmul(a, b) -> T`.
- `pext/pdep` are selected per tier at compile time; where BMI2 is absent they use a **correct software
  fallback** and may also be runtime-dispatched. Magic bitboards on Tier A/B use classic (non-PEXT) magics.
- `cpu_has("avx2"|"bmi2"|…) -> bool` — runtime feature query for single-binary dispatch.

### 7.1 I/O builtins (implemented)
Standard streams: `read_line() -> str`, `putstr(str)`, `putc(i64)`, `putint(i64)`, `flush()`,
`now_ms() -> i64`.

Binary file reads (little-endian, sequential; needed to load NNUE weight files):

| builtin | signature | notes |
|---|---|---|
| `fopen` | `fopen(path: str) -> i64` | returns a handle, or `-1` if the file cannot be opened |
| `fread_u8` | `fread_u8(h: i64) -> i64` | `0..255`, or `-1` at EOF/error |
| `fread_i16` | `fread_i16(h: i64) -> i64` | sign-extended `i16` |
| `fread_i32` | `fread_i32(h: i64) -> i64` | sign-extended `i32` |
| `feof` | `feof(h: i64) -> bool` | true once the stream is exhausted (or `h < 0`) |
| `fclose` | `fclose(h: i64) -> i64` | `0` on success, `-1` for an invalid handle |

EOF is signalled by `-1` **and** by `feof`; `fread_i16`/`fread_i32` cannot use `-1` alone as a
sentinel because it is a legitimate value of those ranges. Reads are unbuffered from the language's
point of view — a short read yields `-1` rather than partially-filled data.

`fopen` takes a length-aware `str` (the bytes are copied into a NUL-terminated
buffer), so `fopen(substr(line, i, n))` works. The string builtin `substr(s, start, len) -> str`
returns a view into `s` and clamps out-of-range arguments.

## 8. Compile-time evaluation
- `const fn` must be **pure** (no I/O, no mutation of `static`/outer state, no external calls);
  recursion allowed, depth-capped by the compiler.
- `const` items/locals are folded at build time. **Attack tables and magic bitboards (including the
  magic-number search) are produced here** — zero runtime setup, all in read-only data.
- If a `const` expression can't be folded, it's a compile error (keeps guarantees strong).

## 9. SIMD vector types (for NNUE)
Widths track the tier; ops lower to AVX2 (SSE2 128-bit / AVX2 256-bit) or NEON, with scalar fallback:
- Types: `i8x16 i16x8 i32x4 i64x2` (128) and `i8x32 i16x16 i32x8 i64x4` (256).
- Ops: `load/store` (aligned + unaligned), `splat/broadcast`, `add sub` (per-lane), `mul` with
  **widening** (`i16*i16→i32`), `widen_madd` (multiply-accumulate widening — the NNUE core),
  horizontal `hsum`, `min max`, shifts, `and or xor xor` `not`, and compare→`select`.
- `#[target_feature("avx2,bmi2")]` on a `fn` emits that ISA for the body; combine with a dispatcher
  using `cpu_has(...)` for one binary across tiers.
- `asm!(...)` escape hatch for tier-specific hot spots (e.g., hand-tuned pext).

## 10. Attributes
`#[bounds(off)]`, `#[inline]` / `#[inline(always)]`, `#[target_feature("…")]`, `#[repr(T)]`,
`#[noinline]`.

## 11. Diagnostics
Spans + line/column; readable primary/help messages; error codes. Example:
```
error[E0301]: type mismatch
  --> engine\move.ter:42:20
   |
42 |     let slot: u64 = ctz(board);
   |                    ^^^^^^^^^^^ expected `u64`, found `i32`
   |
help: use `as u64` to convert
```

## 12. Example — bitboards + perft
```tessera
type BitBoard = u64;

enum Color { White = 0, Black = 1 }

struct Position {
    pieces: [u64; 6],      // by piece type
    colors: [u64; 2],      // by color
    stm: Color,
    castling: u8,
    ep: u8,                // en-passant square (64 = none)
    half: u16, full: u16,
}

const KNIGHT_ATT: [u64; 64] = build_knight_table();   // const fn -> folded at build time

fn perft(pos: &Position, depth: i32) -> u64 {
    if depth == 0 { return 1; }
    var nodes: u64 = 0;
    for mv in gen_moves(pos) {
        let undo = make_move(pos, mv);
        nodes += perft(pos, depth - 1);
        unmake_move(pos, mv, undo);
    }
    return nodes;
}
```

## 13. Example — NNUE inner layer (portable across tiers)
```tessera
#[target_feature("avx2")]
fn nnue_dot_i8(weights: &[i8x32], inputs: &[i8x32], n: i32) -> i32 {
    var acc = i32x8::splat(0);
    var i: i32 = 0;
    while i < n {
        acc = acc.widen_madd(weights[i], inputs[i]);   // int8 dot -> int32, vectorized
        i += 1;
    }
    return acc.hsum();
}
```

## 14. Explicit non-goals (v1)
No GC, no closures, no exceptions, no RAII/destructors, no threads in-language (engine threads are
host-level for now), no user generics, no `match`. These keep the spec small and the semantics predictable.
