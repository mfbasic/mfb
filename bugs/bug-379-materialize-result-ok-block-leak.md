# bug-379: `materialize_current_result` OK path leaks the intermediate copied success block

Last updated: 2026-07-23
Effort: small (<1h)
Severity: LOW
Class: Memory-safety

Status: Open
Regression Test: (to add) an arena-accounting / no-growth runtime test that
materializes an OK `Result OF String` (or `Result OF <collection>`) in a loop and
asserts the function-scope arena does not grow per iteration.

When `materialize_current_result` builds an **OK** `Result` whose success type is
a block (`String`, a flat record/data-union, or a collection), it first
deep-copies the success value into a freshly `arena_alloc`'d block
(`copied_success`), stores that as the payload, then `emit_build_result_inline`
allocates the `Result` and **memcpies the payload into it**. After that inline
copy the intermediate `copied_success` block is redundant and is never freed —
whereas the sibling ERR_BLOCK path frees its intermediate immediately after the
identical inline build. The result is one leaked arena block per OK-Result
materialization of a block-typed value.

This is a leak, not a use-after-free or double-free: the block stays reachable
only via nothing and is reclaimed at the enclosing arena's teardown, so it is
bounded and cannot corrupt memory. It matters because `materialize_current_result`
is on the Result-return path; a function that materializes OK block-Results in a
loop (e.g. repeatedly returning/`TRAP`-wrapping `Result OF String`) grows its
function arena unbounded until scope exit, an avoidable memory-pressure /
throughput cost (and, on the arena's transient-churn path, feeds the known
quadratic free-list behavior).

The single correct behavior a fix produces: the OK block path frees (or never
creates) the redundant intermediate block, so materializing an OK block-Result
allocates exactly the one `Result` block — matching the ERR_BLOCK path's
allocation accounting.

References:

- The correct sibling: ERR_BLOCK adopt path frees its intermediate,
  `builder_arena_transfer.rs:163` (`emit_free_error_block_from_slot(payload_slot)`).
- `emit_build_result_inline` deep-copies a block payload into the Result,
  `builder_arena_transfer.rs:26-64` (`is_block` → `emit_copy_bytes`).
- `copy_value_to_current_arena` → `copy_flat_block` = `arena_alloc` + memcpy
  (a fresh block), `builder_arena_transfer.rs:247`.
- Related memory: `[[arena-transient-churn-quadratic-graphemes]]` (leaked
  transient blocks worsen this).
- Found during the 2026-07-23 runtime security audit (arena/lifetime sweep).

## Failing Reproduction

Static, from the code (no crash — a leak):

```
src/target/shared/code/builder_arena_transfer.rs:122-135  (OK path)
    load value → scratch9
    copied_success = copy_value_to_current_arena(success_type, &scratch9)  // fresh arena_alloc for a block type
    store copied_success → payload_slot
    ok_result = emit_build_result_inline(tag_slot, success_type, payload_slot) // ALLOCATES Result + memcpies payload in
    store ok_result → result_slot
    branch have_payload                                    // <-- copied_success never freed
```

Contrast — the ERR_BLOCK path (`:150-163`) does the same inline build then frees:

```
    adopted = emit_adopt_current_error_block()
    store adopted → payload_slot
    adopt_result = emit_build_result_inline(tag_slot, "Error", payload_slot)
    store adopt_result → result_slot
    emit_free_error_block_from_slot(payload_slot)          // <-- intermediate freed
```

- Observed: OK block-Result materialization allocates two blocks
  (`copied_success` + the `Result`) and frees zero; the first leaks.
- Expected: it nets exactly one live block (the `Result`), like the ERR path.

Immune case: an OK Result whose success type is scalar (`Integer`/`Float`/…) —
`copy_value_to_current_arena` returns the value in a register (no allocation) and
`emit_build_result_inline` stores it inline (`is_block == false`), so there is
nothing to leak. Only block success types are affected.

## Root Cause

`src/target/shared/code/builder_arena_transfer.rs:122-135` — the OK branch of
`materialize_current_result` creates an owned intermediate copy of the success
value to serve as the payload, but `emit_build_result_inline` copies the payload
into the freshly-allocated `Result` block rather than adopting the pointer, so
the intermediate is dead after the call. The ERR_BLOCK branch handles this with an
explicit free; the OK branch omits the symmetric free. The scalar path is immune
because no intermediate block exists.

## Goal

- Materializing an OK `Result` of a block type leaves exactly one live arena
  block (the `Result`); no per-iteration arena growth in a loop.

### Non-goals (must NOT change)

- The `Result` layout/contents, the ERR_BLOCK adopt path, or the scalar OK path.
- Must not free anything on the scalar path (there is no block there).
- Must not turn the leak-fix into a use-after-free: the intermediate may only be
  freed **after** `emit_build_result_inline` has finished copying it.

## Blast Radius

- `builder_arena_transfer.rs:122-135` (OK block path) — this bug.
- ERR_BLOCK path (`:150-163`) — already frees; the model to mirror.
- Scalar OK path — no block; unaffected. Any other caller of
  `emit_build_result_inline` that passes an owned intermediate should be audited
  for the same missing free (grep confirms error/worker paths already free or
  reuse a parked block).

## Fix Design

Two options; recommend (A) for minimal risk:

- **(A) mirror the ERR path — free after the inline build.** After
  `emit_build_result_inline` on the OK path, when `success_type` is a block type
  (`result_payload_is_block(success_type)`), free the intermediate block held in
  `payload_slot` (the generic flat-block free, analogous to
  `emit_free_error_block_from_slot`). Clearly correct, localized, matches the
  established pattern. Free strictly after the inline copy completes.

- **(B) skip the redundant copy.** For a block `success_type`, store the original
  value pointer directly into `payload_slot` and let `emit_build_result_inline`
  do the single deep copy into the `Result`, dropping the
  `copy_value_to_current_arena` call entirely. Eliminates both the leak and a
  redundant allocation (a small perf win). Rejected as the primary fix only
  because it assumes the original value pointer is safely readable in the current
  arena context at this point; confirm that invariant before choosing (B). If it
  holds, (B) is strictly better.

No golden output moves (the fix only frees/avoids an intermediate; generated
`Result` bytes are unchanged).

## Validation Plan

- Regression test: loop that materializes an OK `Result OF String` many times;
  assert no per-iteration function-arena growth (or an allocation-count check).
- Runtime proof: existing Result/TRAP suites stay green and byte-identical.
- Doc sync: none.

## Summary

A localized arena leak from a missing symmetric free on the OK block path; the
ERR_BLOCK path already shows the intended shape. Low risk — the only hazard is
ordering the free after the payload copy (option A) or verifying the source
pointer's arena validity (option B). No behavior or output change.
