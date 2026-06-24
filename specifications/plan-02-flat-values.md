# MFBASIC Flat Value Representation Plan

Last updated: 2026-06-24

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
- **Resources stay pointers.** A `RES` value wraps a live OS handle and has a
  close-on-scope lifecycle; it cannot be a memcpy'd block. A value that
  transitively contains a resource keeps that one handle and is **not** flat — the
  documented exception.
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
7. **Enable `plan-01` scope-drop frees** — now one generic `arena_free` per
   non-escaping local, gated only by the existing `RETURN`/`transfer` move
   suppressions. (This *is* `plan-01` Phase 5, made trivial.)

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

## 9. Open Decisions

- **Per-object size header** — *recommend* an explicit `U64 size` on every flat
  block (unions already get one). Makes copy/free O(1) and fully type-agnostic;
  costs one word per object. Alternative: recompute size from the static type plus
  data-region walk — saves the word but reintroduces per-type size logic, undercutting
  the "generic copy/free" win. Recommend the word.
- **Offset width / slot encoding** — *recommend* a bare `U64` block-relative byte
  offset for composite slots/entries (sub-block's own header carries its size/len).
  Packing `{offset:u32,len:u32}` is rejected unless profiling demands it.
- **Empty/unset composite slot** — *recommend* a zero-length inlined block (e.g.
  empty `String` = `len 0` + `nul`) over a null/sentinel offset, so reads never
  special-case.
- **Resource-containing values** — a record/collection that transitively holds a
  `RES` keeps that one handle and is **not** flat; its non-resource parts still
  inline. Confirm the copy/free path treats such a block as "mostly flat + one
  handle" (the resource has its own close lifecycle and must not be memcpy-cloned).
- **Mutation cost ceiling** — if O(block) nested mutation proves too costly for
  some real workload (large record holding a large list, hot inner update),
  revisit with a bounded hybrid (e.g. keep large collections pointer-backed) —
  measure first; default to fully flat.
