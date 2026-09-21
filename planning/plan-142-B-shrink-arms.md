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

- [ ] Determine whether codegen can know that a `FUNC` value passed to a builtin
      cannot fail (a named FUNC with no `FAIL`/fallible call, a `LAMBDA`, a
      builtin predicate like `isPositive`). Read `inline_builtin_is_infallible`
      (`builtins/mod.rs`) and how `lower_filter` propagates a callback error.
      Record the answer here; it sizes how often the `G-atomic` decline (plan-142-A Open Decision 1, resolved) applies in C and D.
- [ ] Record every call-target spelling of these five after lowering, from one
      `--nir` build of a probe calling each (store the grep output here).
- [ ] Add `self_update_builtin` with a unit test per spelling.

Acceptance: both answers recorded here with their evidence;
`cargo test --bin mfb self_update_builtin` → pass (est. 10 min).
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

## Summary

One new primitive (single-pass compaction) serves all five; the risk is in it.
Callback fallibility is measured first because C and D depend on the answer.
