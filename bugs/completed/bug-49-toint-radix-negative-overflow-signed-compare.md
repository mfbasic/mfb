# bug-49: `toInt(text, base)` silently returns a wrapped value instead of `ErrOverflow` for out-of-range negatives in power-of-two bases

Last updated: 2026-07-09
Effort: small (<1h)

The two-argument radix `toInt(text AS String, base AS Integer)` uses **signed**
register comparisons in its per-digit overflow guard, but the guard's `cutoff`/`cutlim`
constants are derived from an **unsigned** `limit` of `2^63`. For a negative magnitude
in a power-of-two base whose `cutoff * base` lands exactly on `2^63` (base 2, 8), the
accumulator can reach exactly `2^63`, which as an `i64` register is negative; the
signed compare on the next digit then sees a negative accumulator as *less than*
`cutoff`, skips the overflow trap, and the following `acc * base` wraps — producing a
silent wrong value instead of the `77050010 ErrOverflow` the function documents.

Runtime-confirmed on macOS/aarch64: `toInt("-1" + 64×"0", 2)` (magnitude `2^64`)
returns `0`, while the in-range control `toInt("-1" + 63×"0", 2)` correctly returns
`-9223372036854775808` (`i64::MIN`). The single correct behavior a fix produces: any
radix parse whose value falls outside `[i64::MIN, i64::MAX]` raises `ErrOverflow`.

References:

- `src/target/shared/code/builder_conversions.rs:emit_string_to_int_value_base`,
  limit/cutoff setup `:357-369`, per-digit guard `:395-401`
  (`compare_registers(acc, cutoff)` + signed `branch_gt`/`branch_eq`).
- Contrast: `emit_string_to_int_value` (base-10, `:213-214`) hardcodes
  `cutoff = 922337203685477580` (~`i64::MAX/10`), so `acc` never reaches `2^63` and the
  signed compares stay correct.
- Spec: `toInt` radix form must fail with `ErrOverflow` (`77050010`) out of range —
  see `src/docs/man/builtins/general/toInt*` and the base-10 path's behavior.
- Related radix-parsing bug (different site): bug-41 item (3) (Byte-literal RECOVER
  radix range check). Same "radix path mishandles a boundary" family, distinct code.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

```
IMPORT io
FUNC tryParse(s AS String) AS Integer
  RETURN toInt(s, 2)
END FUNC
FUNC main AS Integer
  LET big AS String = "-10000000000000000000000000000000000000000000000000000000000000000"
  LET a AS Integer = tryParse(big) TRAP(e)
    io::print("trapped (correct)")
    RECOVER 999
  END TRAP
  io::print("big result: " & toString(a))
  RETURN 0
END FUNC
```

- Observed: prints `big result: 0` — no trap, silent wrong value.
- Expected: prints `trapped (correct)` (`ErrOverflow`), because `2^64` is out of `i64`
  range.

Contrast cases that work correctly today (regression guards):

- `toInt("-1" + 63×"0", 2)` → `-9223372036854775808` (exactly `i64::MIN`, in range) —
  must stay accepted.
- Non-power-of-two bases (e.g. base 16): `acc` overshoots `cutoff` while still positive
  (`2^56 → 2^60`), so `branch_gt` fires and they trap correctly.
- Positive inputs use `limit = i64::MAX`; `acc` stays positive and the signed compares
  are correct.
- Base-10 `toInt` (one-arg and two-arg base 10) is unaffected.

## Root Cause

`emit_string_to_int_value_base` computes `cutoff = limit / base` and
`cutlim = limit - cutoff*base` with `limit = 2^63` for the negative case using
`unsigned_divide`, then guards each digit with **signed** `branch_gt`/`branch_eq` on
`acc` vs `cutoff`. For base 2, `cutoff = 2^62`, `cutlim = 0`, and the `acc == cutoff &&
digit <= cutlim` path lets `acc` become exactly `2^63`. As an `i64`, `2^63` is
`i64::MIN` (negative), so on the next digit both `acc > cutoff` and `acc == cutoff` are
false under signed comparison — the trap is skipped and `acc * 2` wraps to `0`. The
constants were computed unsigned but the comparison is signed: the mismatch is the bug.

## Goal

- `toInt(text, base)` raises `ErrOverflow` for any value outside `[i64::MIN, i64::MAX]`,
  in every base including 2 and 8, for negative and positive magnitudes.
- Exactly-`i64::MIN` and exactly-`i64::MAX` inputs remain accepted.

### Non-goals (must NOT change)

- In-range parsing in any base (including the `i64::MIN` boundary).
- The base-10 fast path.
- The set of error codes / trap behavior — only the *reachability* of `ErrOverflow`
  changes for the currently-silent case.

## Blast Radius

- `emit_string_to_int_value_base` — fixed here. Used by the two-arg `toInt` radix form;
  grep confirms other radix consumers (encoding/json/net dedup via `toInt`, per the
  plan-02-cleanup memory note) route through this same helper and inherit the fix.
- `emit_string_to_int_value` (base 10) — unaffected (own correct constants).

## Fix Design

Use **unsigned** comparisons for the `acc` vs `cutoff` and `digit` vs `cutlim` guards
in `emit_string_to_int_value_base` — `branch_hi`/`branch_hs` instead of the signed
`branch_gt`/`branch_ge`. `cutoff`/`cutlim` were already computed against an unsigned
`limit`, so unsigned comparison is the consistent choice: it treats a `2^63`
accumulator as greater than `cutoff` and traps. For positive inputs `acc < 2^63`, where
unsigned and signed order agree, so there is no regression.

Rejected alternative: special-case power-of-two bases. Rejected — the unsigned-compare
fix is the root-cause correction and covers every base uniformly; a base-specific patch
would leave the signed/unsigned mismatch latent for the next boundary.

## Phases

### Phase 1 — failing test

- [x] Add radix `toInt` overflow tests: `2^64`-magnitude negative in base 2 and base 8
      must trap `ErrOverflow`; `i64::MIN` and `i64::MAX` boundaries in those bases must
      parse. Confirm the out-of-range cases return `0` (wrong) today.

### Phase 2 — the fix

- [x] Switch the `acc`/`digit` guards in `emit_string_to_int_value_base` to unsigned
      comparisons.

### Phase 3 — validation

- [x] Regenerate codegen goldens (delta confined to the radix `toInt` helper — no
      existing test carries native goldens for the two-arg `toInt`, so none shift).
- [x] Runtime reproduction re-run; `scripts/artifact-gate.sh` / `scripts/test-accept.sh`
      are run by the orchestrator.

## Validation Plan

- Regression test(s): the base-2/base-8 overflow-and-boundary tests above.
- Runtime proof: build and run the reproduction; the out-of-range parse must trap.
- Doc sync: none expected (behavior now matches the documented `ErrOverflow`).
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

A signed comparison guarding an unsigned-derived cutoff lets a power-of-two radix parse
overshoot `i64::MIN` and wrap to `0` silently. The fix is to make the guard unsigned,
matching how the cutoff was computed; only the currently-silent out-of-range case
changes, and it changes from a wrong value to the documented `ErrOverflow`.

## Resolution

Fixed in `src/target/shared/code/builder_conversions.rs`
(`emit_string_to_int_value_base`). The two per-digit overflow guards were switched
from the signed `branch_gt` to the unsigned `branch_hi`:

- `compare_registers(acc, cutoff)` → `branch_hi(&overflow)` (was `branch_gt`).
- `compare_registers(digit, cutlim)` → `branch_hi(&overflow)` (was `branch_gt`).

The `branch_eq` mid-check is sign-agnostic and unchanged. `cutoff`/`cutlim` are derived
from an unsigned `limit` (`2^63` for negatives), so unsigned comparison is the consistent
root-cause fix: a `2^63` accumulator (negative as an `i64` register) is now correctly
seen as greater than `cutoff = 2^62` and traps. The stale comment claiming signed
compares was corrected too. Lowering is shared (the `aarch64` `abi` module is the neutral
MIR emitter used by all backends), so the fix applies to aarch64, x86_64, and riscv64
uniformly.

Runtime proof (macOS/aarch64): the doc's reproduction now prints `trapped (correct)`
instead of `big result: 0`; `toInt("-1"+64×"0", 2)` traps `77050010 ErrOverflow`, while
`i64::MIN` (`-1` + 63×`0`, base 2 / `-1000000000000000000000`, base 8) and `i64::MAX`
still parse.

Tests:

- `tests/acceptance/src/general.mfb` — new TCASE "power-of-two radix boundaries (bug-49)"
  asserts the `i64::MIN`/`i64::MAX` boundary parses in base 2 and base 8 (valid path).
  `mfb test tests/acceptance` → 349/349 pass.
- `tests/rt-error/general/toInt_radix_overflow_pow2_neg/` — new golden test: `-2^64` in
  base 2 must fail `77050010 ErrOverflow` (was silently `0` before the fix). Runtime
  `[exit 255]`.

Golden impact: no existing test carries native `.ncode/.nplan/.nir/.mir` goldens for the
two-arg `toInt`, so the codegen delta shifts no existing goldens; only the new test's
own goldens are added.
