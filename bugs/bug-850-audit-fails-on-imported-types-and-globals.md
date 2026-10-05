# bug-850: `mfb audit` exits 3 with spurious errors on any project that uses an imported package's exported types or globals, so `EXPORT MUT` is never surfaced

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/<audit cli test> (proposed)

The spec makes exported package `MUT` state an audit concern: it "must be surfaced by
audit tooling", and `src/audit/collect/findings.rs` implements
`AUDIT-PKG-STATE-EXPORTED-MUT` for it. But `mfb audit` aborts with exit code 3 before
collecting any finding whenever the project:

- imports a package that exports any `TYPE`/`ENUM`. It prints one
  `PACKAGE_INVALID ... has exported type `X` that references unknown type `pkg.X``
  per exported type and signature. This happens even with an installed
  `packages/<name>.mfp` the build accepts; or
- reads an imported `EXPORT LET`/`EXPORT MUT` (`toString(pkg::state)` gives
  `TYPE_CALL_ARGUMENT_MISMATCH ... (Unknown)`).

`mfb build` of the same project succeeds and the executable runs correctly. Audit is
therefore unusable on most projects with real package dependencies. Projects that read
exported state directly are exactly the ones the EXPORT-MUT warning targets, and they
are among those that cannot be audited. **Correct behavior:** `mfb audit` accepts every
project `mfb build` accepts and reports its findings, including
`AUDIT-PKG-STATE-EXPORTED-MUT`.

Class: CODE BUG.

References:

- src/docs/spec/language/13_modules-and-packages.md: "Exported top-level `MUT` ... is package state visible to importers and must be surfaced by audit tooling."
- bugs/completed/bug-551-exported-package-constant-does-not-resolve-for-an-importer.md (fixed the build path only)

## Failing Reproduction

```
cd /tmp/hardening/B03/modules
mfb init-pkg audpkg ; mfb init aud1
# audpkg/src/lib.mfb
EXPORT MUT state AS Integer = 1
EXPORT FUNC get() AS Integer
  RETURN state
END FUNC
(cd audpkg && mfb build -q) ; mkdir aud1/packages ; cp audpkg/audpkg.mfp aud1/packages/
# aud1/project.json "packages": [{"name":"audpkg","version":"=0.1.0","direct":true,"requiredBy":[]}]
# aud1/src/main.mfb
IMPORT io
IMPORT audpkg
FUNC main() AS Nothing
  io::print(toString(audpkg::state))
END FUNC
cd aud1 && mfb build -q && ./build/aud1.out && mfb audit ; echo exit=$?
```

- Observed: the build writes the executable and it prints `1`. Audit then prints
  `./src/main.mfb:4 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: ... argument type(s) (Unknown)`,
  with no report, and `exit=3`.
- Expected: the audit report, including
  `warning AUDIT-PKG-STATE-EXPORTED-MUT package `audpkg` exports mutable state `state``, with exit 0.

Exported-type variant (/tmp/hardening/B03/modules/rs_mfp: installed `shapes.mfp`
exporting `TYPE Circle`, main reads `shapes::Limit`). The build is fine. Audit gives 6×
`error[6-605-0001 PACKAGE_INVALID]: ... Imported package `./packages/shapes.mfp` has exported type `Circle` that references unknown type `shapes.Circle`.`

Contrast (works): change main to `io::print(toString(audpkg::get()))` (function only, no
exported types). Audit then exits 0 and lists `AUDIT-PKG-STATE-EXPORTED-MUT`, which proves the
collector itself is correct.

## Root Cause

`src/audit/mod.rs:run` re-runs a front end that diverged from `src/cli/build/mod.rs`. It
calls `resolver::resolve_project`, `monomorph`, `resolve_augmented`, and then
`ir::shape::check_project(&location, &concrete_hir, &imported_signatures)` with only the
imported *function signatures*. The build path also computes and passes `imported_types`
(`imported_type_defs`) and, since bug-551, `imported_globals` (`imported_global_defs`)
into the shape pass. Audit omits both, so imported types and globals type as `Unknown`
or unresolved. The `PACKAGE_INVALID ... references unknown type` diagnostics come from
the package-reading path, which runs without the type table the build supplies. Audit
also never runs `build_source_dependencies`, so a `local://`/source dependency has no
compiled `.mfp` to read unless a previous build left one.

## Goal

- `mfb audit` succeeds (exit 0/1 by findings) on every project `mfb build` accepts,
  including ones that use imported exported types and globals.

### Non-goals (must NOT change)

- Audit's finding codes and report formats.

## Blast Radius

- `src/audit/mod.rs:run` — fixed by this bug
- `src/cli/build/mod.rs` front-end sequence — the reference implementation; better to share it than to copy it again
- `src/audit/collect/findings.rs` EXPORTED-MUT reads only `packages/<name>.mfp`; a source
  dependency's exported MUT is never surfaced even after the fix. Same hazard, decide in scope.

## Fix Design

Factor the build front end (manifest → parse → resolve → monomorph → shape with
imported types and globals) into one function both commands call. Have audit compile
source dependencies first, or resolve through `resolved_package_file`.

## Validation Plan

- Regression: an audit CLI test over a two-project fixture (exported TYPE + EXPORT MUT,
  importer reads the global), asserting exit 0 and the EXPORTED-MUT line.
- Full suite: the project's acceptance suite.

## Summary

The risk is the duplicated front end. The durable fix is sharing it with build.
