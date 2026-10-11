# bug-1023: the field-kind grid expects `TYPE_STATE_INVALID` for records that hold an enum

Last updated: 2026-10-10
Effort: small (<1h)
Severity: MEDIUM
Class: Other

Status: Fixed (2026-10-10)
Regression Test: `cargo test --test rt_inplace_self_update every_field_kind_meets_its_expectation -- --ignored` (exists; red)

`tests/runtime/inplace_self_update/field_kinds.tsv` gives 19 record kinds the
expectation `na:TYPE_STATE_INVALID` at the eight `STATE` sites (`T1`–`T8`), on the rule
its generator states: "a type with no default (it needed a literal) cannot be a `STATE`
payload field". plan-172-N made an enum defaultable (its first variant), so a record
whose only undefaultable fields were enums now has a default, and the grid's programs
for it compile. The grid reports every such site as failed: "expected
`na:TYPE_STATE_INVALID`, but the program compiles — the line needs a real expectation".

The grid is `#[ignore]`d and run by `coverage.yml`, so a plain `cargo test` does not
show this.

**The single correct behavior a fix produces:** each of the 19 rows carries, at `T1`–`T8`,
the expectation its class gives a defaultable kind (`field_kinds_gen.py:expects` with
`state_ok=True`) wherever the kind is now defaultable, the generator computes
`state_ok` from defaultability instead of from "needed a literal", and
`every_field_kind_meets_its_expectation` passes for those rows.

References:

- `tests/runtime/inplace_self_update/field_kinds_gen.py:expects`, and the `state_ok`
  line in its emit loop (`kind not in LITERALS or kind in ("KFix", "KVar")`).
- `planning/completed/plan-172-N-*.md` — the rule change.
- `bugs/bug-1014-field-kinds-tsv-disagrees-with-its-generator.md` — the table's
  `S`-site drift, which is separate and older.
- Found during plan-172-F, which aligned the check column of five canvas rows with the
  generator and ran the grid for them.

## Failing Reproduction

```
MFB_SELF_UPDATE_FILTER='canvas::Rectangle|canvas::Line|canvas::Polygon|canvas::PolyLine|canvas::Picture' \
  cargo test --test rt_inplace_self_update every_field_kind_meets_its_expectation -- --ignored
```

- Observed (2026-10-10, worktree-P-172 during plan-172-F): `32 of 75 case/site pair(s)
  failed`. All 32 are `T` sites of `canvas::Rectangle`, `canvas::Line`,
  `canvas::Polygon` and `canvas::PolyLine` (eight each): `T1`–`T6` and `T8` "expected
  `na:TYPE_STATE_INVALID`, but the program compiles", and `T7` "expected
  `na:TYPE_STATE_INVALID`, got `TYPE_FOR_EACH_REQUIRES_COLLECTION`". The 43 passing
  pairs are those four kinds' seven `S` sites and all 15 of `canvas::Picture`.
- Expected: `0` failed.
- Scope: `grep -c 'T1=na:TYPE_STATE_INVALID' tests/runtime/inplace_self_update/field_kinds.tsv`
  → 19 rows. Only four were run; the other 15 are expected to fail the same way
  wherever the kind's undefaultable fields were all enums (a guess until the grid is
  run for them — `json::Json`, for one, is `STATE`-invalid for a different reason and
  should still pass).

## Root Cause

The cause is reasoned, not bisected: the generator's rule ties `STATE` eligibility to
"has no default", and plan-172-N changed which records have one. It was not re-run at
the commit before plan-172-N.

## Fix Plan

- [x] Run the grid for all 19 rows and list which `T` sites now compile. Observed:
      `136 of 208 case/site pair(s) failed` — 17 kinds, eight `T` sites each. The two
      that still do not compile are `json::Json` and `http::Route` (a function-valued
      field), as they should not.
- [x] `field_kinds_gen.py`: `state_ok = defaultable(kind)`, a function that restates
      `src/ir/verify/resources.rs:is_defaultable` over the `mfb man` field lists.
- [x] Those 17 rows' `T1`–`T8` set to what `expects` gives; only those sites were
      written, by a script that left every `S` site as committed (bug-1014). The
      generator's new rule names exactly the 17 kinds the grid measured.
- [x] The grid passes for them:
      `MFB_SELF_UPDATE_SITES=T1,…,T8 MFB_SELF_UPDATE_FILTER=<the 19 kinds> cargo test
      --test rt_inplace_self_update every_field_kind_meets_its_expectation -- --ignored`
      → `1 passed` (208 pairs, 692 s). No `T` site failed its allocation bound.

The whole grid, unfiltered, is then `4 of 1155 case/site pair(s) failed`: `http::Route`
at `S4`, `S5`, `S9` and `S10`, which is bug-1014's drift and fails the same way at
`main` (`932ea9956`).

## Notes

This is not bug-1013. That one is a scalar or `String` `STATE` payload miscompiling;
this is a test table whose expectation of a compile error is out of date for record
payloads, which are the supported kind.
