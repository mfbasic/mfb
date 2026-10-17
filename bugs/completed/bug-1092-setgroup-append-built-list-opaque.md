# bug-1092: `setGroup` of an `append`-built list neither takes over nor moves the resources it holds

Last updated: 2026-10-14
Effort: medium (1h-2h)
Severity: HIGH
Class: Correctness (silent wrong picture) + Footgun (missed TYPE_USE_AFTER_MOVE)

Status: Closed
Regression Test: tests/canvas/rt_canvas_group_ownership.rs (`a_loop_built_list_keeps_its_font_past_the_scope`, `an_appended_let_item_keeps_its_font_past_the_scope`, `two_appended_resources_are_both_kept`, `the_other_placing_collections_members_keep_their_font`); tests/syntax/resources/canvas-setgroup-append-consumes-items-invalid; tests/syntax/resources/canvas-append-unconsumed-valid; `codegen::resource_containment_tests`; `codegen::registry::tests::the_placing_operands_name_real_members`

`canvas::setGroup(name, items)` has a consuming parameter (plan-116-J): the images and fonts
reachable from `items` become the group's, so the caller's bindings must not close them at
scope exit and naming them afterwards is `TYPE_USE_AFTER_MOVE`. "Reachable" is found by two
twin structural walks that treated EVERY call as opaque, so a list built with
`items = collections::append(items, canvas::Tile[image := sheet, ...])` (in a loop, or from a
`LET` item) recorded no `items -> sheet` edge. The binding `sheet` was closed at its scope
exit and the installed group drew nothing for it - silently, a second close being a no-op -
and naming `sheet` after `setGroup` was not refused.

**The single correct behavior:** a collection built by a `collections` member that places its
operands (`append`, `prepend`, `insert`, `set`, `add`, `union`, `merge`, `repeat`, `flatten`)
holds what those operands hold, accumulated across repeated `items = f(items, x)` assignments;
`setGroup` of it takes all of that over (no scope-exit close, later naming refused), while
every other call stays opaque.

## STATUS: FIXED (b0e3fe802)

Shipped as designed: one table (`resource::PLACING_OPERANDS`) read by both walks, the
codegen pass extends (union) on a self-referential `Assign`, and the verifier - which
previously recorded containment only on `Bind` - now records it on `Assign` too (its
`Local` arm already contributes the old holdings, so the result is the union).

References:

- plan-116-J (consuming parameters); `.ai/canvas-threading.md`; `mfb spec app` "A named group is the exception".

## Failing Reproduction

/tmp/probe3 (loadImage of a PNG, four groups, headless dump):

```
target/release/mfb build --debug /tmp/probe3
MFB_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_DUMP=/tmp/probe3/f.rgba /tmp/probe3/build/probe3.app/Contents/MacOS/probe3
```

- Observed: group A (literal list) and C (loop-built, binding alive) draw; B (loop-built,
  function returns) and E (`LET` item appended, function returns) draw nothing.
- Expected: all four draw.
- Verifier: `tests/syntax/resources/canvas-setgroup-append-consumes-items-invalid` compiled
  clean before the fix; it now fails with `2-203-0055 TYPE_USE_AFTER_MOVE`.
- RED evidence (before the fix, `cargo test --test rt_canvas_group_ownership`): 5 of the new
  tests failed (glyph counts 0 vs the control's); after: all 15 pass.

## Root Cause

- `src/codegen/mod.rs:resource_containment` + `CodeBuilder::collect_consumed_locals`
  (`src/codegen/resource/cleanup/builder_resource_cleanup.rs`), read by
  `deactivate_consumed_cleanups`, had no arm for `NirValue::Call`; the `append` result held
  nothing.
- `src/ir/verify/link.rs:collect_held` likewise had no `IrValue::Call` arm, and
  `src/ir/verify/resources.rs:check_resource_moves` recorded `contains` only for
  `IrOp::Bind`, never `IrOp::Assign` - so `items = append(items, x)` could not record anything
  even with the call understood.
- `resource_containment` handled `Assign` by REPLACING the entry; with `items =
  append(items, x)` the value names `items` itself (removed by `retain`), so a second,
  differing append dropped the first's resources.

## Goal

- Both walks treat the placing `collections` members as placing the listed operands.
- A self-referential `Assign` extends the entry in both walks.
- Valid programs that append into a list that is never consumed still compile and still
  close their resources.

### Non-goals (must NOT change)

- Opaque calls stay opaque (a spurious containment edge rejects valid programs / leaks).
  Subset-keeping members (`filter`, `take`, `drop`, `mid`, `remove`, `removeAt`,
  `removeKey`, `intersection`, `difference`, `distinct`, `replace`, ...) are NOT placing.
- Weakening the existing plan-116-J tests or fixtures: none touched.

## Blast Radius

- `collect_consumed_locals` callers: `deactivate_consumed_cleanups`, `resource_containment` - fixed.
- `collect_held` callers: `held_resources` <- `consumed_contained`, `check_resource_moves` Bind - fixed (and Assign now added).
- `set` over a list: result no longer holds the replaced element but the table still lists
  operand 0 - latent over-approximation (only reachable when a resource held by the list is
  replaced out of it and the list is then handed to a consuming parameter); same direction
  `append` already assumes of its list; not observed to fail.
- Flow-insensitivity of `resource_containment`: an `Assign` that does not name its target
  still replaces (and `x = []` leaves a stale entry). Pre-existing, codegen-only, latent;
  not changed here because replacing vs. extending without flow information trades a leak
  for a double-close; not observed to fail.
- `src/ir/resource_escape.rs` already treats `collections.append` structurally in its own
  scan - unaffected.
- `flatten` is a monomorphised MFB-bodied member (`#collections_flatten$...`); the table
  normalises that name.

## Fix Design

Rejected: making every call that returns a collection placing (false moves/leaks);
re-deriving containment from the callee body (needs inter-procedural analysis).

## Phases

### Phase 1 - failing tests
- [x] Runtime tests and verifier fixture added; 5 runtime tests failed, fixture compiled clean.
Commit: b0e3fe802 (tests and fix landed together).

### Phase 2 - the fix
- [x] Table + both walks + verifier Assign recording.
Commit: b0e3fe802.

### Phase 3 - validation
- [x] `cargo test --no-fail-fast`, `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
No golden shifted.

## Validation Plan

- Regression tests: listed above.
- Doc sync: `mfb spec app` canvas "A named group is the exception".

## Summary

Risk concentrates in the verifier's new `Assign` recording (it affects every program that
reassigns a container) - guarded by the full acceptance suite - and in `deactivate_consumed_cleanups`
now dropping more cleanups, which is exactly the intended ownership transfer.

Gate record: `cargo test --no-fail-fast` all green except `rt_compress_throughput` (a timing
bound, failed only while another session loaded the machine; passes alone, 12.7s, and at
HEAD); `scripts/test-accept.sh` 1729 fixtures, only the end-to-end `acceptance` app timed
out under that load and passes alone; `artifact-gate.sh collections`: 0 diffs.
