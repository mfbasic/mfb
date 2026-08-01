# bug-395: `.mfp` foreign-type re-export resolver joins an unvalidated owner name → path traversal / `*.mfp` existence oracle

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Security

Status: Open
Regression Test: tests/ — add a binary_repr unit test asserting a `foreign_owner`
of `../evil` / absolute path is rejected (bare-name validated) before the join.

`read_package_type_exports_resolved` (`src/binary_repr/mod.rs:581`) resolves a
package's re-exported foreign types (bug-390, kind-12 `FOREIGN_TYPE`) by locating
the owner package's `.mfp` next to the importing package:

```rust
let owner_path = dir.join(format!("{owner}.mfp"));   // mod.rs:614
if !owner_path.is_file() { continue; }
let owner_exports = read_package_type_exports_resolved(&owner_path, depth + 1)?;
```

`owner` is `export.foreign_owner`, a `String` decoded verbatim from a package's
`.mfp` type-export table (written at `src/binary_repr/builder.rs:275`, carried on
`BinaryReprTypeExport.foreign_owner` at `mod.rs:245`). It is **never validated as
a bare filename** on the read/resolve path. Because `PathBuf::join` with an
absolute string replaces the base, and `../` components walk upward, a hostile
`.mfp` that exports a foreign type whose `owner_package` is `"../../../../etc/foo"`
(or an absolute path) makes a consumer:

1. `stat` an arbitrary `<path>.mfp` outside the packages directory — an
   **existence oracle** for any `*.mfp`-suffixed path on the victim's machine, and
2. if that file exists and parses, recursively decode it and splice its type
   definitions into the malicious package's view (depth-capped at 64).

This is the same missing-guard class as bug-58 / bug-195 (a package/dependency
name path-joined without `validate_package_name`), at a **new site the earlier
fixes never touched**. The sibling native-library locator `source` field, which
also feeds a path join, IS re-validated here — `sections.rs:978` calls
`crate::manifest::libraries::source_is_bare(&source)` with a comment explaining a
hostile `.mfp` naming `../../etc/foo` must not reach the join. `foreign_owner` is
the asymmetric gap: same module, same hazard, no guard.

The path was introduced by bug-390 (commit 75c23464a, 2026-07-26), *after* the
goal-05 package-decode security audits (audit-1/audit-2, authored 2026-07-13/14),
so it was never in that audit's scope.

References:

- `src/binary_repr/mod.rs:581` (`read_package_type_exports_resolved`), `:614`
  (the unvalidated join), `:245` (`foreign_owner` field).
- Sibling that DOES validate: `src/binary_repr/sections.rs:978`
  (`source_is_bare`); validator `src/manifest/libraries.rs::source_is_bare`.
- Prior same-class: bug-58, bug-195 (`validate_package_name`).
- Introduced by bug-390 (`bugs/completed/`); found during goal-07.

## Failing Reproduction

Reasoned from source (crafting a valid signed `.mfp` with a kind-12 foreign-type
export whose `owner_package` is hostile is non-trivial by hand). The traversal
join at `mod.rs:614` is unconditional whenever `foreign_owner` is `Some` and
`path.parent()` is `Some`, and no component/absolute/`..` check exists between the
decode and the join.

- Observed: `dir.join(format!("{owner}.mfp"))` with `owner = "../../../../etc/foo"`
  resolves to `<packages>/../../../../etc/foo.mfp`, which is `stat`ed and (if
  present + parseable) read.
- Expected: `foreign_owner` is validated as a bare package name (no `/`, `\`,
  `..`, NUL, not absolute) at decode or before the join; a non-bare owner is a
  clean decode error, exactly like the `source` locator.

Contrast (immune): the native-library `source` locator in the same module is
guarded by `source_is_bare` (`sections.rs:978`), so it cannot traverse.

## Root Cause

`read_package_type_exports_resolved` trusts the decoded `foreign_owner` string as
a filename component. `foreign_owner` originates from untrusted package bytes and
is never run through `validate_package_name` / `source_is_bare`.

## Goal

- A `foreign_owner` containing a path separator, `..`, NUL, or an absolute path is
  rejected (bare-name validated) before `dir.join`, producing a clean decode error
  rather than a filesystem access outside the packages directory.

### Non-goals (must NOT change)

- The legitimate same-directory sibling-`.mfp` resolution for a well-formed bare
  owner name (bug-390's feature) must keep working.
- No change to the `.mfp` on-disk format or the `foreign_owner` field itself.

## Blast Radius

- `src/binary_repr/mod.rs:614` — fixed by this bug (add bare-name validation of
  `owner` before the join).
- `src/binary_repr/sections.rs:978` (`source` locator) — already guarded; the
  model to copy.
- Any other consumer of `foreign_owner` — none found beyond the resolver above
  (grep: `foreign_owner` is only read at `mod.rs:597/608` in the same function).

## Suggested fix (test-first, NOT landed in goal-07)

Add a RED test that decodes/constructs a `BinaryReprTypeExport { foreign_owner:
Some("../evil"), .. }` and asserts `read_package_type_exports_resolved` errors
(or skips) rather than joining. Then, before `dir.join` at `mod.rs:614`, validate
`owner` with the existing bare-name rule (`manifest::libraries::source_is_bare` or
`validate_package_name`) and turn a non-bare owner into a decode error / `continue`.
