# bug-295: x86-64 ties-away rounding emulation double-rounds `0.49999999999999994` to 1 (platform-divergent wrong result)

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (platform divergence)

Status: Fixed
Regression Test: tests/rt-behavior/math/round-ties-away-boundary-rt + src/arch/x86_64/encode/tests.rs::ties_away_model_matches_aarch64_semantics

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

## Resolution

Both sites now compute the fraction exactly instead of adding a half:

    t = trunc(x)        exact
    f = t − x           exact (x − trunc(x) is representable)
    d = trunc(2f)       ∈ {−1, 0, +1}
    result = t − d

No step rounds, so the just-below-half family can never be nudged onto a tie; at a
genuine tie |2f| is exactly 1, which is what carries the away-from-zero step. Beyond
2^52 every double is already an integer, so f is zero and the value passes through.

- scalar (`f2i_nearest`/`fcvtas_x_from_d`): `cvttsd2si` → `cvtsi2sd` → `subsd` →
  `addsd` → `roundsd $3` → `cvttsd2si` → `sub`, borrowing one GPR across the
  sequence.
- packed (`frinta_v`): `roundpd $3` → `movapd` → `subpd` → `addpd` → `roundpd $3` →
  `subpd`. dst holds `t` and src is read-only, so both operands stay live and only
  xmm15 is needed — this version drops the `push rax` / two `movabs` the old
  constant-materialization required.

### Verification, given no x86 hardware

Both remote x86 boxes (2227, 2228) refused connection this session, so the runtime
claim could not be checked on real hardware. Three independent lines of evidence
stand in for it, and none of them is "the tests still pass":

1. **The emitted bytes were disassembled.** The sequences were assembled into an
   x86-64 ELF object via `clang -target x86_64-unknown-linux-gnu` and disassembled
   with `llvm-objdump`, confirming instruction-for-instruction that the encoder
   emits the intended sequence (and that operand order, REX bits and the roundsd
   immediate are right). The byte-exact test pins those verified bytes.
2. **The arithmetic was modelled independently of the encoding.**
   `ties_away_model_matches_aarch64_semantics` evaluates exactly what the sequence
   computes, in Rust f64. Every step is exact in IEEE-754 double, so the model
   computes the same values the SSE sequence does. It asserts the old formula
   returns `1.0` for the reported input and the new one returns `0.0`, covers
   genuine ties in both signs, and sweeps 120 exponents asserting the two formulas
   agree *everywhere except* the family the old one got wrong — so the change is
   surgical, not merely different.
3. **A cross-target runtime fixture.** `round-ties-away-boundary-rt` records the
   aarch64 reference answers (`below=0`, `half=1`, `twoHalf=3`, …). It passes here
   and is what fails on linux-x86_64 if the leg regresses.

Runtime confirmation on an x86 box remains outstanding and should be run when one is
reachable; the fixture is already in place to do it.

### A test asserted the old mechanism and was rewritten, not deleted

`f2i_nearest_never_touches_rax` (bug-17) failed. It made two assertions: no `movabs`
in the sequence, and a literal `0x3FE` immediate present. The first is the property
bug-17 existed to protect; the second pinned the *copysign-materialization
mechanism*, which this fix removes entirely — the sequence no longer builds
`bits(0.5)` at all.

The invariant was preserved and strengthened rather than dropped. The test is now
`f2i_nearest_never_clobbers_its_own_dst`: it keeps the no-`movabs` check and adds a
direct assertion of what bug-17 actually cared about — that the GPR borrowed for the
correction is pushed first, popped last, and is never `dst` — checked across
`rax`, `rcx`, `rbx` and `r10`. The `dst == rax` case bug-17 was filed for is covered
explicitly (the sequence borrows rcx there).

Artifact gate: 1171 goldens across 990 tests, 0 diffs — the change is confined to
the two x86 sequences, which no aarch64 golden exercises. Full `cargo test` green.
