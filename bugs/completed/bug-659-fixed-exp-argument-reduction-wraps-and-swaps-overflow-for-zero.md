# bug-659: `math::exp` on a large-magnitude `Fixed` wraps its argument reduction — overflow and underflow swap, silently

Last updated: 2026-09-22
Effort: small–medium
Severity: **MEDIUM** — a silently wrong value (`0.00`) where the correct behavior is
`ErrOverflow`, with no diagnostic. The mirrored case raises where it should return zero.
Class: Correctness (Fixed transcendental kernel)

Status: Fixed
Regression Test: `tests/rt-behavior/math/bug659_fixed_exp_large_argument_reduction`
(Phase 1) and `tests/rt-behavior/math/bug659_fixed_pow_fractional_product_wrap`
(Phase 2)

**STATUS: FIXED.** Reproduced first and confirmed to fail for the documented
mechanism, not merely the documented symptom: all five repro lines matched the
filed output on `main` at `5e58663d9`, and the run took 1.88s — the signature of
the negative arm of `emit_fixed_scale_by_power_of_two` halving ~1.44e9 times,
which only a sign-flipped `n` can reach.

The blast-radius audit named `pow` as "very likely" carrying the same swap. It
does, by a **second, independent** wrap one level up: `pow`'s fractional path is
`exp(exponent * ln(base))`, and that product is itself an unchecked Q32.32
multiply. `|ln(base)|` reaches ~22 across the `Fixed` domain, so an exponent past
~1e8 wrapped it. Measured on `main`: `pow(2e9, 200000000.5)` returned `0.00`
where the true result overflows, and `pow(1e-9, 200000000.5)` raised
`ErrOverflow` where the true result underflows to zero. Fixed as Phase 2 with a
saturating multiply; `exp`'s new gate then reads the correct outcome off the
preserved sign. The third `emit_fixed_mul` caller (`log10`'s `ln(x) * inv_ln10`
in `emit_fixed_log`) was audited and cannot wrap: `|ln(x)| <= 22.2` and
`inv_ln10 < 1`. `emit_fixed_log`'s own reduction is a shift-normalisation with no
multiply, so it is not implicated.

Deviation from the fix sketch: the sketch proposed guarding `scaled` so `n`
cannot change sign relative to `x`. The landed fix gates `x` itself instead,
which is strictly simpler and strictly stronger — it removes the wrap rather
than repairing its output, so the sign-dispatch hazard in
`emit_fixed_scale_by_power_of_two` becomes unreachable from `exp` rather than
merely survivable. `emit_fixed_scale_by_power_of_two`'s unchecked negative arm
is left as-is: it is correct for every `n` it can now be handed.

`math::exp` on a `Fixed` argument computes `2^n * exp(r)` with `n = round(x / ln2)`
(`src/codegen/builtins/money/gen_fixed_math.rs:1155` — `emit_fixed_exp`). The `x / ln2`
step is a Q32.32 multiply by `fixed_inv_ln2()` with **no overflow check**, so once
`|x| > 2^31 · ln2 ≈ 1.4885e9` the product leaves `Fixed` range and wraps. `n` is then
extracted from the wrapped value (`n = (scaled + FIXED_HALF) >> 32`,
`gen_fixed_math.rs:1169-1172`) and comes out with the **wrong sign**.

`emit_fixed_scale_by_power_of_two` (`gen_fixed_math.rs:1221`) branches on that sign
(`compare_immediate(n, "0")` / `branch_lt(negative)`, `:1236-1237`): a non-negative `n`
doubles with an overflow check that raises `ErrOverflow`, a negative `n` halves with no
check at all. So a wrapped sign does not merely misreport — it selects the *opposite*
arm, and the overflow check is skipped entirely.

**The single correct behavior a fix produces:** `math::exp` on a `Fixed` raises
`ErrOverflow` for every argument whose true result exceeds `Fixed` range, and returns
`0.00` for every argument whose true result underflows to zero — with no magnitude of
input at which those two swap.

## Failing Reproduction

```basic
IMPORT io
IMPORT math

FUNC probe(v AS Float) AS String
  LET x AS Fixed = toFixed(v)
  LET r = toString(math::exp(x)) TRAP(e)
    RECOVER "RAISED " & toString(e.code)
  END TRAP
  RETURN r
END FUNC

SUB main()
  io::print("exp(-2e9) = " & probe(-2000000000.0))
  io::print("exp(-1e9) = " & probe(-1000000000.0))
  io::print("exp(1e9)  = " & probe(1000000000.0))
  io::print("exp(1.5e9)= " & probe(1500000000.0))
  io::print("exp(2e9)  = " & probe(2000000000.0))
END SUB
```

Observed (macos-aarch64, release compiler built from `main` at `be60f76eb`):

```
exp(-2e9) = RAISED 77050010      <- WRONG: e^-2e9 underflows to 0, must not raise
exp(-1e9) = 0.00                 <- correct
exp(1e9)  = RAISED 77050010      <- correct
exp(1.5e9)= -0.00                <- WRONG: must raise ErrOverflow (note the negative zero)
exp(2e9)  = 0.00                 <- WRONG: must raise ErrOverflow
```

The two behaviors are exactly inverted above the threshold. Measured boundary: `exp(1488522191.0)`
still raises correctly; `exp(1500000000.0)` returns `-0.00`. That bracket contains
`2^31 · ln2 = 1488522190.6`, which is the predicted wrap point of the `x / ln2` product.

Control — the ordinary range is unaffected, so this is a large-magnitude defect only:

```
exp(21)  = 1318815732.25   exp(22) = RAISED 77050010   exp(30)   = RAISED 77050010
exp(-40) = 0.00            exp(-100) = 0.00            exp(-1e6) = 0.00
```

## Root cause

`emit_fixed_exp` (`gen_fixed_math.rs:1158`):

```rust
// n = round(x / ln2).
let inv_ln2 = self.allocate_register();
self.emit_const_i64(&inv_ln2, fixed_inv_ln2());
let scaled = self.emit_fixed_mul(&x, &inv_ln2)?;   // <- unchecked; wraps for |x| > 2^31*ln2
let n = self.allocate_register();
self.emit(abi::move_immediate(&n, "Integer", &FIXED_HALF.to_string()));
self.emit(abi::add_registers(&n, &scaled, &n));
self.emit(abi::arithmetic_shift_right_immediate(&n, &n, 32));
```

`emit_fixed_mul` produces a Q32.32 product with no range check, so `scaled` wraps and the
`asr #32` yields a sign-flipped `n`. `emit_fixed_scale_by_power_of_two` then dispatches on
that sign, and its negative arm has no overflow check (`gen_fixed_math.rs:1259` onward) —
which is why the failure is silent rather than a wrong-but-raised result.

The declared-error list is not implicated: `ErrOverflow` is correctly declared on the
`Fixed` overload (and bug-617 confirms it belongs only there). This is a kernel defect.

## Non-goals

- Changing `math::exp`'s behavior anywhere in the ordinary range. `exp(21)` through
  `exp(1e9)` and `exp(-40)` through `exp(-1e9)` are all correct today and must stay
  byte-identical.
- Changing the `Float` overloads, which raise `ErrFloatInf` by a different path and were
  measured correct.

## Blast-radius audit

- `emit_fixed_scale_by_power_of_two` is shared with `pow` ("Used to recombine the exponent
  in `exp`/`pow`", `gen_fixed_math.rs:1218-1219`). **`math::pow` on a `Fixed` with a large
  exponent must be probed for the same swap** — it very likely has it.
- Any other caller of `emit_fixed_mul` that feeds a shift-count or branch-sign derived from
  the product. Audit for the same unchecked-product-then-sign-branch shape.
- `emit_fixed_log` / the other Fixed transcendentals that perform an argument reduction.

## Fix sketch

Range-check the argument reduction before extracting `n`: if `|x|` exceeds the largest
argument whose `x / ln2` fits Q32.32, the result is already outside `Fixed` range in one
direction or zero in the other, and the branch can be decided from the SIGN OF `x` rather
than from the wrapped product. Equivalently, clamp/guard `scaled` so `n` cannot change sign
relative to `x`. Either way the overflow check must not be skippable by a negative `n`.

## Fix as landed

- [x] **Phase 1 — `emit_fixed_exp` argument gate.** Decide the result from the
  SIGN OF `x` before the reduction runs, not from the wrapped product. `Fixed`
  spans just under `[-2^31, 2^31)`, so `exp` overflows above `ln(2^31) = 21.4876`
  and rounds to zero below `-33*ln2 = -22.8742`; the gate sits at `|x| = 64`,
  outside both thresholds (no in-range result moves) and far inside the wrap
  point `2^31*ln2 = 1.4885e9` (the reduction can no longer leave range, since
  `|64/ln2| < 93`). Above `+64` raise `ErrOverflow`; below `-64` return `0.00`.
  Test: `tests/rt-behavior/math/bug659_fixed_exp_large_argument_reduction`.
  Commit: `0cd31775f`
- [x] **Phase 2 — `pow`'s fractional product saturates instead of wrapping.**
  New `emit_fixed_mul_saturating` tests bits[127:95] of the 128-bit product for
  sign-extension and clamps to `i64::MAX`/`i64::MIN` on the product's true sign;
  bit-identical to `emit_fixed_mul` in range. `emit_fixed_pow_general`'s
  `exponent * ln(base)` uses it, so `exp`'s Phase-1 gate reads the correct
  outcome off the preserved sign.
  Test: `tests/rt-behavior/math/bug659_fixed_pow_fractional_product_wrap`.
  Commit: `0cd31775f`
- [x] **Phase 3 — spec.** `mfb spec architecture math-kernels` gained a
  `Fixed exp and fractional pow` subsection recording the sign-dispatch hazard
  and both gates; the surrounding `Fixed` kernel sections documented the trig
  family's contracts but said nothing about this one. The `mfb man math exp`
  page already stated the correct contract ("a very negative argument gives 0";
  "ErrOverflow for a Fixed") — it was the compiler that disagreed with it, so no
  man change was needed.
  Commit: `0cd31775f`

### Verification

- Original reproduction, all five lines, re-run end to end on macos-aarch64: now
  `0.00 / 0.00 / RAISED / RAISED / RAISED`, and the runtime fell from 1.88s to
  0.00s (the multi-billion-iteration halving loop is gone).
- Every control line in the doc is byte-identical: `exp(21) = 1318815732.25`,
  `exp(22)`/`exp(30)` raise, `exp(-40)`/`exp(-100)`/`exp(-1e6)` are `0.00`.
- Dense self-consistency sweep, `x` from `-30.00` to `24.00` in `0.01` steps
  (5401 points): `exp` is non-decreasing, never negative, and never returns to a
  finite value after its first raise — 0 violations. The raise threshold lands
  at exactly 252 points, i.e. `21.49` upward, matching `ln(2^31) = 21.4876`.
  The `|x| = 64` gate is continuous with that: `exp(63.9)`, `exp(64.0)` and
  `exp(64.1)` all raise; `exp(-63.9)`, `exp(-64.0)` and `exp(-64.1)` are all
  `0.00`.
- **The non-goal is measured, not asserted.** A 29,601-value differential probe
  run under the pre-fix compiler (a detached worktree at `main`, `4576eef56`)
  and the fixed one produced **a zero-byte diff**: `exp` over `[-40.00, 22.00]`
  at `0.01` resolution; `exp(±7e6·k)` for `k = 1..200`, i.e. out to `±1.4e9`,
  which is inside the old wrap point and so must not move; `pow` with fractional
  exponents `1.5 / 0.25 / 4.5 / -2.5` over 3000 bases (the new saturating
  multiply, exercised in range); `pow` with integer exponents `3 / -2` (the
  untouched exact-multiply path); and `log`/`log10` over 5000 arguments (the
  third `emit_fixed_mul` caller). Every value printed at `toByte(10)` precision.
- `scripts/test-accept.sh`: 1513 fixtures, 0 mismatches — every behavioral golden
  held, including the `Fixed` exp/pow fixtures `func_math_exp_fixed_overflow_rt`,
  `func_math_pow_fixed_overflow_rt`, `func_math_pow_fixed_domain_rt` and
  `bug137-fixed-pow-underflow-overflow-rt`.
- **`scripts/artifact-gate.sh` is the gate that caught the one golden this fix
  legitimately moves, and acceptance alone would have missed it.** `.ncodesum` is
  the artifact-gate's own kind — `test-accept.sh` compares no `.ncodesum` on any
  path (`scripts/artifact-kinds.sh:69-72`) — so the acceptance suite's clean run
  was never evidence about emitted code. The gate reported 5 diffs, all of them
  `tests/byte-identity/math`'s per-target `.ncodesum`, the fixture that pins the
  math package's emitted native code and which calls `math::exp(Fixed)` and
  `math::pow(Fixed, Fixed)` directly. Every other package's cover fixture
  (`general`, `http`, `io`, `json`, …) stayed byte-identical, and this fixture's
  own `.ast`/`.ir` goldens were unchanged — the change is lowering-only, and its
  blast radius is exactly the one lowering it touches.
  - The regeneration was justified before it was done, not after: dumping
    `-ncode` under both compilers and comparing label sets shows the delta is
    **purely additive**. The only symbols the new dump introduces are the ones
    this fix creates — `fixed_exp_within_upper`, `fixed_exp_in_range`,
    `fixed_exp_done`, the `fixed_exp_result` slot, `fixed_mul_sat_in_range`,
    `fixed_mul_sat_done` — and **no pre-existing label disappeared**
    (30632 → 30753 instructions). Each target's new hash was confirmed stable
    across repeated builds before being written, since these goldens are
    determinism pins (bug-388).
  - Re-run after regeneration: 1487 tests, 1662 builds, 2102 goldens, **0 diffs**.
- `cargo test --release`: `test result: ok` on all 71 suites, 4293 passed in the
  lib suite, 0 failed. (Run it ALONE: `test-accept.sh` and the `golden` test's
  `artifact_gate_all` take the same per-tree gate lock, so running them
  concurrently makes `artifact_gate_all` fail with "another gate run holds the
  lock" — a refusal that checks nothing, not a regression.)
- `scripts/spec-census.sh --citations`: 1655 citations, 0 MISS-PATH /
  MISS-LINE / MISS-SYMBOL.

## Provenance

Found 2026-09-19 by a sub-audit during bug-617's per-overload error verification (an agent
probing `math::exp`'s `Fixed` overload for `ErrOverflow` reachability noticed
`exp(toFixed(2000000000.0))` returning `0.00`). Independently reproduced and the boundary
bisected on the main thread before filing. Not caused by bug-617's fix, which is
declaration-only and does not touch any lowering — reproduced on the unmodified `main`
compiler.
