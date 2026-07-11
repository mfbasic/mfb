# bug-58: `write_package` builds the `.mfp` output path from an unsanitized `metadata.name`, allowing path traversal outside the project directory

Last updated: 2026-07-09
Effort: small (<1h)

`write_package` computes the package output path as
`project_dir.join(format!("{}.mfp", metadata.name))`, where `metadata.name` comes from
`project.json` and is validated only for non-emptiness and a 255-byte length cap — never
for path separators or `..` components. A `name` such as `"../../tmp/evil"` makes
`write_package` write `<project_dir>/../../tmp/evil.mfp`, escaping the project directory
and overwriting an arbitrary `*.mfp`-suffixed path with the builder's privileges.

Severity is LOW because `name` is normally the developer's own project field, so
exploitation requires building a project whose `project.json` is attacker-supplied
(e.g. an untrusted checkout). It is the same class as bug-27's package-name traversal, at
the package-write site. The single correct behavior a fix produces: a package name
containing a path separator or `..` is rejected before it is used to build a filesystem
path.

References:

- `src/target/package_mfp/mod.rs:write_package` (`:55`):
  `project_dir.join(format!("{}.mfp", metadata.name))`.
- `src/target/package_mfp/mod.rs:validate_metadata` (`:164`) →
  `validate_string("name", &metadata.name, NAME_LIMIT, true)` (`:165`).
- `src/target/package_mfp/mod.rs:validate_string` (`:181-192`): checks only
  `is_empty()` and `len() > limit` — no charset / path-component check.
- Manifest layer (`src/manifest/mod.rs`) likewise constrains `name` only as a non-empty
  string.
- Same class: bug-27 (pkg-install path traversal / pre-verify symlink write) — HIGH there
  because the name is network-sourced; here it is developer-sourced, hence LOW.
- Found during the goal-01 compiler source review of `src/target/package_mfp/`.

## Failing Reproduction

Set `"name": "../../tmp/evil"` in a project's `project.json`, then build the package.

- Observed: `write_package` writes `<project_dir>/../../tmp/evil.mfp`, outside the
  project directory, clobbering any existing file at that resolved path.
- Expected: the build rejects the name with an invalid-name error before writing.

Contrast: an ordinary `name` like `"shape"` writes `<project_dir>/shape.mfp` correctly;
the container's internal bytes are length-prefixed and tamper-evident (tests at
`mod.rs:326-359`), so only the *output filename* is the vulnerable surface.

## Root Cause

`validate_string` enforces non-empty + a byte-length cap but performs no path-safety
check, and `write_package` interpolates `name` directly into a `Path::join`. `join` with
a value containing `/` or `..` traverses out of `project_dir`.

## Goal

- A package `name` containing a path separator (`/`, `\`) or a `..` component is rejected
  at validation, before any path is built from it.
- Ordinary names still write `<project_dir>/<name>.mfp`.

### Non-goals (must NOT change)

- The container byte format / signature welding.
- Legitimate names (any name that is a single safe path component).

## Blast Radius

- `write_package` (`package_mfp/mod.rs:55`) — the sink.
- `validate_metadata` / `validate_string` — where the check belongs.
- Any other consumer that builds a path from `metadata.name` — grep; the `.mfp` write is
  the known sink.
- Cross-reference bug-27's name-sanitization fix so both use the same rule.

## Fix Design

In `validate_metadata` (or `validate_string` for the `name` field), reject a `name` that
is not a single safe path component — e.g. `Path::new(name).file_name() == Some(name)`
and no separator/`..`/leading-dot-dot — before it reaches `write_package`. Reuse bug-27's
sanitizer if it lands first.

## Phases

### Phase 1 — failing test

- [x] Add a build test with a traversing `name` asserting an invalid-name error and that
      no file is written outside the project dir. Confirm it writes outside today.

### Phase 2 — the fix

- [x] Add the path-component validation for `name`; share bug-27's helper if available.

### Phase 3 — validation

- [x] `scripts/test-accept.sh`; confirm ordinary package builds are unchanged.

## Validation Plan

- Regression test(s): the traversing-name rejection test.
- Runtime proof: build with `name = "../x"` → rejected, nothing written outside the dir.
- Doc sync: document the package-name constraint if not already stated.
- Full suite: `scripts/test-accept.sh`.

## Summary

A package name flows unsanitized into the `.mfp` output path, so a hostile `project.json`
can write outside the project directory. The fix rejects non-single-component names at
validation, matching bug-27's class. LOW because the name is developer-sourced in the
normal flow.

## Resolution

Fixed in `src/target/package_mfp/mod.rs`.

The build-side name guard was already present: bug-27 (commit `59f79e64`, an ancestor of
HEAD) added `crate::manifest::package::validate_package_name(&metadata.name)` into
`validate_metadata`, which `build_package_bytes` calls. `write_package` invokes
`build_package_bytes`, so a traversing name was already rejected before `fs::write`.

This change hardens the sink itself and adds the missing regression test:

- **Fail-fast at the sink.** `write_package` now calls `validate_metadata(metadata)?` as
  its first statement, *before* `build_package_binary_repr_bytes` (any lowering work) and
  before `project_dir.join(format!("{}.mfp", metadata.name))` (any path construction). The
  name is now proven to be a single safe path component before it is ever interpolated
  into a filesystem path. `build_package_bytes` still re-validates; the top-level call is a
  no-op for legitimate names, so output bytes are byte-identical and no goldens shift.
- **Regression test** `write_package_rejects_a_traversing_name_and_writes_nothing_outside_the_dir`
  drives `write_package` with `metadata.name = "../../evil"` and asserts (a) an
  invalid-name error containing "not a valid path component" and (b) that nothing was
  written to the resolved traversal target outside the project directory. The project dir
  is nested two levels down so the naive sink `project_dir/../../evil.mfp` resolves back
  inside the auto-cleaned tempdir rather than polluting the shared temp root.

Reuses bug-27's `validate_package_name` (charset `[A-Za-z0-9_][A-Za-z0-9_.-]*`) — no second
validator, no new diagnostic rule (the helper returns a plain `Result<(), String>`), so no
`errorCode::` Constant Registry / spec change was required.

Proof the test detects the vulnerability: temporarily reverting `write_package` to the
pre-fix shape (build path from `name`, then `fs::write` with no validation) makes the new
test FAIL — `write_package` returns `Ok` and writes to `.../project/../../evil.mfp` outside
the project dir. Restoring the fix makes all 7 `package_mfp` unit tests pass.

Validation: `cargo test --bin mfb package_mfp` → 7 passed / 0 failed.
