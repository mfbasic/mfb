# bug-828: built-ins raise ErrOutOfMemory without declaring it

Last updated: 2026-10-05
Effort: small (measured: descriptor edits plus one guard narrowing)
Severity: MEDIUM
Class: Correctness (declared errors / fallibility census)

Status: Fixed (plan-165-B commit; see `git log -S 'bug-828'`)
Regression Test: `declared_errors_all::every_{json,math,astrings}_member_declares_exactly_the_errors_its_lowering_raises`
(new sweeps / newly lowered members) and
`infallible_proof_coverage::every_inline_builtin_census_grant_is_raise_checked`.

Found by plan-165-B Phase 2 (2026-10-05), on main `f23af579c`, when the new
raise-checking sweeps lowered members no guard had lowered before.

## Failing Reproduction

`cargo test --bin mfb -- every_json_member every_math_member every_astrings_member`
with the sweeps added and the `astrings` skip list removed:

```
json::sciParts: undeclared ["ErrOutOfMemory"]
math::abs: undeclared ["ErrOutOfMemory"]     (and 18 more math members)
astrings::fromString: undeclared ["ErrOutOfMemory"]
astrings::writeSpans: undeclared ["ErrOutOfMemory"]
```

Each lowering loads `ErrOutOfMemory` on a failed arena allocation:
`emit_sci_parts_call`'s `raise_error_bare("ErrOutOfMemory")`; every `math` `List OF`
form's result list (`_mfb_simd_alloc_list` returns 0 and the caller raises); the
`AttributedString` block `fromString`/`writeSpans` build.

## Root Cause

Hand-written `errors` lists that no raise-checking guard read: `json`, `math` had no
declared-errors sweep, and the two `astrings` members were skipped by theirs (no
record layout in the sweep's bare `TypeModel`). `json::sciParts`, `astrings::fromString`
and `astrings::writeSpans` declared nothing, so the fallibility census granted them:
an inline `TRAP` on `fromString` was flagged dead and wrapped always-`Ok`, deleting
a handler that an allocation failure should have reached.

## Resolution

- `json::sciParts`, `astrings::fromString`, `astrings::writeSpans` declare
  `ErrOutOfMemory`.
- Every `math` `List OF` overload declares `ErrOutOfMemory`
  (`math::list_form_errors`); the scalar forms are unchanged.
- `math::log`/`log10`'s `Fixed` form loads `ErrOverflow` from the shared
  `emit_fixed_divide`, unreachable because the only division is `(m-1)/(m+1)` with
  `m` in `[1, 2)`; recorded as `MATH_UNREACHABLE_OWN_RAISE` rather than declared.
- `mfb_body_declared_errors`'s overload narrowing now treats a provably scalar
  argument (an arithmetic expression or a numeric-typed name) as selecting the
  scalar overloads, so `color`/`vector` bodies calling `math::abs(2.0 * l - 1.0)` are
  not charged the list forms' `ErrOutOfMemory`.
