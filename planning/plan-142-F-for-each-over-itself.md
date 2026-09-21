# plan-142-F: In place inside a `FOR EACH` over the binding (S7)

Last updated: 2026-09-20
Effort: medium (1h–2h)
Depends on: plan-142-E

Prerequisites: see plan-142-A.

Today every arm declines while a `FOR EACH` walks the binding it would mutate
(gate G7), because the loop holds the binding's block pointer and count
(`lower_for_each`, `builder_control.rs:2442`: `lower_value` at `:2454`, stored at
`:2493-2497`, `count` read once at `:2555-2559`). A realloc would free that block
under the loop; a shift would be visible to it. The fallback then **leaks** the old
block every iteration (`:1309-1343`). After F, `FOR EACH v IN x` whose body
self-updates `x` runs every such statement in place, the loop still visits
exactly the elements `x` held at entry, and nothing leaks.

References: plan-141 findings (S7: all `n (G7)`); bug-142 (the realloc-under-iterator
bug G7 exists for); `src/docs/spec/memory/05_collections.md:510-512` (the loop
iterates the value at entry).

## 1. Goal

- `S7` is added to `ENABLED_SITES`; every `Arm` row fires at S7 in the matrix test.
- The harness gains an S7 site template (`FOR EACH v IN x` … `x = <statement>` …
  `NEXT`): alloc count flat in `N` beyond the one entry copy per loop.
- Runtime cases: the loop visits the entry-time elements (append, removeAt,
  sort, filter inside the loop), and a `perf.mfb_free` count shows no leaked block.

### Non-goals

- Snapshot semantics unchanged (the loop never sees its own writes).
- A loop whose body does not self-update the iterable keeps borrowing (no new copy).
- Record-field iterables (`FOR EACH v IN r.xs`, G15) are records — out of scope.

## 3. Design

When the loop's iterable is a local `x` and the body contains an assignment to `x`
(a direct `NirOp::Assign { name: x }` anywhere in the body, or a by-ref capture of
`x` in a `forEach` lambda inside it — both are syntactic, found by one walk of the
body's ops), `lower_for_each` lowers the iterable **owned**: the loop walks a
private copy made once at entry and freed at loop exit (normal exit and `EXIT FOR`).
`x` is then not pushed to `for_each_iterable_locals`, so G7 no longer fires and the
self-updates take the arms; the copying fallback's leak path is no longer reached
for this loop.

The cost is one copy per loop entry instead of one copy (and one leak) per
iteration. That copy is the loop's snapshot, not the statement's: each
`x = op(x, …)` inside is in place.

Rejected:
- **Iterate the live binding by index** — the loop would see its own appends,
  breaking snapshot semantics.
- **Copy on first write (runtime flag)** — saves the copy only when the loop runs
  zero writes; adds a branch per statement and a per-loop flag. Open Decision 1.

Risk: the loop-exit free on every exit edge (`EXIT FOR`, `RETURN` from inside,
`TRAP` unwinding); a missed edge leaks, a double edge double-frees.

## Phases

### Phase 1 — Owned iterable when the body writes it

- [x] Body scan (`NirOp::Assign` to the iterable's name, including nested blocks;
      by-ref `forEach` captures of it). `ops_write_local`
      (`engine/function/function_lowering.rs`): an `Assign` to the name or a
      `LocalRef` of it (the by-ref capture) anywhere under the body, through the
      exhaustive `NirVisitor` seam.
- [x] Owned iterable + exit-edge frees in `lower_for_each`; skip the
      `for_each_iterable_locals` push for this loop. The copy is bug-666's operand
      snapshot (`operand_snapshot.rs`), already used for a global iterable the body
      can store: `lower_value` deep-copies the iterable into a statement-scope
      temporary, which the `FOR EACH` statement's pending-temp drain frees on every
      exit edge — no new exit-edge code (Correction F2). Gated on
      `is_freeable_flat_value` (a graph-typed local, which the snapshot cannot copy,
      keeps borrowing and keeps the G7 exclusion).
- [x] Runtime cases (`tests/runtime/rt_for_each_self_update.rs` + stanza): snapshot
      visit order for append/removeAt/sort/filter; `EXIT FOR` and `RETURN` from the
      body; `perf.mfb_free.count` equals `perf.mfb_alloc.count` for the loop's blocks.
      Ten programs: append, removeAt, sort+prepend, filter, `List OF String`, a Map,
      a Set, `EXIT FOR`, `RETURN` from the body, and nested loops over the same
      binding. The balance is `arena.0.alloc_calls == free_calls` and
      `live_bytes == 0` (Correction F1). RED with the pre-F binary
      (`MFB_TEST_EXE=<5bbca2a25 build>`): all ten print the right elements and all
      ten leak — e.g. "append: … 21 allocated / 18 freed / 224 B live",
      "nested: … 24 allocated / 18 freed / 480 B live".

Acceptance: `cargo test --test rt_for_each_self_update` → pass (est. 5 min).
Verified 2026-09-21: `test result: ok. 1 passed; 0 failed` (2.49s).
Commit: 2d8a3db26

### Phase 2 — Enable S7

- [x] Add S7 to `ENABLED_SITES` and to the harness site templates.
      `Site::ForEach` in both. The matrix probe is `FOR EACH each1 IN x` / `x = <call>`
      / `NEXT`. The harness runs the line's `N`-iteration loop inside the `FOR EACH`'s
      first visit and then `EXIT FOR`s, so the entry copy is taken once at any `N`;
      a program whose loop never ran returns 3 (a failed run) rather than passing
      vacuously. The `exempt` check at S7 keeps the statement inside the loop in
      both programs behind a run-time guard (`len(x) >= 0` / `< 0`), so both take the
      entry copy and the difference is the statement alone. The `&` row has no S7:
      a `String` is not a collection (Correction F3). RED with the pre-F binary on
      five lines: "collections::append(value AS List OF T, item AS T) … at ForEach:
      marked `arm`, but 2000 more runs allocated 4000 more blocks (4167 at N=2000,
      8167 at 2N)" (likewise `append(List)`, `removeAt`, `sort`, `union`); S1 and the
      `shake256` exempt line pass on both.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update`
→ pass at S1 and S7 for every `Arm` row (est. 15 min: the harness doubles).
Verified 2026-09-21: `cargo test --bin mfb self_update` → `test result: ok. 4 passed`
(incl. `every_arm_row_fires_at_every_enabled_site`); `cargo test --test
rt_inplace_self_update` → `test result: ok. 1 passed; 0 failed` (296.00s; 127
case/site pairs: 64 at S1, 63 at S7).
Commit: a0e65b4a7

### Phase 3 — Expected outputs

- [x] Fixtures with a self-update inside a `FOR EACH` over the same binding —
      measure with `rg -lP 'FOR EACH \w+ IN (\w+)\n(?:.*\n)*?\s*\1 = ' -U tests examples --glob '*.mfb'`
      and record the count here; regenerate any committed golden among them (the
      diff must be the loop's entry copy + arms replacing per-iteration copies).
      `tests/rt-behavior/collections/bug142_foreach_inplace_append` must still pass.
      Measured: **10 files** — 6 under `tests/rt-behavior/collections`
      (incl. `bug142_foreach_inplace_append`), `tests/rt-behavior/resources/res-rebind-alias-runtime`,
      and 3 under `examples/browser`. Their goldens are `.ast`/`.ir`/`.run`/`build.log`
      only — none records native code — so none regenerates; the `.run` outputs must
      hold, and do (acceptance below; the resources fixture:
      `acceptance tests passed (1 test(s) ran)`).

Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual` green
on `tests/rt-behavior/collections` (est. 10 min).
Verified 2026-09-21: `scripts/test-accept.sh target/debug/mfb target/accept-actual
'rt-behavior/collections/*'` → `acceptance tests passed (65 test(s) ran)`.
Commit: a0e65b4a7

## Validation Plan

- Tests: `rt_for_each_self_update`; matrix and harness at S7; bug142 fixture.
- Artifact gate at `a0e65b4a7`: `scripts/artifact-gate.sh <debug mfb> all` →
  `1473 tests, 1648 build(s), 2078 golden(s) checked, 0 diff(s)`.

## Open Decisions

1. **RESOLVED (user, 2026-09-20): copy at loop entry**, only when the body
   self-updates the iterable (the design above). Copy-on-first-write is not built.

## Corrections

- **F1 (Phase 1): the balance is the arena's counters.** `perf.mfb_alloc.count` is
  capped (plan-142-A Correction A4); the case reads `arena.0.alloc_calls`,
  `free_calls` and `live_bytes`, which `--debug` builds print uncapped — the same
  counters `rt_reduce_collection_accumulator_frees` uses.
- **F2 (Phase 1): the copy already existed.** §3 asks for a private copy and a
  free on every exit edge. bug-666 built exactly that for a global iterable
  (`operand_snapshot_wanted` in `lower_for_each`): the snapshot is a
  statement-scope pending temp, freed by the statement's drain on the normal exit,
  `EXIT FOR` and `RETURN`. F routes the written-local case through it rather than
  adding a second copy-and-free; the `EXIT FOR`/`RETURN`/nested cases prove the
  edges.
- **F3 (Phase 2): the `&` row has no S7.** §1 says every `Arm` row fires at S7, but
  `FOR EACH` walks collections only (`lower_for_each`: "native code FOR EACH target
  '…' is not a collection"), and `x` in the `&` row is a `String`. `Probe::source`
  returns `None` for it at S7 and the harness's `Site::applies` skips it; every
  collection `Arm` row fires at S7.
- **F4 (found by plan-142-I's full `cargo test`): three codegen tests asserted the
  old mechanism.** `tests/codegen/codegen_inplace_remove_at_insert.rs` required
  `insert`, `removeAt` and Set `remove` to *decline* under a `FOR EACH` over the
  same binding (plan-121-B). They protect "the loop never sees the shift", which
  F keeps by giving the loop its own copy (Open Decision 1, resolved by the user;
  `rt_for_each_self_update`'s `remove_at` case visits `1 2 3`). Only the disproved
  assertion changed: each now requires the loop's `operand_snapshot` slot and the
  arm's label; `G7` itself is still pinned by `inplace_dest.rs`'s unit tests.

## Summary

A small lowering change with an exit-edge obligation; the arms need no change.
