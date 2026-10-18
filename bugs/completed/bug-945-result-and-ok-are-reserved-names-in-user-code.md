# bug-945: `Result` and `Ok` are reserved in user code — a user cannot declare their own `TYPE Result` or `TYPE Ok`

Last updated: 2026-10-18
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness (language surface)

Status: Fixed (bs24-agent-h, bug-staging-24)
Regression Test: tests/runtime/rt_user_type_named_result_ok.rs (`cargo test --release --test rt_user_type_named_result_ok`)

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

## Fix

The internal type is renamed at its definition: `ParameterType::ResultOf` renders and
parses as `#Result OF T` (`types::RESULT_TYPE`), and the internal success member is
`#Ok` (`types::RESULT_SUCCESS_MEMBER`). `#` is the internal sigil the lexer rejects in
user identifiers, so no user spelling reaches either. Every site that spelled them
follows: `types.rs` (`parse`, `name`, `split_user_generic`, `type_prefix_len`),
`binary_repr` (kind-6 decode/encode spelling; the type table is kind-keyed), and the
defensive `#Result`-scrutinee arms in `ir/lower.rs`. The IR section writes every type
as an inline canonical string, so a v6 payload's `Result OF T` would now decode as a
user template: `BINARY_REPR_VERSION` goes 6 -> 7 (exact gate, as at 5 -> 6), and the
committed packages are rebuilt (`scripts/sync-package-mfp.sh`, the five crafted
`tools/security-package-sources/pkg-0{2,2b,2c,3,7}` vectors via their `generate.py`,
and the vendored `libsnd`/`sqlite3` copies rebuilt from `packages/` with each
fixture's original manifest, i.e. without the later Windows library entries).
`repository/tests/fixtures/libsnd.mfp` is left at v6: the repository crate's pin reads
only MFPC sections, never the IR payload. The user-name special cases are deleted: the resolver's
`TYPE_RESULT_NOT_USER_VISIBLE` arm and `report_result_not_user_visible`, `"Result"` in
`BUILTIN_TYPES`, the `Ok` skips in pattern/constructor resolution, the parser's
`Result OF` base, the `Ok` constructor typing in `ir::lower`/`monomorph::lower`, the
`Ok`/`Result` checks in `ir::shape` and `ir::verify::compat` (`TYPE_RESULT_IS_IMPLICIT`),
the `Result`/`Ok`/`Error`/`Err` arm rule in `ir::verify::matching`
(`TYPE_RESULT_NOT_MATCHABLE`), and `Ok` in the NIR local whitelist. The three rules
(2-203-0040/0070/0071) are retired as reserved rows.

`CASE Error(e)` over a non-union scrutinee used to be caught only by
`TYPE_RESULT_NOT_MATCHABLE`; with that rule gone it built silently. The verifier now
counts the built-in records (`Error`, `ErrorLoc`) as declared types when classifying an
arm, so it is the ordinary `TYPE_MATCH_PATTERN_MISMATCH`.

RED (before): all four tests in `rt_user_type_named_result_ok` failed with
`TYPE_RESULT_NOT_USER_VISIBLE`. GREEN (after): 4/4 pass.

Spec: `language error-model` §8.5b/§8.8, `language pattern-matching`, `language grammar`,
`language collections`/`modules-and-packages`/`native-libraries`, `diagnostics rule-codes`,
`architecture` ir/native-ir/monomorphization/type-name-encoding, `memory`
heap-values/native-calling-convention/fallible-call-abi, `package` type-table/
ir-section/functions, `threading` thread-runtime-helpers. Man: `types`, `errors`,
`flow match`.

### Goldens expected to move (coordinator regenerates)

- Every `*.ir` golden that dumps a trap temporary or result type (197 files,
  `grep -rl 'Result OF' tests --include='*.ir'`): `"type": "Result OF T"` becomes
  `"#Result OF T"`; nothing else in the dump moves (checked on
  `func_typesystem_result_pattern_valid`).
- `tests/syntax/app/app-mouse-surface` `.nir`/`.nplan` (same spelling).
- `tests/syntax/functions/func_typesystem_ok_invalid`, `func_typesystem_result_invalid`,
  `func_typesystem_result_pattern_invalid` `build.log`: the retired rules give way to
  `SYMBOL_UNKNOWN_TYPE` for the undeclared `Ok`/`Result OF Integer`.
- Every package fixture's `.info` golden (`package binary hash` / `content hash`;
  measured on `package-simple` and `package-private-overload-export`), with its
  `golden/*.mfp`.
- Every package `golden/*.mfp` (rewritten by `sync-package-mfp.sh`, BINARY_REPR 7), and
  any `.ir`/native golden of a consumer whose package-identity id (a content hash
  over the package payload) changed with it.
- Deleted `tests/syntax/types/result-not-matchable-invalid` and
  `result-not-user-visible-invalid`: each pinned only a rule the owner decision
  retires.
