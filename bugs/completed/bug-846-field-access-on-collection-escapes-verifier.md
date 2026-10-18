# bug-846: field access on a MapEntry / List / Map escapes the verifier and fails in native lowering with an uncoded, unlocated internal error

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (diagnostics)

Status: Fixed
Regression Test: tests/syntax/control-flow/for-each-map-entry-unknown-field-invalid, tests/syntax/types/field-access-on-collection-invalid

## Cause (coordinator, bug-staging-24)

- Rule: a field access is checked against the target's type: `entry.key`/`entry.value` are a `MapEntry`'s only fields, anything else is `TYPE_UNKNOWN_FIELD` (`mfb spec language` §10, map loops); a member access on a non-record is `TYPE_FIELD_ACCESS_REQUIRES_RECORD`, as for primitives and unions.
- Where: `src/ir/verify/values.rs`, the member-access check ending in `TYPE_UNKNOWN_FIELD`: after the primitive and union arms it checks only `record_fields`; collections and `MapEntry` fall into "left unchecked" and native lowering is the first to notice.
- Fix: the check resolves fields for `MapEntry OF K TO V` (key, value) and refuses member access on `List`/`Map`/`Set` with the non-record diagnostic, located and coded; the "left unchecked" bucket for these types is deleted, so no such access reaches codegen.

Reading a nonexistent field off a `FOR EACH` map entry (`e.foo`) or any field off a
`List`/`Map` value (`l.foo`, `m.key`) is not rejected by the semantic verifier.
Depending on context the user sees a misleading downstream diagnostic
(`TYPE_UNKNOWN_VALUE`, `TYPE_CALL_ARGUMENT_MISMATCH`) — or, when the access feeds a
comparison or a builtin call, nothing until native lowering aborts with a bare
message that has no file, line, rule code or caret.

**Correct behavior:** `e.foo` on a `MapEntry OF K TO V` is `TYPE_UNKNOWN_FIELD`
(located at the access); member access on a List/Map/Set is a located, coded
front-end diagnostic (`TYPE_FIELD_ACCESS_REQUIRES_RECORD`, as for primitives and
unions). No program reaches codegen with such an access.

References:

- `src/docs/spec/language/10_control-flow.md` (also `src/docs/man/flow/forEach.md`):
  "A map loop binds the loop variable as `MapEntry OF K TO V`; `entry.key` has type
  `K` and `entry.value` has type `V` (any other field access is `TYPE_UNKNOWN_FIELD`)."
- Related but distinct (un-imported package record): bugs/completed/bug-466-unknown-field-type-escapes-to-codegen.md

## Failing Reproduction

```
IMPORT io
FUNC main() AS Nothing
  LET m = Map OF String TO Integer { "a" := 1 }
  FOR EACH e IN m
    IF e.foo = "x" THEN io::print("eq")
  NEXT
  io::print("done")
END FUNC
```
`/tmp/hardening/mfb build <proj>`

- Observed (entire output, build fails):
  `error: native code map entry 'MapEntry OF String TO Integer' has no field 'foo' while lowering if while lowering for each e`
- Expected: `<file>:5 error[2-203-0045 TYPE_UNKNOWN_FIELD]: record field does not exist`

Other observed shapes:

| Source | Observed |
| --- | --- |
| `io::print(e.foo)` in the map loop | `error: native code map entry ... has no field 'foo' while lowering eval call io.print ...` |
| `LET z = e.foo` | only `TYPE_UNKNOWN_VALUE` ("Initializer for binding `z` does not have a known type") — no TYPE_UNKNOWN_FIELD |
| `io::print(toString(e.foo))` | only `TYPE_CALL_ARGUMENT_MISMATCH` |
| `LET l = [1, 2]` / `io::print(l.foo)` | `error: native code field access target 'List OF Integer' is not a record or variant while lowering eval call io.print` |
| `IF l.foo = 3 THEN ...` | same internal error, `while lowering if` |
| `LET s = Set OF Integer { 1 }` / `IF s.size = 1 THEN ...` | `error: native code field access target 'Set OF Integer' is not a record or variant while lowering if` |
| `IF m.key = "a" THEN ...` (Map) | `error: native code field access target 'Map OF String TO Integer' is not a record or variant while lowering if` |

Contrast (correct): `LET r = R[1]` / `LET z = r.foo` on a user record emits
`TYPE_UNKNOWN_FIELD` ("record `R` has no member `foo`."); a field read on a
union or primitive emits `TYPE_FIELD_ACCESS_REQUIRES_RECORD`.

## Root Cause

`src/ir/verify/values.rs` member-access check (the function ending in
`TYPE_UNKNOWN_FIELD`, just before `hidden_from_here`): after the primitive and
union arms it only checks `self.record_fields(&target_type)`; per its own comment,
"otherwise (collections, unresolved includes, unknown types) the access is left
unchecked." `MapEntry OF K TO V`, `List`, `Map` and `Set` all fall into that
unchecked bucket; the access types as `Unknown`, which only some consumers reject,
and native lowering (`src/codegen/memory/value/builder_value_semantics.rs`,
"native code map entry '{}' has no field '{}'" / "is not a record or variant")
is the first to notice.

## Goal

- MapEntry: only `key`/`value` accepted, anything else `TYPE_UNKNOWN_FIELD`.
- List/Map/Set targets: `TYPE_FIELD_ACCESS_REQUIRES_RECORD`.
- All six table rows produce a located coded diagnostic.

### Non-goals (must NOT change)

- `entry.key` / `entry.value` typing; record field visibility checks.

## Blast Radius

- MapEntry, List, Map, Set targets — fixed by this.
- Unresolved INCLUDES / Unknown targets — remain skip-if-unknown (cascade suppression).

## Validation Plan

- tests/syntax fixtures for the table rows; full suite.

## Summary

Verifier gap: the "left unchecked" fallthrough admits collection-typed targets.

## Fix

`src/ir/verify/values.rs:check_member_access`: before the record-field lookup, a `MapEntry OF K
TO V` target resolves its fields (`key`, `value`; anything else is `TYPE_UNKNOWN_FIELD`), and a
`List`/`Set`/`Map` target is `TYPE_FIELD_ACCESS_REQUIRES_RECORD`; both located at the access. The
"left unchecked" bucket now holds only unresolved includes and unknown types. Spec:
`src/docs/spec/language/04_types.md` (the collection sentence after the union paragraph).

Tests (fixtures, goldens added): `control-flow/for-each-map-entry-unknown-field-invalid` (the
`if`, `print`, `LET z`, `toString` rows) and `types/field-access-on-collection-invalid` (the
List/Set/Map rows). RED on the pre-fix compiler: the Map-entry fixture reported only the cascades
(`TYPE_UNKNOWN_VALUE`, `TYPE_CALL_ARGUMENT_MISMATCH`) and the collection fixture built to the
bare `native code field access target ... is not a record or variant` error; GREEN: every access
has its located, coded diagnostic.

Remaining, by design and identical to a user record's unknown field (measured: `LET z = r.foo`
on `TYPE R` gives the same pair): `ir::shape` still cascades `TYPE_UNKNOWN_VALUE` /
`TYPE_CALL_ARGUMENT_MISMATCH` beside the `TYPE_UNKNOWN_FIELD` at the same line.

Goldens expected to change: none existing (the new fixtures' own `golden/build.log` only).
