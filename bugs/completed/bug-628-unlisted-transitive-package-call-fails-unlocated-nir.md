# bug-628: a call into a package's own dependency fails to build with an unlocated `NIR call target … does not resolve` when the importer does not list that dependency

Last updated: 2026-09-16
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: FIXED
Regression Test: tests/runtime/rt_package_dependency_closure.rs

An executable that lists only package `userpkg` in its manifest, where `userpkg` itself imports
package `basepkg` and calls `basepkg::base()`, does not build. The build of both packages succeeds
(`Wrote package …/basepkg.mfp`, `Wrote package …/userpkg.mfp`), and then the executable build fails
with the internal, unlocated `error: NIR call target 'basepkg.base' does not resolve`. The program
never names `basepkg`; the diagnostic points nowhere and names an internal stage.

**The single correct behavior a fix produces:** the build either works (the dependency the package
needs is merged as it would be had the importer listed it) or is refused at resolve with a
**located**, user-facing diagnostic naming the package that must be declared. Never the internal
NIR error. Which of the two is an Open Decision.

References:

- `mfb spec language modules-and-packages` (package dependencies and imports);
  `mfb spec architecture binary-representation` §"Decode-and-Merge of Package Dependencies".
- Found during plan-136-B Phase 3 (`planning/plan-136-B-package-parameter-defaults.md`,
  Corrections: "The app imports B only cannot build a cross-package call at all").
- Precedent that works around it: `tests/runtime/rt_top_level_initializer_globals.rs`
  `a_package_initializer_calling_another_package_sees_its_initialized_globals` lists BOTH packages
  in the app manifest.

## Failing Reproduction

Measured 2026-09-13, macOS aarch64, release binaries at main `caf191edd` (pre-plan-136) and at
plan-136-B Phase 3 (`6136b5648`). Three sibling projects:

`basepkg/project.json`: `kind: package`, no packages. `basepkg/src/lib.mfb`:

    EXPORT FUNC base() AS Integer
      RETURN 5
    END FUNC

`userpkg/project.json`: `kind: package`, `packages: [{"name":"basepkg","version":"=0.1.0","source":"file:../basepkg"}]`.
`userpkg/src/lib.mfb`:

    IMPORT basepkg

    EXPORT FUNC g() AS Integer
      RETURN basepkg::base()
    END FUNC

`app/project.json`: `kind: executable`, `entry: main`,
`packages: [{"name":"userpkg","version":"=0.1.0","source":"file:../userpkg"}]`. `app/src/main.mfb`:

    IMPORT userpkg
    IMPORT io

    FUNC main() AS Integer
      io::print(toString(userpkg::g()))
      RETURN 0
    END FUNC

```
mfb build app
```

- Observed: `error: NIR call target 'basepkg.base' does not resolve`, no executable.
- Expected: prints `5`, or a located diagnostic that `basepkg` must be declared (Open Decisions).

| Case | Result |
| --- | --- |
| app lists `userpkg` only; `userpkg::g()`'s body calls `basepkg::base()` | fails ✗ unlocated NIR error (main and plan-136-B) |
| app lists `userpkg` only; `userpkg::f(x = basepkg::base())` default (plan-136-B) | fails ✗ identically |
| app lists `userpkg` **and** `basepkg`; same sources | works ✓ prints `5` |

## Root Cause

**Confirmed: hypothesis 1.** `cli::build::build_project` hands lowering
`installed_package_files(&options.location, &manifest)` (the `packages_cache` in
`src/cli/build/mod.rs`), which resolves only the entries the importing `project.json` declares, and
`target::shared::nir::lower::merge_packages` merges exactly that list. Measured on the repro at
`3b94f621e`: `app/build/packages/` held only `userpkg.mfp`, and `mfb pkg info` on it showed an import
table entry `basepkg` (ident `basepkg`, used symbol `base`). Nothing merged `basepkg`, so
`userpkg`'s `basepkg.base` reference reached NIR undefined. Hypothesis 2 is eliminated: `basepkg` is
not in the merge list at all. The registry resolver has the same shape — `cli::resolve::resolve`
seeds nodes only from declared dependencies and silently drops an import edge naming an undeclared
ident.

The original hypotheses, for the record:

1. **The executable merge walks only the importer's declared packages.** `userpkg`'s build installs
   `basepkg.mfp` under `userpkg/build/packages/`, but the executable's package list — and so
   `target::shared::nir::lower::merge_packages` — holds only what the app's manifest declares, so
   `basepkg`'s functions are never merged and `userpkg`'s reference to `basepkg.base` has no
   definition. Confirm by reading how `cli::build` assembles the package paths it hands the merge
   (`grep -rn 'merge_packages' src`) and printing that list for the repro.
2. **The dependency is merged but its references are not identity-qualified.**
   `ir::package::apply_package_identity` is applied per merged package; if `basepkg` is merged under
   an identity `userpkg`'s IR never learns, the reference stays unqualified. Eliminated if (1) shows
   `basepkg` absent from the merge list.

Why the contrast case works: listing `basepkg` puts it in the merge list, and its identity rewrite
reaches every merged project's references.

## Goal

- The repro builds and prints `5`, or is refused at resolve with a located diagnostic naming
  `basepkg` — per the Open Decision.
- No program shape reaches `NIR call target … does not resolve` for a package-to-package reference.

### Non-goals (must NOT change)

- Programs that already list every package (the contrast case and every committed fixture).
- The `.mfp` format and package identities.
- **Tempting wrong fix (forbidden):** catching the NIR error and rewording it. The failure must be
  prevented (merged) or diagnosed before lowering, with a location.

## Blast Radius

Audited in Phase 1. Every shape below reaches the merge the same way — through the requirer's
import table, which the package build writes from ALL of its manifest's `packages[]`
(`manifest::package::package_dependencies` → `ImportTable::from_metadata`) — so one check on the
import tables covers all of them:

- a package function body calling another package — reproduced; `rt_package_dependency_closure`;
- a package parameter default calling another package (plan-136-B) — same edge;
  `rt_package_parameter_defaults` now declares the closure with `requiredBy`;
- a package top-level initializer reading another package's global — same edge;
  `rt_top_level_initializer_globals` declares the closure;
- a package re-exporting another package's type (bug-390) — the owner was only *read* as a sibling
  `.mfp` for its type definitions, never merged; it must now be declared
  (`rt_foreign_type_reexport`, `rt_reexport_union_transitive_field_types`).
- a version mismatch between what an importer was compiled against and the declared dependency —
  found while writing the tests: the conflict case reached compilation and failed with the internal
  `TYPE_CALL_ARITY_MISMATCH: Call to '<id>.basepkg.base' has 0 argument(s)`. Now
  `PACKAGE_VERSION_CONFLICT`.

## Fix Design

Decided with the user (see Decisions): **the manifest declares the whole closure; `mfb pkg` writes
it; `mfb build` checks it and never repairs.**

- Each `packages[]` entry carries `direct` (the user added it) and `requiredBy` (idents of the
  declared packages whose import tables name it). An entry is dropped only when `direct` is false
  and `requiredBy` is empty.
- `manifest::closure` reads what each declared package needs where the build reads it (installed
  `.mfp` import table, else the source directory's own `packages[]`, else the build cache):
  `check` lists every disagreement, `conflicts` lists used symbols whose ABI hash (or pinned
  version) the declared dependency does not provide, `reconcile` computes the manifest `pkg` writes.
- `verify_and_report_packages` → `refuse_inconsistent_closure`: `PACKAGE_DEPENDENCIES_INCONSISTENT`
  (6-605-0013, "run `mfb pkg update`") and `PACKAGE_VERSION_CONFLICT` (6-605-0014, "run
  `mfb pkg verify`"), located at the `packages` field, before anything is compiled.
- `apply_manifest_change` closes every `pkg add/remove/update`; `pkg update` does so for projects
  without registry dependencies too; `pkg verify` prints both diagnostics with per-symbol detail.

The originally forbidden wrong fix (rewording the NIR error) was not used: the undeclared dependency
is refused before lowering.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add a runtime test with the repro (app lists `userpkg` only); confirm it fails with the NIR
      error.
- [x] Confirm or eliminate hypotheses 1 and 2; complete the blast-radius audit.

Acceptance: the test fails for the documented reason; root cause cited to `file:symbol`.
Commit: 7621ed1f5

### Phase 2 — the fix

- [x] Implement the chosen option.
- [x] Migrate every committed `project.json` and every test harness that writes one.
- [x] Spec: tooling project-manifest, language modules-and-packages, architecture
      binary-representation / packages, tooling cli-reference.

Acceptance: the Phase 1 tests pass; every committed fixture builds.
Commit: 42d19e908, e182f3078, 4c86df537, e4e12e77e, 9e47748e8

### Phase 3 — regenerate expected outputs + full validation

- [x] `scripts/test-accept.sh` and `cargo test --release --no-fail-fast`.

Acceptance: full suite green; golden delta empty or justified.
Commit: — (no golden changed; results below)

## Validation Plan

- Regression test: the Phase 1 runtime test.
- Runtime proof: the repro prints `5` (or refuses with the located diagnostic).
- Doc sync: `mfb spec language modules-and-packages` states whether a package's dependencies must be
  declared by its importer.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`.

## Decisions

1. ~~Merge a package's own dependencies transitively vs. require the importer to declare them.~~
   Decided with the user (2026-09-16): this is a package-management rule, not a language one. If a
   project needs A and A needs B, B is pulled in **at `mfb pkg add` time** and written to
   `project.json`; every package used by the app is listed there. The build does nothing the
   manifest does not say.
2. Entries record why they are present: `"direct": bool` and `"requiredBy": [<idents>]` (idents, not
   names). Removal only when `direct` is false and `requiredBy` is empty.
3. `mfb build` validates `project.json`: incorrect `requiredBy`s (and missing dependencies) are listed
   and the build stops, telling the user to run `mfb pkg update`.
4. A dependency version conflict is an error, not an attempt to compile, telling the user to run
   `mfb pkg verify` for details.
5. All existing `project.json` files in the repo are updated to the new fields.

## Corrections

- **Effort** was estimated medium (1h–2h) against a fix that turned out to be a manifest rule, a
  build gate, `pkg` add/remove/update/verify changes, and a migration of 119 committed manifests and
  ~20 test harnesses.
- **Non-goal "every committed fixture builds identically"** could not hold as written: under
  decision 5 every fixture's `project.json` gained `direct`/`requiredBy`. No `.mfp`, IR, or native
  output changes (the fields are not in the import table, and `projectHash` excludes them).
- **The bug-390 spec sentence** "a consumer that installs the owning dependency (transitively — it
  need not be declared directly)" described a types-only sibling read; the owner's functions were
  never merged. Corrected in `architecture binary-representation`.

## Summary

An unlocated internal error for a natural manifest, pre-existing on main. The risk is in choosing
the rule and, if merging, de-duplicating a dependency reached through two paths.

## STATUS: FIXED (42d19e908)

Landed on `worktree-B-628` as 7621ed1f5 (RED tests), 42d19e908 (closure rule, build gate, `pkg`
wiring), e182f3078 (119 committed manifests), 4c86df537 (install what the requirer builds against;
undecodable payloads contribute nothing), e4e12e77e (test harnesses), 9e47748e8 (spec), 7494b93bc
(bug-653's harness, after merging main).

Validation on the tree merged with main `f0b889231`:

- `cargo test --release --no-fail-fast`: 193 test binaries `test result: ok`, 0 failed
  (`grep -c '^test result: ok'`, exit 0).
- `scripts/test-accept.sh`: `acceptance tests passed (1484 test(s) ran)`; no golden, `.mfp`, IR or
  native artifact changed (`git status` clean after the run).
- The repro: refused with `PACKAGE_DEPENDENCIES_INCONSISTENT` located at the `packages` field; after
  `mfb pkg update` it declares `basepkg` (`direct: false`, `requiredBy: ["userpkg"]`,
  `source: file:../basepkg`), builds, and prints `5`. Measured on macOS aarch64 only.

Deviation from the doc as drafted: neither option A nor B as first written — the manifest declares
the closure, `mfb pkg` writes it, `mfb build` checks it (see Decisions and Corrections). Known limit,
unchanged by this bug: `mfb pkg remove` accepts only registry idents, so a local dependency is removed
by editing `project.json` and running `mfb pkg update`.
