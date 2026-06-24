# MFBASIC Inline String Storage Plan

Last updated: 2026-06-24

This plan makes a `String` stored inside a composite value an **inline, relative
addressed `{len, bytes, nul}` blob** instead of an absolute pointer to a
separately-allocated object. The single behavioral outcome a correct
implementation produces: copying a record/union whose fields are scalars and
`String`s is a **single `memcpy`** of one self-contained, pointer-free allocation,
and the copy shares **nothing** with the original.

It is the first step toward the broader "everything inline, deep-copy is one
`memcpy`" goal, and directly unblocks the deferred scope-drop frees from
`plan-01`: once a composite is a contiguous pointer-free blob, freeing it is a
single `arena_free` and there are no aliased sub-objects to double-free.

It complements:

- `specifications/memory_layouts.md` (Standalone String, Record, Collection data
  region — the inline-payload precedent this generalizes)
- `specifications/note-1.md` (why aliasing blocks scope-drop frees; the store-site
  list)
- `specifications/plan-01-arena-update.md` (§5.5; the arena free-list / entropy
  fill this builds on)
- `specifications/mfbasic.md` (value/copy semantics — must be unchanged)

## 1. Goal

- A `String` field of a record or union is stored **inline** in the owning
  allocation, addressed by a **relative offset** into a trailing data region, not
  by an absolute pointer to a separate `StringObject`.
- Copying such a record/union is `arena_alloc(totalSize)` + one `memcpy` — no
  per-field recursion, no shared sub-allocation. The result is observably a deep
  copy.
- The existing standalone `StringObject` layout (`{U64 len, bytes, U8 nul}`) is
  reused verbatim **as the inline payload** — a field read yields a pointer to a
  blob that is bit-identical to today's standalone String, so String consumers are
  unchanged.

### Non-goals (explicit constraints)

- **No language-surface change.** Source programs compile unchanged; `String`
  value/copy semantics are identical (immutable, deep-copied on assignment).
- **No change to standalone `String` locals' observable behavior.** A `String`
  held in a local/parameter/return/global is still reached through an 8-byte
  handle (a register / stack slot / global slot cannot hold a variable-length
  value); that handle points at a standard inline blob. Only *storage inside a
  composite* changes from pointer to inline.
- **Collections are already inline** for `String` payloads (`memory_layouts.md`
  Collection String Payloads) — this plan does **not** change collection layout;
  it brings records/unions up to the same model.
- **Not in scope:** inlining nested **collections**, **records**, or **unions**
  inside another composite (those stay pointer payloads for now — see Phase list
  and Open Decisions). This plan inlines `String` only.
- **No reference counting, no GC.** Same constraint as `plan-01`.

## 2. Current State

- A standalone `String` is `{U64 byteLength, utf8Bytes, U8 nul}`, a contiguous
  **pointer-free** blob (`memory_layouts.md` Standalone String). Copying one is
  already a single `arena_alloc(len+9)` + `memcpy(len+9)`.
- A **record** is a flat array of fixed 8-byte slots; field `n` lives at offset
  `8 * n` (`memory_layouts.md` Record). A `String` field's slot holds an
  **absolute pointer** to a separate `StringObject` (`note-1.md` site 2). A
  **union** is `{U64 tag, payload slots}`; an owned payload slot is likewise a
  pointer (`note-1.md` site 3).
- Because the slot is an absolute pointer, copying a record is a **shallow** copy
  that shares the String (aliasing). The deep-copy glue
  (`builder_misc.rs:1279`, `copy_record_to_current_arena` at `:1493`) exists
  precisely to walk fields and re-copy each owned pointer; it is what `plan-01`
  Phase 5 would have to trust.
- A **collection** already stores `String` payloads inline as UTF-8 bytes in a
  trailing data region, addressed by a `{offset, length}` entry relative to the
  data-region base (`memory_layouts.md` Collection String Payloads;
  `builder_collection_layout.rs`). This is the proven precedent we mirror.

## 3. Design Overview

Give records and unions the same shape collections already have: a **fixed slot
table** followed by a **trailing data region**, with variable-length `String`
payloads living in the data region and referenced from the slot by a
**record-base-relative offset**.

```text
RecordObject (inline-String form)
  +0                      Slot[fieldCount] fields        ; fixed, 8 bytes each
  +8*fieldCount           Byte[...] dataRegion           ; inline String blobs
```

- A **scalar** field slot is unchanged: the value inline in the slot.
- A **`String`** field slot holds a **U64 relative offset** from the object base
  to an inline `StringObject` (`{len, bytes, nul}`) in the data region. (A sentinel
  — e.g. offset `0`, which can never be a valid payload offset because the slot
  table occupies `[0, 8*fieldCount)` — encodes the empty/unset case if needed.)
- Reading a `String` field: `stringPtr = recordBase + slot`. The result is a
  pointer to a standard inline blob — **bit-identical to today's standalone
  String** — so `len`/`io::print`/concat/compare and every other String consumer
  is unchanged. The pointer is a **borrow** into the record (same rule as reading
  a String out of a collection); code that stores it elsewhere copies it.
- Copying the record: read `totalSize` (slot table + data region), `arena_alloc`,
  one `memcpy`. The slots hold **relative** offsets, so they remain valid in the
  copy with no fix-up. Result: a fully independent deep copy, no shared bytes.
- Unions get the identical treatment: `{tag, fixed payload slots, dataRegion}`;
  a `String` payload slot is a relative offset.

Why **relative** offsets (not absolute pointers): an absolute pointer embedded in
the object would point into the *original* allocation after a `memcpy`, so the
copy would need per-field fix-up — defeating the single-`memcpy` goal. A relative
offset survives `memcpy` untouched. This is exactly why the collection data region
uses offsets.

The correctness risk concentrates in (a) computing record/union size from
constructor arguments at allocation time and (b) the now base-relative field read
threaded through every record/union access site.

## 4. Detailed Design

### 4.1 Object layout and size

For a record type with fields `f0..f(n-1)`:

- Slot table: `8 * n` bytes (unchanged offsets `8*i`).
- Data region: the concatenation of an inline `StringObject` (`8 + len_i + 1`,
  padded to 8) for each `String` field that is set.
- `totalSize = 8*n + sum(padded blob sizes)`.

Records/unions are **variable-size**: the allocation size depends on the String
lengths, computed at construction (§4.2). The fixed slot table keeps **field
offsets static** (`8*i`) — only the *payload* is variable — so the pervasive
`[record + 8*i]` access pattern keeps its compile-time offset; only the *meaning*
of a `String` slot changes (relative offset, not pointer).

### 4.2 Construction

`Rec(a, b, s)` (record with a trailing `String` field `s`):

1. Lower each argument. Scalar args are values; a `String` arg is a handle
   (pointer) to a blob (its `len` is readable at `[handle]`).
2. Compute `totalSize = 8*n + Σ pad8(8 + len_i + 1)` over String fields.
3. `arena_alloc(totalSize, 8)`.
4. Write scalar slots inline. For each String field: append its blob to the data
   region with a `memcpy` of `8 + len + 1` bytes, and store the **relative offset**
   into the field's slot.

This replaces the current "store the arg pointer into the slot"
(`builder_values.rs:681-684` record, `:750-753` union).

### 4.3 Field read

A `String` field read changes from "load slot as pointer" to "load slot, add base":

```text
slot  = [recordBase + 8*i]
strPtr = recordBase + slot          ; borrow into the inline blob
```

Scalar field reads are unchanged. Every record/union field-access lowering must
branch on "is this field a `String`" and apply the base-relative resolution.
(Member access, `MATCH`/union extract, `WITH` reads, pattern binds.)

### 4.4 `WITH` update

`r WITH { name := "new" }` changes a String field's length, so the object is
resized: compute the new `totalSize`, `arena_alloc`, copy the unchanged slots,
re-emit the data region with the new blob for the updated field and copies of the
others. (Today `WithUpdate` shallow-copies the record and overwrites a slot
pointer; it becomes a rebuild — still a single new allocation.)

### 4.5 Copy becomes `memcpy`

`copy_record_to_current_arena` / `copy_union_to_current_arena`
(`builder_misc.rs:1493`, `:1550`) collapse, **for the all-scalar/String case**, to:
read `totalSize`, `arena_alloc`, `emit_copy_bytes`. The per-field recursion is
kept **only** for fields that are still pointer payloads (nested collection /
record / union) until those are inlined by later plans. A record whose fields are
all scalar + `String` is copied by one `memcpy`.

### 4.6 Standalone Strings, locals, parameters, returns, globals — unchanged

A `String` that is **not** a field of a composite (a local, a parameter, a return
value, a global, a bare temporary) stays a standalone `StringObject` reached
through an 8-byte handle, because a fixed-width register/slot cannot carry a
variable-length value. The inline blob produced by a field read **is** a valid
standalone `StringObject`, so promoting a field read to a local is either a borrow
(transient use) or a single-`memcpy` materialization — identical to reading a
String out of a collection today. No ABI change for String handles.

### 4.7 Collections of records

A `List OF SomeRecord` already stores the record inline in its data region. With
records now self-contained (relative offsets), the inline record copies correctly
under the collection's existing `memcpy` of the data region — and the collection's
`fix_collection_transfer_payloads` record-fixup step is **no longer needed** for
the String fields (the relative offsets are copy-stable). This is a simplification
the plan should realize and test.

## 5. Layout / ABI Impact

- `memory_layouts.md` **Record** and **Union**: document the trailing data region
  and the relative-offset `String` slot; note that scalar slots and field offsets
  (`8*i`) are unchanged.
- `Error` / `ErrorLoc` are records (`memory_layouts.md:101-113`): their `message`
  / `filename` `String` fields become inline relative offsets too — `Error` copy
  becomes closer to a `memcpy` (its `source` `ErrorLoc` is still a record pointer
  until records-in-records are inlined; see Phase list).
- Binary/golden impact: native codegen for record/union **construction, field
  read, `WITH`, copy, and pattern matching** changes; expect broad `ncode`/`nplan`
  golden churn and `memory_layouts` doc updates. Runtime output must be identical.
- **Unchanged:** standalone `String` layout, the `String` handle ABI, collection
  layout, the arena allocator/free-list/entropy fill from `plan-01`.

## 6. Phases

1. **Spec + reference model.** Update `memory_layouts.md`; add a Rust reference
   model of record/union size + relative-offset read used by unit tests. Land
   independently.
2. **Record construction + size.** Emit variable-size records with inline String
   fields and relative-offset slots. No reader changes yet — gate behind a
   self-check that round-trips construct→read in one helper.
3. **Field read (records).** Switch every record `String`-field read to
   base-relative resolution. Full overload coverage. Acceptance must stay green.
4. **Unions.** Same construction + read change for union `String` payload slots
   and `MATCH`/extract.
5. **`WITH` update + `Error`/`ErrorLoc`.** Rebuild-on-update; inline the built-in
   record String fields.
6. **Copy → `memcpy` + collection-fixup simplification.** Collapse record/union
   copy to a single `memcpy` for the scalar/String case; drop the now-unnecessary
   collection record-payload String fix-up. Heavy differential tests.

(Each phase keeps records with non-String owned fields working via the existing
per-field pointer copy; only `String` is inlined here.)

## 7. Validation Plan

- Function tests: `tests/func_<pkg>_<func>_valid/**` and `_invalid/**` for every
  record/union/`WITH`/`MATCH`/`Error` surface touched, full overload coverage.
- Runtime proof: programs that (a) build a record with `String` fields, copy it
  (`LET b = a`), mutate nothing, and print both; (b) `WITH`-update a `String`
  field and confirm the original is unchanged (no aliasing); (c) put records with
  `String` fields in a `List`, copy the list, and confirm independence; (d) round
  a record with `String` fields through `thread::transfer`. Each checks exact
  stdout. Run under `plan-01` entropy poisoning so any residual aliasing (a copy
  that shares bytes) surfaces as a loud use-after-free.
- Doc sync: `memory_layouts.md` Record/Union/Error sections; if any error code or
  diagnostic changes, `error_codes.md` / `mfbasic.md` / `standard_package.md`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`,
  goldens re-synced.

## 8. Open Decisions

- **Slot encoding for a `String` field** — *recommend* a bare `U64`
  record-base-relative byte offset to the inline blob (length is read from the
  blob's own `len` header), with offset `0` as the unset/empty sentinel. Alt: pack
  `{offset:u32, len:u32}` into the slot to save the indirection on length reads —
  rejected unless profiling shows the extra load matters, since it duplicates the
  blob header and complicates `WITH`.
- **Relative vs. absolute slot** — *recommend* relative (enables the single
  `memcpy`; the whole point). Absolute pointers would require copy-time fix-up.
- **Variable-size object discovery** — records become variable-size; *recommend*
  computing `totalSize` at construction from argument `String` lengths and storing
  it implicitly via the layout (slot table size is static; data region walked by
  the field blobs). Decide whether to also stash an explicit `U64 totalSize`
  header for O(1) copy sizing vs. recomputing by walking — *recommend* an explicit
  size header (one word, makes copy and `arena_free` O(1) and unambiguous).
- **Scope of inlining beyond `String`** — this plan stops at `String`. Inlining
  nested records/unions (and eventually growable collections) into their parent is
  a much larger follow-up; flag it as plan-03+ and keep pointer payloads for those
  in the meantime.
- **Empty/`""` representation** — *recommend* a zero-length inline blob
  (`len=0`, just the `nul`) rather than a null slot, so reads never special-case.
