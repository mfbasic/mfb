# bug-624: a package's PRIVATE type collides with a program type of the same name

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/cli (to add, Phase 1)

A program that declares a `TYPE` whose name matches a **private** (non-`EXPORT`) type inside a
package it imports fails to build. The package's own code is checked against the program's
type of that name. A private type is not part of the package's surface, so its name must not
be visible to — or shadowed by — the consumer.

**The single correct behavior a fix produces:** a private type name inside a package and a type
of the same name in the consuming program are distinct types; the program below builds and
prints `mine 4`.

References:

- `src/docs/spec/language/13_modules-and-packages.md` (EXPORT and package-private names).
- Found by plan-133-A Phase 2: a probe declaring `TYPE Frame` while importing the browser's
  `dom` package (which has a private `TYPE Frame` in `parse.mfb`) failed with
  `PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: member Frame::kids is annotated as List OF Node,
  but the field is declared List OF String`.

## Failing Reproduction

`/tmp/plan-133-a/collide/` (plan-133-A harness), main `14c9fc1ca`, macOS:

`packages/pk/project.json` — a `kind: "package"` named `pk`; `packages/pk/src/lib.mfb`:

```
IMPORT collections

' A PRIVATE record: never exported.
TYPE Frame
  tag AS String
  kids AS List OF Integer
END TYPE

EXPORT FUNC depth(n AS Integer) AS Integer
  MUT stack AS List OF Frame = [Frame["root", []]]
  MUT i AS Integer = 0
  WHILE i < n
    stack = collections::append(stack, Frame["x", [i]])
    i = i + 1
  END WHILE
  RETURN len(stack)
END FUNC
```

`src/main.mfb` of an executable depending on `{"name": "pk", "source": "file:packages/pk"}`:

```
IMPORT io
IMPORT pk

' The program's own Frame: same name as pk's PRIVATE record, different fields.
TYPE Frame
  tag AS String
  kids AS List OF String
END TYPE

SUB main()
  LET f AS Frame = Frame["mine", ["a"]]
  io::print(f.tag & " " & toString(pk::depth(3)))
END SUB
```

- Observed: `target/release/mfb build /tmp/plan-133-a/collide/app` → `Building app (executable)
  … error: TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 2 for Frame has type List OF Integer,
  expected List OF String for field kids.`, exit 1. The error is in `pk`'s code
  (`Frame["x", [i]]`) checked against the program's `Frame`.
- Expected: builds; prints `mine 4`.
- With the browser's `dom` (recursive private `Frame`) the same collision surfaces as
  `PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE` (`src/ir/verify/compat.rs`,
  `check_member_access_type`).

## Root Cause

Unconfirmed. Hypothesis: when a package's IR is merged into the program
(`merge_packages`, `src/target/shared/nir/lower.rs`), its private type definitions enter the
same type table under their bare names, so `field_type` / constructor checks in
`src/ir/verify/compat.rs` and `src/ir/verify/values.rs` resolve the package's `Frame` against
whichever definition wins. Confirm: dump the merged type table for the repro; check whether
private types are qualified (`pk::Frame`) or dropped at merge.

## Goal

- The repro builds and prints `mine 4`.
- The same with the names swapped (program type recursive, package type flat).

### Non-goals (must NOT change)

- Exported types keep their qualified `pkg::Name` spelling and behavior.
- **Tempting wrong fix:** renaming the browser's private types — the collision is general.

## Blast Radius

- Every package with a private `TYPE`/`UNION`/`ENUM` and every consumer — fixed by the fix.
- `.mfp` binary representation of private types — audit whether the name is serialized
  unqualified.

## Fix Design

Qualify private type names with their package at merge (or keep them in a package-scoped table)
so lookups from package code and program code can never see each other's private names.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] A CLI test building the repro (source package), and the `.mfp` form; confirm both fail.
- [ ] Locate where private type names enter the program's type table; record it here.

Acceptance: tests fail for the documented reason; root cause cited.
Commit: —

### Phase 2 — the fix

- [ ] Package-scoped private type names at merge.

Acceptance: Phase 1 tests pass.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate any `.mfp`/IR goldens whose private type names change; full suite;
      `scripts/test-accept.sh`.

Acceptance: full suite green; golden deltas are only private-name qualification.
Commit: —

## Validation Plan

- Regression tests: Phase 1 CLI tests.
- Runtime proof: the repro prints `mine 4`.
- Doc sync: `13_modules-and-packages.md` if it does not already state private-name scope.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- None.

## Summary

A name-scoping bug at package merge; the audit of the `.mfp` form is where the surprises would be.

## Phase 1 findings (fix-bug, 2026-09-15)

- Reproduced at main `9b5e5b55f` in both forms: the source package and a prebuilt
  `file:packages/pk.mfp` fail identically with `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 2
  for Frame has type List OF Integer, expected List OF String for field kids.`
- Root cause confirmed by reading: `merge_packages` (`src/target/shared/nir/lower.rs`) calls
  `ir::prefix_package_symbols` (`src/ir/package.rs`), which identity-prefixes a package's
  functions, globals and LINK aliases but documents "Types are left unqualified", then
  `ir::merge_package` merges `types` with `push_unique(|a, b| a.name == b.name)` — first wins.
  The program's `Frame` is already in `merged.types`, so the package's private `Frame` is
  dropped and `verify_semantics` checks the package's constructor against the program's type.
- RED tests: `tests/runtime/rt_package_private_type_collision.rs` — flat/flat, recursive
  package/flat program (the `dom` shape) and flat package/recursive program, each from the
  source and the `.mfp` form.

## Phase 2 findings (fix-bug, 2026-09-15)

- Fix (`src/ir/package.rs:scope_private_types`, `src/internal_name.rs:private_name_parts`):
  before the function prefix, every referenced private type of a decoded package is renamed
  to `#<package identity id>$<name>` — the file-PRIVATE mangle shape (the identity id is 16
  hex digits, `FILE_HASH_HEX_LEN`), so `internal_name::display_name` still prints `Frame` — and
  every reference is rewritten: type annotations, union variants and `UnionWrap` /
  `UnionExtract` types, `MATCH` variant patterns (`Local("Frame")`) and `Enum.Member` targets.
  Kept bare: the exported surface and its transitive field/variant/include closure plus every
  LINK/CSTRUCT type (codegen re-registers exported types from the `.mfp` by bare name), builtin
  package types (`json.Json`, `#json_Node`), and declarations nothing references
  (`TYPE Integer` shadowing). No `.` in the new name, so `ir::verify`'s qualified-to-bare
  fallback cannot equate it with a bare `Frame`.
- Private ENUMs collided too: an unpatched edge probe failed with `TYPE_UNKNOWN_ENUM_MEMBER`.
- Diamond import (app → pa, app → pc → pa) still collapses (same identity id); two packages
  each with a private `Frame` stay distinct. The browser `dom` package imported beside a
  program `TYPE Frame` builds and runs (unpatched: `TYPE_UNKNOWN_FIELD`).
- Known limit: a literal `CASE x` whose local variable is spelled like a private type would
  be renamed; not reachable from any tested program.
- Verification: `rt_package_private_type_collision` 6/6 (RED at `9b5e5b55f`);
  `rt_imported_type_qualified_name`, `rt_imported_union_enum_members`,
  `rt_imported_record_map_field_keys`, `rt_foreign_type_reexport`,
  `rt_imported_package_global`, `rt_imported_resource_scope_drop`,
  `rt_package_parameter_defaults` 39/39; `cargo test --release --bin mfb ir::` 906/906.
- **Sub-issue B (new):** a package whose exported type's field, or exported function /
  binding signature, names a non-exported type builds, and every importer rejects it:
  `PACKAGE_INVALID: Imported package … has exported type Pub that references unknown type
  Hidden` (and `exported function mk return type …`, `… parameter p …`), at `9b5e5b55f` and
  after the fix. The `.mfp` type table carries only the exported surface
  (`binary_repr`'s bug-435 closure only fills types re-exported from other packages). The
  package build must reject it at the declaration; writing the hidden type into the table
  instead would let importers name a type its author did not export. RED tests:
  `an_exported_type_naming_a_non_exported_type_is_rejected_at_the_package_build`,
  `an_exported_signature_naming_a_non_exported_type_is_rejected_at_the_package_build`;
  guard `an_exported_type_naming_an_exported_type_still_builds_and_imports`.
