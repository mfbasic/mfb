# plan-01 — Collection Memory Management

Last updated: 2026-06-25

This document is the **normative definition and implementation plan** for
replacing the collection mutation codegen with a faster underlying memory
manager. It is purely a **runtime/codegen** change. It does **not** change the
language: value semantics, ownership, copy-on-pass, scope-drop frees, and the
`MUT`-vs-`LET` rules are all preserved exactly. The only thing that changes is
*how the bytes behind a `List`/`Map` are allocated, grown, and copied*.

The on-arena layout itself does **not** change either — it is already specified
in `specifications/memory_layouts.md` (§*Collection Layout*) with the exact
fields this plan needs (`capacity` separate from `count`, `dataCapacity`
separate from `dataLength`, data-region addressing relative to the data base).
Today's codegen ignores those fields; this plan makes them do their job.

Referenced code:

- `src/target/shared/code/builder_collection_updates.rs` — append/prepend/insert/remove/concat + the entry-copy loops
- `src/target/shared/code/builder_collection_layout.rs` — `emit_collection_data_pointer`, payload packing, literal builders
- `src/target/shared/code/mod.rs` — the `COLLECTION_*` layout constants
- `src/builtins/collections.rs`, `src/builtins/collections_package.mfb` — the native shims and the MFBASIC higher-order functions built on top of them

See also: `specifications/memory_layouts.md` (layout contract),
`specifications/standard_package.md` §6 (the `collections` surface),
`specifications/mfbasic.md` §14 (ownership / `MUT` destructive-update wording).

---

## 1. Problem

Every collection-building primitive today **reallocates a fresh block and
re-packs the entire collection on every call**, and the copy is done **one byte
at a time**. Concretely, `collections::append` routes through
`lower_list_insert_collection` (`builder_collection_updates.rs:347`), which:

1. `arena_alloc`s a brand-new block sized `count+1`. There is **no headroom**:
   the header writer (`emit_write_list_header_from_registers:630`) sets
   `capacity = count` and `dataCapacity = dataLength`, so the *next* append is
   always full and must reallocate again.
2. Re-copies all existing entries by **repacking the data region in entry
   order** through a running `dest_data_offset` cursor — entry by entry.
3. Copies each payload with a scalar `ldrb`/`strb` **byte loop**
   (`emit_copy_collection_entries:739`), so even an 8-byte `Integer` is moved
   one byte per iteration.

Two independent costs compound:

- **No capacity headroom** → realloc + full copy on *every* mutation. Append in
  a loop is **O(n²)**.
- **Byte-at-a-time repack** → a large constant factor on top, and it throws
  away the data region's offset-stability (see §3).

Measured (`benchmark/append`, 1000 int + 1000 string appends): mfb spends
~44 ms of real work — **2× slower than CPython** — despite mfb *beating* CPython
on startup (`benchmark/empty`) and on compute (`benchmark/primes`). The append
path, not arithmetic codegen, is the bottleneck.

## 2. Goals and non-goals

**Goals**

- Amortized **O(1)** append/grow via capacity headroom (geometric growth).
- Bulk **`memcpy`/word copies** instead of byte loops.
- **Offset-stable** data region: copy surviving payloads verbatim; never repack
  on insert/append.
- No wasteful over-allocation for small collections (see §5 growth policy).

**Non-goals / invariants (must not regress)**

- **No language-semantics change.** `collections::append(x, v)` still yields a
  logically-new collection value; ownership, copy-on-pass, borrows, and
  scope-drop frees behave exactly as today. This is *only* memory management.
- **No layout-format change.** The `memory_layouts.md` collection layout is
  unchanged. `flagsVersion` stays at version 1. Existing golden `.bin`/runtime
  expectations for *observable* values are unchanged.
- **No new observable state.** `capacity` and `dataCapacity` are not observable
  from MFBASIC (`len()` returns `count`); growing them changes nothing a program
  can see.

## 3. Why the layout already supports this

From `memory_layouts.md` goals: *"Copying a collection snapshot can be
implemented as one contiguous memory copy"* and *"Collection mutation can
minimize payload copying by moving lookup metadata instead of moving packed
item bytes."* The format was designed for exactly this plan:

- **`capacity` ≥ `count`** and **`dataCapacity` ≥ `dataLength`** are already
  distinct header fields. `emit_collection_data_pointer`
  (`builder_collection_layout.rs:1355`) already computes the data-region base as
  `HEADER + capacity * ENTRY_SIZE` — i.e. spare lookup slots between the live
  entries and the data region are **already** accounted for. The codegen just
  never sets `capacity > count`.
- **`valueOffset` is relative to the data-region base.** Because entries address
  payloads by offset, **the data region does not need to be in entry order.**
  An old payload keeps its offset no matter where its entry moves in the table.
  This is the property that lets insert/append copy the data region *verbatim*
  and splice only the lookup table — the "2 table copies + 1 data copy" shape.

## 4. Design

### 4.1 Offset-stable, memcpy-based buffer production (constant-factor win)

Replace the per-entry/per-byte repack with the offset-stable scheme. For an
**insert at index `i`** into a list of `n` entries (append = `i == n`,
prepend = `i == 0`):

```
memcpy(dst.data,        src.data, src.dataLength)   ; verbatim; all offsets stay valid
append new payload at dst.data + src.dataLength      ; the only new bytes
memcpy(dst.table[0..i), src.table[0..i))             ; head lookup entries
write dst.table[i]                                   ; the new entry (valueOffset = old dataLength)
memcpy(dst.table[i+1..], src.table[i..n))            ; tail lookup entries
```

Three bulk copies plus one entry write — no payload repack, no byte loop. The
inner copy helper (`emit_copy_collection_entries` / `emit_copy_one_map_entry`)
is replaced by word-sized / `memcpy`-style block copies. This alone is correct
with `capacity == count` and is a pure constant-factor improvement (still O(n)
per op); it is the foundation Layer 4.2 builds on.

`Map` differs only in that an entry carries a key payload too, and the data
region honors per-payload alignment (`emit_align_offset_register`). Verbatim
data copy preserves that alignment (offsets and the data base alignment are
unchanged); only the one appended payload must be aligned.

### 4.2 Capacity headroom + in-place growth (amortized O(1))

Result buffers are **over-allocated** with geometric headroom so `capacity >
count` and `dataCapacity > dataLength`. Then the documented `MUT`
destructive-update path (`mfbasic.md` §14: *"the compiler performs the update
destructively in place"*) is honored for real:

- When a mutating builder's result is assigned back to the **same uniquely-owned
  `MUT` binding** (`nums = collections::append(nums, v)`) and the live buffer
  has room (`count < capacity` **and** `dataLength + need ≤ dataCapacity`), the
  new entry/payload is written **in place** — no `arena_alloc`, no copy. Just
  bump `count`/`dataLength`.
- Otherwise (no room, or not the same-binding case) allocate a new buffer with
  headroom via §4.1, then proceed.

This is the amortized-O(1) lever: most appends in a `MUT` loop touch only the
spare slot; reallocation fires only on the occasional geometric grow. It relies
solely on the *already-documented* `MUT` semantics — value semantics for `LET`
snapshots are untouched (a `LET`-bound collection still produces a new buffer on
update, exactly as today).

### 4.3 Snapshots/copies are shrink-to-fit

When a collection value is **copied** (passed by value, assigned to a new
binding, embedded in a record/another collection — the existing copy-insertion
points), copy only the **used prefix** (`HEADER + count*ENTRY_SIZE +
dataLength`) and set `capacity = count`, `dataCapacity = dataLength`. Headroom is
a property of a *mutable working buffer*, not of a value; copies stay tight and
the "one contiguous memory copy" property holds over the used prefix. Observable
result is identical — this only governs allocation size.

### 4.4 Removal and hole compaction

`removeAt` / `removeKey` splice the lookup table (two block copies) and **leave
the removed payload as a hole** in the data region (offset-stable: surviving
payloads keep their offsets — no repack). `dataLength` stays as the high-water
mark; a separate `liveBytes` is not tracked in v1. To bound waste from
remove-heavy workloads, **compact during the next grow**: when a reallocation
happens anyway, re-pack live payloads tightly into the new block (the only place
repacking is justified). Optionally compact eagerly when holes exceed a
threshold (≥50% of `dataCapacity`); this is a tuning knob, not a correctness
requirement.

## 5. Growth policy

The user constraint: geometric growth is fine, but **don't waste memory on small
collections** — start small, don't pre-reserve large blocks.

- **Lookup capacity.** Start at the exact size for a known-size build (literals,
  `transform`, `mid`, …) — no headroom when the final size is known up front.
  For open-ended growth (`MUT` append loop): first grow `0 → 4`, then **×2 while
  `capacity < 1024`**, then **×1.5 above** (cap the multiplier to curb waste on
  large collections). All integer arithmetic; round up.
- **Data capacity (bytes).** Grow independently with the same shape: `0 → 32`
  bytes, then ×2 to a threshold (e.g. 64 KiB), then ×1.5, always at least
  `dataLength + need`. Fixed-width element lists (e.g. `Integer`) grow lookup and
  data in lockstep (8 B/entry); string/variable lists grow them independently.
- **Literals & known-size builders** allocate exact (`capacity = count`,
  `dataCapacity = dataLength`) — they never grow, so headroom would be pure
  waste.

Exact constants are an implementation tuning detail; the shape (small start,
geometric, taper the multiplier when large) is the contract.

## 6. Which `collections::*` use the new manager

The improvement lands in the **native codegen primitives**. Everything else —
including the MFBASIC-source higher-order functions in `collections_package.mfb`
— inherits it transparently, because those are implemented *on top of* the
native primitives (mostly `append`).

### 6.1 Native primitives that build/grow (adopt §4 directly)

| Primitive (codegen) | `collections::*` surface | Shape | Benefits from |
|---|---|---|---|
| `lower_collection_append` | `append(list,item)`, `append(list,items)` | tail growth | headroom + in-place (4.2), memcpy (4.1) |
| `lower_collection_prepend` | `prepend` | head insert | memcpy splice (4.1) |
| `lower_collection_insert` | `insert` | mid insert | memcpy splice (4.1) |
| `lower_collection_set` (list) | `set(list,…)` | overwrite one entry | offset-stable: append new payload, retag entry (4.1/4.4) |
| `lower_collection_set` (map) | `set(map,…)` | key scan → replace/append | in-place on hit; headroom append on miss |
| `lower_collection_remove_at`, `lower_list_remove_at` | `removeAt` | entry splice | table memcpy + hole (4.4) |
| `lower_collection_remove_key`, `lower_map_remove_key` | `removeKey` | key scan + splice | table memcpy + hole (4.4) |
| `lower_map_concat` | `merge` (∪) | bulk-append `b` into `a` | one data memcpy + table memcpy (4.1) |
| `lower_list_replace` | `replace(list,old,new)` | rebuild w/ substitutions | memcpy unchanged spans (4.1) |
| `lower_list_literal` | `[a, b, …]` | exact-size build | exact alloc + memcpy fill (5) |
| `lower_map_literal` | `Map{…}` | exact-size build | exact alloc (5) |
| `lower_collection_transform_call` | `transform` | accumulate via append | headroom + in-place (4.2) |
| `lower_collection_filter_call` | `filter` | accumulate via append | headroom + in-place (4.2) |
| `lower_collection_keys` / `values` / `values_builtin` / `lower_map_projection` | `keys`, `values` | build fresh list from map | exact alloc + memcpy (5) |

### 6.2 Read-only natives (unaffected by growth; still benefit from §4.3 tight copies)

`lower_list_get` / `lower_collection_get`, `get_or`, `lower_collection_has_key`,
`lower_collection_contains`, `lower_list_find_item` / `find_sublist`,
`lower_list_mid`, `lower_collection_reduce_call`, `lower_collection_for_each_call`,
`lower_collection_sum`. These don't grow a collection; no behavior change.

### 6.3 MFBASIC-source functions (inherit via §6.1, no per-function work)

Built on the native primitives in `collections_package.mfb`, so they get the
amortized-O(1) append automatically: `sort`, `sortBy`, `take`, `drop`,
`reverse`, `slice`, `reduceRight`, `any`, `all`, `findIndex`, `findLastIndex`,
`groupBy`, `mapValues`, `flatten`, `zip`, `chunks`, `window`, `distinct`,
`merge`, `partition`, `toMap`, `zipWith`, `filterEntries`. (Those that
accumulate with repeated `append` — `distinct`, `flatten`, `zip`, `chunks`,
`window`, `groupBy`, `partition` — see the largest wins.)

## 7. Phases

1. **Memcpy entry/data copy.** Replace the byte loops in
   `emit_copy_collection_entries` / `emit_copy_one_map_entry` with word/`memcpy`
   block copies. Pure constant-factor; behavior-identical. (Adds a small
   `memcpy`-style runtime helper or inlined word loop with byte tail.)
2. **Offset-stable splice.** Rework `lower_list_insert_collection` (and the map
   concat path) to the §4.1 verbatim-data + table-splice scheme. Append/prepend
   become the `i==n` / `i==0` special cases. Still `capacity == count`.
3. **Headroom on grow.** Header writer emits geometric `capacity`/`dataCapacity`
   per §5. Reads already honor `capacity` for the data base, so no read-side
   change. Verify nested-collection inlining still uses the *used prefix* size.
4. **In-place `MUT` growth.** Detect the same-uniquely-owned-`MUT` assign-back
   for the building primitives and route to an in-place fast path (bump
   `count`/`dataLength`, write into spare slot) when there is room; else fall to
   Phase 2/3. This is where amortized O(1) lands.
5. **Shrink-to-fit copies + removal holes + compaction.** Make copy-insertion
   tight (§4.3); make `removeAt`/`removeKey` leave holes; compact-on-grow (§4.4).
6. **Tuning + docs.** Settle growth constants against the benchmarks; add a
   short note to `memory_layouts.md` that `capacity`/`dataCapacity` now carry
   real headroom and document the growth shape.

Phases 1–2 are independently shippable wins with zero semantic risk; 4 is the
headline. Each phase keeps all existing golden tests green.

## 8. Tests

- **Golden invariance.** Existing `collections-*` and JSON/CSV goldens must pass
  unchanged at every phase — values are identical; only allocation differs.
- **Amortized growth.** A runtime test appending N items asserts the result and
  (via an internal probe / debug dump if available) that reallocation count is
  `O(log N)`, not `O(N)`.
- **Offset-stability.** Insert/prepend/append at head, middle, tail; verify
  payloads and offsets after each, including string (variable-length) and
  fixed-width element types, and nested-collection payloads (the inlined-block
  case from `memory_layouts.md`).
- **Map dedup + alignment.** `set`/`removeKey` on existing vs new keys; verify
  alignment of newly appended key/value payloads matches a freshly-built map.
- **Holes + compaction.** Interleaved remove/append cycles; assert correctness
  and that `dataCapacity` does not grow unboundedly (compaction fires).
- **Copy tightness.** A copied/passed collection has `capacity == count` and
  `dataCapacity == dataLength` (no headroom leaks into snapshots).
- **Ownership/scope unchanged.** Re-run the ownership/scope-drop suites; no
  double-free or leak regressions (this plan must not perturb copy/move).
- **Benchmarks.** `benchmark/append` mfb time drops below CPython and the
  per-op cost flattens (amortized O(1)); `benchmark/primes` (which appends each
  prime) improves; `benchmark/empty` unchanged.

## 9. Risks and gotchas

- **`arena_alloc` clobbers scratch registers** (`x9,x10,x14,x15,x20–x28`) — see
  the existing in-file comments and the `arena-alloc-clobbers-x14-x15` /
  `copy-record-register-aliasing` memories. Any new grow path must stash live
  values across the call.
- **In-place uniqueness.** The Phase-4 fast path is only sound when the `MUT`
  buffer is provably uniquely owned at the assignment (no live alias/borrow).
  When in doubt, fall back to allocate-with-headroom — never mutate a possibly
  shared buffer. This is what keeps copy/move semantics intact.
- **Data base uses `capacity`.** With `capacity > count`, the data region moves
  relative to `count`; all data addressing must go through
  `emit_collection_data_pointer` (capacity-based), never `count`-based math.
- **Nested/aligned payloads.** Verbatim data copy must preserve map payload
  alignment and inlined nested-collection blocks; the appended element is the
  only payload that needs fresh alignment.
- **Thread transfer.** A collection sent across a thread boundary is copied into
  the destination arena; ensure that copy is shrink-to-fit (§4.3) so headroom
  never crosses arenas.
