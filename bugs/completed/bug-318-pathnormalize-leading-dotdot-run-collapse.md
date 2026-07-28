# bug-318: `fs::pathNormalize` collapses a run of leading `..` components — `"../../a"` normalizes to `"a"`

Last updated: 2026-07-18
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (path normalization; security-relevant)

Status: Fixed 2026-07-18
Regression Test: `tests/rt-behavior/fs/func_fs_pathNormalize_valid` — leading `..`
runs are preserved

## Resolution

`previous_ready` left `scratch13 == 0` for two different situations: the scan found
the root `/` at index 0, or it ran off the front without finding any slash. The
second is the leading-component case, and it fell straight into `pop_previous`,
skipping the `..`-detection that only ran when a real slash preceded the component.

A `leading_component` block now distinguishes them on `content[0]`: `/` routes to
`pop_previous` unchanged (bug-132's keep-the-root-slash behavior), and otherwise the
same two-dot test runs with `prev_start = 0`, `prev_len = out_len`. The existing
slash-preceded block gained an explicit `branch(&pop_previous)` since it no longer
falls through.

Verified by building both ways:

| input | pre-fix | post-fix |
|---|---|---|
| `../..` | `.` | `../..` |
| `../../a` | `a` | `../../a` |
| `../../..` | `..` | `../../..` |
| `../a/..` | `..` | `..` |

The first eight fixture cases (bug-79 / bug-132 coverage) are byte-identical either
way, and the regenerated golden diff is exactly the four added lines — the fix
changes no other fixture's output. Full acceptance green.

One correction to this report: it predicted `"../../.."` → `"."`, but the measured
pre-fix result was `".."`. Wrong either way; the stated cause and fix were right.

Its other claim — that the checked-in golden is stale at 4 lines for 8 `io::print`
calls, and that this masked the bug — is **not correct**, and no golden was
"fixed". The 4-line file is `func_fs_pathNormalize_valid.run`, which is a *trigger*
telling the harness to execute the fixture; its contents are never compared. The
real golden is `build.log`, which had all 8 lines and was accurate. The bug was
untested simply because no input exercised a leading `..` run.

`fs::pathNormalize` deletes a leading `..` when it is immediately followed by
another `..`: `"../.."` returns `"."` (expected `"../.."`), `"../../a"` returns
`"a"` (expected `"../../a"`), `"../../.."` returns `"."`. This silently strips
leading parent-directory traversal, so a caller using `pathNormalize` to decide
"does this path escape the root?" is defeated — a path that escapes two-or-more
levels normalizes to an in-root-looking path. It also contradicts the documented
spec, which states a leading `..` (or a run of them) has no earlier component to
cancel and each is kept in place.

The single correct behavior a fix produces: a leading run of `..` components is
preserved verbatim — `"../../a"` → `"../../a"`, `"../.."` → `"../.."` — matching
the man page.

References:

- `src/docs/man/builtins/fs/pathNormalize.md:34-35` ("a leading `..` (or a run of
  them) … each such `..` is kept in place").
- `bugs/completed-bugs/bug-79-followup-low-cluster.md` (fixed `"a/.."`/`"a/../.."`,
  leading *normal* component), bug-132 (absolute-root pop), bug-65 (alloc size).
- Found during goal-06 review of `src/target/shared/code/builder_fs_paths.rs`.

## Failing Reproduction

```
io::print(fs::pathNormalize("../.."))      ' -> "."   (expected "../..")
io::print(fs::pathNormalize("../../a"))    ' -> "a"    (expected "../../a")
io::print(fs::pathNormalize("../../.."))   ' -> "."    (expected "../../..")
```

- Observed: leading `..` runs collapse.
- Expected: leading `..` runs are preserved.

Contrast (correct today): `"../a/.."` → `".."` (the normal component `a` pops via a
real slash); `"a/.."` → `"."`; `"/a/.."` → `"/"`.

## Root Cause

`src/target/shared/code/builder_fs_paths.rs:530-533` (`lower_fs_path_normalize`,
`previous_ready` block): when a `..` is processed and the previous component is the
leading one (starting at offset 0), `previous_scan` drives `scratch13` to 0. At
`previous_ready` the code compares `scratch13 == 0` and unconditionally branches to
`pop_previous`, **bypassing the `..`-detection at lines 534-544** (which only runs
when `scratch13 != 0`). So a leading `..` is never recognized as un-poppable:
`pop_previous`/`pop_scan` find no preceding `/`, reach `pop_store` with `scratch13
== 0`, and truncate `out_len` to 0 — deleting the leading `..`.

The valid fixture only tests inputs starting with a poppable component or root, so
the broken case is untested; the checked-in golden also appears stale (4 output
lines for 8 `io::print` calls), which is a test-masking issue to fix alongside.

## Goal

- A leading run of `..` components is preserved by `pathNormalize` on every backend.

### Non-goals (must NOT change)

- The correct cases (`"a/.."` → `"."`, `"/a/.."` → `"/"`, `"../a/.."` → `".."`).
- Do NOT "fix" this by editing the golden without extending the fixture to actually
  exercise the leading-`..`-run inputs.

## Blast Radius

- `lower_fs_path_normalize` `previous_ready` block — fixed here (shared across all
  backends, so one fix covers aarch64/x86/riscv).
- The stale golden `func_fs_pathNormalize_valid.run` — regenerate after extending
  the fixture.

## Fix Design

At `previous_ready`, when `scratch13 == 0`, treat the leading component as
`[0..out_len)` and run the same `..`-check before falling into `pop_previous`: if
`out_len == 2` and `content[0] == '.'` and `content[1] == '.'`, branch to
`append_dot_dot`; otherwise pop. Symmetric to the existing `scratch14 = scratch13 +
1` / len-==-2 / two-dot check at lines 534-544, with `prev_start = 0`,
`prev_len = out_len`.

## Phases

### Phase 1 — failing test
- [ ] Extend `func_fs_pathNormalize_valid` with `"../.."`, `"../../a"`,
      `"../../.."`; confirm they collapse today; regenerate the stale golden so it
      has the right line count.
### Phase 2 — the fix
- [ ] Add the leading-`..` check at `previous_ready`.
### Phase 3 — validation
- [ ] Full fs suite green on all backends; correct cases unchanged.

## Validation Plan

- Regression: the extended pathNormalize fixture (leading-`..` runs + the existing
  poppable/root cases).
- Runtime proof: `"../../a"` → `"../../a"`.
- Doc sync: none (restores documented behavior).

## Summary

A leading `..` run is silently deleted because `previous_ready` skips the
`..`-detection when the previous component is at offset 0; adding the leading check
fixes it. Security-relevant because it defeats `pathNormalize`-based escape checks;
the stale golden hid it.
