# bug-628: a call into a package's own dependency fails to build with an unlocated `NIR call target … does not resolve` when the importer does not list that dependency

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: Open
Regression Test: none yet — see Phase 1

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

Not yet localized. Hypotheses, ordered by likelihood:

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

To audit in Phase 1 (search, not memory):

- a package function body calling another package — reproduced;
- a package parameter default calling another package (plan-136-B) — reproduced;
- a package top-level initializer reading another package's global
  (`rt_top_level_initializer_globals` avoids it by listing both);
- a package re-exporting another package's type in its public API (bug-390 path) — unknown.

## Fix Design

Depends on the Open Decision. (A) Merge transitive dependencies: collect each merged package's own
installed dependencies recursively, de-duplicated by identity, before the merge. (B) Refuse: after
reading an imported package's import table, report any dependency the importer does not declare,
located at the importer's `IMPORT` line.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a runtime test with the repro (app lists `userpkg` only); confirm it fails with the NIR
      error.
- [ ] Confirm or eliminate hypotheses 1 and 2; complete the blast-radius audit.

Acceptance: the test fails for the documented reason; root cause cited to `file:symbol`.
Commit: —

### Phase 2 — the fix

- [ ] Implement the chosen option.

Acceptance: the Phase 1 test passes; every committed fixture still builds identically.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/test-accept.sh` and `cargo test --release --no-fail-fast`.

Acceptance: full suite green; golden delta empty or justified.
Commit: —

## Validation Plan

- Regression test: the Phase 1 runtime test.
- Runtime proof: the repro prints `5` (or refuses with the located diagnostic).
- Doc sync: `mfb spec language modules-and-packages` states whether a package's dependencies must be
  declared by its importer.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

1. Merge a package's own dependencies transitively (the importer never lists them) vs. require the
   importer to declare them and refuse with a located diagnostic. Recommended: merge transitively —
   a package's dependencies are its implementation, and the importer already trusts that package.
   Decisions: do no work until we talk about this. the language as a whole does not merge dependencies transitively at the moment.

## Summary

An unlocated internal error for a natural manifest, pre-existing on main. The risk is in choosing
the rule and, if merging, de-duplicating a dependency reached through two paths.
