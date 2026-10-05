# bug-849: `WITH` can overwrite a `PRIVATE` record field from another file or an importing project

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/syntax/types/<new>-with-private-field-invalid (proposed)

A `PRIVATE` field is file-local. Reading it or setting it in a constructor from another
file is rejected with `TYPE_MEMBER_NOT_VISIBLE`. A `WITH` update of the same field from
another file, or from a project importing the package, compiles and runs, and the hidden
field takes the new value. That makes `PRIVATE` fields forgeable: take any value the
owner hands out and `WITH` it. **Correct behavior:** `WITH value { privateField := ... }`
outside the declaring file emits `TYPE_MEMBER_NOT_VISIBLE`, just as read and construct
already do.

Class: CODE BUG.

References:

- src/docs/spec/language/04_types.md: "A `PRIVATE` field can be used only in the declaring source file. Outside code that cannot see a field also cannot set it in a constructor, read it with `.`, or update it with `WITH`"
- src/docs/spec/language/13_modules-and-packages.md: "Fields in `TYPE` declarations may also use `PRIVATE`, `PUBLIC`, or `EXPORT`"
- Related: B03-public-field-visible-to-importers (same verifier tables)

## Failing Reproduction

In-project (/tmp/hardening/B03/modules/vis5):

```
# src/b.mfb
TYPE Account
  name AS String
  PRIVATE balance AS Integer
END TYPE
FUNC mk() AS Account
  RETURN Account["a", 10]
END FUNC
FUNC bal(a AS Account) AS Integer
  RETURN a.balance
END FUNC

# src/main.mfb
IMPORT io
FUNC main() AS Nothing
  LET a = mk()
  LET b = WITH a { balance := 999 }
  io::print(toString(bal(b)))
END FUNC

/tmp/hardening/mfb build -q && ./build/vis5.out
```

- Observed: `Wrote executable to ./build/vis5.out`, and it prints `999`.
- Expected: `src/main.mfb:4 error[... TYPE_MEMBER_NOT_VISIBLE]` (field `Account::balance` not visible from this file).

Cross-package (/tmp/hardening/B03/modules/neg_privwith): an `EXPORT TYPE Priv` with `PRIVATE closed`,
updated by the importer with `WITH s { closed := 77 }`, also builds and runs.

Contrast cases (correct today):
- Reading `a.balance` in main.mfb gives `TYPE_MEMBER_NOT_VISIBLE` (tests/syntax/types/types-member-visibility-invalid).
- Constructing `Account["x", 1]` in main.mfb gives `TYPE_MEMBER_NOT_VISIBLE`.

## Root Cause

The `IrValue::WithUpdate` arm in `src/ir/verify/values.rs` (`check_value_depth`) checks
read-only records, duplicate fields and field types, but never calls
`hidden_from_here(type_, &update.field)`. That is the predicate the `.field` read path
(`check_member_access` region, same file) and the constructor path
(`src/ir/verify/compat.rs`, "cannot set hidden field") both use.

## Goal

- Each `WITH` update naming a field for which `hidden_from_here` is true emits
  `TYPE_MEMBER_NOT_VISIBLE`, e.g. "`WITH` cannot set hidden field `balance` of `Account` from this file."

### Non-goals (must NOT change)

- `WITH` on visible fields, and on private fields inside the declaring file, stays legal.

## Blast Radius

- `src/ir/verify/values.rs` `WithUpdate` arm — fixed by this bug
- `.field` read and constructor paths — already correct for PRIVATE (PUBLIC gap: separate finding)

## Fix Design

In the loop over `updates`, call `self.hidden_from_here(&resource_base_type(type_), &update.field)`
and emit. Once B03-public-field-visible-to-importers lands, the same predicate covers
package-visible fields too.

## Validation Plan

- Regression test(s): syntax fixture as above (expect `TYPE_MEMBER_NOT_VISIBLE`), plus a
  cross-package variant.
- Full suite: the project's acceptance suite.

## Summary

This is a one-predicate omission. The risk is low.
