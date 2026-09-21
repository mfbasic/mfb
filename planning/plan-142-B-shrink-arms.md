# plan-142-B: In-place arms for `filter`, `take`, `drop`, `mid`, `distinct`

Last updated: 2026-09-20
Effort: large (3h–1d)
Depends on: plan-142-A

Prerequisites: see plan-142-A (they gate every letter).

Five `List` self-updates that only ever **remove** elements get in-place arms,
registered in `SELF_UPDATE_TABLE` and dispatched through A's
`try_inplace_self_update`. After B, `x = collections::filter(x, p)`,
`x = collections::take(x, n)`, `x = collections::drop(x, n)`,
`x = collections::mid(x, s, n)` and `x = collections::distinct(x)` allocate
nothing at S1 (the alloc count does not scale with `N` in
`rt_inplace_self_update`).

References: plan-142-A §3 (seam, failure-atomicity rule); plan-141 findings §1 rows
for these five; `.ai/collections.md` ("`FOR EACH` permits an append but not a
shift"; "the SECOND aliasing surface" — payload relocation).

## 1. Goal

- The five table rows flip `Pending("B")` → `Arm(…)`; their `cases.tsv` lines flip
  `pending:B` → `arm`; `every_arm_row_fires_at_every_enabled_site` and
  `rt_inplace_self_update` pass for them.
- Failure atomicity: if `filter`'s predicate fails partway, or a `take`/`drop`/`mid`
  argument is invalid, `x` is unchanged (runtime cases).

### Non-goals

- No change to the results (same elements, same order).
- No new language surface; records untouched.
- The copying lowerings stay (they serve non-self-update calls, `LET y = filter(x, p)`).

## 2. Current State

(Research 2026-09-20, read.)

| fn | body kind | lowering today |
|---|---|---|
| `filter` | `Body::abi_inline(lower_filter)` | `func_filter.rs:118`, builds a fresh list |
| `take` | `Body::Mfb` `__collections_take` | `func_take.rs:126`, a call to `_mfb_ifn_collections_take$T` |
| `drop` | `Body::Mfb` | `func_drop.rs:117` |
| `mid` | `Body::Intrinsic` | `lower_mid`, `collection/search/builder_search.rs:663` |
| `distinct` | `Body::Mfb` | `func_distinct.rs:160` |

- `native_builtin_target` returns `None` for a `Body::Mfb` member
  (`src/codegen/builtins/mod.rs:178`; the monomorphizer rewrites the call to
  `__collections_X`, and the sort fast path sees `#collections_sort$T`,
  `func_sort.rs:20`). So an arm cannot match `take`/`drop`/`distinct` by
  `native_builtin_target` — see Phase 1.
- Existing primitive: `lower_list_remove_at_in_place` (`list_mutate.rs:4112`)
  shifts survivors down one element and frees the removed element's graph
  (plan-134-H). Using it per element is O(n²); nothing compacts in one pass, and
  there is no truncate.

## 3. Design

**Matching.** A helper `self_update_builtin(target: &str) -> Option<&'static str>`
in `self_update.rs` answers the bare name for every spelling a self-update call can
have after lowering: `collections.X`, the monomorphized `__collections_X$…`, and the
fast-path `#collections_X$…`. The arms match through it; the census test gains a
case per spelling.

**Two new primitives** in `src/codegen/collection/list/list_mutate.rs`, both
non-reallocating (they only shrink), both freeing the dropped elements' graphs
exactly as `lower_list_remove_at_in_place` does:

- `lower_list_compact_in_place(buffer_slot, keep: KeepSource, list_type, element_type)`:
  one pass, survivor `i` moves to position `k` (entry and payload), then `count := k`
  and `dataLength` repacked. `KeepSource` is either a bitmap slot (filter,
  distinct) or an index range `[start, start+len)` (take, drop, mid).
- `take`/`drop`/`mid` are the range form: validate arguments first (the same errors
  the copying lowering raises, same order), then compact.

**filter, atomically.** Pass 1 calls the predicate for every element and records
keep bits into a scratch bitmap (`ceil(n/8)` bytes, arena scratch, freed at
statement end). A failure in pass 1 leaves `x` untouched. Pass 2 compacts.

**distinct.** Pass 1 marks the first occurrence of each value into the bitmap
using a scratch hash index of **element indices** (8 bytes per element, not
element payloads) with the language's equality; pass 2 compacts. Cannot fail
(equality is total for comparable types).

Payload relocation is safe: `collections::get` returns an owned copy for every
element kind (bug-538, plan-134-D), which is why plan-134-H lifted G24.

Risk: the compaction primitive (entry/payload moves, graph frees). The shape
already exists for one element (`removeAt`); the new code generalizes it to a keep set.

## Phases

### Phase 1 — Callback fallibility and call-target spellings (measure first)

- [x] Determine whether codegen can know that a `FUNC` value passed to a builtin
      cannot fail (a named FUNC with no `FAIL`/fallible call, a `LAMBDA`, a
      builtin predicate like `isPositive`). Read `inline_builtin_is_infallible`
      (`builtins/mod.rs`) and how `lower_filter` propagates a callback error.
      Record the answer here; it sizes how often the `G-atomic` decline (plan-142-A Open Decision 1, resolved) applies in C and D.
      **Answer: no.** `inline_builtin_is_infallible` answers for builtin *call
      targets* only (`len`, `bits::*`, the pure collection queries), never for a
      `FUNC` value. The one project-function fallibility oracle,
      `ir::fallible::Fallibility` (`src/ir/fallible.rs`, the fixpoint behind the
      inline-`TRAP` desugar), lives in IR lowering and never reaches codegen; and a
      `FUNC` value at a builtin call site may be any function or lambda. Every
      callback call is checked at run time: `lower_filter` tests
      `RESULT_TAG_REGISTER` after each predicate call and routes a failure through
      `emit_callback_failure_exit` (`func_filter.rs`, the `filter_call_ok` branch).
      So every arm treats every callback as fallible: it calls all of them before
      its first write, keeping their results in the function's self-update
      scratch (Correction B1). That scratch holds results of any width — a
      variable-width result is a pointer — so `G-atomic` (decline when results
      cannot be held without a copy of `x`) never has to fire; C and D record that
      against their own arms.
- [x] Record every call-target spelling of these five after lowering, from one
      `--nir` build of a probe calling each (store the grep output here).
      Probe: `x = filter(x, isPositive)`, `take(x, 4)`, `drop(x, 1)`, `mid(x, 0, 2)`,
      `distinct(x)` on a `List OF Integer`, plus `take` on a `List OF String`;
      `mfb build --nir`, then `grep -oE '"target": ?"[^"]*"' probe2.nir`:
      ```
      "target": "#collections_distinct$Integer"
      "target": "#collections_drop$Integer"
      "target": "#collections_take$Integer"
      "target": "#collections_take$String"
      "target": "collections.filter"
      "target": "collections.mid"
      ```
      `take`/`drop`/`distinct` (`Body::Mfb`) arrive as the internalized monomorph of
      `__collections_X OF T` (`internal_name::internalize` swaps `__` for `#`);
      `filter` (`abi_inline`) and `mid` (`Intrinsic`) keep the qualified name.
- [x] Add `self_update_builtin` with a unit test per spelling.
      (`self_update_builtin_names_every_spelling` in `self_update.rs`.)

Acceptance: both answers recorded here with their evidence;
`cargo test --bin mfb self_update_builtin` → pass (est. 10 min).
Verified 2026-09-21: `cargo test --bin mfb self_update` → `test result: ok. 4 passed`
(includes `self_update_builtin_names_every_spelling`).
Commit: 0f6e69a36

### Phase 2 — The compaction primitive

- [x] `lower_list_compact_in_place` (bitmap and range forms) in `list_mutate.rs`,
      for fixed-width (kind-2) and entry-based lists, freeing dropped graphs.
      Landed in its own file, `src/codegen/collection/list/list_compact.rs`
      (`KeepSource::{Marks, Range}`); marks are one byte per element, not bits
      (Correction B6). Entry lists probe payload order first (Correction B3); the
      out-of-order path reuses `emit_repack_list_data`, now `pub(crate)`.
- [x] Unit codegen test in `src/codegen/builtins/tests/inplace.rs` for both forms.
      In a sibling file, `src/codegen/builtins/tests/inplace_compact.rs`: range form
      on a fixed-width list is one block move (`compact_k2_range_*`, no loop, no
      probe); marks form walks the elements (`compact_k2_loop`); both forms on an
      entry list emit the order probe and both paths (`compact_probe_loop`,
      `compact_ord_loop`, `compact_dis_loop`, `compact_dis_repack`). The primitive
      has no entry point but the arms, so the tests drive it through them and
      Phases 2–3 land in one commit (Correction B6).

Acceptance: `cargo test --bin mfb compact_in_place` → pass (est. 3 min).
Verified 2026-09-21: `test result: ok. 3 passed; 0 failed` (after correcting the
first run's own assertion: the range labels are `compact_k2_range_{wloop,btail,done}`,
never a bare `compact_k2_range`).
Commit: —

### Phase 3 — The five arms

- [x] `take`, `drop`, `mid` arms (range form, arguments validated first).
      `src/codegen/collection/assign/builder_inplace_shrink.rs`. `take`/`drop` clamp
      (they are total — `mfb man collections take`); `mid` repeats
      `lower_list_mid`'s five checks in its order and raises the same
      `collections.mid` `ErrIndexOutOfRange` before compacting.
- [x] `filter` arm (two passes, bitmap). Pass 1 stores each verdict in the
      function's self-update scratch (Correction B1); a failing predicate routes
      through `emit_callback_failure_exit` with `x` untouched.
- [x] `distinct` arm (index hash, bitmap). No hash: the copying body's own
      algorithm and equality (Correction B2).
- [x] Flip the five table rows to `Arm`, add them to `SELF_UPDATE_ARMS`, flip their
      `cases.tsv` lines to `arm`. `resolve_self_update` now matches through
      `self_update_builtin`, so `#collections_take$T` reaches the arm. Differential
      probe (each arm vs the copying call on a copy, over `Integer`, in-order
      `String`, an out-of-order `String` list built with `prepend`/`insert`/`set`, a
      record with a `String` field, and a 500-iteration append/filter/drop loop):
      27/27 `ok`, `arena.0.alloc_calls 2884` = `free_calls 2884`, `live_bytes 0`;
      its `--ncode` shows every self-update in `main` took its arm (marker counts
      5 filter, 5 take, 7 drop, 4 mid, 3 distinct = the source's).
- [x] `tests/runtime/rt_inplace_failure_atomic.rs` (+ stanza): `filter` with a
      predicate that fails on the 3rd element, `take`/`mid` with an invalid count,
      each inside a function-level `TRAP` whose handler prints `x`: prints the
      original list. `take` has no invalid count (Correction B5), so the cases are
      `filter` (Integer and String lists) and `mid` with a negative count, a range
      past the end, and a negative start: `test result: ok. 1 passed`.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_inplace_failure_atomic`
→ pass; the five `cases.tsv` lines are `arm` (est. 10 min).
Verified 2026-09-21: `self_update` → `4 passed` (the matrix fires all five new arms at
S1); `rt_inplace_failure_atomic` → `1 passed`; `rt_inplace_self_update` → `1 passed`
(141s, all 64 lines; `grep -c 'pending:B' cases.tsv` → 0). Full artifact-gate with
the arms: `2078 golden(s) checked, 0 diff(s)` — no committed fixture self-updates
one of the five.
Commit: a70924e35

### Phase 4 — Expected outputs

- [x] `examples/network-client/src/main.mfb` self-updates one of these (measured:
      `rg -lP "(\w+) = collections::(filter|take|drop|mid|distinct)\(\1\b" tests examples --glob '*.mfb'`
      → 1 file). If a committed golden covers it, regenerate it; the diff must be
      the arm replacing the copy. Run `scripts/test-accept.sh target/debug/mfb target/accept-actual`
      on the collections fixtures.
      No committed golden covers it: `examples/network-client` holds only
      `project.json` and `src/` (the artifact gate sweeps `tests/`), and the full
      gate above is 0 diffs. Built with the new compiler (`--ncode`), both
      `buf = collections::drop(buf, nl + 1)` statements take the arm (`runTcp` and
      `runTls` each carry one `inplace_drop_count` slot).

Acceptance: acceptance run green on `tests/rt-behavior/collections` (est. 10 min).
Verified 2026-09-21: `scripts/test-accept.sh <debug mfb> <dir> 'rt-behavior/collections/*'`
→ `acceptance tests passed (65 test(s) ran)`.
Commit: a70924e35

## Validation Plan

- Tests: `rt_inplace_self_update` rows for the five; `rt_inplace_failure_atomic`;
  the matrix unit test.
- Runtime proof: alloc count flat in `N` for each (the harness).
- Doc sync: none here (letter I).

## Corrections

- **B1 (Phase 1): no per-statement scratch allocation.** §3 put `filter`'s keep
  bitmap (and `distinct`'s index) in "arena scratch, freed at statement end". The
  harness bound plan-142-A holds every arm to — `count(2N) - count(N) < N/8` over
  every `alloc_calls` (A Correction A6) — counts that block exactly as it counts a
  copy, so that design fails the acceptance it is measured by. Corrected design: a
  function whose body holds a self-update needing scratch gets one hidden frame
  slot, `su_scratch`, holding a `List OF Integer` block used as raw bytes. It is
  zero at entry, grown geometrically on demand (so N statements allocate it
  O(log n) times, not N), and freed on every exit of the function by an ordinary
  function-level `ActiveCleanup::OwnedValue` — the same drop, with the same null
  guard and prologue zeroing, a `LET` list gets. The arms hold their per-element
  marks (1 byte per element) there.
- **B2 (Phase 3): `distinct` needs no hash index.** §3 planned a scratch hash of
  element indices. The copying lowering is `__collections_distinct`: for each
  element, `collections::contains(result, item)` — a linear scan, O(n·u). The arm
  keeps that algorithm and its equality exactly: element `i` is compared with every
  earlier *kept* element through `emit_collection_payload_match_branch`, the
  compare `lower_contains` uses. Same result, same complexity, n bytes of marks.
- **B3 (Phase 2): compaction handles out-of-order payloads.** §3's "survivor `i`
  moves to position `k` (entry and payload)" assumes payloads sit in entry order.
  They need not: `insert`/`prepend` put new payloads at the data tail, a
  length-changing `set` leaves dead bytes, and a sorted list permutes entries over
  unmoved data (`lower_list_mid`'s `mid_list_disordered` probe exists for exactly
  this). The primitive probes first. In order (each payload at the aligned end of
  the previous): one pass moves each survivor's entry and payload down; the data
  region ends tight. Out of order: the survivors' entries move down and the
  dropped payloads stay behind as dead bytes, which lists already tolerate
  (`emit_repack_list_data`'s contract); when the dead bytes then exceed the live
  ones, the same repack runs, so a loop of shrinks cannot grow the block without
  bound, and the repack is paid for by at least as many bytes dropped.
- **B4 (Phase 3): a pre-existing bug in the copying path, fixed first.** Writing the
  `filter` arm's failure exit, `emit_callback_failure_exit` turned out to emit a
  bare `ret` when no inline-`TRAP` capture was active: a callback failing inside
  `filter`/`transform`/`forEach`/`reduce`/`sortBy`/`mapValues`/`findLastIndex`
  skipped the enclosing function-level `TRAP` and the scope's frees. Fixed in its
  own commit (`0157f7b80`, with `tests/runtime/rt_callback_failure_reaches_trap.rs`,
  RED on all seven before the fix) — the arm's failure path relies on it.
- **B5 (Phase 3): `take` cannot fail.** The atomicity task named "`take`/`mid` with
  an invalid count", but `take` and `drop` are total — every count clamps
  (`mfb man collections take`: "every `Integer` value of `count` is accepted and no
  index is ever rejected"). Only `mid` raises; its three failing shapes are the
  cases, with `filter`'s callback failure.
- **B6 (Phase 2): marks are bytes, and Phases 2–3 land together.** One byte per
  element instead of one bit: n bytes of reused scratch, and no bit arithmetic on
  every test. The primitive's only callers are the arms, so its unit test drives it
  through them and cannot land before them.

## Summary

One new primitive (single-pass compaction) serves all five; the risk is in it.
Callback fallibility is measured first because C and D depend on the answer.
