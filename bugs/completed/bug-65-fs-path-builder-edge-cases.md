# bug-65: `fs::pathBaseName` of an all-slash path returns `""` instead of `/`, and `fs::pathNormalize("")` writes its `.` null terminator one byte past the allocation (masked by arena rounding)

Last updated: 2026-07-09
Effort: small (<1h)

Two LOW-severity edge-case defects in the fs path-string builders, batched (same file).

**(1) `pathBaseName("//")` → `""` (correctness).** The whole-root shortcut fires only when
the *original* `length == 1`. For an all-slash path of length ≥ 2 (`"//"`, `"///"`), the
trailing-slash trim collapses it to a lone slash but does not re-route to the shortcut; the
backward scan then finds the slash at index 0, sets `index = 1`, and computes `span =
length - index = 0`, materializing an empty string. Runtime-confirmed: `pathBaseName("/")`
= `/` (correct), but `pathBaseName("//")` and `pathBaseName("///")` = `""` (should be `/`).

**(2) `pathNormalize("")` writes 1 byte past the request (memory-safety, latent).** The
buffer is sized `length + 9` (`8-byte header + content + 1 NUL`), exactly right for any
non-empty path (output ≤ input length). But the empty input triggers the `"."` fallback,
which manufactures 1 content byte from a 0-byte input and stores the NUL at `result + 9` —
one byte past the 9-byte request. It is currently harmless only because `_mfb_arena_alloc`
rounds every request up to a 16-byte granule, so offset 9 lands in padding; the safety
depends on a non-local allocator detail, not on this code.

The single correct behavior a fix produces: (1) `pathBaseName` of any all-separator path
returns `/`; (2) `pathNormalize("")` sizes its buffer for the `.` fallback so the NUL is
in-bounds.

References (all under `src/target/shared/code/builder_fs_paths.rs`):

- `lower_fs_path_base_name` (`:86-131`): `whole_root` shortcut gated on pre-trim
  `length == 1` (`:86-90`); `found_slash` sets `index = 1` (`:116`) → `span = 0`.
- `lower_fs_path_normalize`: alloc `length + 9` (`:339`); `"."` fallback (`finish`,
  `:622-629`) + `finish_nonempty` NUL store at `result + out_len` offset 8 (`:631-634`) →
  write at `result + 9`.
- Arena rounding that masks (2): `entry_and_arena.rs:720` (`round_up(max(size,1), 16)`).
- Open spec question (not filed as a defect): `pathExtension(".bashrc")` returns
  `".bashrc"` (leading-dot dotfile treated as an extension) — matches Go's `filepath.Ext`,
  differs from Python `splitext`; confirm against the MFBASIC spec before changing.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

```
IMPORT io
IMPORT fs
FUNC main AS Integer
  io::print("[" & fs::pathBaseName("/") & "]")
  io::print("[" & fs::pathBaseName("//") & "]")
  io::print("[" & fs::pathBaseName("///") & "]")
  RETURN 0
END FUNC
```

- Observed: `[/]`, `[]`, `[]`.
- Expected: `[/]`, `[/]`, `[/]`.

(2) `fs::pathNormalize("")` returns `"."` correctly, but writes the NUL at `result + 9`
(1 byte past the 9-byte allocation) — visible under a guard allocator; harmless under the
default 16-byte-rounded arena.

Contrast: `pathBaseName("/")`, `"/a"`, `"//a"` are correct; `pathNormalize` of any non-empty
path is in-bounds (output ≤ input length, NUL at ≤ size-1).

## Root Cause

(1) The trailing-slash trim collapses `"//"` to a conceptual lone slash, but only the
pre-trim `length == 1` case routes to `whole_root`; a post-trim length-1 slash falls through
to the normal scan and yields span 0. (2) The allocation is sized for `output ≤ input`, but
the `"."` fallback produces 1 byte from a 0-byte input, needing 10 bytes where 9 were
requested.

## Goal

- `pathBaseName` of any all-separator path returns `/`.
- `pathNormalize("")` allocates room for the `.` fallback's NUL (no OOB even under a
  byte-exact allocator).

### Non-goals (must NOT change)

- `pathBaseName`/`pathNormalize` of non-degenerate inputs (byte-identical).
- `pathExtension` dotfile behavior (pending the spec decision above).
- The arena rounding (it should not be the thing that makes (2) safe, but do not change it
  here).

## Blast Radius

- `lower_fs_path_base_name` — item (1).
- `lower_fs_path_normalize` alloc sizing — item (2).
- Audit sibling path builders (`pathDirName`, `pathJoin`) for the same trim/fallback
  asymmetry; fold any found.

## Fix Design

(1) After the trailing-slash trim, if `length == 1` and the remaining byte is `'/'`, route
to `whole_root` (move/duplicate the single-slash check to after `trim_done`). (2) Size the
allocation `max(length, 1) + 9` (or `length + 10`) so the `.` fallback's NUL is in-bounds.

## Phases

### Phase 1 — failing tests

- [x] Add `pathBaseName("//")`/`"///"` → `/` tests (fail today). Add a `pathNormalize("")`
      test run under a guard allocator to catch the OOB write.

### Phase 2 — the fixes

- [x] Re-route all-slash `pathBaseName` to `whole_root`; enlarge the `pathNormalize`
      allocation for the fallback.

### Phase 3 — validation

- [x] Regenerate path goldens (delta = the two edge cases); `scripts/artifact-gate.sh`,
      `scripts/test-accept.sh`.

## Validation Plan

- Regression test(s): the all-slash `pathBaseName` cases and the guard-allocator
  `pathNormalize("")` case.
- Runtime proof: the reproduction prints `[/]` three times.
- Doc sync: resolve the `pathExtension` dotfile spec question separately.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

`pathBaseName` drops an all-slash path to `""` because only the pre-trim single-slash case
routes to the root shortcut, and `pathNormalize("")` under-allocates by one byte for its `.`
fallback (masked only by arena rounding). Both fixes are local; non-degenerate paths are
unchanged, and the `pathExtension` dotfile question is left to the spec.

## Resolution

Fixed in `src/target/shared/code/builder_fs_paths.rs` (2026-07-09).

**(1) `pathBaseName` all-slash → `/`.** Removed the *pre-trim* `length == 1` root shortcut
and moved the single-slash check to *after* the trailing-slash trim (`trim_done`): if the
trimmed `length == 1` and the remaining byte is `/`, control now branches to the existing
`whole_root` block (`index = 0`, `length = 1`), yielding `/`. A new `scan_start` label is the
fall-through for the non-root case. This routes `"/"`, `"//"`, `"///"`, … all to the root
shortcut while leaving `"/a"`, `"//a"`, `"a/b/"`, `"out.txt"` untouched (their trimmed length
is `> 1` or the trimmed byte is not a slash). Sibling audit: `pathDirName` already handles
this correctly (its `found_slash` routes `index == 0` to the `root` block); `pathExtension`
returns `""` for all-slash input (correct — no extension); `pathJoin` delegates to a runtime
helper and is unaffected. No man/spec change: `pathBaseName.txt` already documents "consists
only of '/' separators, '/' is returned" — the code merely failed to match the spec.

**(2) `pathNormalize("")` NUL now in-bounds.** The allocation request changed from
`length + 9` to `length + 10` (the `add_immediate ..., 10` before `_mfb_arena_alloc`). The
`.` fallback manufactures 1 content byte from a 0-byte input and stores its NUL at
`result + out_len + 8` = offset 9; `length + 10` = 10 bytes for the empty input makes that
write in-bounds without depending on the arena's 16-byte rounding. Every non-empty path keeps
`output ≤ input`, so the extra byte is unused padding and the observable result is unchanged.
No man/spec change (output is still `.`).

Verified at runtime (`target/debug/mfb build` + execute):
`pathBaseName` prints `[/] [/] [/] [a] [a] [b] [out.txt]` for `/ // /// /a //a a/b/ out.txt`;
`pathNormalize("")` = `.` and all existing `pathNormalize` fixture cases are byte-identical.

Tests: `tests/rt-behavior/fs/func_fs_pathBaseName_valid` gained `"//"`, `"///"`, `"//a"`
cases (goldens regenerated); `func_fs_pathNormalize_valid` already covered `""` → `.`. The
`*_invalid` fixtures are unchanged (the signatures did not change). The invariant that the
`.` fallback's NUL is in-bounds is now guaranteed by construction (10 bytes ≥ 8 header + 1
content + 1 NUL), independent of any allocator rounding.

Out of scope (noted, not fixed): `pathNormalize("a/..")` returns `"a"` rather than the
spec-implied `"."` — the `pop_scan` loop gives up (leaves `out_len` unchanged) when the
component being cancelled has no preceding `/`. This is a pre-existing, separate defect (the
allocation-size change cannot affect the pop logic) and is explicitly a non-goal here.
