# plan-25-C: Return-value copy elision + bignum in-place list set

Last updated: 2026-07-05
Effort: medium (1h–2h)

Two smaller, independent wins:

1. **`RETURN param` deep-copies the whole collection.** The `list copy` benchmark
   (`copyStrings`/`copyRecs` are just `RETURN xs`) costs **31.6 ms** vs Python's
   2.5 — a full header+entry+data deep copy of a 1000-element list, done 2000×.
   When a function returns a parameter unchanged and that parameter is not used
   after the return, the copy is unnecessary.
2. **`collections::set(list, i, v)` in bignum's inner loop rebuilds the list.**
   `bignum modmul` (234 ms) / `modexp` (128 ms) lose to interpreted Python; the
   schoolbook multiply's `r = collections::set(r, idx, ...)` inner line is the
   hot path. There is already an in-place list-set fast path
   (`try_inplace_set_assign`); confirm it fires here and widen it if the
   fixed-width Integer case is missing headroom.

It complements:

- `./mfb spec language functions` (return semantics — the copy-elision must not
  change observable value semantics).
- `planning/plan-25-A-arena-large-block.md` (A removes the churn component of
  bignum's cost; C removes the algorithmic rebuild).

## 1. Goal

- A `RETURN <param>` (or `RETURN <local>` whose value is provably not used after
  the return) returns the value without a deep copy, when doing so cannot violate
  value semantics (caller copies on bind if it needs an independent value).
- `bignum` inner-loop `collections::set` on a uniquely-owned MUT Integer list is
  an in-place fixed-width overwrite (no rebuild, no alloc).

### Non-goals (explicit constraints)

- No change to observable value semantics: if eliding the copy could let a caller
  observe aliasing (mutation of the returned value affecting the callee's local,
  or vice versa), the copy must stay. The elision is sound only when the source is
  dead after return.
- No change to layout/ABI.

## 2. Current State

**Return copy.** `lower_returned_value`
(`src/target/shared/code/builder_codegen_primitives.rs:~1684`) calls
`value_needs_owning_copy` → `copy_flat_block` → `copy_collection_tight`
(`builder_collection_layout.rs`) for any returned local/parameter of a freeable
collection type. `value_is_aliasing_source` (`builder_values.rs:74-84`) treats
`NirValue::Local` as always-aliasing, so every `RETURN xs` copies. For a
1000-element `List OF String` that is ~40 KB memcpy × 2000 calls ≈ the measured
31 ms.

**bignum set.** `try_inplace_set_assign`
(`builder_inplace_assign.rs:77+`) already handles `name = collections::set(name,
idx, item)` on a uniquely-owned MUT list, overwriting in place when
`newLen ≤ oldLen` (always true for fixed-width Integer). Need to confirm the
bignum limb lists (`List OF Integer`) actually hit this path and are not demoted
by an escape/aliasing check inside the `bnMul` helper
(`benchmark/mfb/src/main.mfb` bignum section).

## 3. Design Overview

- **C1 — copy elision on dead-source return.** In the return lowering, when the
  returned value is a `Local`/parameter and a simple liveness check shows it is
  not read on any path after the `RETURN` (for a `RETURN` as the last executed
  statement of the function, the local is trivially dead), skip
  `copy_flat_block` and return the existing pointer. The callee's stack slot is
  torn down at return anyway; ownership transfers to the caller, which already
  performs its own copy-on-bind when binding to a fresh `LET`/`MUT`.
- **C2 — verify/extend in-place set.** Trace the bignum `collections::set` to
  confirm in-place firing; if a generic/escape gate blocks it, widen the gate the
  same way plan-25-B widens append (uniquely-owned MUT list, fixed-width element).

Correctness risk is entirely in C1's liveness/aliasing judgment — be
conservative: elide only the `RETURN name` tail case where `name` is a
parameter or a local with no later use, and where the return type's copy is what
the caller would otherwise redo. When in doubt, keep the copy.

## Phases

### Phase 1 — C2: confirm/extend bignum in-place set (lowest risk)

- [x] Add a timed runtime proof isolating `bnMul` inner-loop `collections::set`;
      dump `-nir` to confirm in-place vs rebuild.
- [x] If rebuild: widen `try_inplace_set_assign` gate for the uniquely-owned MUT
      `List OF Integer` fixed-width case (`builder_inplace_assign.rs`).

**Result: already fires — no widening needed.** A debug-instrumented build proved
`bnMul`'s two `r = collections::set(r, idx, ...)` sites both hit
`try_inplace_set_assign` → `lower_list_set_in_place`. For a `List OF Integer` the
payload is fixed-width (8 bytes), so `need == oldLen` always and the same-size
overwrite branch runs (no rebuild, no alloc). plan-25-B's set fast path already
covers this case; the C2 gate did not need extending. modmul/modexp gains come
from C1 (below) removing the helpers' `RETURN r` copies, not from the set (which
was already in-place).

Acceptance: bignum `set` lowers to in-place overwrite (confirmed); acceptance green.
Commit: 599885f8 (no code change — verification only; folded into C1's commit)

### Phase 2 — C1: return-value copy elision

- [x] Add a dead-source check to `lower_returned_value`
      (`builder_codegen_primitives.rs`); skip `copy_flat_block` for the movable
      `RETURN <owned-local>` case.
- [x] Verify soundness against `value_is_aliasing_source`; add a runtime aliasing
      test (`tests/return-copy-elision-runtime`).
- [x] Tests: identity-return, owned-local move, conditional move+free, and
      nested-block return — all asserting value semantics hold.

**Corrected soundness model.** The plan's premise — "the caller already performs
its own copy-on-bind" — is **false** in the current ABI: a call result
(`NirValue::Call`) is *not* an aliasing source, so the caller does **not** copy it
on bind, and arguments are passed as raw pointers (no call-site copy). A parameter
is therefore a **borrow** of the caller's block. Eliding the copy on `RETURN
<param>` (the `list copy` benchmark's `copyStrings`/`copyRecs`) would let the
caller's `LET c = f(strs)` binding own — and later free — the *caller's* `strs`
block: a double-free. So param-return **keeps its copy** (per the plan's own
non-goal: "if eliding could let a caller observe aliasing, the copy must stay").

What *is* sound and implemented: `RETURN <owned-local>` (a binding with a live
`OwnedValue` scope-drop free) **moves** its uniquely-owned block to the caller.
`plan_returned_move` removes that binding's free for the return path (restoring it
afterward so a sibling path or the block's normal exit still frees it), and
`lower_returned_value` returns the existing pointer with no `copy_flat_block`. This
is the collection sibling of the existing thread/resource/`List OF RES`
move-on-return. 59 return sites elide across the benchmark (bignum
`bnAdd`/`bnShl1`/`bnNorm`/`bnMod`, and the whole collection/csv/encoding/regex/json
stdlib whose helpers `RETURN result`).

Acceptance: aliasing test passes; A/B measured (below); full acceptance green
(1040 tests). No committed native golden changed — none of the 23 `.nir`/8
`.ncode` golden tests happens to return an owned freeable-flat local, so the
elision's codegen effect is proven by the A/B benchmark and the 59 firing sites,
not a golden diff. All deterministic benchmark checksums are byte-identical to the
pre-change binary (behavior unchanged, only timing).
Commit: 599885f8

## Layout / ABI Impact

None. Copy elision changes when a copy happens, not any layout. Callers already
copy on bind, so transferred ownership is invisible to program semantics.

## Validation Plan

- Function/runtime tests: identity-return aliasing test; bignum set in-place proof.
- Whole-benchmark: `list copy`, `bignum modmul`, `bignum modexp` medians.
- Acceptance: `scripts/test-accept.sh` — copy-elision must not change any golden
  output (it changes timing, not values).

## Theorized gains (median)

| bench          | now (ms) | driver                       | Δ     |
|----------------|---------:|------------------------------|------:|
| list copy      |   31.6   | C1 elide `RETURN xs` copy    | −85%  |
| bignum modmul  |  233.8   | C2 in-place set (+A churn)   | −25%  |
| bignum modexp  |  128.2   | C2 in-place set (+A churn)   | −25%  |

## Measured gains (median, `--run 5`, A/B same binary with/without C1)

| bench          | no C1  | with C1 | Δ       |
|----------------|-------:|--------:|--------:|
| list copy      |  32.2  |  32.2   |  0%     | ← param-return, copy is required (see above)
| bignum modmul  | 242.9  | 223.1   | −8.2%   |
| bignum modexp  | 133.0  | 122.5   | −7.9%   |
| list take      |  14.05 |  10.59  | −24.6%  |
| list flatten   |  15.06 |  12.33  | −18.1%  |
| list zip       |   8.56 |   6.92  | −19.2%  |
| list chunks    |  30.96 |  28.82  | −6.9%   |

The `list copy` benchmark cannot improve: `copyStrings`/`copyRecs` return a
parameter (a borrow), and its deep copy is the single, semantically-required copy
(the caller does not re-copy). The plan's −85% target rested on the false
"caller-copies" premise. bignum's residual gap is arena large-block churn
(plan-25-A, not yet implemented) plus its append-heavy `bnMod` inner loop, not a
return copy. C1's real payoff is the 59 owned-local-return sites — most of the
collection stdlib.

## Summary

C1 is a targeted, conservative copy-elision that removes the single biggest
non-list-op copy cost; the risk is the aliasing judgment, contained by keeping the
copy whenever the source might be live. C2 is a verification-plus-widen of an
existing fast path. Together they close bignum's gap to Python and the list-copy
gap.
