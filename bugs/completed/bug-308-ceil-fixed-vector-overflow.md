# bug-308: `math::ceil(Fixed[])` vector body returns a wrong negative value for a Fixed in (2147483647, 2147483648)

Last updated: 2026-07-17
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (platform / vector-vs-scalar divergence)

Status: Fixed
Regression Test: tests/rt-behavior/math/ceil-fixed-vector-overflow-rt

The vectorized `CeilFixed` kernel computes `ceil(x) = floor(x + (2^32−1))` as
`vector_add(v0, v0, broadcast(0xFFFFFFFF))` then arithmetic `sshr 32`. The raw add is
modular i64 and overflows whenever `raw > i64::MAX − (2^32−1)` — i.e. any Fixed whose
real value is in the open interval (2147483647, 2147483648). The add wraps to a large
negative i64 and the `sshr 32` yields a large negative integer. The result value
(2147483648) is representable in `Integer`, so the kernel is not supposed to error —
it just returns garbage. The scalar path (`whole = src>>32; if frac != 0 { whole +=
1 }`) never overflows, so the same program diverges between a length‑1 list (odd tail
→ scalar → correct) and a length‑2 list (vector body → wrong), and diverges from the
scalar `math::ceil(Fixed)` overload.

The single correct behavior a fix produces: the vectorized `math::ceil(Fixed[])`
returns the same value as the scalar overload for every lane, including Fixeds just
below 2147483648.

References:

- `bugs/completed-bugs/bug-175-*` (H documents `FIXED_ONE_MINUS_1_STR` as CeilFixed's
  bias but not this near-max overflow).
- Found during goal-06 review of `src/target/shared/code/builder_simd_math.rs`.

## Failing Reproduction

```
LET xs AS List OF Fixed = [2147483647.5F, 2147483647.5F]
LET c AS List OF Integer = math::ceil(xs)   ' vector body
LET ys AS List OF Fixed = [2147483647.5F]
LET d AS List OF Integer = math::ceil(ys)   ' scalar tail
```

- Observed (macOS-aarch64): vector body prints `-2147483648`, `-2147483648`; scalar
  tail prints `2147483648` (correct).
- Expected: all print `2147483648`.

RoundFixed/FloorFixed are unaffected (they only ever add ≤1 to `whole`, never bias
the raw).

## Root Cause

`src/target/shared/code/builder_simd_math.rs:378-390` (`emit_simd_unary_vector`,
`CeilFixed` arm): biasing the raw i64 by `0xFFFFFFFF` before the shift overflows near
i64::MAX for Fixeds just below 2^31 real value.

## Goal

- Compute vector ceil without biasing the raw: `whole = sshr(x,32)`;
  `frac = x & 0xFFFFFFFF`; `bump = (frac != 0) ? 1 : 0` (via `vector_cmeq`-against-zero
  + `and 1`); `result = whole + bump` — overflow-free and bit-identical to the scalar
  tail for every lane.

### Non-goals (must NOT change)

- RoundFixed/FloorFixed kernels (correct).
- The scalar `math::ceil(Fixed)` overload.

## Blast Radius

- `emit_simd_unary_vector` `CeilFixed` arm — fixed here.
- Confirm the aarch64/x86/riscv vector paths share this kernel (fix once) and that
  the scalar tail is already correct.

## Fix Design

Replace the bias-and-shift with the frac-test-and-bump used by the scalar path,
vectorized via `vector_cmeq`. Rejected alternative: widening the add to avoid
overflow — the frac-test approach matches the scalar path exactly and avoids the
bias entirely.

## Phases

### Phase 1 — failing test
- [ ] rt-behavior test for a 2-element and 1-element Fixed list near i32::MAX;
      confirm the vector/scalar divergence today.
### Phase 2 — the fix
- [ ] Rewrite the `CeilFixed` vector kernel.
### Phase 3 — validation
- [ ] Artifact gate + rt-behavior green on all three backends; vector == scalar.

## Validation Plan

- Regression: the near-max Fixed ceil test across list lengths.
- Runtime proof: vector body matches scalar overload.
- Doc sync: none.

## Summary

The vector ceil biases the raw value and overflows near i64::MAX, diverging from the
scalar path; switching to the scalar's frac-test-and-bump fixes it overflow-free.
Small, well-scoped, cross-backend.

## Resolution

Reproduced exactly as reported: for `toFixed("2147483647.5")` the scalar overload and
a length-1 list both gave `2147483648`, while a length-2 list gave `-2147483648`.

The kernel no longer biases before shifting. It now computes
`floor(x) + (frac != 0)` — the same formula the scalar tail already used — by
deriving the whole part with the arithmetic shift **first** and only then adding one:

    whole = sshr(x, 32)
    frac  = x AND 0xFFFFFFFF
    out   = whole + (frac > 0 ? 1 : 0)

That ordering is what makes it safe. The old `floor(x + (2^32-1))` added the bias to
the *raw* Q32.32 value, and that add is modular i64, so it wrapped for any raw above
`i64::MAX - (2^32-1)`. Shifting first narrows to the integer part, which always fits,
and the `+1` is applied there. Since `frac` is masked it is never negative, so a
`cmgt` against zero is sufficient.

The shape mirrors `RoundFixed`, which already solved the same problem the same way —
the two Fixed kernels now read alike rather than one carrying a bias trick the other
avoided.

### Verification

The fixture checks eleven values, and each one compares **three** paths against each
other: the scalar overload, an even-length list (vector body), and a length-1 list
(odd tail). Comparing all three is the point — the bug's signature was precisely that
they disagreed, so a test asserting only the expected number would not have located
it, and one testing only even lengths would not have shown the divergence.

Coverage spans the reported interval, both signs, and both extremes
(`-2147483648.0` through `2147483647.5`). All agree.

Full `cargo test` green; artifact gate 0 diffs; acceptance 1011/1011.
