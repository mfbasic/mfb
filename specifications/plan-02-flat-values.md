# MFBASIC Flat Value Representation Plan

Last updated: 2026-06-24

## Implementation status

- **Phase 1 — DONE.** `emit_flat_block_size` (the self-describing size primitive
  for already-flat blocks: `String` = len+9, collection = header+table+data) and
  `copy_flat_block` (generic `arena_alloc` + `memcpy`) landed in
  `builder_collection_layout.rs`. `copy_value_to_current_arena` now routes
  `String` and inline-payload collections through the generic copy; the
  per-payload transfer fix is kept only for collections that still embed pointer
  payloads (`collection_needs_transfer_fix`). No layout change. Runtime output
  identical (verified via `thread-return-string`,
  `thread-return-list-of-string`, `thread-return-map-of-string-to-string` under
  entropy poisoning); acceptance green. The generic `arena_free` wrapper is
  deferred to Phase 8 where it gains a call site (its only new primitive, the
  block-size computation, already landed here).
- Phases 2–8 — pending. **See the scoping correction below before starting
  Phase 2 — the original phase split is not landable as written.**

### Phase 2 scoping correction (discovered while implementing)

The plan calls Phase 2 ("inline `String` in records/unions") "the smallest
layout step" and defers the union reshape (Phase 4), nested records (Phase 3),
and collection inlining (Phase 5) to later. **That separation does not hold.**
A standalone record/union with a `String` field can be changed in isolation, but
records and unions do not occur in isolation in the existing, green test suite —
they flow through containers, equality, map keys, nesting, and union wrapping,
and every one of those paths assumes a record is a fixed `8*fieldCount` block of
slots in which a `String` slot is a **pointer**. Concrete, exercised evidence:

- `tests/types-record-comparable-runtime` puts `Person { name AS String, city
  AS String }` into `List OF Person`, `Map OF Person TO Integer` (record as a
  **map key**), and nests it in `Badge { owner AS Person, level AS Integer }`,
  and compares records for **equality**.
- `tests/builtin-pair-partition-valid` stores `Pair OF Integer, String` in a
  `List` and reads fields back.
- `tests/types-recursive-record-valid` wraps a `String`-bearing record
  (`ConfigJsonStr { value AS String }`) inside `UNION ConfigJson`, then reads
  `tree.value` after a `MATCH`, and stores the union recursively in
  `Map OF String TO ConfigJson`.

Why these force the later phases forward into Phase 2:

1. **Collection embedding becomes variable-size.** A record is embedded in a
   collection by copying `inline_collection_payload_size` = `8*fieldCount` bytes
   (the slots) into the data region (`builder_collection_layout.rs`
   `emit_copy_payload_to_collection`, `emit_payload_length_to_stack`). If a
   `String` slot is now a block-relative **offset**, the bytes it points at (the
   record's data region) must be copied too, so a record payload becomes
   **runtime-variable length**. That is the core of Phase 5, pulled into Phase 2.
2. **Union wrapping inlines record fields.** `UnionWrap` (`builder_values.rs`
   ~761-871) copies a record variant's **slots** into the union payload slots
   (`+8, +16, …`), not a pointer to the record. An inlined-`String` record's
   slots are offsets into the record's data region, which is not copied into the
   union — so the union must inline the record's data region, i.e. become
   `{tag, size, data}`. That is Phase 4, pulled into Phase 2.
3. **Equality / map-key compare must deref.** The record-equality branch
   (`builder_collection_layout.rs` `emit_comparable_values_match_branch_from_slots`
   ~295-325) loads each field slot and recurses; a `String` field slot is now an
   offset, so it must become `base + offset` before the byte compare. Map-key
   matching for record keys rides the same path.
4. **`materialize_inline_value_in_arena`** (`builder_collection_layout.rs` ~349)
   copies a fixed `inline_collection_payload_size` bytes; variable-size records
   break it.

**Conclusion:** inlining `String` into records is an **atomic** change that must
land together with: record construction/default-init/field-read/`WITH`/copy,
record equality + map-key compare, variable-size record payloads in collections
(append/literal/get/length/materialize), record nesting, and union-payload
reshaping for record variants. A partially-applied version corrupts the heap for
the tests above (the exact layout-sensitive, "passes small tests then fails at
scale" failure mode AGENTS.md and the arena memory notes warn about).

**Recommended re-sequencing for whoever resumes:**

- **2a. Size header up front.** Add the explicit `U64 size` header (§9) to the
  record block *now* (and the union `size` word), placed so field-slot access
  stays `base + 8*n` (e.g. keep slots at `8*n`, data region after them, and store
  total size by walking — or accept a header at `+0` and shift slot access to
  `8 + 8*n` everywhere). The header makes copy/embed/length one `load`, removing
  the per-type size walk and most of the risk multiplier. `emit_flat_block_size`
  (landed in Phase 1) is the seam to extend.
- **2b. One atomic commit per "value kind goes flat,"** each kept green by the
  existing copy-dispatches-on-"is it flat yet" rule, but scoped to a *kind* that
  is closed under the operations above. The smallest closed unit is **all of**:
  record construct/read/with/default/copy/equality + variable-size record payload
  in collections + record-variant union wrap/extract — landed together for the
  `String`-field case.
- Validate each commit with `scripts/test-accept.sh` **and** a direct run of
  `types-record-comparable-runtime`, `builtin-pair-partition-valid`, and
  `types-recursive-record-valid` under entropy poisoning (a missed inline shows
  up as a loud crash, not silent garbage).

A complete, file-and-line site map for the record and union changes was produced
during this investigation (record construction `builder_values.rs:644-690`; field
read `builder_misc.rs:197-301`; `WITH` `builder_misc.rs:303-383`; default init
`builder_misc.rs:145-193`; copy glue `builder_misc.rs:1498-1553` /
`copy_record_fields_into_existing`; equality
`builder_collection_layout.rs:295-325`; collection embedding
`builder_collection_layout.rs:743-846`; union construct/wrap/extract
`builder_values.rs:692-936`; union copy `builder_misc.rs:1555-1594` /
`copy_union_fields_into_existing`). The change is mechanically large but well
understood; it is **not** safe to land as the under-scoped single "smallest step"
the original Phase 2 describes.

This plan makes **every non-resource value a flat, self-describing,
single-allocation block** — all sub-values inlined, no pointers to other
allocations. The single behavioral outcome a correct implementation produces:
copying **any** value (String, record, union, list, map, error, result) is one
`arena_alloc` + one `memcpy`, and freeing it is one `arena_free` — with **no
per-type copy/drop glue and no aliasing**.

This dissolves the problem `note-1` / `plan-01 §5.5` documented: today the heap is
an aliased pointer graph, so a shallow copy shares sub-objects and a naive
scope-drop free would double-free. When every value is a pointer-free block, a
`memcpy` **is** a deep copy and a single `arena_free` reclaims the whole thing —
so `plan-01`'s deferred scope-drop frees become trivial and sound.

It complements:

- `specifications/note-1.md` (the aliasing store-sites and why frees are blocked)
- `specifications/plan-01-arena-update.md` (arena free-list + entropy fill;
  §5.5 the aliasing reality; Phase 5 this unblocks)
- `specifications/memory_layouts.md` (String / Record / Union / Collection
  layouts — all change here)
- `specifications/mfbasic.md` (value/copy semantics — must stay observably
  identical)

## 1. Goal

Three layout terms, used throughout:

- **flat** — single allocation, all sub-values stored inline (no pointer to a
  separate allocation; relative offsets within the *same* allocation are allowed).
- **offset** — a sub-value stored inline in the same allocation, located by a
  block-relative offset.
- **pointer** — a separate allocation (what this plan eliminates for non-resource
  types).

Targets:

- Every non-resource value is **flat**: scalars, `String`, `Record`, `Union`,
  `List`, `Map`, `Error`, `ErrorLoc`, `Result`.
- Every value is **self-describing**: it carries its own total byte size, so copy
  and free are generic and O(1) regardless of type.
- **Copy = one `memcpy`. Free = one `arena_free`.** The per-type deep-copy glue
  (`copy_value_to_current_arena`, `builder_misc.rs:1279`) and the per-type drop
  glue `plan-01` Phase 5 would have needed both collapse to a single generic
  routine.
- **`plan-01` scope-drop frees become trivial**: a non-escaping local is freed by
  one `arena_free` of its block; there is no aliasing, so no escape graph and no
  double-free risk — only the `RETURN`/`thread::transfer` move suppressions
  remain.

### Non-goals (explicit constraints)

- **No language-surface change.** Programs compile unchanged; value/copy/move/
  freeze semantics are observably identical (immutable values, deep-copy on
  assignment — now realized by `memcpy`).
- **Scalars stay inline** in registers/stack slots; a standalone `String`/composite
  value is still reached through an 8-byte handle (a fixed-width slot cannot hold a
  variable-length value). The *handle* is a pointer to the flat block; the **block
  itself** is what becomes flat.
- **Resources stay pointers — the single remaining pointer.** A `RES` value is a
  pointer to the one and only arena-global instance of that resource; resources are
  **never copied** (move-only) and never inlined. The language already bounds where
  they appear (§9): a **record can never own a resource**
  (`TYPE_RESOURCE_FIELD_FORBIDDEN`), and a **union is all-data or all-resource**
  (`rules.rs:790`) — an all-resource union is a *resource union*. So the resource
  pointer is an opaque word that generic `memcpy`/`arena_free` copy and skip
  correctly (the resource's lifecycle is its own close op, never `arena_free`).
- **No reference counting, no GC** (same as `plan-01`).

## 2. Current State

`note-1.md` is the authority. Summary: owned values are shared by raw pointer at
almost every store site, so the heap is an aliased graph. Concretely, today:

| Type                | Arena object
|---------------------|-------------------------------------------------------
| scalars             | inline (never a standalone arena object)
| `String`            | **flat** `{len, bytes, nul}` — already pointer-free
| `Record`            | 8-byte slots; scalar slots inline, **composite slots are pointers**
| `Union`             | `{tag, payload slots}`, sized to the largest variant; composite payload slots are **pointers**
| `List`/`Map`        | header + entry table + data region; scalar & `String` payloads **inline**, nested collection / resource payloads are **pointers**
| `Error`/`ErrorLoc`  | records → `message`/`source`/`filename` are **pointers**
| `Result`            | `{tag, payload}` → owned payload is a **pointer**

So `String` and `List of <scalar/String>` are already flat; everything else
leaks pointers, which is why copy needs recursive glue and scope-drop frees would
double-free (`note-1`).

The standalone `String` blob (`{U64 len, bytes, U8 nul}`) is the model the rest of
the types adopt: a contiguous, self-describing, pointer-free block.

## 3. Design Overview

Give every value the shape `String` and collections already nearly have: a
**self-describing flat block** with a fixed part and a trailing **data region**,
where any variable-length or composite sub-value lives inline and is located by a
**block-relative offset** (offsets survive `memcpy`; absolute pointers would not).

The two structural changes, plus their generalization:

1. **Records**: scalar slots stay inline; **every composite slot** (`String`,
   `Record`, `Union`, `List`, `Map`) becomes a **relative offset** into the
   record's own trailing data region, where the sub-value's flat block is embedded.
2. **Unions** change shape to `{tag, size, data}` — sized to the **active**
   variant, with that variant's fields laid out (and inlined) in `data`.
3. **Collections** keep their header + entry table + data region, and the data
   region inlines **all** payloads — including **nested collections** — by offset;
   no payload is a pointer (except a resource handle).
4. **Every block carries its total `size`**, so copy/free are generic.

Result — the target arena classification (your table, generalized):

| Type                          | Arena
|-------------------------------|-----------------------------------------------
| scalars                       | inline
| `String`                      | flat
| record slot scalar            | flat (inline)
| record slot String/Record/Union/Collection | **offset** (inline in the record's data region)
| `Union`                       | **flat** `{tag, size, data}`
| `List`/`Map` of flat element  | flat (incl. nested collections, now inlined by offset)
| `Error`/`ErrorLoc`/`Result`   | flat (they are records / unions)
| `Resource`                    | pointer (unchanged — wraps an OS handle)

The correctness win: **a flat block has no internal pointers, so `memcpy` is a
deep copy and `arena_free` of `size` bytes reclaims everything.** The correctness
risk: this touches the layout and every access/construct/copy/mutate site for
records, unions, and collections.

## 4. Detailed Design

### 4.1 Self-describing blocks

Every flat object begins with (or otherwise carries) its **total byte size** so
copy and free never dispatch on type:

```text
copy(obj):  n = size(obj);  dst = arena_alloc(n, 8);  memcpy(dst, obj, n);  → dst
free(obj):  arena_free(obj, size(obj))
```

`size` is read in O(1). For a union it is the explicit `size` word (§4.3); for a
record an explicit size header (recommended — see Open Decisions) or recomputed
from the static type's fixed part plus the data-region extent; for a collection it
is already derivable from `capacity`/`dataCapacity` in its header.

These two routines **replace** `copy_value_to_current_arena` and all its per-type
helpers (`copy_record_*`, `copy_union_*`, `copy_collection_*`,
`fix_collection_transfer_payloads`, …) and the per-type drop glue `plan-01` Phase 5
would have needed.

### 4.2 Record block

```text
RecordObject (flat)
  [ size header? ]                 ; explicit total size (Open Decision)
  Slot[fieldCount] slots           ; fixed, 8 bytes each, field n at the usual 8*n
  Byte[...] dataRegion             ; inlined composite sub-blocks
```

- **Scalar slot** — unchanged: the value inline in the slot.
- **Composite slot** (`String`/`Record`/`Union`/`List`/`Map`) — a **U64
  block-relative offset** to the sub-value's flat block in the data region.
- **Field read**: scalar → load slot; composite → `subBlock = recordBase + slot`
  (a borrow pointer to the inlined flat block, itself a valid standalone value).
- Field slot **offsets stay static** (`8*n`); only the *meaning* of a composite
  slot changes (offset, not pointer). This preserves the pervasive `[rec + 8*n]`
  access pattern.

### 4.3 Union block

```text
UnionObject (flat)
  +0   U64 tag         ; active variant index
  +8   U64 size        ; total byte size of THIS object (active variant)
  +16  data            ; the active variant's fields, laid out like a record
                       ; (its own slots + inlined composite sub-blocks)
```

Sized to the **active** variant (not the largest), so a small variant is small.
`data` is itself a record-style flat layout (scalar slots inline, composite slots
as offsets relative to the union base). The `size` word makes copy/free generic
and removes the need to dispatch on `tag` to learn the size.

`Result` is a union (`Ok`/`Err`) and `Error`/`ErrorLoc` are records, so they all
become flat automatically under §4.2/§4.3 (e.g. `Error`'s `message` and `source`
inline; `Result`'s payload inline).

### 4.4 Collection block

Header + entry table + data region, as today, but the data region inlines **every**
payload by offset — adding the one case that is still a pointer: a **nested
collection** payload is embedded as its own flat block in the data region and
referenced by the entry's relative offset, exactly like a record payload already
is. After this, a collection has **no pointer payloads** (only a resource payload
stays a handle). `List of List`, `Map of String to List`, etc. become flat.

### 4.5 Construction and mutation

- **Construction** computes the block's total size from its parts (sum of the
  fixed part plus each inlined sub-block's size), `arena_alloc`s once, writes the
  scalar slots, and appends each composite sub-block into the data region while
  recording its relative offset in the owning slot/entry.
- **Mutation / resize** (changing a field, `WITH`-update, collection `append`):
  rebuild the block — `arena_alloc(new size)`, `memcpy` the unchanged **prefix**,
  write the new/resized sub-block, `memcpy` the unchanged **suffix**, and **bump
  the slot/entry offsets of everything after the change** by the size delta (a few
  integer adds over the slot/entry table, not data movement). Cost: **1 alloc + ≤2
  `memcpy`s + offset fixup** — sequential, no pointer chasing. This is the same
  shape as the Phase-3 `readLine` buffer grow already in the tree.

### 4.6 Standalone handles unchanged

A standalone value (local, parameter, return, global, temporary) is still reached
through an 8-byte handle pointing at its flat block — a fixed-width slot cannot
carry a variable-length value. The inlined sub-block produced by a field/element
read is itself a valid standalone block, so promoting it to a local is a borrow or
a single generic `memcpy`, never recursive.

### 4.7 What this does to copy, drop, and `plan-01` Phase 5

- **Copy glue deleted**: replaced by §4.1 generic `memcpy`.
- **Transfer glue deleted**: `thread::transfer` is the same generic `memcpy` into
  the receiver arena.
- **`plan-01` Phase 5 (scope-drop frees) becomes trivial and sound**: there is no
  aliasing, so the elaborate ownership/escape analysis and per-type drop glue are
  unnecessary. A non-escaping owned local is freed by one generic `arena_free`;
  the only suppressions are the existing **move** sites (`RETURN`,
  `thread::transfer`), which already deactivate cleanups. Entropy poisoning
  (`plan-01` §6) remains the safety net.

## 5. The tradeoff (accepted, documented)

Flat blocks have **no structural sharing**. Today pointers let an unchanged
sub-object be shared between an old and new value, so a mutation rebuilds only the
changed path. In the flat world, changing a nested field rebuilds the **whole
enclosing block** (§4.5).

- **Cheaper**: copy (one `memcpy`), free (one `arena_free`), transfer, and the
  whole-value churn that value semantics already pays.
- **More expensive**: mutating one small field deep inside a large block, and
  collection `append`, go from O(changed part) to **O(block size)** — though it's
  the cheap, sequential-`memcpy` kind of O(n), and value semantics already
  deep-copies on mutation, so it is rarely a *new* cost.

This is the deliberate exchange: pay O(block) sequential rewrites on nested
mutation to get O(1)-glue, alias-free, single-`memcpy` copy/free everywhere.

## 6. Layout / ABI Impact

- `memory_layouts.md`: rewrite **Record**, **Union**, **Error/ErrorLoc**,
  **Result**, and **Collection** sections for the flat/offset/size-header model;
  keep **Standalone String** (it is already the model) and the **handle** ABI.
- Native codegen changes for record/union/collection **construction, field/element
  read, `WITH`, `MATCH`/extract, copy, transfer, and mutation/append**; broad
  `ncode`/`nplan`/`nobj` golden churn. **Runtime output must be identical.**
- **Deletions**: the per-type copy/transfer glue in `builder_misc.rs`; eventually
  the per-type drop glue that `plan-01` Phase 5 would have introduced.
- **Unchanged**: the arena allocator / free-list / entropy fill (`plan-01`),
  scalar inline storage, the value-handle ABI, and `Resource` representation.

## 7. Phases

Each phase keeps the program working by letting copy dispatch on "is this type
flat yet?" — generic `memcpy` for already-flat types, the existing recursive glue
for the rest — until the last type is converted and the glue is deleted.

1. **Size header + generic copy/free.** Add the self-describing size and the
   generic `memcpy`/`arena_free` routines; route already-flat types (`String`,
   `List of scalar/String`) through them. No layout change yet. Land independently.
2. **Inline `String` in records/unions.** The smallest layout step: composite
   `String` slots become relative offsets (proves the offset model end to end —
   construction, read, `WITH`, copy).
3. **Inline nested records in records** (`Record slot Record → offset`).
4. **Reshape unions to `{tag, size, data}`** and inline their composite payloads;
   `Result`/`Error` follow.
5. **Inline collections everywhere** — nested collections in collection data
   regions, and `List`/`Map` fields of records/unions (`Record slot Collection →
   offset`).
6. **Switch copy/transfer to generic `memcpy`; delete the per-type glue.**
7. **Flat-representation hardening — exhaustive copy / independence / mutation
   tests (see §7a).** A dedicated phase: before any free relies on flatness, prove
   under entropy poisoning that every value is a self-contained flat block whose
   copy shares **nothing** and whose mutation/resize is correct. This phase adds
   tests only — no new codegen — and must be fully green before Phase 8.
8. **Enable `plan-01` scope-drop frees** — now one generic `arena_free` per
   non-escaping local, gated only by the existing `RETURN`/`transfer` move
   suppressions. (This *is* `plan-01` Phase 5, made trivial.)

### 7a. Phase 7 test matrix (explicit)

Every test follows the same shape unless noted: **build a value `a`; copy it
(`LET b = a`); then mutate or drop one side and assert the other is byte-for-byte
unchanged** — all run under `plan-01` entropy poisoning so any residual shared
bytes (a missed inline) surface as a loud use-after-free, and each asserts exact
stdout. Fixtures land under `tests/` (`.run`/build.log goldens); stdin/thread
cases go in the native integration tests.

**A. Record field-type coverage** — a record with each composite field, alone and
combined:
- all-scalar record (flat baseline)
- one `String` field; **several** `String` fields (offset fixup across many)
- nested record field; nested record that itself has a `String`
- `Union` field; `List` field; `Map` field
- a record combining **all** of the above (worst case)
- per record: construct → copy → on the copy resize a `String` field shorter **and**
  longer and mutate a scalar → assert original unchanged, copy correct → drop one
  side → assert the other intact.

**B. Container element-type coverage:**
- `List of <each scalar>`; `List of String` (variable-length inline)
- `List of <record>`; `List of <record-with-String>`; `List of <union>`
- `List of List` and `List of Map` (nested collection inlined — the new case)
- `Map of String to <each of the above>`
- per container: build → copy → mutate an element on the copy (including a resize)
  → assert independence.

**C. Records-with-containers and containers-with-records (the cross cases):**
- `record { a AS List OF Integer }` → copy → append to the copy's list (forces
  data-region growth + offset fixup) → assert the original's list is unchanged
- `record { a AS List OF String }` → resize a string inside the nested list inside
  the record
- `record { a AS Map OF String TO <record> }`
- `List OF <record-with-list>`; `Map OF String TO <record-with-list>`
- deep nest: `record { x AS List OF record { y AS List OF String } }` → mutate the
  innermost string on a copy, assert every level of the original is intact.

**D. Unions (reshaped `{tag, size, data}`):**
- data union with a scalar / `String` / record / collection variant
- switch the active variant (changes `size`) → copy → assert `size` word and
  contents correct on both
- `Result OF <each type>`; `Error` carrying both `message` and `source`.

**E. Mutation / `WITH` / append placement:**
- `WITH`-update a `String` field shorter and longer (exercises prefix+suffix
  `memcpy` and the post-change offset fixup)
- `WITH`-update a field at the **start**, **middle**, and **end** of a block (the
  1-memcpy vs 2-memcpy paths)
- collection `append` that grows the data region and shifts following offsets.

**F. Empty / boundary:**
- empty `String` field, empty `List`, empty `Map` (zero-length inline blocks)
- single-element and large (many-KB) values; a flat block large enough to cross an
  arena block boundary (exercises large `arena_alloc` + large `memcpy`).

**G. `thread::transfer`:**
- transfer a record-with-strings, a `List` of records, a record-with-list, and a
  deeply nested value; the receiver reads correct values and (if the sender keeps a
  copy) the two are independent.

**H. Resource exception (must NOT be deep-copied):**
- a resource union and a `List OF RES`: copying the surrounding flat block copies
  the resource **pointer verbatim** (not the resource), the resource is closed
  **exactly once**, and the block copies/frees correctly around the opaque handle.

**I. Size header + generic copy/free:**
- a copy allocates exactly `size` bytes and the copy's `size` header matches the
  source
- a churn loop (build → drop, repeated) shows the freed bytes are reused with no
  growth (proves `arena_free` returns exactly `size`)
- a generic-copy round-trip of every type yields byte-identical contents.

**J. Entropy-poisoning negatives:**
- copy `a`→`b`, **drop `a`**, then read `b`: must be fully correct (no shared bytes
  were scrubbed). Run under poisoning so any residual alias crashes loudly rather
  than silently returning garbage.

## 8. Validation Plan

- Function tests: `tests/func_*` valid/invalid across every record/union/
  collection/`WITH`/`MATCH`/`Error`/`Result` surface, full overload coverage.
- Runtime proof per phase: build a value, `LET b = a` (copy), confirm independence
  (mutating/ dropping one doesn't affect the other); `WITH`-update a nested field
  and confirm the original is unchanged; put composites in a `List`, copy the
  list, confirm independence; round a value through `thread::transfer`. All under
  `plan-01` entropy poisoning so any residual shared bytes surface as a loud
  use-after-free.
- Differential: same program, golden stdout identical before/after each phase.
- Doc sync: `memory_layouts.md`; `error_codes.md`/`mfbasic.md`/
  `standard_package.md` if any diagnostic changes.
- Acceptance: `scripts/test-accept.sh`, goldens re-synced.

## 9. Decisions

Resolved:

- **Per-object size header — YES.** Every flat block carries an explicit `U64
  size`. Copy/free are O(1) and fully type-agnostic; cost is one word per object.
  (No recompute-from-type alternative — that would reintroduce per-type size logic
  and undercut generic copy/free.)
- **Offset width / slot encoding — bare `U64`, no packing.** A composite slot/entry
  holds a `U64` block-relative byte offset; the sub-block's own header carries its
  size/len. No `{offset:u32,len:u32}` packing.
- **Empty/unset composite slot — zero-length inlined block, no special cases.**
  e.g. empty `String` = `len 0` + `nul` embedded inline; never a null/sentinel
  offset. Spends a few extra bytes to keep every read uniform.
- **Resources — always a pointer; never copied; the only remaining pointer.** A
  resource is a pointer to the single arena-global instance. The language already
  bounds this so no "partly flat" copy path is needed:
  - a **record can never own a resource** (`TYPE_RESOURCE_FIELD_FORBIDDEN`,
    `rules.rs:770` / `typecheck.rs:1463`) → every record is fully flat;
  - a **union is all-data or all-resource** (`rules.rs:790` / `typecheck.rs:1529`);
    a data union is fully flat, an all-resource union is a *resource union*
    (move-only, `RES`-bound, no STATE);
  - a `List OF RES` / resource-union slot holds the resource pointer as a borrow of
    the unique instance. Generic `memcpy` copies that word verbatim (a borrow/move,
    governed by the existing `RES` ownership rules) and generic `arena_free` frees
    only the containing block — **never the resource**, which is reclaimed by its
    own close op. So the resource pointer is opaque to flat-block copy/free; no
    special handling.
- **Mutation cost — go fully flat first.** Accept O(block) sequential rewrites on
  nested mutation/append for the O(1)-glue, alias-free copy/free. Only revisit with
  a bounded hybrid if a measured real workload demands it.

Still open: none — proceed to Phase 1.
