# bug-853: importing a source dependency whose manifest is `kind: "executable"` first compiles and links it into its own `build/`, then refuses it

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: tests/<cli build test> (proposed)

A `packages` entry may point at a source directory (`local:///abs`, `file:<dir>`, or
implicit `packages/<name>/`). The spec says that directory's manifest "`kind` must be
`package`", and the resolver does refuse it with `IMPORT_PACKAGE_KIND_INVALID`. Before
that check runs, though, `build_source_dependencies` has already run a full
`build_project` on the directory. For an executable that means it clears **that other
project's** `build/` directory, then compiles and links an executable there. The same
happens when the manifest `name` does not match (`IMPORT_PACKAGE_NAME_MISMATCH`).
The user only built their own project, yet a foreign project's build output is wiped and
rewritten, and the time for a full native link is wasted. **Correct behavior:** a source
dependency whose `project.json` has the wrong `kind` (or `name`) is rejected without
being built, and nothing outside the importing project's own `build/` is written.

Class: CODE BUG (spec plausibly right: "the manifest `name` must match the import and `kind` must be `package`").

References:

- src/docs/spec/language/13_modules-and-packages.md, import resolution steps 3 and 5
- src/docs/spec/language/13_modules-and-packages.md §13.1: "The build compiles it before resolving any import against it"

## Failing Reproduction

```
cd /tmp/hardening/B03/modules
mfb init exe_shapes     # then set "name": "shapes" in exe_shapes/project.json (kind stays executable)
mfb init rs_kind2
# rs_kind2/project.json "packages": [{"name":"shapes","version":"=0.1.0",
#   "source":"local:///tmp/hardening/B03/modules/exe_shapes","direct":true,"requiredBy":[]}]
# rs_kind2/src/main.mfb: IMPORT io / IMPORT shapes / FUNC main ... shapes::Limit
cd rs_kind2 && /tmp/hardening/mfb build
ls ../exe_shapes/build
```

- Observed:
  ```
  Building shapes (executable) for macos-aarch64
  Wrote executable to /tmp/hardening/B03/modules/exe_shapes/build/shapes.out
  Building rs_kind2 (executable) for macos-aarch64
  ./src/main.mfb:2 error[2-201-0007 IMPORT_PACKAGE_KIND_INVALID]: imported source package must be a package
  ```
  and `exe_shapes/build/shapes.out` exists.
- Expected: only the `IMPORT_PACKAGE_KIND_INVALID` error, with no `Building shapes` and no write under `exe_shapes/`.

The same happens with the implicit `packages/shapes/` form (scratch: `rs_kind3`, which writes
`./packages/shapes/build/shapes.out`) and with a name mismatch (`kindexe`, which builds `app2.out`).

## Root Cause

`src/cli/build/source_packages.rs:source_dependency_dirs` selects every declared
source-directory dependency that has a `project.json`, and
`build_source_dependency` calls `build_project` on it unconditionally. Neither reads
the dependency manifest's `kind` or `name`. That validation lives only in
`src/resolver/packages.rs` (`validate_source_package_manifest`), which runs later
against the IMPORT line. For an executable, `build_project` ignores
`package_output_dir` and writes into the dependency's own `build/` (after
`clear_build_dir`).

## Goal

- `build_source_dependencies` skips, or refuses, a dependency whose manifest `kind` is not
  `package` or whose `name` differs from the entry. The resolver then reports the existing
  `IMPORT_PACKAGE_KIND_INVALID` / `IMPORT_PACKAGE_NAME_MISMATCH` against the IMPORT line,
  and no foreign `build/` is touched.

### Non-goals (must NOT change)

- Diagnostics codes and their IMPORT-line placement.
- Not every declared dependency is imported, so do not turn an unimported mis-kinded
  entry into a hard error unless the owner decides so.

## Blast Radius

- `src/cli/build/source_packages.rs:source_dependency_dirs` / `build_source_dependency` — fixed by this bug
- `src/resolver/packages.rs:validate_source_package_manifest` — unaffected (the check that should come first)

## Fix Design

In `source_dependency_dirs`, parse the dependency `project.json` (cheap). Drop entries
whose `kind != "package"` or whose `name` differs, leaving them for the resolver to report.

## Validation Plan

- Regression: a CLI test asserting that the dependency dir has no `build/` after the
  failing build, and that stderr has no `Building <dep> (executable)` line.
- Full suite: the project's acceptance suite.

## Summary

The side effect is on someone else's project directory. The fix is a pre-filter.
