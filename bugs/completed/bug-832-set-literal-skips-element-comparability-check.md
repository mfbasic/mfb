# bug-832: a `Set OF T { … }` literal skips the element-comparability check (non-comparable T accepted; ICE when non-empty with a List element)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: tests/syntax/collections/set-literal-noncomparable-element-invalid

## Cause (coordinator, bug-staging-24)

- Rule: a Set's element type must be comparable, as a Map's key type must (the verifier's `check_map_key_comparable`, applied to `IrValue::MapLiteral` and to explicit annotations).
- Where: `src/ir/verify/values.rs:check_value_depth`, the `IrValue::SetLiteral` arm, calls only `check_collection_element_ownership`; the comparability half is never called, so an inferred `LET a = Set OF … {}` reaches neither check.
- Fix: the SetLiteral arm applies the same comparability check the MapLiteral arm applies to its key type, so every Set literal is checked whatever its annotation; nothing is special-cased by element type.

The spec requires a `Set OF T` element type to be comparable. That requirement is
enforced for a declared `Set` type (`MUT a AS Set OF List OF Integer` gives
`TYPE_REQUIRES_COMPARABLE`) and for `collections::toSet([[1]])`. A **set literal**
bound with an inferred type is never checked, so a non-comparable element type gets
through:

- `LET a = Set OF List OF Integer { [1], [1] }` gets past the checker and then crashes
  codegen with an internal error:
  `error: native collection packed payload does not support type 'List OF Integer'`.
- `Set OF U { P[1], P[1] }` (U a union), `Set OF Set OF Integer { }`, and
  `Set OF FUNC(Integer) AS Integer { }` all compile and run. The union set even
  deduplicates its elements, comparing values of a type the spec calls not comparable.

The single correct behavior: every `Set OF T { … }` literal whose `T` is not comparable
is rejected with `TYPE_REQUIRES_COMPARABLE` ("Set element type requires a comparable
type"), exactly as the declared-type path reports it.

References:

- `src/docs/spec/language/04_types.md` §4.7: "its element type `T` must be comparable
  exactly as a `Map` key must be". §4.11: "a `Set` is itself not comparable, so it
  cannot be a `Set` element"; unions, functions and `List` are not comparable.

## Failing Reproduction

```basic
IMPORT io
FUNC main() AS Nothing
  LET a = Set OF List OF Integer { [1], [1] }
  io::print(toString(len(a)))
END FUNC
```
`mfb build` (repro dir /tmp/hardening/B01/repro/r_setlit1)

- Observed: `error: native collection packed payload does not support type 'List OF Integer' while lowering bind a AS Set OF List OF Integer` (an internal codegen error, no diagnostic code).
- Expected: `error[2-203-0061 TYPE_REQUIRES_COMPARABLE]` … `Set element type requires a comparable type, got `List OF Integer`.`

Second repro (/tmp/hardening/B01/repro/r_setlit2): `Set OF U { P[1], P[1] }`,
`Set OF Set OF Integer { }`, and `Set OF FUNC(Integer) AS Integer { }` build, and the
program prints `100`. Expected: three `TYPE_REQUIRES_COMPARABLE` diagnostics.

Contrast (already correct): `MUT a AS Set OF List OF Integer` → TYPE_REQUIRES_COMPARABLE
(r_setlit3). `LET m = Map OF List OF Integer TO Integer { }` → TYPE_REQUIRES_COMPARABLE.

## Root Cause

`src/ir/verify/values.rs:check_value_depth`, in the `IrValue::SetLiteral` arm: it calls
only `check_collection_element_ownership(element, "element")`, which is the
resource/thread half. The comparability half is never called. The sibling
`IrValue::MapLiteral` arm calls `check_map_key_comparable(type_)`, and so does the
explicit-annotation branch in `src/ir/verify/ops.rs`. An inferred `LET a = Set OF … {}`
therefore reaches neither check.

## Goal

- Every set literal with a non-comparable element type yields exactly one
  `TYPE_REQUIRES_COMPARABLE`. There is no double report when the binding is also
  annotated, because the ops.rs explicit path already reports.

### Non-goals

- Comparable element sets (records of comparable fields, enums, scalars) must keep
  compiling unchanged.

## Blast Radius

- `values.rs` SetLiteral arm: fixed by this bug.
- `ops.rs` explicit binding path: already correct. The inferred case relies on the
  literal arm, the same way MapLiteral does.

## Fix Design

Call `self.check_map_key_comparable(type_)` in the SetLiteral arm, mirroring MapLiteral.
The function recurses through `SetOf` and calls `check_collection_element_comparable`.
Then remove the now-redundant explicit ownership call, or keep it if
`check_collection_element_comparable` already covers ownership (it does).

## Validation Plan

- A syntax fixture with the four shapes above. Then run the full suite.

## Summary

This is a one-line verifier gap. The risk is double reporting on annotated bindings.
Check the golden for an annotated set literal.

## Fix

`src/ir/verify/values.rs:check_value_depth`, `IrValue::SetLiteral` arm, now calls
`check_map_key_comparable(type_)` exactly as the `MapLiteral` arm does (it recurses through
`SetOf`, and `check_collection_element_comparable` covers the ownership half, so the separate
ownership call is removed). Every set literal is therefore checked whatever its annotation; no
element type is special-cased.

Not met (flag for the coordinator): the Goal's "no double report when the binding is also
annotated". An annotated binding whose initializer is a literal of the same type
(`LET e AS Set OF List OF Integer = Set OF List OF Integer { }`) is reported by BOTH the literal
arm and the explicit-annotation check in `ops.rs`; the same already happens for an annotated
`Map` literal at HEAD (measured: `LET e AS Map OF List OF Integer TO Integer = Map OF ... { }`
gives 2 reports). I tried skipping the `ops.rs` check for a same-type literal, but the pinned
test `ir::verify::tests::rejects_a_resource_in_a_set_literal` ("Once for the declared type, once
for the literal -- as the front end did", plan-107-B) expects exactly that double report for the
ownership half, and a test is not edited without proof it is wrong; so the `ops.rs` change was
reverted and the fixture does not pin the annotated shape either way.

Test: fixture `tests/syntax/collections/set-literal-noncomparable-element-invalid` (four inferred
shapes from the bug and comparable `Set OF P` / `Set OF
Integer` literals that still build). RED on the pre-fix compiler: no report for any of the four shapes
(0 of 4; the build only failed on a separate annotated line, since removed). GREEN: 4 reports, one per line. Ran `bash scripts/test-accept.sh <mfb>
<dir> 'set-literal-noncomparable*'` and `bash scripts/test-fast.sh --accept ...`.

Goldens expected to change: none beyond the new fixture's own `golden/build.log`.
