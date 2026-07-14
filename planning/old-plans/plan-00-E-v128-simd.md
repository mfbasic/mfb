# plan-00-E — The `v128` SIMD Layer

Last updated: 2026-06-29

> **Status: DONE (all phases).** The 48-op NEON tail moved from the MIR `mirror`
> group into a new `simd` group with neutral `v128.*` mnemonics (load/store, the
> f64 arith + `fma`/`fms`, lane compares + zero forms, `fmin`/`fmax`, the round
> family `v128.fround_{ceil,floor,nearest,even,trunc}`, `v128.f2i_*`/`v128.i2f`,
> integer lane ops, bitwise + `bsl`/`bit`, `dup_from_gpr`/`umov_to_gpr`). Like
> plan-00-C it is a **mnemonic-only** rename at the seam: each `v128` MirOp maps
> 1:1 to its NEON `CodeOp`, so selection/encoder are untouched and the output is
> byte-identical — only `-mir` changes (no `*_v`/`*_q` survives). Kernels +
> `vector::` reach the vocabulary through the builder seam (no rewrite). The FP
> `RegisterModel` class now spans `d`/`v`/`q` (the 128-bit Q view). Lane
> semantics pinned as a 48-row contract test (`v128_lane_semantics_contract`);
> golden vectors = the unchanged ULP harness + `func_vector_*`/`func_math_*array*`
> fixtures. The 36-fixture op-family sweep bans the NEON mnemonics and asserts a
> `v128.*` op appears. Validation: codegen-selfdiff byte-identical (0 failures),
> ULP harness unchanged (exp/log 100% ≤1 ULP; the lone tan 2-ULP miss is the
> pre-existing macOS-libm reference quirk), acceptance 975/975, no `.mir` golden
> changed (the two committed goldens use no SIMD).

The hardest neutralization: the large NEON tail of `CodeOp` (the transcendental kernels, the
`vector::` package, the array overloads) becomes a fixed-width **`v128`** MIR vocabulary
(`mir.md §6`, §12.1 — confirmed: fixed-128, NEON ↔ SSE2+FMA3+SSE4.1, rv64 scalarizes).

Depends on plan-00-A–C. Stays AArch64-**byte-identical** under `-codegen mir`.

## 1. Goal

- A neutral **`v128`** value type + lane-op vocabulary covering exactly what the kernels and
  `vector::` use: `v128.load/store`, `fadd/fsub/fmul/fdiv/fsqrt/fabs/fneg`,
  `fma`/`fms` (`fmla`/`fmls`), lane compares (`fcmgt/ge/eq` + the zero forms), `fmin/fmax`,
  the round family (`frintp/m/a/n/z`), `fcvt`/`scvtf` lanes, integer lane ops
  (`add/sub/cmgt/cmge/cmeq/shl/sshr/ushr/neg/abs`), bitwise (`and/orr/eor/bsl/bit`),
  `dup`/`umov` (scalar↔lane). Lanes: `2×f64` / `4×f32` / `16×i8` as the op needs.
- AArch64 backend lowers each `v128` op → its NEON instruction **byte-identically**; the
  kernels (`builder_simd_float_math.rs`, `builder_pow.rs` SIMD bits) and `vector::` re-emit
  in `v128`.

### Non-goals

- No x86_64/rv64 lowering here (SSE in plan-00-H, scalarize in plan-00-I) — but the op set
  and **lane semantics are pinned now** so those backends have an exact contract.
- No new SIMD capability; this is a 1:1 re-expression of today's NEON usage.

## 2. Current State

`CodeOp` SIMD block: `LdrQ/StrQ`, `FAddV..FMaxV`, `FMlaV/FMlsV`, `FRint*V`, `FCvt*V`,
`ScvtfV`, `Cm*V`/`FCm*V` (+ zero forms), `Add/Sub/Neg/AbsV`, `Sshl/Ushl/Shl/Sshr/UshrV`,
`And/Orr/Eor/Bsl/BitV`, `DupVFromX`, `UmovXFromV`. The kernels rely on the exact NaN/select/
rounding semantics (plan-03's branch-on-quadrant + `BIT`-selects; the ≤1 ULP polynomials).

## 3. Design

- `v128` MIR ops over `%qN` vector vregs (the FP register class already exists; extend the
  RegisterModel to the Q view). NIR→MIR emits `v128.*`; AArch64 select → NEON, byte-id.
- **Pin the lane-semantics contract** (the silent-bug surface for H/I): `fmin/fmax` NaN
  behavior (`fminnm` ↔ x86 `minpd`), `bsl`/`bit` mask polarity (↔ x86 `blendv`), round-mode
  ties (`frintn` round-to-even ↔ `roundpd`), lane-compare result patterns. Capture as a test
  matrix + golden lane vectors so x86/rv64 are validated against the same contract.

## 4. Phases

1. `v128` op set + `%qN` register-class view + AArch64 NEON selection (1:1).
2. Re-emit the transcendental kernels in `v128` (byte-identical; the ULP harness must be
   unchanged — this is the accuracy-critical part).
3. Re-emit `vector::` array ops + the array overloads in `v128`.
4. The lane-semantics test matrix + golden vectors (the contract for H/I).
5. Byte-identical gate (suite) + ULP harness unchanged.

## 5. Validation

- Suite **byte-identical** under `-codegen mir`; **`runtime_ulp.py` unchanged** for every
  kernel (accuracy is non-negotiable — this is where a wrong lane op silently breaks ≤1 ULP).
- The lane-semantics test matrix passes on AArch64 (it becomes the x86/rv64 acceptance later).

## Summary

The biggest and most accuracy-sensitive neutralization: turn the NEON usage into a fixed
`v128` vocabulary that maps cleanly to SSE2+FMA3+SSE4.1 and scalarizes on rv64. Done
byte-identically on AArch64 *and* with the ULP harness held flat, it makes the hand-tuned
kernels and `vector::` write-once — the whole reason SIMD-per-ISA was rejected. The lane
semantics pinned here are the contract the new backends are judged against.
