# bug-916: `mfb pkg verify` reports a working `file:<dir>` source dependency as `Invalid Package`, and exits 0 when a dependency is `Invalid Package` or `Needs Update`

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Footgun

Status: Open
Regression Test: tests/cli/pkg-verify-source-dir-dependency (new) — name suggested

Three related `pkg` hygiene defects, each measured:

1. A source-directory dependency declared as `"source": "file:../pA"` builds and
   runs (`mfb build` compiles it into `build/packages/pA.mfp`), but `mfb pkg verify`
   prints `pA @ 0.1.0 : Invalid Package` — it only looks at `packages/pA.mfp` and
   `packages/pA/project.json`, never the dependency's declared source directory or
   the build's package cache.
2. `mfb pkg verify` exits **0** while printing `Invalid Package` (case 1) or `Needs Update` (declared `9.9.9`, installed `0.1.0`). The
   CLI reference lists `pkg verify` exit statuses as "0 ok; 2 usage; 1 failed";
   only the closure/conflict diagnostics produce 1. A CI step `mfb pkg verify`
   therefore passes on a dependency the command itself calls invalid.
3. A failed `mfb pkg add file:///…/pB.mfp` (`error: package `pB` requires `pA`,
   which cannot be located from `pB``) still leaves `packages/pB.mfp` copied into
   the project while `project.json` is unchanged.

**Correct behavior:** (1) verify resolves a source-directory dependency where the
build does (its `source` directory / package cache) and reports OK; (2) any
non-OK status exits 1 (or the spec says otherwise); (3) a failed add leaves the
project untouched.

References:

- `src/docs/spec/architecture/03_packages.md` "Verify Status Model": compiled `.mfp` first, then `packages/<name>/project.json`, "If neither exists, the status is `InvalidPackage`."
- `./mfb spec tooling cli-reference` command table: `pkg verify … 0 ok; 2 usage; 1 failed`; "`pkg verify` Output" section.
- `src/docs/spec/architecture/07_flows.md` step 2: "Build source-directory dependencies into compiled package artifacts".

## Failing Reproduction

1. /tmp/hardening/B11/c3 (`mfb init c3`), `project.json` `packages`:
   `[{"name":"pA","version":"0.1.0","source":"file:../pA","direct":true,"requiredBy":[]}]`,
   `src/main.mfb` calls `pA::show(7)`.
   ```
   /tmp/hardening/mfb build -q . && ./build/c3.out   # Wrote package to ./build/packages/pA.mfp … int:7
   /tmp/hardening/mfb pkg verify; echo rc=$?
   ```
   - Observed: `pA @ 0.1.0 : Invalid Package` / `rc=0`
   - Expected: `pA @ 0.1.0 : OK (0.1.0)` / `rc=0`
2. /tmp/hardening/B11/c1 with `pB`'s entry edited to `"version":"9.9.9","pin":false`:
   - Observed: `pB @ 9.9.9 : Needs Update (0.1.0) [Unsigned]` / `rc=0`
   - Expected: `rc=1`
3. `mfb init c1; cd c1; mfb pkg add file:///tmp/hardening/B11/pB/pB.mfp` (pB depends on pA, not yet added):
   - Observed: error above, then `ls packages` → `pB.mfp`; `project.json` has no `packages`.
   - Expected: no `packages/pB.mfp`.

## Root Cause

1. `src/cli/pkg.rs:verify_package_dependency` probes only
   `packages/<name>.mfp` and `packages/<name>/project.json`; it ignores
   `dependency.source` and `crate::manifest::package::resolved_package_file`'s
   source-package cache that the build uses.
2. `src/cli/pkg.rs:verify_packages` prints each status line but returns `Err` only
   for `rotation_errors` and closure/conflict diagnostics; `InvalidPackage`/
   `NeedsUpdate` statuses never set a failure.
3. `add_package` copies the `.mfp` before the dependency-closure check fails and
   does not roll the copy back (hypothesis; observed effect only).

## Goal

- The three repros give OK/rc 0, rc 1, and an untouched project respectively.

## Blast Radius

- `pkg verify --proof` shares `verify_packages` — same exit behavior.
