# bug-132 — `pathNormalize("/a/..")` returns "/a" — pop at root-adjacent slash never shrinks out_len

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G9).
**Severity:** MED — wrong normalized path for `..` directly under root.
**Class:** correctness.

## Finding

`src/target/shared/code/builder_fs_paths.rs:538-555` (`pop_scan` in
`lower_fs_path_normalize`). `pop_scan` walks the output backward for a `/`; when
the slash is found at index 0 (the root slash), the `scratch13 == 0 → branch
component_loop` guard skips the `out_len` store entirely, leaving the popped
component in place. The intent was to keep the root slash (out_len = 1); instead
out_len is unchanged.

## Trigger

- `fs::pathNormalize("/a/..")` → "/a" (expected "/").
- `fs::pathNormalize("/a/../b")` → "/a/b" (expected "/b").

The fixture (tests/rt-behavior/fs/func_fs_pathNormalize_valid) only covers pops
with a non-root preceding slash ("/tmp///a/../b/").

## Fix

When the pop finds the slash at index 0, set out_len = 1 (keep the root slash)
instead of skipping the store. Best folded into the bug-79(3) fix (the
sibling "a/.." → "a" case in the same routine).

## Prior art

Sibling of bug-79(3) (`"a/.."` → `"a"`, no preceding slash); this is the
distinct preceding-slash-at-index-0 case, same routine.
