# bug-295: x86-64 ties-away rounding emulation double-rounds `0.49999999999999994` to 1 (platform-divergent wrong result)

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (platform divergence)

Status: Open
Regression Test: tests/rt-behavior (new) — `math::round(0.49999999999999994)` is 0 on all targets

The x86 backend emulates AArch64 ties-away rounding as `trunc(x + copysign(0.5, x))`.
The addition itself rounds: for `x = 0x3FDFFFFFFFFFFFFF` (0.49999999999999994 =
0.5 − 2⁻⁵⁴), `x + 0.5 = 1 − 2⁻⁵⁴`, which is exactly halfway between the predecessor
of 1.0 and 1.0; round-to-nearest-even picks 1.0, so `cvttsd2si` yields 1 (and −x
yields −1). AArch64 `fcvtas`/`frinta` and riscv `fcvt` RMM return 0 — the input is
strictly below 0.5, not a tie — so on linux-x86_64 the result is simply wrong and
divergent from every other target.

The single correct behavior a fix produces: ties-away rounding of a value strictly
below 0.5 in magnitude yields 0 on x86-64, matching aarch64/riscv64 (no double
rounding).

References:

- `bugs/completed-bugs/bug-158-*` (riscv fmls two-rounding — same cross-ISA
  divergence class, file-worthy).
- Found during goal-06 review of `src/arch/x86_64/encode/emitter.rs`.

## Failing Reproduction

```
io.print(toString(math::round(0.49999999999999994)))
```

- Observed (linux-x86_64): `1` (and `-1` for the negation).
- Expected (all targets): `0`.

Also affects `toFixed(Float)` (`builder_conversions.rs:1246`) and `math::round` over
Float lists (`builder_simd_math` RoundFloat → `frinta_v`).

## Root Cause

`src/arch/x86_64/encode/emitter.rs:848-864` (`f2i_nearest`/`fcvtas_x_from_d`) and
`:1088-1104` (`frinta_v`): both compute `trunc(x + copysign(0.5, x))`, and the
`x + 0.5` addition rounds a just-below-0.5 input up to an exact 0.5-tie that then
rounds to 1.0.

## Goal

- x86 ties-away rounding returns the same integer as AArch64 `fcvtas`/`frinta` for
  all doubles, including the `0.5 − 2⁻⁵⁴` family.

### Non-goals (must NOT change)

- Correct results for genuine ties and other inputs (must stay identical).
- aarch64/riscv64 paths.

## Blast Radius

- `f2i_nearest`/`fcvtas_x_from_d` and `frinta_v` in the x86 emitter — fixed here.
- Consumers `math::round(Float)`, `toFixed(Float)`, Float-list round — all benefit.

## Fix Design

Compute the fraction exactly and correct: `t = trunc(x)`, `f = x − t` (exact for
|x|<2⁵² by Sterbenz), add ±1 when `|f| ≥ 0.5`; or use `roundsd` to nearest-even then
fix the exact-half cases via the sign/fraction bits. Rejected: the current
`x + 0.5` add-then-trunc — the addition's rounding is the bug.

## Phases

### Phase 1 — failing test
- [ ] rt-behavior test for `0.49999999999999994` (and its negation), plus a
      genuine-tie contrast (`0.5 → 1`, `2.5 → 3`). Confirm x86 returns 1 today.
### Phase 2 — the fix
- [ ] Replace the add-then-trunc with the exact-fraction correction in both sites.
### Phase 3 — validation
- [ ] Artifact gate + rt-behavior suite green on x86; results match aarch64/riscv64.

## Validation Plan

- Regression: the boundary + tie tests, cross-checked against aarch64 output.
- Runtime proof: `math::round(0.49999999999999994) == 0` on x86.
- Doc sync: none.

## Summary

Add-then-truncate double-rounds a just-below-half input to 1 on x86 only; an
exact-fraction correction restores cross-target agreement. Risk is getting the
tie/half handling exactly right for both scalar and SIMD sites.
