# bug-30: Two LOW cli robustness nits — `package_dependency_status` `pin` branch is dead, and `compare_versions` silently maps unparseable components to 0

Last updated: 2026-07-08
Effort: small (<1h)

Two small, independent LOW-severity defects in the `mfb` CLI package tooling.
Batched (both cli, both LOW, both trivial); distinct root causes noted separately.

**(1) `package_dependency_status` has an identical-branch `if/else` (dead code).**
`src/cli/pkg.rs:1382-1387`:
```
if dependency.pin {
    package_version_status(&dependency.version, actual_version)
} else {
    package_version_status(&dependency.version, actual_version)
}
```
Both arms are byte-identical, so `dependency.pin` has **no effect** here. Compounding
the smell, `package_version_matches` (`pkg.rs:1397`) returns `true` when `expected`
is empty (`expected.is_empty() || expected == actual`), so a pinned dependency with
an empty version string reports `Ok` rather than demanding an exact match. (Real pin
enforcement lives in `installed_package_files`, `src/manifest/package.rs:287`, which
does compare `header.version == dependency.version` — so this is **not** a security
hole, just misleading dead code in the `mfb pkg verify` output path.)

**(2) `compare_versions` coerces non-numeric version components to 0 (footgun).**
`src/cli/resolve.rs:445-457`: `part.parse::<u64>().unwrap_or(0)` turns any
non-numeric (or `> u64`) component into 0. So `"2.0.0"` and `"2x.0.0"` compare as if
the second were `0.0.0`, and a malformed registry version can mis-order during
resolution with no signal.

The single correct behavior a fix produces: `pin` either affects the status check or
is removed from the branch; and `compare_versions` handles a non-numeric component
deterministically (explicit ordering or rejection) rather than silently coercing to
0.

Severity LOW for both.

References:

- `src/cli/pkg.rs:1382-1387` (identical `if/else`), `:1397` (`package_version_matches`
  empty-matches-anything). Real pin gate: `src/manifest/package.rs:287`.
- `src/cli/resolve.rs:445-457` (`compare_versions`, `unwrap_or(0)` at `:453`).
- Found during goal-01 review of `src/cli/**`.

## Failing Reproduction

(1) A pinned dependency with an empty `version` in the manifest → `mfb pkg verify`
reports `Ok` regardless of the installed version (the branch is dead and empty
`expected` matches anything).
(2) Compare registry versions `"2.0.0"` vs `"2x.0.0"` → the latter sorts as
`0.0.0`, mis-ordering resolution.

- Observed: (1) dead branch, no pin effect; (2) malformed version silently treated
  as `0.0.0`.
- Expected: (1) `pin` differentiates behavior or is removed; (2) a non-numeric
  component is not silently 0.

Contrast: well-formed dotted numeric versions compare correctly (including the
`-prerelease` ranking below `:457`); `installed_package_files` enforces pins.

## Root Cause

(1) Leftover `if dependency.pin` scaffold never differentiated.
(2) Lossy `unwrap_or(0)` with no signal that a component was non-numeric.

## Goal

- `package_dependency_status` has no misleading identical-branch `if/else` (and, if
  `pin` should matter here, it demands an exact non-empty version match).
- `compare_versions` treats a non-numeric component deterministically.

### Non-goals (must NOT change)

- The real pin enforcement in `installed_package_files`.
- Ordering of well-formed numeric versions and `-prerelease` ranking.

## Blast Radius

- `pkg.rs:1382-1387` (+ `package_version_matches` empty-string semantics).
- `resolve.rs:445-457`.

## Fix Design

(1) Delete the redundant branch (call `package_version_status` unconditionally), or —
if `pin` is meant to matter — make the `pin` arm require a non-empty exact match.
(2) Treat a non-numeric component as an explicit ordering rule (lexical compare, or a
resolution error), not `0`; or validate version strings numeric at the source and
document that guarantee.

## Phases

### Phase 1 — failing test + audit

- [ ] (1) Test that an empty-version pinned dep is not silently `Ok` (if pin should
      matter), or assert the branch simplification is behavior-preserving.
- [ ] (2) `compare_versions("2.0.0", "2x.0.0")` test asserting the chosen
      deterministic behavior.
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [ ] Simplify/repair the `pin` branch; make `compare_versions` non-lossy.

### Phase 3 — validation

- [ ] `scripts/test-accept.sh`; resolution/verify goldens unchanged for well-formed
      inputs.

## Validation Plan

- Regression test(s): the two tests above.
- Full suite: `scripts/test-accept.sh`.

## Summary

A dead `pin` branch in `mfb pkg verify` and a lossy version-component parse in the
resolver; both fixes are local and preserve well-formed-input behavior.
