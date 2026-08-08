# plan-86-G — bounds-check elimination (induction-var get/set)

Sub-plan **G** of [plan-86](plan-86-benchmark-perf.md). Open. Correctness-critical dataflow.

**Covers (1 P1 + 1 P2 + 2 P3):** scalarbench listchurn (10.6), bignum modmul (19.5)/modexp (10.9),
mathpipe memo (11.5).

## Root cause
Each hot loop does `collections::get`/`set` on a **loop-invariant-length** list indexed by a **loop-induction
variable**, paying a per-access bounds check the C peers (stack arrays) and Python don't. bignum is a
bounds-check row, not an algorithm swap — the C mirror `benchmark/c/main.c:365` is also bit-serial.

## Fixes
- [ ] **G1** — emit an unchecked `get`/`set` when a dataflow pass proves `0 ≤ index < len` from the induction
  bound and a loop-invariant length. **An unsound elision is silent memory unsafety** (not a
  checksum-catchable wrong answer) — held to the `.ai/compiler.md` register-lifetime + verification bar.
  Covers bignum modmul/modexp, scalar listchurn, mathpipe memo.
- [ ] **G2 (memo only)** — strength-reduce the constant `MOD 1000000007` to a conditional subtract (sum of
  two values each < m is < 2m).

## Acceptance
bignum/memo/scalar checksums + overflow behavior + `scripts/artifact-gate.sh` + a targeted out-of-bounds
**negative test proving the elision does NOT fire when the bound is not provable**.
