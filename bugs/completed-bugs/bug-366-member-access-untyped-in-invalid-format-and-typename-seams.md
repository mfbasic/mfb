# bug-366: a record field cannot be typed by two more codegen seams — Float-into-exact conversions and `typeName` abort the build

Last updated: 2026-07-19
Effort: small
Severity: MEDIUM (valid programs fail to build; the message is an internal error
with no rule code, no file, and no line)
Class: Compiler / native codegen

Status: FIXED (2026-07-19)
Regression Test: `tests/rt-behavior/codegen/bug366_record_field_exact_conversion`
(seam half) and `tests/rt-behavior/codegen/bug366_money_float_invalid_format`
(modelling half); both verified to abort on the pre-fix compiler. Broader
coverage in `tests/rt-behavior/codegen/record-field-arithmetic-rt`.

## Summary

Direct sibling of bug-363, found by widening its record-field arithmetic matrix
to Integer / Float / Fixed / Money and `math::*`. bug-363 fixed
`static_nir_value_type`; **two more walks over the same NIR had the identical
`MemberAccess` gap**, plus one predicate that was simply modelled wrong.

Three distinct build aborts, all with no rule code, path, or line:

```
$ mfb build .
error: native code string literal 'Text parse or non-finite numeric representation conversion failed.' has no data object while lowering return
error: native code cannot determine typeName argument type while lowering eval call io.print
```

## Reproductions

**A — seam gap, `Float` into `Fixed` through a record field.**

```
TYPE Rate
  factor AS Fixed
END TYPE

FUNC scale(r AS Rate, x AS Float) AS Fixed
  RETURN x * r.factor
END FUNC
```

Replacing `r.factor` with a plain `Fixed` parameter compiles. It is the
`MemberAccess` operand that does it — exactly bug-363's shape, different seam.

**B — modelling gap, `Money` with a `Float` operand. No record field needed.**

```
FUNC total(amount AS Money, rate AS Float) AS Money
  RETURN amount * rate
END FUNC
```

`M * F`, `F * M`, and `M / F` all abort, with plain locals for both operands.

**C — `typeName` of a record field.**

```
io::print(typeName(c.radius))   ' cannot determine typeName argument type
```

## Root cause

Two independent defects that happen to surface through the same message object.

**A/C — `MemberAccess` is untypeable in two more seams.** `ERR_INVALID_FORMAT` is
emitted only when `value_may_return_invalid_format` says so
(`data_objects.rs:987`). That routes through `binary_may_promote_float_to_fixed`
-> `static_type_name_with_types`, which had no record-field arm: it fell straight
to `parse_map_entry_type` and answered `None` for every record field. The
predicate then answered false, the message object was omitted, and the lowering —
which does know the field's type — emitted the check that loads it.

`CodeBuilder::static_type_name` (`builder_value_semantics.rs`) had the same hole,
which is what broke `typeName(rec.field)`. Note that bug-363's fix comment
claimed this builder consulted record fields; **it did not** — that claim was
wrong when written and is corrected here.

So the family is three parallel walks over the same NIR:

| Walk | Fixed in |
| --- | --- |
| `static_nir_value_type` | bug-363 |
| `static_type_name_with_types` | bug-366 (this) |
| `CodeBuilder::static_type_name` | bug-366 (this) |

**B — the predicate only modelled `Fixed`.** `binary_may_promote_float_to_fixed`
required `numeric_binary_result_type(...) == TYPE_FIXED`. But the spec gives a
non-finite `Float` operand consumed by `Money` the same `ErrInvalidFormat`
(77050003) failure (`mfb spec language types` §4.1, "Money"): rounding applies to
`M / k`, `M * Float`, `M * Fixed`. Every `Money`-with-`Float` expression was
therefore unguarded regardless of operand shape.

## Fix

- Thread the module's field table (`type_utils::FieldTypes`, built by
  `module_analysis::module_field_types` — bug-363's helper) through
  `static_type_name_with_types` and every walk that carries the local-`types` map,
  and resolve `MemberAccess` from it before falling through to the `MapEntry`
  members.
- Give `CodeBuilder::static_type_name` a record / union-variant field arm, read
  from the `type_model` tables the field-access lowering already uses.
- Rename `binary_may_promote_float_to_fixed` -> `binary_may_consume_float_into_exact`
  and accept a `Money` result as well as a `Fixed` one. The rename is the point:
  the old name is why the `Money` case was never noticed.

## A golden had frozen this bug as expected output

`tests/rt-behavior/money/money_inexact_float_warn` — added by plan-29-F §4.6 to
assert the `MONEY_INEXACT_FLOAT_LITERAL` warning — had this recorded as its
committed expected output:

```
error: native code string literal 'Text parse or non-finite numeric representation conversion failed.' has no data object while lowering bind w1 AS Money
[exit 1]
```

That is reproduction B, enshrined. The bug-309 pattern exactly: a live failure
recorded as expected output, with the suite then defending it.

It was proven wrong on four independent grounds before the golden was touched:

1. The test's own source comments say *"The warning never changes the type — the
   program still builds and runs, producing the inexact Float-scaled result"*,
   and annotate both prints as `10.79`.
2. `mfb spec diagnostics rule-codes` lists `2-203-0109` as severity **warn**, and
   states every other rule is `error`. A warn-only program must build and run.
3. `src/rules/table.rs` and `src/syntaxcheck/inference.rs` agree it is warn-only.
4. The recorded text is this bug's own abort, reproduced and root-caused
   independently of that test.

The regenerated golden changes 6 lines at the tail; the 41 lines of warning
assertions the test exists for are **byte-identical**, and the program now prints
the two `10.79` values its source documents.

## Blast radius

Codegen only; no accepted-language change. `scripts/artifact-gate.sh` was
byte-identical across 1195 goldens and full acceptance showed zero churn — no
committed fixture used `typeName` on a record field or mixed `Money` with
`Float`. Turning the predicate on for more modules only ever *adds* a data
object.

## Found by

Extending bug-363's regression coverage to the full numeric matrix at the user's
request (2026-07-19). The `Money` half (B) is the one worth noting: it fails with
plain locals and would have been missed entirely by testing record fields alone —
vary the *type* as well as the operand shape.
