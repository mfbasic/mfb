# bug-430: accumulating into a resource STATE collection field is O(n²) (inlined field cannot grow in place)

Last updated: 2026-08-03
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Footgun (silent super-linear performance; no wrong result, no crash — but effectively a hang / potential OOM at scale)

Status: Open
Regression Test: `tests/rt_res_state_inplace_mutation.rs::collection_state_field_grows_in_place` (present, `#[ignore]`d pending this fix — remove the `#[ignore]` when it lands).

Split out of bug-424. bug-424 covered two independent halves of the same
whole-record-rebuild mechanism; its Layer 1 (scalar STATE field stored in place)
landed in `204e4c481` and made a scalar STATE mutation O(1) regardless of any
sibling buffer's size. This bug is the remaining, harder half: a **collection**
STATE field (`raw AS List OF Byte`) grown chunk-by-chunk still rebuilds the
whole STATE record and re-inlines the entire accumulated buffer on every append,
so accumulating N chunks is O(n²).

`f.state.raw = collections::append(f.state.raw, chunk)` is quadratic for two
compounding reasons:

- **Write side.** The single-field assign desugars to a whole-state `WITH`
  update (`src/ast/stmt.rs`); because the updated field `raw` is an *inlined*
  flat collection (`record_field_is_inlined` is true for a flat `List OF Byte`),
  it does not qualify for the Layer-1 scalar in-place store and falls through to
  `NirOp::StateAssign`'s whole-record rebuild (`emit_build_inlined_record`),
  which re-inlines and re-copies the whole current buffer.
- **Read side.** `f.state.raw` (the append's first argument) is a field-alias
  into the inlined block, not a uniquely-owned `MUT` local, so
  `collections::append` cannot take the in-place headroom path
  (`lower_list_append_in_place`) and first materializes an owned copy of the
  whole buffer.

Net: ~two full-buffer copies per append → Σ O(k) for k=1..n → **O(n²)**.

## Goal

- `s.state.collField = collections::append(s.state.collField, chunk)` grows the
  collection in place with amortized-O(1) append, matching a `MUT` local.
- The repro's STATE column becomes linear in N and within a small constant
  factor of the MUT-local baseline (target: same order of magnitude, not 2000×).
- The §15 aliasing/visibility contract, drop/free correctness, `.mfp` STATE
  encoding, and thread-transfer STATE copy all stay correct — **no leak, no
  double-free, no UAF** (all invisible to output goldens; see bug-374/375).

### Non-goals (must NOT change)

- Ordinary record `WITH` semantics stay a rebuild (records are immutable values).
- The Layer-1 scalar in-place store (bug-424) stays as-is.

## Why it is hard (audit from bug-424)

For a collection field to grow in place it must be a **pointer to a separately
allocated growable buffer** (capacity headroom), not inlined in the fixed-size
record block — inlined and growable are mutually exclusive. That is a record
**layout** change, and it must be threaded through, in lockstep, every place that
touches a STATE record — each a leak/double-free/UAF if wrong:

- `emit_build_inlined_record` / default STATE init (`emit_resource_state_init`) —
  build the out-of-line buffer(s).
- Field read of `f.state.coll` (`lower_field_access`) — load the pointer, not a
  block-relative offset.
- The append pattern (a `try_inplace_append_assign` sibling for STATE fields) —
  load the field pointer to a temp slot, `lower_list_append_in_place`, store the
  (possibly reallocated) pointer back into the field slot.
- `emit_free_resource_state_block` (`builder_resource_cleanup.rs:400`) — free
  each out-of-line buffer **separately** from the record block.
- `emit_inlined_block_size_from_ptr_slot` — feeds both free and the
  thread-transfer copy; must stop counting the out-of-line buffer as inlined.
- Thread-transfer STATE copy (`builder_arena_transfer.rs:460`) — deep-copy the
  out-of-line buffer into the destination arena.
- Whole-state boundary conversions: `record_field_is_inlined` is keyed on the
  record **type**, so the layout cannot diverge only "when used as STATE"
  without a whole-program STATE-only-type analysis. The alternative is a
  STATE-specific representation with conversions at whole-state read
  (`LET a = f.state`) and whole-state write (`f.state = <ordinary record>`).
  In-tree, whole-state read appears only in an `-invalid` fixture (never runs)
  and whole-state write is scalar-`WITH` or a scalar state — so the
  collection-bearing boundary is currently unexercised, but the language allows
  it and a correct fix must handle it, not error.

Rejected alternative: make collection-in-record fields out-of-line for **all**
records (no divergence, no conversions) — but that makes every
record-with-a-collection non-flat, wide behavior/golden churn against the
plan-02 inlining direction.

Lower-divergence alternative to weigh: keep the collection field inlined but
over-allocate the record block with capacity headroom and grow it in place with
geometric record-block realloc (fewer subsystems diverge, but block sizing at
free must account for the headroom, and it only works cleanly for a single
growable collection laid out last).

## Failing Reproduction

Two projects, identical 1 MB accumulation (N chunks × 64 bytes). One appends into
a `List OF Byte` **STATE field**, the other into a `MUT` local. Debug `mfb`,
macOS-aarch64, timed with `/usr/bin/time -p` (user CPU seconds), measured against
`204e4c481` (Layer 1 already landed, so the scalar half is O(1)):

STATE version (`raw AS List OF Byte` STATE field, append per iteration):

| N | payload | STATE collection append | MUT local append |
| --- | --- | --- | --- |
| 4000  | 256 KB | ~1.5s (quadratic) | ~0.00s |
| 8000  | 512 KB | ~6s   (quadratic) | ~0.00s |
| 16000 | 1.0 MB | 23.8s (quadratic) | 0.01s  |

(collection-only figure isolated in the bug-424 audit: 23.81s at N=16000, and
the quadratic quadruples when N doubles). Expected after this fix: STATE within a
small constant factor of the MUT-local baseline, and linear in N.

## Deterministic regression signal

`tests/rt_res_state_inplace_mutation.rs::collection_state_field_grows_in_place`
(build-only `--ncode`, cross-target `linux-x86_64`) asserts, for a STATE
collection-append function: `state_assign_value == 0` (no whole-record replace)
and at least one `append_inplace_realloc` label (the in-place grow path). It is
`#[ignore]`d today; un-ignore it when the fix lands. Add a runtime linearity /
RSS-ceiling proof and a thread-transfer-of-collection-STATE correctness fixture.

References: see bug-424 (`bugs/completed/`) for the full mechanism write-up, the
IR dump confirming the `WITH`-rebuild desugar, and the blast-radius reads.
Related memory: `records-inline-their-string-fields`, `collection-memory-mgmt`,
`resource-union-state-layout-and-wiring`, `union-state-needs-file-layout-record`.
