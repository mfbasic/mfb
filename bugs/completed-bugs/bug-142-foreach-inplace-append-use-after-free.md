# bug-142 — In-place append inside `FOR EACH` over the same list frees the iterator's buffer (use-after-free)

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — use-after-free / poison read when appending to the list
being iterated.
**Class:** memory-safety.

## Finding

`src/target/shared/code/builder_inplace_assign.rs:19-75`
(`try_inplace_append_assign`) and :91-156 (`try_inplace_bulk_append_assign`) —
neither checks `for_each_iterable_locals`, unlike `try_inplace_set_assign`:195
and `try_inplace_prepend_assign`:333. The grow path of
`lower_list_append_in_place` (builder_collection_mutate.rs:978-1016) frees the
old buffer that the `FOR EACH` snapshot (builder_control.rs:1120-1146) still
reads.

The soundness comment ("append only writes beyond the snapshot count") predates
the bug-01/bug-47 old-buffer frees; `emit_free_pre_grow_buffer`'s doc claim
"never on a live FOR EACH iterable (try_inplace_* guards)" is false for
append/bulk-append. Once an append outgrows capacity mid-loop, the snapshot
buffer is freed and entropy-scrubbed; subsequent iterations read poison (a
scrubbed String length word becomes a huge size).

## Trigger (reproduced)

```
MUT xs = ["aa","bb","cc","dd"]
FOR EACH x IN xs
  xs = collections::append(xs, x & "!")
NEXT
```
→ aborts `Code: 77010001 Message: allocation failed` on the first post-grow
iteration.

## Fix

Add the `for_each_iterable_locals` guard to `try_inplace_append_assign` and
`try_inplace_bulk_append_assign`, forcing the out-of-place (copying) path when
the target is a live FOR EACH iterable, matching set/prepend.

## Resolution

FIXED in commit e0fa88b8. append + bulk-append helpers gained the for_each_iterable_locals guard, forcing the copying path.

Regression test: `tests/rt-behavior/collections/bug142_foreach_inplace_append` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
