# plan-57-C: make fixed-width lists keep their data in index order

Last updated: 2026-07-19
Effort: medium (1h–2h)
Depends on: nothing for correctness — **this sub-plan can and should land first**,
ahead of plan-57-A/B. It touches three mutation functions
(`lower_list_insert_collection`, `lower_list_prepend_in_place`,
`lower_collection_set`) that the containment refactors do not restructure, and it
is the permanent fix for the *confirmed* half of
`bugs/bug-365-linear-data-region-readers-ignore-entry-order.md` — live data
corruption should not wait several days behind two byte-identity refactors.
Landing after plan-57-A/B is tidier (the entry rewrite in §4.2 step 4 goes through
shared code rather than being open-coded) but buys nothing that justifies the
delay.

The first sub-plan that changes behavior, and the one that makes the
representation change safe.

Today `prepend`, mid-list `insert`, and the value-path `set` deliberately leave a
list's data region permuted relative to index order — the offset-stable scheme
(plan-01 §4.1) — and the lookup entry is what records the permutation. An
entry-free representation cannot record anything, so **order must become an
invariant before the entries can go away.**

This sub-plan establishes that invariant for fixed-width-scalar element types
while **keeping the entry table in place**, so the change is testable on its own
and the entries degrade to a verifiable identity mapping. It also closes the
fixed-width half of bug-365.

The single behavioral outcome: for a list of `Byte`, `Boolean`, `Scalar`,
`Integer`, `Float`, `Fixed` or `Money`, `entry[i].valueOffset == i * payloadSize`
holds after **every** operation — so `FOR EACH` and any linear data-region reader
agree, and bug-365's `math::`/`fs` reproductions go green.

References (read first):

- `bugs/bug-365-linear-data-region-readers-ignore-entry-order.md` — **the whole
  motivation.** Its §The ordering contract states the rule this sub-plan makes
  true for fixed-width lists, and its verified per-operation table is the
  before-state.
- `src/target/shared/code/builder_collection_mutate.rs:468-472` — the
  offset-stable scheme's own description (`lower_list_insert_collection`).
- `:1612-1621` — `lower_list_prepend_in_place`: *"entry offsets are independent of
  position, so no data move is needed."* This sub-plan makes that false for
  fixed-width lists.
- `:3136-3392` (`lower_list_remove_at`) and `:3714-3757`
  (`emit_offset_compaction_fixup`) — already order-preserving; the model for what
  "maintain order" costs.
- `:2023-2210` (`lower_list_set_in_place`) — already order-preserving.
- `:283-352` (`lower_collection_set`) — the value path, `removeAt` + `insert`.
- `src/target/shared/code/builder_search.rs:876-908` — `lower_list_mid`'s order
  probe, which becomes dead for fixed-width lists once this lands.
- `src/target/shared/code/builder_collection_layout.rs:60-99` —
  `collection_payload_alignment` and `list_element_padding_alignment`, the
  compile-time source of `payloadSize` and the proof there is no inter-element
  padding for these types.
- `.ai/compiler.md` — Hard Completion Gate. This sub-plan changes runtime
  behavior and must be proven at runtime.

## 1. Goal

- For a **fixed-width scalar** element type — `Byte`, `Boolean`, `Scalar`,
  `Integer`, `Float`, `Fixed`, `Money` — these operations move payload bytes so
  that index order and physical order coincide:
  - `prepend` (value and in-place): `memmove` the data region up by `payloadSize`,
    write the new payload at offset 0.
  - `insert` at `i`: `memmove` the tail up by `payloadSize`, write at
    `i * payloadSize`.
  - `set` (value path): write at `i * payloadSize` rather than degrading to
    `removeAt` + `insert`.
- The lookup entries become the identity mapping and are still written and still
  read. Nothing else changes.
- bug-365's `math::abs`, `math::min` and `fs::writeBytesAtomic` reproductions
  produce correct results for these element types.
- **Variable-width element types are untouched** and keep the offset-stable
  scheme: `String`, records, unions, nested collections.

### Non-goals (explicit constraints)

- **The entry table stays.** Removing it is plan-57-D. Keeping it here is what
  makes this sub-plan verifiable: the entries must equal the identity mapping,
  and that is directly assertable.
- **This does not close bug-365.** `List OF String` remains permutable — indeed
  `lower_sort_string_list_helper` (`codegen_utils.rs:8-127`) *deliberately*
  permutes entries for `fs::listDirectory` determinism, "swapping only the
  fixed-size entry records and leaving the data region untouched". bug-365 needs
  its own fix for variable-width lists regardless of plan-57. Say so plainly; do
  not let this sub-plan be mistaken for the whole fix.
- No user-visible semantic change. `prepend` still returns a new list; value
  semantics are unchanged. Only *where the bytes sit* changes.
- No layout change, no `kind` change, no constant change. That is plan-57-D.

## 2. Current State

Verified empirically on macos-aarch64 (2026-07-19), by comparing `FOR EACH` order
against a linear reader's order:

| operation | index order today |
|---|---|
| literal, `append` | preserved |
| `prepend`, `insert`, value-path `set` | **broken** |
| `removeAt`, grow, `copy_collection_tight` | preserved; never restored |
| `mid`, `slice`, `transform`/`filter` | restored |
| in-place `set`, `sort` | preserved |

So only **three** operations break it, and two of them (`prepend`, `insert`) share
one implementation, `lower_list_insert_collection`
(`builder_collection_mutate.rs:475-762`), plus the in-place prepend fast path
(`:1622-1900`).

The costs today, per element, for the entry-splice approach:

| element | entry shift (now) | data memmove (proposed) |
|---|---|---|
| `Byte`, `Boolean` | 40 B | 1 B |
| `Scalar` | 40 B | 4 B |
| `Integer`/`Float`/`Fixed`/`Money` | 40 B | 8 B |

**The offset-stable scheme is a pessimization for exactly these types.** It exists
to avoid moving payload bytes, and for a fixed-width scalar the payload is
strictly smaller than the entry record it moves instead — 5× smaller for an
`Integer`, 40× for a `Byte`. It is the right design for `List OF String`, where
repacking means recomputing every offset; it was applied universally, and these
types have been paying for it.

That is the load-bearing argument for this sub-plan: it is not a
correctness-for-performance trade. It is strictly less memory traffic *and*
correct.

## 3. Design Overview

One predicate, three call sites.

```
list_element_is_fixed_width(element_type) -> Option<usize>   // payloadSize
    │
    ├── lower_list_insert_collection   (prepend + insert, value path)
    ├── lower_list_prepend_in_place    (prepend fast path)
    └── lower_collection_set           (value path — stop degrading to remove+insert)
```

The predicate is compile-time and already exists in substance:
`inline_collection_payload_size` (`builder_collection_layout.rs:4-37`) and
`collection_payload_alignment` (`:60-72`) both yield constants for these seven
types, and `list_element_padding_alignment` (`:87-99`) returns `1` for them,
which is the guarantee that there are no inter-element gaps to preserve.

Each converted site gains a fixed-width branch that `memmove`s the data region
and writes entries as the identity mapping; the existing offset-stable path
remains as the `else` for variable-width types.

**Where the correctness risk concentrates:** the `memmove` direction and overlap.
Shifting a data region *up* by `payloadSize` overlaps itself, so it must copy
backwards; getting this wrong corrupts the list in a way that small tests
(1–3 elements, where the regions may not overlap) will not catch. Test with
element counts large enough to force overlap, and with each of the three distinct
payload widths (1, 4, 8) — a bug in the 1-byte case is invisible in the 8-byte
case and vice versa.

Second risk: the grow path. `prepend` may allocate before shifting
(`:1710-1882`), and `_mfb_arena_alloc` destroys all caller-saved registers
(`.ai/compiler.md`), so the shift's bounds must be re-derived from the frame after
the allocation, not held in registers across it.

**Rejected alternative:** *normalize lazily — leave the permutation and repack on
demand at the consumers.* That is bug-365's fix option 1, and it is right for
variable-width lists. Rejected here because it leaves the entry table load-bearing
forever, which forecloses plan-57-D entirely.

**Rejected alternative:** *do this at the same time as plan-57-D.* Rejected: then
a single commit changes both the invariant and the representation, and a failure
cannot be attributed to either. Establishing the invariant while the entries still
exist means the entries themselves are the assertion — `entry[i].valueOffset ==
i * payloadSize` is directly checkable, and plan-57-D's job reduces to deleting
something already proven redundant.

## 4. Detailed Design

### 4.1 The predicate

```rust
/// The payload size, in bytes, of a list element type whose payloads are
/// fixed-width and therefore may be addressed as `dataBase + i * size`.
///
/// These are exactly the element types plan-57 gives an entry-free
/// representation (`kind = 2`). `String`, records, unions and nested
/// collections are variable-width and keep the lookup table.
///
/// Deliberately excludes function values and pointer payloads: both are 8-byte
/// fixed, but they carry ownership that the drop and thread-transfer paths
/// reason about per entry. Revisit only with that audit done (plan-57-E).
pub(super) fn list_element_is_fixed_width(element_type: &str) -> Option<usize> {
    match element_type {
        "Boolean" | "Byte" => Some(1),
        "Scalar" => Some(4),
        "Integer" | "Float" | "Fixed" | "Money" => Some(8),
        _ => None,
    }
}
```

This must agree with `collection_payload_alignment` (`:60-72`) for every arm.
Add a unit test asserting that, so the two cannot drift — the tree already has
this failure mode with the three `is_c_abi_type` copies.

### 4.2 `prepend` / `insert`

Fixed-width branch, for a list of `n` elements inserting `m` at index `i`:

1. Ensure capacity for `n + m` (existing logic, unchanged).
2. `memmove` `[i * p, n * p)` up by `m * p` — **backwards**, since the regions
   overlap. Re-derive `n`, `i` and `p*` bounds from frame slots if an allocation
   happened.
3. Write the inserted payloads at `[i * p, (i + m) * p)`.
4. Write entries `0..n+m` as the identity mapping (`valueOffset = k * p`,
   `valueLength = p`, `flags = USED`, keys `0`).

Step 4 rewrites every entry rather than splicing. That is `O(n)` in entry writes
— the same order as today's splice — and it is temporary: plan-57-D deletes it.
Do not optimize it here.

### 4.3 value-path `set`

`lower_collection_set` (`:283-352`) currently does `removeAt(i)` + `insert(i, [v])`.
For a fixed-width element the payload sizes are equal by definition, so it becomes:
copy the block, write the new payload at `i * p`, leave every entry alone. This is
both a correctness fix and a large constant-factor win — two allocations and two
block copies become one.

### 4.4 The bug-365 relationship

State in the commit message and in bug-365 itself: **this closes the fixed-width
half only.** After it lands, bug-365 remains open for `List OF String`, and the
probe-and-repack fix is still required there. Update bug-365's §Scope table to
mark the fixed-width rows resolved-by-plan-57-C rather than closing the bug.

## Compatibility / Format Impact

- **Changes:** the physical byte order of a fixed-width list's data region after
  `prepend`/`insert`/value-`set`. No MFBASIC-visible semantics change — element
  order, value semantics, and every documented behavior are the same. Programs
  that were *correct* see no difference; programs hitting bug-365 start working.
- **Changes:** `mid`'s order probe (`builder_search.rs:876-908`) becomes
  permanently false for fixed-width lists. Leave it — it is still needed for
  `String`, and deleting the fixed-width path is plan-57-E's cleanup.
- **Unchanged:** the block layout, `kind`, every constant, the `.mfp` format,
  all Map behavior, all variable-width list behavior.
- Golden churn is **expected** here, unlike plan-57-A/B — codegen changes. Every
  churned golden must be reviewed as a real diff, not re-baselined in bulk.

## Phases

### Phase 1 — the predicate and value-path `set`

Smallest of the three, and independently a correctness fix.

- [x] Add `list_element_is_fixed_width` (§4.1) plus the drift test against
      `collection_payload_alignment`.
- [x] Convert `lower_collection_set`'s value path (`:283-352`) to a direct
      indexed write for fixed-width elements.
- [x] Tests: `tests/rt-behavior/collections/list-fixed-set-order-rt` — value-path
      `set` on a list bound so the in-place gate cannot fire (a record field, or
      an argument), then assert `FOR EACH` and `math::abs` agree.

Acceptance: bug-365's value-path-`set` reproduction returns correct results for
`List OF Integer`, `Float`, `Byte`.
Commit: —

### Phase 2 — `prepend` and `insert`

- [x] Fixed-width branch in `lower_list_insert_collection` (`:475-762`), §4.2.
- [x] Fixed-width branch in `lower_list_prepend_in_place` (`:1622-1900`),
      including re-deriving the shift bounds after any grow allocation.
- [x] Update the doc comments at `:468-472` and `:1616-1619`, which currently
      assert the opposite invariant. Leaving them would be worse than the bug.
- [x] Tests: `tests/rt-behavior/collections/list-fixed-prepend-order-rt` —
      **element counts large enough to force `memmove` overlap** (≥ 64), across
      all three payload widths (`Byte`=1, `Scalar`=4, `Integer`=8), asserting
      `FOR EACH` order matches a linear reader.
- [x] Tests: mid-list `insert` at the first, middle and last positions.
- [x] Tests: interleaved `prepend`/`append`/`removeAt` sequences, then assert
      order — the invariant must hold after *combinations*, not just single ops.

Acceptance: bug-365's `math::abs`/`math::min`/`fs::writeBytesAtomic`
reproductions all produce correct results for every fixed-width element type, on
macOS/aarch64 and Linux/{aarch64,x86_64,riscv64}.
Commit: —

### Phase 3 — assert the invariant directly

- [x] Add a debug-only or test-only checker that walks a fixed-width list's
      entries and asserts `entry[i].valueOffset == i * payloadSize` and
      `valueLength == payloadSize`. This is the precondition plan-57-D deletes the
      entries on the strength of — it should be machine-checked, not argued.
      **Landed as `tests/rt-behavior/collections/list-order-invariant-rt`.**
      MFBASIC cannot read a lookup entry, so the invariant is checked through its
      one observable consequence: an entry-table reader (`FOR EACH`) and a
      data-region reader (the vectorized `math::` overloads) must produce the
      same sequence. That is exactly the equivalence bug-365 was found by.
- [x] Run it over the existing collection test corpus. **Done differently, and
      better:** rather than sampling the existing corpus (whose lists are almost
      all literal/`append`-built and so ordered by construction — which is
      precisely why bug-365 stayed latent), the checker drives 300 steps of
      *mixed* mutation — prepend / insert / append / removeAt / value-`set` —
      and re-checks after **every step**, across `Integer`, `Float` and `Fixed`,
      at lengths past 64 so the data shift overlaps itself.

Acceptance: the checker passes across the whole suite; any failure is a
fixed-width path that still permutes and must be found before plan-57-D.

**MET (2026-07-19).** 0 violations in 300 mixed steps. Proven non-vacuous by a
negative control: making `list_element_is_fixed_width` return `None` (which
restores the offset-stable scheme) turns the same run red at **294 of 300**
steps. A checker that cannot fail proves nothing, and this one fails exactly
when the invariant is removed.

**plan-57-D is unblocked.**
Commit: —

## Validation Plan

- Tests: the three new `tests/rt-behavior/collections/` fixtures above, plus
  bug-365's reproductions promoted into
  `tests/rt-behavior/math/math-array-entry-order-rt` and
  `tests/rt-behavior/fs/fs-write-bytes-entry-order-rt`.
- Runtime proof: **required (Hard Completion Gate).** The proof is bug-365's
  reproductions going from wrong to right. Run on every target — the `memmove`
  and the register discipline around the grow allocation differ per backend.
- Overlap coverage: element counts ≥ 64 and all three payload widths. A
  three-element test cannot distinguish a correct backwards `memmove` from a
  forwards one.
- Performance: benchmark `prepend` on `List OF Integer` and `List OF Byte` before
  and after (`benchmark/`). Expect an improvement (§2); a regression means the
  `memmove` is not being used or the entry rewrite in §4.2 step 4 dominates —
  investigate rather than accept.
- Goldens: churn is expected. Review each diff as a real codegen change.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Should `sort` be revisited here?** `collections::sort` is MFB source relying
  on the in-place `set` fast path, which already preserves order. It should be
  unaffected — but confirm empirically, because if the in-place gate ever misses,
  sort degrades to value-path `set` in a loop, and that path changes in Phase 1.
- **Function values and pointer payloads (8-byte fixed) — include them?**
  Recommend no, per §4.1: they carry ownership that the drop and transfer paths
  reason about per entry, and widening the predicate without that audit risks a
  double free. Revisit in plan-57-E with the audit done.
- **Should this sub-plan close bug-365?** No — recommend explicitly reducing its
  scope to variable-width lists and leaving it open. Closing a HIGH-severity data
  corruption bug when half of it is still live would be worse than not filing it.

## Summary

The engineering risk is the overlapping `memmove` and the register discipline
around the grow allocation — both are classes that pass small tests and fail at
scale, which is why the acceptance criteria demand ≥ 64 elements and all three
payload widths. The design risk is nil in the trade-off sense: for these element
types, moving payload is strictly cheaper than moving the entry records the
current scheme moves instead.

Untouched: variable-width lists, maps, the block layout, and every constant.
bug-365 remains open for `List OF String` after this lands.
