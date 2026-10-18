# bug-945: `Result` and `Ok` are reserved in user code — a user cannot declare their own `TYPE Result` or `TYPE Ok`

Last updated: 2026-10-05
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (language surface)

Status: Open
Regression Test: tests/rt-behavior/types/user-type-named-result-rt and user-type-named-ok-rt (to add)

## Cause (coordinator, bug-staging-24)

- Rule: `Result` and `Ok` mean nothing to a user (owner decision 2026-10-05): the internal fallible representation is renamed to names no user identifier can spell (`#Result` / `#Ok`, the `#` sigil already used for internal names), and a user may declare and use `TYPE Result` / `TYPE Ok` like any type.
- Where: the plain spellings are special-cased in `src/resolver/resolution.rs` (`TYPE_RESULT_NOT_USER_VISIBLE`, `is_named("Ok")`, the `["Error", "Ok", "Err"]` lists), `src/resolver/mod.rs` (reserved names), `src/types.rs` (built-in generic base, two sites).
- Fix: rename the internal type at its definition; every site that spelled it follows the rename and the user-name special cases (`TYPE_RESULT_NOT_USER_VISIBLE` and the reserved-list entries) are deleted, not kept as exceptions.

**Owner decision (2026-10-05):** `Result` and `Ok` mean nothing to a user. The runtime's
fallible-call representation is internal and must not claim either name. A user can
declare `TYPE Result` / `TYPE Ok` with their own fields, construct them, use them as
union members, and `MATCH` on them like any other type. The internal type is renamed to
a name no user identifier can spell, e.g. `#Result` / `#Ok` (the `#` sigil already used
for internal symbols). Only `Error` is user-visible on the fallible path, through `TRAP`
and `error(code, msg)` / `FAIL`. The user should know nothing about `Result` and need not
care about it.

Today both names are reserved. A user `TYPE Result` is rejected with an error that tells
the user about the internal type. A user `TYPE Ok` declares, but its constructor and
union membership are hijacked by the internal `Ok`.

References:

- Owner instruction during the spec hardening review (bug-917 item 8 follow-up).
- `src/docs/spec/language/08_error-model.md` ("`Result` and its success member `Ok` are the
  runtime's private representation … not types a user may write", `TYPE_RESULT_NOT_USER_VISIBLE`),
  and the §8.8 desugar (`FUNC f(a AS A) AS T => … AS Result OF T`).
- `src/docs/spec/diagnostics/01_rule-codes.md`: `2-203-0070 TYPE_RESULT_NOT_USER_VISIBLE`,
  `2-203-0071 TYPE_RESULT_NOT_MATCHABLE`.

## Failing Reproduction

All measured with `/tmp/hardening/mfb-head` (HEAD 28a15a153) in `/tmp/hardening/coord/rn`.

```basic
IMPORT io
TYPE Result
  code AS Integer
  note AS String
END TYPE
FUNC main() AS Nothing
  LET r = Result[1, "hi"]
  io::print(r.note)
END FUNC
```
- Observed: `main.mfb:7 error[2-203-0070 TYPE_RESULT_NOT_USER_VISIBLE]: Result is an internal type and cannot be named in user code` / "`Result` is an internal type; declare the success type directly …"
- Expected: prints `hi`.

```basic
IMPORT io
TYPE Ok
  v AS Integer
END TYPE
FUNC main() AS Nothing
  LET r = Ok[1]
  io::print(toString(r.v))
END FUNC
```
- Observed: `main.mfb:6 error[2-203-0043 TYPE_UNKNOWN_VALUE]: … Initializer for binding `r` does not have a known type.`, then `TYPE_CALL_ARGUMENT_MISMATCH` on line 7.
- Expected: prints `1`.

A union member named `Ok` (`UNION Outcome` / `Ok` / `Bad` / `END UNION`, with `TYPE Ok` and
`TYPE Bad` records):
- Observed: `main.mfb:9 error[2-203-0070 TYPE_RESULT_NOT_USER_VISIBLE]` on the union member line.
- Expected: builds; `MATCH o` / `CASE Ok(x)` matches the user's `Ok`.

Contrast (already fine): a user `FUNC Ok(n AS Integer)` builds and runs (prints `3`), and
`LET Result = 5` builds and runs (prints `5`). Only type positions are hijacked.

## Root Cause

The internal fallible representation is spelled with the plain identifiers `Result` and
`Ok`, so the resolver and type layer special-case those spellings wherever a type name
is resolved, and the user's declaration loses:

- `src/resolver/resolution.rs` — `ParameterType::Named(name) if matches!(name.resolve(), "Result" | "Ok")`
  reports `TYPE_RESULT_NOT_USER_VISIBLE`. `is_named("Ok")` and the `["Error", "Ok", "Err"]`
  member lists special-case `Ok` in pattern and constructor handling.
- `src/resolver/mod.rs` — `"Result"` in the built-in/reserved type-name list.
- `src/types.rs` — `"Result"` treated as a built-in generic base alongside `List`/`Set`/`Map`
  (two sites).
- `src/ast/expr.rs` — `name.eq_ignore_ascii_case("Result")` parsed as a generic base.
- `src/monomorph/lower.rs` — `type_.is_named("Ok")` special case.
- `src/ir/verify/matching.rs` — `TYPE_RESULT_NOT_MATCHABLE` keyed on the member names.

## Goal

- User `TYPE Result`, `TYPE Ok` (records, unions, union members, template instantiations,
  exported across packages) behave exactly like any other user type: declare, construct,
  read fields, `MATCH`/`CASE`, `typeName` returns the user's name.
- The internal fallible type and its success member use a name no user identifier can
  spell (`#Result` / `#Ok`, or equivalent), everywhere: HIR/IR/NIR, monomorph, the `.mfp`
  type table / metadata encoding (if the name is serialized), dumps, and diagnostics.
- No diagnostic, `typeName`, error message, `mfb man` or `mfb spec` *language* page mentions
  `Result`/`Ok` as something the user must know about. `TYPE_RESULT_NOT_USER_VISIBLE` and
  `TYPE_RESULT_NOT_MATCHABLE` are removed (or narrowed to the `#`-spelled internal names,
  which a user can't write anyway), and the diagnostics registry and the build-input
  error-code table are updated.

### Non-goals (must NOT change)

- The fallible-call ABI, register-level result tags and runtime layout
  (`mfb spec memory fallible-call-abi`). This is a naming change, not a semantics change.
- `Error`, `TRAP`, `FAIL`, `error(code, msg)` and `e.code`/`e.source` behavior.
- Forbidden wrong fix: keeping the reservation and only rewording the diagnostic.

## Blast Radius

- Every site above (fixed by this bug).
- `.mfp` encoding: if the internal name appears in a package's type table or IR section,
  renaming it changes the package byte format. Check `src/binary_repr` and the `package`
  spec, and bump/handle the format version per `mfb spec package` if so.
- Spec topics that describe the desugar with `Result OF T` (`language/08`, `architecture/04`
  IR, `memory/02` fallible-call ABI) move to the internal spelling or the compiler-contributor
  surface only.
