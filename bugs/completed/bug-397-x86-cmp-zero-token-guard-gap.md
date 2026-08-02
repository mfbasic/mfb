# bug-397: x86_64 `cmp`/`cmp_imm` (and the `lhs` side of `enc_add_carry`/`enc_sub_borrow`) lack the zero-token guard, so an `xzr` operand would silently encode as `r8`

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Memory-safety / Correctness (latent, defense-in-depth)

Status: FIXED (merge 23cc215ca / fix 92591fecf)
Regression Test: tests/ — an encoder unit test asserting `cmp`/`cmp_imm` with a
zero-token (`xzr`) operand errors (or encodes a true zero read) rather than
emitting `r8`.

## STATUS: FIXED (merge 23cc215ca, fix 92591fecf)

All four unguarded paths in the r8-sentinel family now treat a zero-token operand
the way `alu3`/the carry-`rhs` guards do — an explicit zero read or a loud error,
never a silent `r8`:

- `cmp` (`emitter.rs`): a zero-token `rhs` routes to the immediate form
  `cmp lhs, 0` (`enc_alu_imm32(7, lhs, 0)`); a zero-token `lhs` (`cmp 0, rhs`, no
  scratch-free x86 form) is a loud error.
- `cmp_imm`: a zero-token `lhs` is a loud error.
- `enc_add_carry`: a zero-token `lhs` is normalized into `rhs` (add commutes), so
  the existing guarded `rhs`-zero path handles it; both operands zero → loud error.
- `enc_sub_borrow`: a zero-token `lhs` is a loud error (subtraction does not
  commute and has no CF-preserving scratch-free form).

Reproduction was unit-level (no MIR producer emits these shapes). RED-verified
before the fix: `cmp rax, xzr` encoded `[0x4C,0x39,0xC0]` = `cmp rax, r8`, and
`add_carry` with a zero `lhs` encoded `mov rbx, r8; add rbx, rdi` — the exact r8
leak. Five new tests (`cmp_zero_token_rhs_compares_immediate_zero`,
`cmp_zero_token_lhs_rejected`, `cmp_imm_zero_token_lhs_rejected`,
`add_carry_zero_token_lhs_commutes`, `sub_borrow_zero_token_lhs_rejected`) are now
GREEN. Full suite green: `mfb` bin 3697/0, full workspace `cargo test` exit 0.

Because no current producer emits these operand shapes, this is a byte-identity
no-op for every real program — no golden/artifact-gate churn. Defense-in-depth,
as ranked.

In the x86_64 encoder, the zero register is a sentinel: `reg("xzr")` returns 16,
and `modrm`/REX encoding takes `16 & 7 == 0` plus a REX.R/REX.B bit, so a
zero-token operand that reaches a raw `alu_rr`/`modrm` path silently becomes **r8**
— reading a caller's leftover `r8` value instead of the intended zero. This exact
family produced shipped bugs bug-123 and bug-154, which were fixed by adding
`is_zero_token` guards — but only to `enc_add_carry`/`enc_sub_borrow` (guarding the
`rhs` only) and `alu3` (guards a zero `lhs`, errors on a zero `rhs`).

Two paths in the same family remain unguarded:

1. `cmp` (`src/arch/x86_64/encode/emitter.rs:643`) calls `alu_rr(0x39, lhs, rhs)`
   with **no** zero-token guard on either operand; `cmp_imm` likewise has no `lhs`
   guard.
2. `enc_add_carry`/`enc_sub_borrow` guard only `rhs`, not `lhs` — a zero-token
   `lhs` there would also encode as `r8` via `enc_mov`/`alu_rr`.

**This is latent / defense-in-depth, not a live miscompile.** A grep of the shared
lowering (`abi::subtract_registers`, `compare_registers`, the `cmp` producers in
`mir.rs`) finds no current producer that emits `cmp` with `xzr`, nor
`add_carry`/`sub_borrow` with a zero-token `lhs`: comparisons against zero lower
through `cmp_imm`/`fcmp_zero`, and negations use `subtract_registers(dst, ZERO, x)`
which routes through `alu3`'s guarded zero-`lhs` path. Ranked LOW accordingly —
the value is closing the same defensive gap that already produced two shipped bugs
before a new producer trips it.

References:

- `src/arch/x86_64/encode/emitter.rs:643` (`cmp` → unguarded `alu_rr`), `cmp_imm`
  (no lhs guard), `enc_add_carry`/`enc_sub_borrow` (rhs-only guard).
- Prior same-family fixes: bug-123, bug-154 (`bugs/completed/`). Found during
  goal-07.

## Failing Reproduction

Not reachable from any current MIR producer (see above), so no end-to-end repro.
The encoding defect is demonstrable at the unit level: constructing a `cmp` op
whose operand is the zero token and asserting the emitted ModRM selects the zero
read rather than `r8`.

- Observed (by inspection): `cmp`/`cmp_imm` with a zero-token operand → ModRM/REX
  encodes `r8`.
- Expected: an explicit zero read (or a loud encoder error), matching the guarded
  `alu3`/`enc_add_carry(rhs)` paths.

## Root Cause

The `is_zero_token` remediation for the r8-sentinel collision (bug-123/bug-154) was
applied narrowly to `enc_add_carry`/`enc_sub_borrow` (rhs) and `alu3`, but not to
`cmp`/`cmp_imm` or the `lhs` side of the carry helpers.

## Goal

- `cmp`/`cmp_imm` and both operand sides of `enc_add_carry`/`enc_sub_borrow` treat
  a zero-token operand the same way `alu3` does (explicit zero, or a loud error) —
  never silently encoding `r8`.

### Non-goals (must NOT change)

- No change to the register-sentinel scheme or to the already-correct
  `alu3`/carry-`rhs` guards.

## Blast Radius

- `src/arch/x86_64/encode/emitter.rs`: `cmp`, `cmp_imm`, `enc_add_carry` (lhs),
  `enc_sub_borrow` (lhs) — fixed by this bug.
- aarch64/riscv64 encoders: not affected — their zero register (`xzr`/`x0`) is a
  real architectural register, not an out-of-range sentinel.
