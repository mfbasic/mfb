# bug-405: `integer_constant_value` negates an `i128` without `checked_neg`, so an `i128::MIN` literal panics the verifier in debug builds

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Footgun / panic (integer overflow on malformed-but-structurally-valid IR)

Status: FIXED (3528e0e1d)
Regression Test: tests/ — a verify unit test with `Unary("-", Const{Integer,
i128::MIN})` reaching the `EXIT PROGRAM` code path must return a range error, not
panic. Added as `ir::verify::tests::rejects_exit_program_i128_min_without_panic`
(`src/ir/verify/tests.rs`): confirmed RED (`attempt to negate with overflow` at
`mod.rs:1105`), now GREEN (emits `EXIT_PROGRAM_CODE_OUT_OF_RANGE`, no panic).

## STATUS: FIXED (commit 3528e0e1d)

`integer_constant_value`'s `Unary("-")` arm now negates with `wrapping_neg()`
instead of `.map(|n| -n)`, so a negated `i128::MIN` operand no longer
overflow-panics in debug builds.

Deviation from the doc's Goal (which named `checked_neg()`): a literal
`checked_neg()` returns `None` on `i128::MIN`, and the caller
(`src/ir/verify/ops.rs:412`) treats `None` as "not a constant" and skips the
range check — silently *accepting* the malformed exit code. That would regress
the current **release** behavior, which wraps `-i128::MIN` back to `i128::MIN`
and correctly emits `EXIT_PROGRAM_CODE_OUT_OF_RANGE`, and contradicts this doc's
own Regression Test ("must return a range error"). `wrapping_neg()` makes the
debug build match release exactly: no panic, and the still-out-of-range value is
reported rather than dropped. Sole negation site (Blast Radius confirmed); the
`syntaxcheck` front-end never negates an `i128` (it bounds the magnitude as a
`u64`), so no sibling fix was needed.

Verified: full bin-target suite green (3726 passed, 0 failed) after merging
current `main`; no goldens/codegen shifted (verifier-only, crafted-IR path).

`integer_constant_value` (`src/ir/verify/mod.rs:1105`) computes
`integer_constant_value(operand).map(|n| -n)` on an `i128`. A crafted
`Unary("-", Const{Integer, "-170141183460469231731687303715884105728"})`
(`i128::MIN`) parses to `i128::MIN`, and `-i128::MIN` overflows. This is reached
from the `EXIT PROGRAM` arm (`src/ir/verify/ops.rs:412`), which passes the raw
value with no prior literal-range check on this operand.

In a **debug** build this panics (arithmetic-overflow) *inside the verifier* —
which must return `Err`, not panic — and `cargo test` runs debug, so a malformed
`.mfp`/IR fixture would trip it under test. In shipping **release** builds the
negate wraps back to `i128::MIN` and the code merely emits
`EXIT_PROGRAM_CODE_OUT_OF_RANGE`, so there is no crash in production. Hence LOW.

References:

- `src/ir/verify/mod.rs:1105` (`integer_constant_value` unchecked `-n`), reached
  from `src/ir/verify/ops.rs:412` (`EXIT PROGRAM`). Found during goal-07.

## Failing Reproduction

Static analysis (no crafted `.mfp`/unit-IR fixture built). `-i128::MIN` is UB-free
in release (wraps) but a debug-build overflow panic.

- Observed (debug): `attempt to negate with overflow` panic in the verifier.
- Expected: a bounded `EXIT_PROGRAM_CODE_OUT_OF_RANGE` (or similar) error.

## Root Cause

`-n` on an `i128` without `checked_neg()`; `i128::MIN` has no positive counterpart.

## Goal

- [x] `integer_constant_value`'s negation is overflow-safe (`wrapping_neg()`, see
  STATUS block for why not `checked_neg()`), so no operand value can panic the
  verifier in any build profile.

### Non-goals (must NOT change)

- The range-checking semantics for in-range exit codes.

## Blast Radius

- `src/ir/verify/mod.rs:1105` — fixed by this bug. Grep for other `-n` / negation
  of parsed `i128` literals in verify/ to confirm no sibling site.
