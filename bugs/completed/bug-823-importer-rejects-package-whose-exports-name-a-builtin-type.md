# bug-823: importing a package whose exports name a builtin type fails unless the importer also imports that builtin

Last updated: 2026-10-04
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Fixed
Regression Test: `src/ir/shape.rs:a_dependency_s_builtin_types_resolve_without_the_importer_s_import`; tests/rt-behavior/packages/package-builtin-type-export-rt; tests/syntax/packages/package-builtin-record-field-needs-import-invalid (bug-466 guard); `src/ir/tests.rs:a_package_s_builtin_bindings_initialize_first` and tests/rt-behavior/packages/package-builtin-binding-init-rt (the sibling init-order defect below)

A consumer that does `IMPORT pkg` fails to build with `PACKAGE_INVALID`. This happens
when `pkg`'s exported functions, records or unions mention a builtin package's type
(`json::Json`, `datetime::DateTime`, `crypto::KeyPair`, ...) and the consumer file does
not import that builtin package itself. The consumer does not have to use the
affected export. Calling a different function, `pkg::g()`, is enough to trigger it.
The diagnostic blames the package container:

```
error[6-605-0001 PACKAGE_INVALID]: package container is malformed or incompatible
  Imported package `.../pkg.mfp` has exported function `f` parameter `x` that references unknown type `json.Json`.
```

**The single correct behavior:** a package's exported signatures and type fields may
mention any builtin package's type. An importer resolves that type whether or not
it imports the builtin. Only *naming* the type (`json::Json` in the consumer's source)
or reading a builtin record's fields (bug-466) needs the consumer's own `IMPORT`.
Passing a `json::Json` value from one `pkg` call into another needs no `IMPORT json`.
Before bug-750's fix, that worked and ran correctly (measured below).

References:

- Found while adding `packages/currency`. Its doc-example check showed that
  `packages/mustache/check-doc-examples.sh` and `packages/json_schema/check-doc-examples.sh`
  fail on exactly the DOC examples that do not `IMPORT json`:
  - mustache `ex3`: the `escapeHtml` example.
  - json_schema `ex7`, `ex8` and `ex9`: the `dialect`, `patternMatches` and
    `translatePattern` examples.

  This was measured with the worktree's separate manifest fix (`"direct":true,"requiredBy":[]`) applied:
  `packages/mustache/check-doc-examples.sh /Users/justinzaun/Development/mfb/target/release/mfb`
  gives "1 of 4 example(s) failed". The json_schema script gives "3 of 10 example(s) failed".
- Regression from bug-750's fix, `a03ba910e` ("bug-750: a package never exports a builtin
  package's types"). See `bugs/completed/bug-750-a-package-that-imports-io-exports-io-s-key-types.md`.
- `mfb spec language modules-and-packages`: "Imports remain non-transitive for
  **names**: ... an importing project cannot write `basepkg::` merely because a
  package it declares imports `basepkg` — that still requires its own `IMPORT
  basepkg`." The rule is about names. No spec text requires an importer to import a
  builtin that it never names. `grep -rn "references unknown type\|validate_package_type" src/docs`
  finds no spec rule for this rejection.
- `mfb spec package type-table` (bug-750's added sentence): "the type table may hold `io.Key`,
  but the package does not export it". The fix commit message assumes that "every
  importer gets the type from its own builtin import under the same spelling". That
  assumption is the defect.
- Package READMEs document the intended contract. `packages/mustache/README.md`,
  `packages/jwt/README.md` and `packages/yaml/README.md` say an importer writes
  `IMPORT json` "to name the type".
- bug-466: builtin record **fields** need the file's own import. This must not regress.
- Siblings with the same symptom and a different cause: bug-435 and bug-624 (fixed).

## Failing Reproduction

```sh
MFB=/Users/justinzaun/Development/mfb/target/release/mfb
mkdir -p /tmp/bug820/jpkg/src /tmp/bug820/app/src && cd /tmp/bug820
cat > jpkg/project.json <<'J'
{"name": "jpkg", "version": "0.1.0", "description": "bug repro", "mfb": "1.0", "kind": "package", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "targets": ["native"]}
J
cat > jpkg/src/lib.mfb <<'B'
IMPORT json
EXPORT FUNC f(x AS json::Json) AS Integer
  RETURN 1
END FUNC
EXPORT FUNC g() AS Integer
  RETURN 2
END FUNC
B
cat > app/project.json <<'J'
{"name": "app", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "packages": [{"name": "jpkg", "version": "=0.1.0", "source": "local:///tmp/bug820/jpkg", "direct": true, "requiredBy": []}], "entry": "main", "targets": ["native"]}
J
cat > app/src/main.mfb <<'B'
IMPORT io
IMPORT jpkg
FUNC main() AS Nothing
  io::print(toString(jpkg::g()))
END FUNC
B
$MFB build app
```

- Observed (exit 1):
  ```
  app/src/main.mfb:2 error[6-605-0001 PACKAGE_INVALID]: package container is malformed or incompatible
                 Imported package `app/build/packages/jpkg.mfp` has exported function `f` parameter `x` that references unknown type `json.Json`.
  ```
- Expected: builds, and `app/build/app.out` prints `2`.

Contrast cases measured with the same binary:

- Adding `IMPORT json` to `app/src/main.mfb` makes the build pass (`Wrote executable`), and the program prints `2`.
- `IMPORT fs` with `EXPORT FUNC f(RES x AS fs::File)` builds without `IMPORT fs` in
  the consumer. Resources are exempt (`self.is_resource_type`).
- The same rejection occurs for `datetime::DateTime` and `crypto::KeyPair` parameters.
  It also occurs for an exported `TYPE Box` with a `v AS json::Json` field ("exported
  type `Box` that references unknown type `json.Json`"). Finally, it occurs for the pass-through
  `LET v = pkg::make()` / `pkg::f(v)` (make returns `json::Json`, f stringifies it).

Regression matrix (macOS arm64, release). The probes live in `/tmp/bug820/{app,probe_datetime,probe_crypto,probe_json,probe_pass}`.

| Compiler | Result |
| --- | --- |
| `a03ba910e~1` (`6ff5d697b`), built in a temp `git worktree add --detach` | all five consumers build. `app.out` prints `2`, and the pass-through prints `{"a":7}` ✓ |
| same tree plus only `git checkout a03ba910e -- src/binary_repr/sections.rs src/ir/mod.rs src/ir/package.rs` | all five fail with `references unknown type` ✗ |
| `target/release/mfb` built today from main | all five fail ✗ |

## Root Cause

The importer knows a type from only two sources: its own HIR and the dependency's
**exported** types. `src/ir/shape.rs:Walker::check_imported_packages` →
`validate_package_type` checks every nominal in an imported package's exported
function signatures and exported type fields against `self.types`. That table is
built in the `Walker` constructor from two sets:

1. the consumer's own HIR `TYPE` declarations. Builtin types appear here only as the
   injected source of a builtin the **consumer** imports;
2. `imported_types`, from `src/manifest/package.rs:imported_type_defs` /
   `imported_type_defs_from_files`. These are read only from each `.mfp`'s
   **exported** type list. That function's doc says: "Built-in packages contribute
   nothing here: their types reach `TypeIndex` through the AST".

Before `a03ba910e`, `src/binary_repr/sections.rs:TypeTable::reserve_source_type` marked a
builtin type riding in the package's IR (`json.Json`) as exported. That was the bug-750
leak. Route 2 then supplied it to every importer. bug-750 correctly stopped exporting it.
The type is still in the package's type table (`mfb pkg info jpkg.mfp`: `types: 16`,
`exports: 2`), but no importer path reads non-exported builtin entries. The writer-side
fix therefore removed the only route by which an importer that doesn't import `json`
learned `json.Json`. The comment in `qualify_package_type` (`src/manifest/package.rs`,
"A BUILT-IN value type a package re-exports (`regex.Group`, `http.Response`)
has no `.mfp`...") shows the old importer path relied on that leak.

The contrast cases are not affected for these reasons: a consumer with `IMPORT json` gets `json.Json` via route 1.
Resources return early on `is_resource_type`. `AttributedString`/`Error`/`ErrorLoc`/`Scalar` return early on
`is_always_in_scope_nominal`.

## Goal

- The `/tmp/bug820/app` reproduction and the four probes (datetime, crypto, json record
  field, json pass-through) build and run with no `IMPORT <builtin>` in the consumer. The
  pass-through prints `{"a":7}`.
- `packages/mustache/check-doc-examples.sh` and `packages/json_schema/check-doc-examples.sh`
  pass with their DOC examples unchanged.

### Non-goals (must NOT change)

- bug-750: a package still never **exports** a builtin type.
  `src/binary_repr/tests/writer_tests.rs:an_imported_builtin_type_is_not_exported` and the
  `package-import-as` golden (`exports: 2`) stay as they are. Do not revert `reserve_source_type`.
- bug-466: reading a builtin record's fields still needs the consumer's own import.
  Making `json.Json` resolvable for validation must not make `datetime.DateTime`'s fields
  readable without `IMPORT datetime`. The `builtin_record_owner` gate in the `Walker`
  constructor keys off `shadowed`, which holds the non-internal `types` entries. If builtin types are
  added to `types` as "imported" (empty `file`), they become `shadowed`, and the bug-466 gate silently turns off.
  Guard against exactly this.
- Name visibility: the consumer still cannot *write* `json::Json` without `IMPORT json`
  (`SYMBOL_UNKNOWN_TYPE` / unknown-package behavior is unchanged).
- Tempting wrong fix: adding `IMPORT json` to the mustache/json_schema DOC examples.
  That hides the regression, and the spec does not require it.

## Blast Radius

Found by `grep -rnE '^EXPORT .*\b[a-z_]+::[A-Z][A-Za-z]*' packages/*/src` and
`grep -rnE '^\s+[a-zA-Z_]+ AS .*\b[a-z_]+::[A-Z]' packages/*/src`:

- `packages/mustache` (`render`, `renderWith`: `json::Json`). DOC example ex3 fails. Fixed by this bug.
- `packages/json_schema` (`compile`, `validate`, `isValid`, `check`: `json::Json`; `index.mfb` record
  field `root AS json::Json`). DOC examples ex7/ex8/ex9 fail. Fixed by this bug.
- `packages/jwt`: 6 exported `json::Json` signatures, record fields `header`/`claims AS json::Json`,
  and `crypto::KeyPair`. Any consumer without both `IMPORT json` and `IMPORT crypto` fails. Not
  observed in its scripts, because those examples import json. Fixed by this bug.
- `packages/yaml` (2 exported `json::Json` signatures and `json::Json` record fields). Same hazard. Fixed by this bug.
- `packages/timezones` (`datetime::Date/DateTime/Instant/Time/Zone`, record field
  `dateTime AS datetime::DateTime`). Its runtime smoke already has `IMPORT datetime` (added by
  `a03ba910e` for field reads, which is legitimate under bug-466). Any consumer that doesn't name
  `datetime` fails. Fixed by this bug.
- `packages/tar`, `packages/zip` (`RES file AS fs::File`) and `packages/logger` (`RES fs::File`,
  `RES tcp::Socket` fields). Not affected, because resources are exempt (probe above).
- `packages/currency` (in progress in this worktree). Affected if its exports name a builtin
  type. Check its DOC examples after the fix.
- Third-party packages with the same shape are affected identically.
- Other importer-side readers of only the exported type list: `src/manifest/package.rs:imported_type_names`
  (parser type-position normalization) and `imported_global_defs` (an `EXPORT LET x AS json::Json`
  global types via `qualify_package_type`). These are not probed, so the same hazard is a guess.
  Audit them in Phase 1.

## Fix Design

Fix the importer, not the writer. When `validate_package_type` (and whatever
lowering/codegen then needs the type's layout) meets a builtin-owned nominal
(`crate::ir::is_builtin_type_name`, package-qualified to a builtin package from
`crate::codegen::registry::registry()`), it must resolve the nominal from the builtin's
own definition, whether or not the consumer imported it. Two options:

1. Seed the walker's and lowering's type tables with the builtin types reachable from
   the imported packages' signatures, taken from the builtin registry or injected source.
   Mark them so that bug-466's `shadowed` set and the name resolver do not treat them as
   project/imported types.
2. Read the non-exported builtin entries the `.mfp` type table already carries (bug-750
   kept them there) and register them as builtin-owned, not exported.

The risk sits in three places. First, keeping bug-466's field gate on. Second, keeping
name visibility unchanged. Third, making sure the native merge already contains the
builtin's functions. The pre-fix pass-through ran correctly, which suggests the merge is
fine, but that is a guess for the post-fix design. Rejected alternatives: reverting bug-750,
and editing the DOC examples (see Non-goals).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add an acceptance fixture: package plus consumer without `IMPORT json`, calling only `g()`,
      plus the pass-through and record-field variants. Add a `shape.rs` unit test. Confirm RED with the message above.
- [x] Add a bug-466 guard test: a consumer of `timezones` without `IMPORT datetime` reading
      `.dateTime.date` must still be rejected.
- [x] Probe `imported_type_names` / `imported_global_defs` with an `EXPORT LET` of a builtin type
      and record the verdicts above.

Acceptance: the new tests fail for the documented reason; every blast-radius site has a verdict.
Commit: —

### Phase 2 — the fix

- [x] Implement the importer-side resolution in `src/ir/shape.rs` and in the importer readers in
      `src/manifest/package.rs` that need it.
- [x] Update `mfb spec package type-table` and `mfb spec language modules-and-packages` to state
      that an importer resolves a dependency's builtin types without its own import.

Acceptance: Phase 1 tests pass; bug-750 and bug-466 tests unchanged and green.
Commit: —

### Phase 3 — full validation

- [x] `cargo test` and `scripts/test-accept.sh target/release/mfb target/accept-actual`. No golden
      should move. If one does, inspect it before touching it.
- [x] Run every `packages/*/check-doc-examples.sh` and package smoke script.

Acceptance: full suite green; mustache and json_schema doc examples all pass unedited.
Commit: —

## Validation Plan

- Regression tests: the Phase 1 fixture and unit tests.
- Runtime proof: the reproduction prints `2`; the pass-through prints `{"a":7}`; both
  check-doc-examples scripts report 0 failures.
- Doc sync: the two spec topics above. The package READMEs already describe the intended behavior.
- Full suite: `cargo test`, `scripts/test-accept.sh`.

## Open Decisions

- Source of the builtin type definition: registry/injected source (option 1) vs. the `.mfp`
  type table (option 2). Recommended: option 1. The compiler owns builtin layouts, and a
  stale `.mfp` must not define them.

## Summary

The engineering risk is in keeping bug-466's field-visibility gate and bug-750's
no-export rule intact while still resolving builtin types for importers. The writer and
the `.mfp` format stay as they are.

## Resolution (2026-10-04)

**Fix (importer side, option 1).** `src/ir/shape.rs:validate_package_type` now
accepts a nominal it has no type-table row for when `is_builtin_package_type`
holds: a `pkg.Leaf` spelling whose qualifier is a builtin import
(`builtins::is_builtin_import`) and whose leaf the registry declares
(`registry().is_builtin_type`). The compiler owns those definitions, so they are
not walked further. The type is NOT inserted into the walker's `types` table, so
`shadowed` is unchanged and bug-466's `builtin_record_owner` gate stays on. The
writer, the `.mfp` format and `reserve_source_type` are untouched, so bug-750's
no-export rule holds.

Nothing downstream needed the type: with only the validator changed, lowering
and native codegen built the probes, and `app_pass` built with `--debug` reports
`arena.0.live_bytes 0` and the same `peak_live_bytes 1104` with and without
`IMPORT json` (the merged package IR already carries the builtin's types and
functions).

**Audit verdicts.** `imported_type_names` reads only user-package type names for
the parser's `pkg::Leaf` normalization; a builtin type is never spelled under a
user package, so it is not affected. `imported_global_defs`: an
`EXPORT LET J AS json::Json` now type-checks and builds without `IMPORT json`.
The probe exposed a separate defect, fixed in its own commit. A package's
identity-prefixed copy of a builtin's bindings (`<identity>.pkg.#JSON_DEPTH_LIMIT`)
escaped bug-673's `order_builtin_bindings_first`. So a package's top-level
`json::parse` initializer read the depth limit as zero, and the program died at
startup with "invalid JSON format: nested too deeply", even when the consumer
did `IMPORT json` (measured on the parent). The fix classifies the binding by
its leaf after the last `.`.

**Proof.**
- RED on the parent (`0865577ea`): the unit test fails with "references unknown
  type `app.Mode`". The rt fixture fails with nine `PACKAGE_INVALID`
  diagnostics. After the fix it prints `42`, `{"a":7}`, `7`, `[1,2]`,
  `2024-02-29T12:00:00.000Z`.
- The `/tmp/bug820`-style probes (`g()` only, pass-through, record field,
  `datetime`, `crypto`) all build. They print `2`, and the pass-through prints
  `{"a":7}`.
- Name visibility is unchanged: `LET v AS json::Json` without `IMPORT json` is
  still `SYMBOL_UNKNOWN_IMPORT`.
- bug-466: `at.date` on a dependency's `datetime::DateTime` is still refused
  ("add `IMPORT datetime`"), as pinned by the new syntax fixture.
- bug-750: `writer_tests::an_imported_builtin_type_is_not_exported` passes. The
  `package-import-as` `.mfp` golden is byte-identical (`exports: 2`).
- Doc examples, run unedited: `packages/mustache/check-doc-examples.sh` goes from
  "1 of 4 example(s) failed" to "all 4". `packages/json_schema` goes from
  "3 of 10" to "all 10". currency, cli, logger and xml stay all-pass. The cli,
  currency, logger and timezones smoke scripts pass on both compilers.
- Reach check: every one of the 1654 `project.json` projects under `tests/` and
  `examples/` was built with the parent and with the fixed compiler. The build
  logs, the 999 executable hashes and the 37 `.mfp` hashes are identical, except
  for the three new fixtures. No existing golden moves, and
  `scripts/sync-package-mfp.sh` is not needed.
