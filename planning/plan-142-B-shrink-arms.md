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
Commit: —

### Phase 2 — The compaction primitive

- [ ] `lower_list_compact_in_place` (bitmap and range forms) in `list_mutate.rs`,
      for fixed-width (kind-2) and entry-based lists, freeing dropped graphs.
- [ ] Unit codegen test in `src/codegen/builtins/tests/inplace.rs` for both forms.

Acceptance: `cargo test --bin mfb compact_in_place` → pass (est. 3 min).
Commit: —

### Phase 3 — The five arms

- [ ] `take`, `drop`, `mid` arms (range form, arguments validated first).
- [ ] `filter` arm (two passes, bitmap).
- [ ] `distinct` arm (index hash, bitmap).
- [ ] Flip the five table rows to `Arm`, add them to `SELF_UPDATE_ARMS`, flip their
      `cases.tsv` lines to `arm`.
- [ ] `tests/runtime/rt_inplace_failure_atomic.rs` (+ stanza): `filter` with a
      predicate that fails on the 3rd element, `take`/`mid` with an invalid count,
      each inside a function-level `TRAP` whose handler prints `x`: prints the
      original list.

Acceptance: `cargo test --bin mfb self_update && cargo test --test rt_inplace_self_update --test rt_inplace_failure_atomic`
→ pass; the five `cases.tsv` lines are `arm` (est. 10 min).
Commit: —

### Phase 4 — Expected outputs

- [ ] `examples/network-client/src/main.mfb` self-updates one of these (measured:
      `rg -lP "(\w+) = collections::(filter|take|drop|mid|distinct)\(\1\b" tests examples --glob '*.mfb'`
      → 1 file). If a committed golden covers it, regenerate it; the diff must be
      the arm replacing the copy. Run `scripts/test-accept.sh target/debug/mfb target/accept-actual`
      on the collections fixtures.

Acceptance: acceptance run green on `tests/rt-behavior/collections` (est. 10 min).
Commit: —

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

## Summary

One new primitive (single-pass compaction) serves all five; the risk is in it.
Callback fallibility is measured first because C and D depend on the answer.
