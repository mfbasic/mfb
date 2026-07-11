# bug-16: Vector 64-bit-lane shift by exactly 64 is mishandled on rv64 (encode error) and x86-64 (silent 0 instead of sign-fill)

Last updated: 2026-07-08
Effort: small (<1h)

AArch64 `SSHR`/`USHR`/`SHL` on `.2d` (64-bit) lanes permit a shift amount of
**64** (arithmetic shift → sign-fill the lane; logical → zero it). Both the rv64
and x86-64 v128 lowerings forward the raw AArch64 shift immediate to their scalar
shift encoders **without** handling the `== 64` boundary, so a `.2d`/i64x2 shift
of 64 is mishandled:

- **rv64** (`src/arch/riscv64/encode/operand.rs:106-114`, `shift()`): rejects
  `value >= 64` with `"rv64 shift immediate 64 is out of range"`, aborting the
  whole rv64 encode. Fail-loud (no wrong runtime value), reached from
  `v128.rs` `ShlV`/`SshrV`/`UshrV` (`riscv64/v128.rs:484-496`) via the emitter's
  `lsl_imm`/`lsr_imm`/`asr_imm`.
- **x86-64** (`src/arch/x86_64/encode/emitter.rs:1138-1158`, `sshr_v`): reads the
  count with a bare `.parse::<u8>()` (no `< 64` guard) and only builds the
  sign-fill for `0 < k < 64` (`:1146`); for `k >= 64` it clears the sign xmm and
  emits `psrlq dst, k`, which on x86 saturates a count `>= 64` to produce **0**.
  So an arithmetic shift-right of a **negative** i64 lane by 64 yields `0` instead
  of the AArch64 result `-1` (sign-fill) — a **silent wrong value**.
  `shl_v`/`ushr_v` (`:1020-1036`) share the missing bound, but for the
  logical/left forms `0` matches AArch64, so only the signed `sshr_v` is a
  correctness defect; the missing bound on the others is defense-in-depth.

The single correct behavior a fix produces: a 64-bit-lane vector shift by 64
produces the AArch64-defined result on both backends (arithmetic → sign-fill;
logical/left → zero), and never aborts the encode or silently returns 0 for the
signed case.

Severity LOW: latent — depends on whether any shipped kernel / `vector::` op
actually emits a `.2d`/i64x2 shift of exactly 64. rv64 fails loud (no wrong
result); x86 is a silent miscompile of the signed variant when triggered.

References:

- rv64: `src/arch/riscv64/encode/operand.rs:106-114` (`shift()` caps `< 64`);
  reached via `src/arch/riscv64/v128.rs:484-496` (`ShlV`/`SshrV`/`UshrV`) and the
  emitter's `lsl_imm`/`lsr_imm`/`asr_imm`.
- x86: `src/arch/x86_64/encode/emitter.rs:1138-1158` (`sshr_v`, count parsed at
  `:1141-1143`, sign-fill only for `0 < k < 64` at `:1146`), `:1020-1036`
  (`shl_v`/`ushr_v`, same missing bound).
- Contrast: scalar shifts route through range-checked paths and are folded/bounded
  `< 64` at the source level, so scalar `lsl_imm`/`shift_imm` never hit this.
- Found during goal-01 review of `src/arch/riscv64/**` and `src/arch/x86_64/**`.

## Failing Reproduction

No confirmed source path emits a 64-bit-lane shift of exactly 64 today (the
transcendental/`vector::` kernels' shift amounts were not shown to reach 64), so
this is demonstrated at the encoders:

- rv64: `shift("64")` → `Err("rv64 shift immediate 64 is out of range")` (whole
  encode aborts).
- x86: `sshr_v` with lane value `0x8000_0000_0000_0000` (negative) and `k = 64` →
  result lane `0x0000_0000_0000_0000`, but AArch64 `sshr .2d, #64` →
  `0xFFFF_FFFF_FFFF_FFFF`.

- Observed: rv64 = encode error; x86 signed = `0`.
- Expected: sign-filled lane (`-1`) for arithmetic; `0` for logical/left.

Contrast: shift amounts `0..=63` encode correctly on both backends (x86's
`0 < k < 64` sign-fill emulation is correct); `k = 0` is handled.

## Root Cause

The v128 shift lowerings forward the AArch64 immediate verbatim to scalar shift
encoders whose valid range is `0..=63` (RISC-V masks shamt to 6 bits; x86 `psrlq`
count `>= 64` saturates to 0). Neither backend special-cases the AArch64-legal
`.2d` shift of 64, and x86's `sshr_v` never range-checks the count at all.

## Goal

- A 64-bit-lane vector shift by 64 produces the AArch64 result on rv64 and x86-64.

### Non-goals (must NOT change)

- Shift amounts `0..=63` on either backend (correct today).
- Scalar shift lowering.

## Blast Radius

- rv64 `ShlV`/`SshrV`/`UshrV` (`v128.rs:484-496`); x86 `sshr_v`/`shl_v`/`ushr_v`.
  All 64-bit-lane vector shifts. Fixed by this bug.

## Fix Design

In each backend's v128 shift lowering, special-case `shift == 64`:
- arithmetic (`SshrV`/`sshr_v`): produce the sign mask (rv64: `asr by 63`; x86:
  the existing `pcmpgtq` sign lane, then move it into every result lane);
- logical/left (`UshrV`/`ShlV`/`ushr_v`/`shl_v`): write a zero lane.
Additionally range-check x86 `sshr_v`'s count (route through the same guard used
by `operand::shift`) so an unexpected `> 64` is rejected rather than silently
truncated.

## Phases

### Phase 1 — failing test + audit

- [ ] rv64: assert the v128 lowering of a `.2d` shift of 64 encodes (does not
      error). x86: assert `sshr_v(negative, 64)` yields an all-ones lane.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Special-case `shift == 64` in both backends' v128 shift lowerings; add the
      x86 count range-check.

### Phase 3 — validation

- [ ] `scripts/artifact-gate.sh`; rv64 + x86 runtime validation of a 64-bit-lane
      shift-by-64 kernel, byte-identical to aarch64.

## Validation Plan

- Regression test(s): the encoder tests above.
- Runtime proof: a vector program shifting an i64x2 lane by 64 on rv64 and x86-64,
  matching aarch64.
- Full suite: `scripts/artifact-gate.sh` + cross-arch validation.

## Summary

The risk is only in the x86 signed sign-fill for the `== 64` boundary; the fix is
a small special-case in each backend's v128 shift lowering plus an x86 count
guard.
