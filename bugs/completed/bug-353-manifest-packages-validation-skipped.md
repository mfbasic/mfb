# bug-353: `mfb build` never validates the `packages` shape — a non-array silently erases every dependency; dependency names skip the bug-195 traversal guard

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (silent config loss) / Footgun (misdirecting diagnostic) / Security-relevant (write-side name-validation gap)

Status: Fixed (2026-07-22)
Regression Test: tests/ (new) — `mfb build` on a project whose `packages` is not an
array must report `PROJECT_JSON_*`, not exit 0; and a package dependency named
`../x` must be rejected at build time

Two independent defects in the same manifest surface, both stemming from
validation that exists but is not wired into the build path.

**A — `packages` shape is never checked on `mfb build`.** `validate_packages_array`
is called only from the `pkg` subcommands. `validate_project_manifest` — the sole
manifest gate for `build`, `test`, `fmt`, `doc`, and `audit` — does not call it. So
a `packages` key that is not an array is silently ignored, `installed_package_files`
returns an empty vec, and **every dependency vanishes with no diagnostic**. If the
program does not import the dropped dependency, the build succeeds against a
silently different dependency set. If it does import it, the build fails with a
diagnostic that actively misdirects: *"Package `greet` is not built in and is not
declared in project.json packages"* — while `greet` is plainly declared six lines
below in the same file. The trust line (`uses greet - [Unsigned]`) also silently
disappears. `src/docs/spec/tooling/01_project-manifest.md:125` states the rule
unconditionally, so the spec is wrong for five of the six commands that read a
manifest.

**B — package dependency names skip the bug-195 guard.** `package_dependencies`
applies neither the blank-name check nor `validate_package_name`, while its sibling
`project_package_dependency` applies both. A dependency named `../../etc/passwd`
therefore travels verbatim into `.mfp` metadata and is interned into the import
table's string pool. This is a **write-side** hole: the produced container violates
a constraint the format spec says is enforced at build time. No read-side traversal
was demonstrated (§Blast Radius), which is why this is MEDIUM and not HIGH.

The single correct behavior a fix produces: `mfb build` rejects a non-array
`packages` with the same diagnostic `mfb pkg update` already gives, and a package
dependency whose name fails `validate_package_name` is rejected at build time
rather than written into the `.mfp`.

References:

- `src/docs/spec/tooling/01_project-manifest.md:125` — states the `packages`-array
  rule as an unconditional manifest constraint.
- `src/docs/spec/package/01_container-format.md:258` — claims the name constraint is
  "enforced both when a package is built and when its header is read, so a crafted
  `name` such as `../../x` cannot escape the project directory." True for the
  package's own name; **false for its dependency names**.
- `bugs/completed-bugs/bug-195-*` — added the `validate_package_name` traversal
  guard that item B bypasses.
- Found during the cleanup review of the manifest surface.

## Failing Reproduction

### A — non-array `packages` on the build path

```
$ cat project.json
{"name":"x","version":"0.1.0","mfb":"1.0","kind":"executable",
 "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
 "entry":"main","targets":["native"],
 "packages":{}}
$ mfb build .
Building x (executable) for macos-aarch64
Wrote executable to ./build/x.out
EXIT=0
```

- Observed: exit 0, zero diagnostics. Identical for `"packages": "oops"`,
  `"packages": 42`, and `"packages": null`. `mfb audit .` also passes (`errors: 0`).
- Expected: the same rejection the `pkg` path already produces.

Contrast — the *same* project directory, different subcommand:

```
$ mfb pkg update
error: project.json field `packages` must be an array when present
EXIT=1
$ mfb pkg verify
error: project.json field `packages` must be an array when present
EXIT=1
```

The misdirecting-diagnostic case. With a real installed `greet` package and a
correct `packages` array, the program builds and runs (exit 42). Changing **only**
the array into an object keyed by name, leaving the dependency record byte-identical
inside:

```
Building x (executable) for macos-aarch64
   1 | IMPORT greet
     | ^
./src/main.mfb:1 error[2-201-0002 IMPORT_PACKAGE_NOT_DECLARED]: imported package is not declared
               Package `greet` is not built in and is not declared in project.json packages.
EXIT=1
```

- Observed: an error asserting the package is not declared, when it is declared in
  the file the error names.
- Expected: `packages` must be an array when present.

### B — traversal name reaches the `.mfp`

A `kind: "package"` project declaring
`{"name":"../../etc/passwd","ident":"evil","version":"9.9.9"}`:

```
$ mfb build .
Building victim (package) for macos-aarch64
Wrote package to ./victim.mfp
EXIT=0
$ # raw byte search of the produced container
b'../../etc/passwd' @ offset 478;  b'evil' @ 498;  b'9.9.9' @ 506
```

- Observed: exit 0, no warning, the traversal name interned verbatim in the `.mfp`.
- Expected: rejected by `validate_package_name`, as the sibling path does.

| Environment | Command | Result |
| --- | --- | --- |
| any | `mfb build` / `test` / `fmt` / `doc` / `audit` | no `packages` validation ✗ |
| any | `mfb pkg update` / `install` / `verify` | validated ✓ |

## Root Cause

**A.** `src/manifest/mod.rs:945-953` (`validate_packages_array`) is correct and does
exactly what the spec describes. `src/manifest/mod.rs:25-124`
(`validate_project_manifest`) calls `validate_required_string` (name/version/mfb),
`validate_sources`, `validate_optional_string` ×4, `validate_kind`, `validate_mode`,
`validate_resources`, and `validate_libraries` — **and never
`validate_packages_array`**.

Exhaustive call-site map for `validate_packages_array` (4 non-test callers, all in
`src/cli/`):

| Site | Reached by |
| --- | --- |
| `src/cli/resolve.rs:516` (`read_manifest`) | `pkg update`, `pkg install` |
| `src/cli/pkg.rs:561` | `pkg` path |
| `src/cli/pkg.rs:625` | `pkg` path |
| `src/cli/pkg.rs:982` | `pkg verify` |

Non-test `validate_project_manifest` callers: `src/cli/build.rs:247`,
`src/cli/fmt.rs:98`, `src/cli/doc.rs:76`, `src/cli/pkg.rs:123,349,396`,
`src/audit/mod.rs:81`, `src/testutil.rs:102`. `src/cli/build.rs:247` is the build
path's only manifest gate.

The silent-erasure mechanism is `src/manifest/package.rs:296-301`
(`installed_package_files`), which returns `Ok(Vec::new())` when `packages` is not
an array — an empty dependency set is indistinguishable from "no dependencies
declared", so nothing downstream can tell the difference. The misdirecting
`IMPORT_PACKAGE_NOT_DECLARED` message is then literally accurate about the
*resolved* set and badly wrong about the *file*.

**B.** `src/manifest/package.rs:457-492` (`package_dependencies`) extracts
`name`/`ident`/`version`/`pin` and builds a `BinaryReprDependency` with no blank
check and no `validate_package_name`. Its sibling
`src/manifest/package.rs:494-545` (`project_package_dependency`) has the blank check
at `:524-526` and the bug-195 guard at `:533-535`.

Nothing upstream compensates. `src/cli/build.rs:630` calls
`installed_package_files` *before* `package_metadata` at `:648` — but
`installed_package_files` filters through `project_package_dependency`, which returns
`None` for `../x` and merely **skips** it rather than erroring. The build proceeds,
and `package_metadata` (`src/manifest/package.rs:420`, assigning
`metadata.dependencies` at `:453`) re-reads the same raw manifest through the
unvalidated `package_dependencies`. `validate_metadata`
(`src/target/package_mfp/mod.rs:212-222`) applies `validate_package_name` only to
`metadata.name`, the package's own name, at `:216` — never to
`metadata.dependencies[].name`. `ImportTable::from_metadata`
(`src/binary_repr/sections.rs:547-560`) then interns `dependency.name` verbatim.

So the skip-don't-error behavior of the *validated* path is precisely what lets the
*unvalidated* path publish the name.

## Goal

- `mfb build` (and `test`/`fmt`/`doc`/`audit`) rejects a non-array `packages` with
  the same diagnostic the `pkg` commands emit.
- A `packages` entry whose name fails `validate_package_name` is rejected at build
  time, and never reaches `.mfp` metadata or the import-table string pool.
- The spec statements at `01_project-manifest.md:125` and
  `01_container-format.md:258` become true.

### Non-goals (must NOT change)

- The documented tolerance for *entries* that are skipped: "an entry whose `name` is
  absent, non-string, or blank-after-trim is silently skipped"
  (`01_project-manifest.md:125`) is the intended behavior for well-formed arrays and
  must be preserved. This bug is about the **container** shape and about
  **traversal-invalid** names, not about tightening entry skipping.
- `validate_packages_array`'s own logic — it is correct; it is simply not called.
- The `pkg` subcommands, which already behave correctly.
- Valid manifests must be unaffected: a correct `packages` array must build
  byte-identically, with no new diagnostics.
- Do NOT fix A by making `installed_package_files` error — it is called from paths
  that legitimately tolerate a missing key. Fix it at the validation gate.

## Blast Radius

- `src/manifest/mod.rs:25-124` (`validate_project_manifest`) — missing the call;
  fixed by this bug. Fixing here covers all five affected commands at once.
- `src/manifest/package.rs:457-492` (`package_dependencies`) — missing both guards;
  fixed by this bug.
- `src/manifest/package.rs:494-545` (`project_package_dependency`) — already correct;
  it is the template. Unaffected.
- `src/manifest/package.rs:296-301` (`installed_package_files`) — the silent-empty
  return that makes A invisible. Not changed (see Non-goals), but it is why the
  failure is silent.
- `src/cli/build.rs:247` (gate), `:630`, `:648` — call sites; behavior changes via
  the gate, no edit needed.
- `src/target/package_mfp/mod.rs:212-222` (`validate_metadata`) — validates only
  `metadata.name`. **Latent, same hazard, in scope as defense-in-depth**: adding the
  dependency-name check here gives a second gate at the container boundary.
- `src/binary_repr/sections.rs:547-560` (`ImportTable::from_metadata`) — interns the
  name verbatim; unaffected once the producer is fixed.
- `src/cli/fmt.rs:98`, `src/cli/doc.rs:76`, `src/audit/mod.rs:81`,
  `src/testutil.rs:102` — inherit the fix through the shared gate. `testutil.rs`
  means **existing fixtures with malformed `packages` would begin failing**; sweep
  before landing.

**Read-side reach — deliberately bounded.** I traced the consumers of an interned
`.mfp` dependency name and found no path where this compiler feeds one back into a
filesystem path: `installed_package_files` builds `packages/<name>.mfp` from the
*manifest*, not from a consumed container's import table. So B is a producer-side
spec violation, not a demonstrated traversal. It still matters, because
`01_container-format.md:258` tells third-party and future consumers that the
invariant is enforced at build time — and they may rely on it. Severity MEDIUM, not
HIGH, on that basis; if a read-side consumer is ever added that trusts the
documented invariant, this becomes HIGH.

## Fix Design

**A.** Add `validate_packages_array(&manifest)` to `validate_project_manifest`,
alongside the existing `validate_resources` / `validate_libraries` calls. One edit
fixes all five commands and makes the spec true, with no per-command plumbing.

**B.** Give `package_dependencies` the same blank check and `validate_package_name`
call `project_package_dependency` has. Decide explicitly between skip and error
(§Open Decisions) — but do not silently skip, since silent skipping in the sibling
is what let this reach the container. Additionally extend `validate_metadata`
(`src/target/package_mfp/mod.rs:212-222`) to check `metadata.dependencies[].name`,
so the container boundary is guarded independently of the manifest reader.

Rejected alternatives:

- **Call `validate_packages_array` from `src/cli/build.rs:247` only.** Leaves
  `test`/`fmt`/`doc`/`audit` broken and leaves the spec false for four commands.
- **Make `installed_package_files` error on a non-array.** Rejected under Non-goals;
  wrong layer, and it is called where a missing key is legitimate.
- **Have `package_dependencies` delegate to `project_package_dependency`.** Tempting
  — they are near-parallel — but they build different types for different consumers;
  unifying them is a refactor that should not ride along with a validation fix.

Expected output shift: none for valid manifests. Invalid manifests that previously
built now fail — which is the point, and which is why the fixture sweep is a
required phase, not an afterthought.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [x] Test: `mfb build` on a project with `"packages": {}` (and `"oops"`, `42`,
      `null`) must fail. Confirmed all four exit 0 on the pre-fix build.
- [x] Test: `mfb build` on a `kind: "package"` project with a dependency named
      `../../etc/passwd` must fail. Confirmed the `.mfp` contained
      `../../etc/passwd` verbatim on the pre-fix build (raw byte search).
- [x] Sweep every fixture and test project for a malformed `packages` value that
      the fix would newly reject; record the list here.

**Fixture sweep — clean, both defects.** Scanned all 1118 `project.json` files:
- Non-array `packages`: **zero** — no fixture carries a non-array `packages`, so
  defect A's gate breaks no existing build.
- Dependency names: **93** across fixtures, all matching
  `^[A-Za-z0-9_][A-Za-z0-9_.-]*$` (checked with the real `validate_package_name`
  regex) — **zero** newly rejected by defect B's gate.

Acceptance: met — reproductions recorded, sweep closes clean.
Commit: —

### Phase 2 — the fixes

- [x] Call `validate_packages_array` from `validate_project_manifest` (via a new
      `validate_packages` helper that emits a `PROJECT_JSON_FIELD_TYPE` diagnostic
      with the field's line/column, consistent with the sibling validators).
- [x] Extend `validate_metadata` to check `metadata.dependencies[].name` (blank
      check via `validate_string(..., required=true)` + `validate_package_name`).
- [~] **Deviation from the doc, with reasoning:** I did NOT add the guard to
      `package_dependencies`. That reader returns a plain `Vec` with no error
      channel, so the only two behaviors available there are *keep* or *silently
      skip* — and silent skipping is exactly the disease (a dropped dependency and
      a later misdirecting `IMPORT_PACKAGE_NOT_DECLARED`) this bug is about; the
      doc's own Non-goals forbid it. Leaving `package_dependencies` to keep the
      raw name lets it reach `validate_metadata`, which is the erroring
      container-boundary gate the doc itself recommends. So the name is rejected
      *with a build error* rather than dropped — the Open-Decision "error, not
      skip" outcome — and the fix lands at one authoritative gate instead of two
      half-gates.

Acceptance: met — Phase 1 tests pass; a valid `packages` array and an absent key
both still validate; artifact-gate shows zero codegen churn; `pkg` and
`package_mfp` suites unchanged.
Commit: —

### Phase 3 — doc sync + full validation

- [x] Re-read both spec statements. `01_project-manifest.md` ("`packages` must be
      an array when present (`validate_packages_array`)") is now true for all five
      commands, not just `pkg`; no wording change needed. `01_container-format.md`
      spoke only of the package's own `name`; added a sentence documenting that
      each **dependency** `name` carries the same single-path-component constraint,
      enforced at build (`validate_metadata`) — a now-guaranteed invariant.
- [x] No fixtures flagged by the sweep, so none to fix.
- [x] Validation: `manifest::` (154), `package_mfp` (8), `spec` (45) suites green;
      `artifact-gate.sh` → `0 diff(s)` (validation-only, no codegen impact).

Acceptance: green; spec statements true; no valid-manifest churn.
Commit: —

## Validation Plan

- Regression test(s): the four non-array `packages` shapes against `mfb build`, and
  the `../../etc/passwd` dependency-name case asserting build failure **and** that
  the string is absent from any produced `.mfp`.
- Runtime proof: the `greet` end-to-end case — with a correct array the program runs
  (exit 42); with the array turned into an object the build now reports the
  `packages`-must-be-an-array error instead of the misdirecting
  `IMPORT_PACKAGE_NOT_DECLARED`.
- Doc sync: `src/docs/spec/tooling/01_project-manifest.md:125` and
  `src/docs/spec/package/01_container-format.md:258` — both become true rather than
  aspirational; re-read after the fix and adjust wording if scope differs.
- Full suite: `tests/test-accept.sh`, plus the `pkg` suite to confirm those commands
  are unchanged.

## Open Decisions

- On a traversal-invalid dependency name: **error** (recommended — silent skipping in
  `project_package_dependency` is exactly what let the name reach the container) vs.
  skip-with-warning (consistent with the documented entry-skipping tolerance, but
  perpetuates the pattern that caused this).
- Whether the `validate_metadata` dependency-name check lands here as
  defense-in-depth (recommended) or as a follow-up.

## Summary

`validate_packages_array` exists, is correct, and is simply not wired into
`validate_project_manifest` — so five commands including `build` accept a malformed
`packages` key and silently discard every dependency, surfacing either no error or
one that contradicts the file it is reading. Separately, `package_dependencies` omits
the blank check and the bug-195 traversal guard its sibling applies, letting
`../../etc/passwd` be written verbatim into a `.mfp` and contradicting the container
spec's explicit enforcement claim. Both fixes are small and land at the validation
gates; the real risk is Phase 1's fixture sweep, since projects that build today may
legitimately start failing, and that must be enumerated before the gate tightens.
