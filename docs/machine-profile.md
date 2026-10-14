# Tessera — Target Machine Profile

Recorded: 2026-10-14. Automated probe. CPU features measured at runtime via Rust
`is_x86_feature_detected!` (CPUID), NOT assumed.

## Hardware
| Item | Value |
|---|---|
| CPU | Intel Core i3-4130 @ 3.40 GHz (Haswell; family 6, model 0x3C) |
| System | Dell OptiPlex 3020 |
| Cores / threads | 2 physical / 4 logical (HyperThreading) |
| Base clock | 3400 MHz (i3-4130 has no turbo — fixed clock) |
| L1d / L1i | 32 KB / 32 KB per core (Haswell microarch spec; not exposed by WMI) |
| L2 | 512 KB total (2 × 256 KB) |
| L3 | 3072 KB shared (3 MB) |
| RAM | ~12 GB (12 835 434 496 B), 2 × DDR3 (Samsung + Micron) |
| OS | Windows 10 Pro 64-bit, build 19045 |

## Instruction sets (measured, runtime CPUID)
YES: `sse sse2 sse3 ssse3 sse4.1 sse4.2 avx avx2 fma f16c popcnt bmi1 bmi2 lzcnt abm rdrand aes pclmulqdq`
NO : `sse4a avx512f`
=> This is a **Tier C** machine (AVX2 + BMI1/BMI2 + POPCNT + FMA + LZCNT all present).

### Honesty caveat
This box cannot bare-metal test genuine pre-Haswell (no-AVX2) hardware. **Tier A/B are exercised
by compiling with those features DISABLED** (codegen tier-down) so the silicon runs only the
lower-tier code paths. That is *simulation by compilation*, not real 2008–2011 silicon — stated plainly,
not glossed over.

## Toolchain (as probed)
- `rustc 1.96.0` (x86_64-pc-windows-msvc) — links via MSVC.
- .NET SDK present; **Visual Studio Community 2026** (MSVC build tools) installed.
- `winget` 1.29.380.
- `clang` / `gcc`: **not installed.** Installing a **portable LLVM clang** via the official
  `clang+llvm-…-x86_64-pc-windows-msvc.tar.xz` tarball to `%USERPROFILE%\tools` (no admin/UAC) for the
  C backend. Chosen: `clang+llvm-23.1.3-x86_64-pc-windows-msvc`.

## Measurement protocol (committed, non-negotiable)
- **Env:** AC power; Windows **High/performance** power plan fixed; turbo N/A (fixed 3.4 GHz here).
- **Affinity:** concurrency 1; each engine pinned to **one physical core**. The exact
  core↔logical / HT-sibling mapping will be detected via `GetLogicalProcessorInformation` **before**
  pinning — not assumed.
- **Thermals:** log instantaneous CPU MHz during runs; **discard thermally-throttled runs**; cool down
  between long runs.
- **Speed:** fixed FEN set, 1 thread, 16 MB hash, per build tier (A/B/C via codegen flags),
  **≥ 5 runs**, report median + spread.
- **Strength:** **tier-matched Stockfish binary** (here `x86-64-avx2`), identical 1 thread / 16 MB /
  time control, balanced opening book, alternating colors, concurrency 1, **SPRT** via
  fastchess/cutechess. Report Elo difference ± error bars + games played. Compare vs the C++
  reference engine to separate language gain from algorithm gain. Elo/Joule only if RAPL/powertop-
  equivalent is readable (likely limited on Windows — will flag if unavailable).
