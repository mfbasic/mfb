# bug-76: `math::clamp` on a `List OF Float` broadcasts garbage when a bound is not a literal

Last updated: 2026-07-10
Effort: small (<1h)
Severity: MEDIUM (silent wrong answer / spurious trap)

`lower_math_clamp_array` (`src/target/shared/code/builder_math.rs`) spills the
scalar `low`/`high` bound with `store_u64` **without** calling `materialize_float`
first. Since plan-01-float-dnative, a `Float` is carried in a `d` register, so
`store_u64` writes whatever integer register the lowering happens to name, not the
bound. The garbage is then broadcast across the vector lanes.

A literal bound works (it is materialized as a constant). A computed or
variable bound does not.

The single correct behavior a fix produces: `math::clamp(xs, low, high)` gives the
same result whether the bounds are literals, locals, or expressions.

## Discovery

Found while fixing bug-68 (commit 420e4123), which owned the SIMD tail encoders and
scoped this out.

## Failing Reproduction

```basic
IMPORT io
IMPORT math
IMPORT collections

FUNC main AS Integer
  LET xs AS List OF Float = [0.0, 2.0, 5.0]

  ' literal bounds: correct
  LET a AS List OF Float = math::clamp(xs, -1.0, 3.0)

  ' computed bound: clamps against 0.0, not -1.0
  LET b AS List OF Float = math::clamp(xs, 0.0 - 1.0, 3.0)

  ' variable bound: raises a spurious ErrInvalidArgument
  LET lo AS Float = -1.0
  LET c AS List OF Float = math::clamp(xs, lo, 3.0)

  io::print(toString(collections::get(b, 0)))
  RETURN 0
END FUNC
```

## Root Cause

`store_u64` of a value that lives in a `d` register. `materialize_float` is the
seam that moves a Float carrier into a GPR (or spills it correctly); it is missing
on this path.

## Goal

- Array `clamp` reads its bounds correctly for every carrier.

## Blast Radius

- `lower_math_clamp_array`. Audit every other array/SIMD builder that spills a
  scalar Float bound or operand: `min`/`max` array overloads take a list, not a
  scalar bound, but any future scalar-broadcast lowering shares the hazard.

## Fix Design

Call `materialize_float` on the bound before the spill, or spill through the
Float-aware store the rest of the float codegen uses. Then broadcast.

## Phases

### Phase 1 — failing test

- [ ] The reproduction above: assert literal, computed, and variable bounds agree.

### Phase 2 — the fix

- [ ] `materialize_float` before the spill; sweep sibling array builders.

### Phase 3 — validation

- [ ] Runtime test with all three bound shapes; `scripts/test-accept.sh`.

## Summary

Array `clamp` spills a `d`-register Float bound as if it were an integer, so any
non-literal bound broadcasts garbage — wrong results, or a spurious
`ErrInvalidArgument`.
