# bug-234: dead/redundant labels and fall-through branches across shared/code builders

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: dead-code

Status: Open

Several codegen builders emit labels that are never branched to, or an
unconditional branch to the immediately-following label (a jump to the next
instruction):

- `src/target/shared/code/builder_fs_paths.rs:288` — redundant unconditional
  `branch(&done)` immediately precedes `label(&done)` in
  `lower_fs_path_extension` (materialize already falls through).
- `src/target/shared/code/builder_collection_queries.rs:748,753` —
  `zip_n_from_b` label allocated and emitted but never branched to (the min
  computation falls through to `n_done`).
- `src/target/shared/code/builder_strings_package.rs:152,200` — `cmp_label` in
  `emit_chars_set_contains_branch` allocated and emitted but never branched to
  (pure fall-through marker).
- `src/target/shared/code/fs_helpers_atomic.rs:748-752` — redundant
  `abi::branch(&done)` after `emit_errno_error_mapping` (which already branches to
  `done` in every case); the accompanying comment claiming the mapping "does not
  branch to done" is also wrong.

Fix: delete each unreferenced label / redundant branch and correct the
fs_helpers_atomic comment.
