# Collections

A collection owns a contiguous block with a header and data region.
Variable-width lists, maps and sets also carry lookup entries; fixed-width
lists omit them. Maps and sets reserve a hash-bucket region.
[[src/codegen/collection/layout/builder_collection_layout.rs:copy_collection_tight]]

## Collection String Payloads

String payloads stored inside a collection data region are UTF-8 bytes only.
The lookup entry stores the byte length.

```text
Data:
  Byte[valueLength] utf8Bytes
```

The collection copy owns these bytes. A String read from a collection may be
materialized as the runtime's ordinary String representation, but the collection
storage itself remains packed bytes.

## Collection Layout

The collection kind determines which regions are present.

```text
CollectionHeader
  U8 kind            ; 0 = List, 1 = Map, 2 = fixed-width List (no LookupEntry), 3 = Set
  U8 keyType         ; 0 for List
  U8 valueType
  U8 flagsVersion
  U8 bucketsReady    ; Map hash index built? 0 = no (rebuild lazily), 1 = yes
  U24 deadEntries    ; Map/Set tombstones inside count; unused for List
  U64 count          ; lookup entries in use (live + deadEntries)
  U64 capacity       ; lookup entry capacity
  U64 dataLength     ; used bytes in data region
  U64 dataCapacity   ; allocated bytes in data region

LookupEntry[capacity]   ; absent entirely when kind = 2
  U8 flags           ; used/deleted/etc.
  U8[7] reserved
  U64 keyOffset      ; Map only, 0 for List
  U64 keyLength      ; Map only, 0 for List
  U64 valueOffset
  U64 valueLength

Data[dataCapacity]
  packed key and value payload bytes

Buckets[2 * capacity]   ; Map and Set only; see Map Hash Index
  U64                   ; entryIndex + 1, or 0 = empty
```

Header and lookup entries have fixed aligned sizes. Version 1 uses a 40-byte
`CollectionHeader` and 40-byte `LookupEntry`. Implementations must not derive
the runtime entry stride from the sum of field sizes without accounting for
padding and alignment.

### Fixed-Width Lists (kind 2)

A list whose element type has a fixed payload width carries **no `LookupEntry`
array at all**. Element `i` is at `Data[i * payloadSize]`, so the entry a kind-0
list would hold is exactly the identity mapping and stores nothing the index
does not already say.

[[src/codegen/error/constants/error_constants.rs:COLLECTION_KIND_LIST_FIXED]]

| Element type | Payload width |
|---|---|
| `Nothing`, `Boolean`, `Byte` | 1 |
| `Scalar` | 4 |
| `Integer`, `Float`, `Fixed`, `Money` | 8 |

Every other element type — `String`, a record, a union, a nested collection, a
resource pointer — is variable-width and keeps the kind-0 layout. A `Map` always
keeps its entries regardless of key or value width, because a map's entry
carries a key offset that is not derivable from the index.

The layouts differ only in the entry array:

|                  | kind 0                            | kind 2                   |
|------------------|-----------------------------------|--------------------------|
| element address  | `Data + entry[i].valueOffset`     | `Data + i * payloadSize` |
| data region base | `block + 40 + capacity*40`        | `block + 40`             |
| block size       | `40 + capacity*40 + dataCapacity` | `40 + dataCapacity`      |

A tight `List OF Byte` with `N` elements occupies `40 + N` bytes before
arena granule rounding.
[[src/codegen/collection/layout/builder_collection_layout.rs:list_element_is_fixed_width]]

Two constraints on any implementation:

- **The data base is derived from `capacity`, never from `count`.** A list built
  by appending carries spare capacity, so a count-derived base lands inside the
  entry array.
- **The allocation size, the free size, and the data base must all read the
  same stride.** They are computed at different sites; if one of them decides
  the representation differently from another, a block is allocated at one
  layout and released at another, which corrupts the allocator's free list
  rather than producing a wrong value.

Kind 2 is an **implementation detail with no surface visibility**. Source cannot
observe which representation a list uses: the element type alone determines it,
`kind` is not readable from MFBASIC, and every operation behaves identically
either way. The `Payload Order` invariant below is what makes the two
interchangeable.

### Sets (kind 3)

A `Set OF T` (`kind = 3`, `COLLECTION_KIND_SET`) is a `Map`-shaped block: a
header, an insertion-ordered lookup table, a packed data region, and a hash index
in the `Buckets` region after the data. It reuses the map machinery verbatim, but
it is a hash-indexed set of **elements, not key/value pairs**: each element is
stored as an entry **key** (in `keyOffset`/`keyLength`), and the entry's value is
a single implementation-detail tag byte carrying no information a program can
read. [[src/codegen/error/constants/error_constants.rs:COLLECTION_KIND_SET]]

Because a set is map-shaped it carries buckets exactly as a map does — the
bucket region is shared through `collection_has_buckets`, which is true for both
kind 1 and kind 3 — so membership (`collections::contains` on a `Set`) is an
O(1)-average hash probe of the element key for a probe-eligible element type
(`Integer`, `Float`, `Fixed`, `Byte`, `Boolean`, `Nothing`, `String`) and a linear scan over
the live lookup entries otherwise, with the index built lazily on first use and
`bucketsReady` set exactly as for a map.
[[src/codegen/engine/types/type_utils.rs:collection_has_buckets]]
The lookup table stays insertion-ordered, so `FOR EACH` over a set and
`collections::toList` visit the elements in insertion order, and a set literal
`Set OF T { … }` builds a tight block (`capacity == count`) with each distinct
element inserted once. A direct shrink-to-fit copy leaves `bucketsReady = 0` for a lazy rebuild,
exactly as for a map; nested whole-block copies follow the rules below.

### Header Fields

- `kind` identifies whether the allocation is a `List` (`0`), a `Map` (`1`), a
  fixed-width `List` carrying no lookup entries (`2`; see *Fixed-Width Lists*), or
  a `Set` (`3`; see *Sets*). [[src/codegen/error/constants/error_constants.rs:COLLECTION_KIND_SET]]
- `keyType` identifies the map key payload type. It is `0` for `List`.
- `valueType` identifies the list item type or map value type.
- `flagsVersion` identifies the layout version and collection-level flags.
- `bucketsReady` is `1` when a `Map`'s hash index (the `Buckets` array) is built
  and `0` otherwise; it is `0` on every fresh, copied, or grown map and set to `1`
  the first time the index is probed (see *Map Hash Index*). It is unused
  for a `List`.
- `deadEntries` (bytes 5..7, read as the `U32` at offset 4 shifted right by 8)
  is the number of **tombstones** among a `Map`/`Set`'s first `count` entries —
  entries an in-place removal flagged `REMOVED` and left in place (see *Map Hash
  Index*, "Tombstones"). Map/Set header construction initializes the `U32` at offset 4 to `0`.
  Lists do not use this metadata word.
  [[src/codegen/error/constants/error_constants.rs:COLLECTION_OFFSET_MAP_STATE]]
- `count` is the number of lookup entries in use. For a `List` every one is live;
  for a `Map`/`Set` the length is `count - deadEntries`.
- `capacity` is the number of lookup entries allocated. It may exceed `count`:
  the spare slots are working-buffer headroom (see *Capacity Headroom*). For a
  kind-2 list no entries are allocated, and `capacity` is the number of element
  slots the `Data` region has room for; the data base is always `block + 40` and the
  block size is `40 + dataCapacity`, independent of the entry capacity.
- `dataLength` is the end of the used bytes in `Data`: every payload lies below
  it. For a variable-width list or a `Map`/`Set` it can exceed the sum of the live
  payload lengths, by the dead bytes an in-place length-changing `set` or an
  in-place removal leaves (see *Compaction*).
- `dataCapacity` is the number of bytes allocated for `Data`. It may exceed
  `dataLength` for the same reason.

`keyType` and `valueType` are compact runtime type identifiers. The **payload
size and alignment** columns are what a decoder strides by; they are not implied
by the identifier's value. [[src/codegen/error/constants/error_constants.rs:COLLECTION_TYPE_SCALAR]] [[src/codegen/collection/layout/builder_collection_layout.rs:collection_payload_alignment]]

| Type                | ID | Payload            | Align |
|---------------------|----|--------------------|-------|
| none/list key       | 0  | —                  | —     |
| `Nothing`           | 1  | 1 byte, always `0` | 1     |
| `Boolean`           | 2  | 1 byte             | 1     |
| `Integer`           | 3  | 8 bytes            | 8     |
| `Float`             | 4  | 8 bytes            | 8     |
| `Fixed`             | 5  | 8 bytes            | 8     |
| `String`            | 6  | `valueLength` (or `keyLength`) UTF-8 bytes | 1 |
| `Byte`              | 7  | 1 byte             | 1     |
| `Money`             | 8  | 8 bytes            | 8     |
| `Scalar`            | 9  | **4 bytes**        | **4** |
| `List`              | 20 | inlined block or pointer, by payload type | 8 |
| `Map`               | 21 | inlined block or pointer, by payload types | 8 |
| other types (including `Set`, records, enums, functions and resources) | 22 | type-directed | type-directed |

The object identifier does not distinguish an inlined block from a scalar
or pointer payload. The statically known element/key/value type supplies that
classification and alignment.
[[src/codegen/engine/types/type_utils.rs:collection_type_code]]
[[src/codegen/collection/layout/builder_collection_layout.rs:collection_payload_alignment]]

> **A `Nothing` payload is the constant byte `0`.** The unit type has one value, so every
> stored payload is identical. A store writes `0` whatever register the value arrived in,
> and any two `Nothing` payloads compare equal without reading them. A `Set OF Nothing` or
> a `Map OF Nothing TO V` therefore holds at most one entry.
> [[src/codegen/error/constants/error_constants.rs:COLLECTION_TYPE_NOTHING]]

> **`Scalar` is the only 4-byte element type.** Every other payload is either
> 1-byte or 8-byte aligned, so a decoder that infers the stride from the shape of
> this table rather than reading the alignment column mis-strides every element of
> a `List OF Scalar`.

> These identifiers are the **runtime** type space. The package wire format uses a
> *different*, independently numbered id space for the same types — see
> `./mfb spec package type-table`. The two must not be read into one another.

### Lookup Entry Fields

- `flags` includes at least a used/deleted state. Every live entry is `used`
  (`1`). An in-place `Map`/`Set` removal marks the entries it takes out `2`
  (`COLLECTION_ENTRY_FLAG_REMOVED`). Tombstones can remain in a mutable
  binding or field until a settling read or compaction; see *Map Hash Index*.
  [[src/codegen/error/constants/error_constants.rs:COLLECTION_ENTRY_FLAG_REMOVED]]
- `keyOffset` and `keyLength` identify the map key payload in `Data`.
- `valueOffset` and `valueLength` identify the list item or map value payload in
  `Data`.

For `List`, lookup entry order is list order. There is no `logicalIndex` field.
The list index is the lookup table index.

For `Map`, lookup entry order is the implementation-defined stable iteration
order — **insertion order**, which the hash index below does not perturb (the
`LookupEntry` array stays insertion-ordered; the buckets are separate derived
metadata). `keys`/`values`/iteration walk this array directly.

### Map Hash Index

A `Map` carries a hash index in the `Buckets` array that sits **after** the data
region (a `List` reserves none). It makes key lookup O(1) average instead of the
linear entry scan, without disturbing the capacity-based data base or the
insertion-ordered entries. [[src/codegen/engine/builder/mod.rs:lower_map_probe_helper]]

- **Size and addressing.** `Buckets` has `2 * capacity` `U64` slots (load factor
  ≤ 0.5), based at `header + capacity*entryStride + dataCapacity`. Each slot holds
  `entryIndex + 1`, or `0` for empty. A key's home slot is a pure function of its
  stored key bytes, chosen by their **length**, and linear probing
  walks on from it:
  - **8 bytes** (an `Integer`, `Float` or `Fixed` key, and a `String` key that
    happens to be 8 bytes long): the bytes read as one little-endian word `k`,
    home = `umulh(k * 0x9E3779B97F4A7C15, 2*capacity)` — a multiplicative
    (Fibonacci) hash whose high bits are reduced onto the slots by a
    multiply-high, with no division. An arithmetic progression of keys spreads
    evenly instead of piling into one residue class. The word is loaded whole
    only from an 8-aligned address (a fixed-width key always is); an unaligned
    `String` key's word is assembled from its bytes.
    [[src/codegen/collection/map/map_hash.rs:word_home_slot]]
    [[src/codegen/collection/map/map_hash.rs:load_key_word]]
  - **Any other length** (a `String`, a 1-byte `Byte`/`Boolean` key):
    `FNV-1a(keyBytes) mod (2*capacity)`.
    [[src/codegen/collection/map/map_hash.rs:fnv_home_slot]]

  Every site that reads or writes the index — the inline probe, the runtime
  probe, the lazy rebuild, the incremental insert and the backward-shift delete
  with its renumbering — computes the home slot from this one definition, so
  they cannot disagree; a site that did would leave a present key unreachable.
  [[src/codegen/collection/map/map_hash.rs:home_slot_of_key_bytes]]
  Equality is the byte-wise comparison over `keyLength` bytes — identical to the
  linear scan (the inline probe compares an 8-byte key as one word, which is the
  same test), so `Float` keys compare bitwise (`+0.0` differs from `-0.0`; any NaN
  bytes compare equal only to the identical bit pattern) and a `Fixed`
  compares by bit pattern.
- **The probe.** When the index is built, a lookup is inline at the call site:
  it computes the home slot and walks the linear-probe run itself, comparing each
  candidate's key, until a match or an empty slot. Only an unbuilt index
  (`bucketsReady = 0`) — and a `String` key of exactly 8 bytes — goes to
  `_mfb_rt_map_probe`, which builds the index first. On a hit the probe yields
  both the entry's address and its index.
  [[src/codegen/builtins/collections/gen_map.rs:emit_map_probe_into]]
- **Derived, lazy, and recomputed — never authoritative.** The entries + data are
  the source of truth; the buckets are rebuildable from them. `bucketsReady` is
  `0` on every fresh allocation (and a fresh allocation's bucket bytes are
  uninitialized), so the index is built lazily on the first probe and `bucketsReady`
  set to `1`. **A shrink-to-fit collection copy does not copy buckets verbatim**: it
  reserves the count-sized bucket region and leaves `bucketsReady = 0`, so
  the destination recomputes against its own capacity. A collection embedded
  in a larger flat block can instead travel with that block verbatim; its
  preserved capacity keeps its bucket indices valid.
- **Incremental maintenance.** In-place `set` inserts a new key into the buckets
  in O(1) when they are already built and invalidates (`bucketsReady = 0`) when a
  capacity grow moves/resizes the region, so building a map with repeated `set`
  stays O(n). The insert reuses the empty slot that ended the `set`'s own probe
  run — exactly where re-hashing the key would put it — which stays valid because
  every event between the probe and the write that could move the block or its
  entries (a grow's fresh header, a reclaim that repacks or drops tombstones)
  clears `bucketsReady`; only when the probe did not walk a built index does it
  call `_mfb_rt_map_bucket_put`, which hashes the key again.
  [[src/codegen/collection/map/map_mutate/set_in_place.rs:lower_map_set_in_place]]
  An in-place value overwrite or value-grow leaves keys/indices unchanged and so
  leaves the index valid. An in-place removal
  (`removeKey`, a `Set`'s `remove`, `difference`, `symmetricDifference`) keeps the
  index built: it deletes each removed entry's bucket by **backward-shift
  deletion** — every later slot of the probe run moves back into the gap unless
  its key's home slot lies cyclically between the gap and that slot, so no
  bucket tombstone is needed and every surviving key stays reachable. What
  happens to the removed **entry** depends on where the map lives:
  [[src/codegen/collection/map/map_bucket_remove.rs:lower_map_bucket_delete_helper]]
- **Tombstones.** For a probe-eligible key type at a binding — a
  local, a module-level global or a by-reference capture — the removed entry
  stays where it is, flagged `COLLECTION_ENTRY_FLAG_REMOVED`, and `deadEntries`
  counts it; nothing moves, so the removal costs the lookup plus O(1)
  (`_mfb_rt_map_retire`). The last entry is truncated instead, together with any
  tombstones directly before it. Once the tombstones outnumber the live entries
  (`2 * deadEntries > count`, or the 24-bit field is full) one pass drops them
  all, sliding the live entries down in order with no hashing, and marks the
  index not ready (every live entry's index changed; the next probe rebuilds it).
  At least as many removals as there are live entries separate two such passes,
  so a removal costs O(1) amortized wherever its key sits.
  [[src/codegen/collection/map/map_bucket_remove.rs:lower_map_retire_helper]]
  A tombstone is visible only to the consumers written for it: a hash lookup
  (`get`, `getOr`, `hasKey`, a `Set`'s `contains` — a tombstone has no bucket and
  the lazy index build skips it), `len`/`isEmpty` (`count - deadEntries`), the
  in-place updates of that binding, the dead-byte reclaim (which drops the
  tombstones before it walks the entries), and the owner's graph drop (which skips
  them — their values were freed at the removal). **Every other read of a `Map`
  or `Set` binding settles it first**: `lower_value` of a map-typed `Local` or
  `Global` calls `_mfb_rt_map_settle` when `deadEntries != 0`, which runs the same
  in-place pass. So a copy, an iteration, `keys`/`values`/`toList`, a callee's
  parameter, a `RETURN`, a thread transfer or any other reader sees a map with no
  tombstones, and pays the pass only when removals left some — a reader that walks
  every entry anyway. A record or `STATE` field's map tombstones the same way
  — a read of the field settles it, and a read of the owner as a whole
  value — a copy, a callee's argument, a `RETURN`, a store, a thread transfer —
  settles every map the owner holds inline, in place, before the record's bytes
  are read; a scalar field read (`rec.before`) and the hash-lookup consumers above
  do not settle. A key type that the entry scan serves never holds a tombstone:
  its removals still slide the later entries down, because a scan would match a
  tombstone's key.
  [[src/codegen/collection/map/map_bucket_remove.rs:lower_map_settle_helper]]
  [[src/codegen/collection/map/map_mutate/mod.rs:emit_map_settle_value]]
  [[src/codegen/collection/map/map_mutate/mod.rs:emit_tombstone_holder_settle]]
  [[src/codegen/collection/map/map_bucket_remove.rs:lower_map_compact_from_helper]]

The data region packs every **flat** payload directly, addressed by the lookup
entry's `valueOffset`/`valueLength`: primitive
payloads, `String` bytes, inlined record blocks, inlined data-union blocks, and —
for **nested flat collections** (a `List`/`Map` whose own payloads
are flat) as their full block (header + lookup table + data region) inlined by
offset, `valueLength` = the block byte size. Because a collection's internal
entry offsets are relative to its own data-region base, an inlined nested collection
relocates correctly under the enclosing block's `memcpy`. The **only** payloads
that remain an 8-byte pointer handle are a **resource** and a **non-flat** nested
collection (one whose own payloads include a function value or a recursive type) — see
`is_pointer_collection_payload_type`. A resource element is an **alias**: the slot
holds the pointer to the one resource record (a resource union's holds its
`{tag, record}` box inline), so a collection of them is a flat block within its
thread. It is copied by `memcpy` at every hand-off to a longer-lived owner and
freed by its one owner like a `List OF Integer`; copying or freeing it closes
nothing. It is never relocated into another thread's arena. [[src/codegen/collection/layout/builder_collection_layout.rs:is_pointer_collection_payload_type]]

A **function value** (`FUNC(...) AS T`, list element or map value) is packed as a
single **8-byte pointer** to the closure object the collection owns (`./mfb spec
memory closures`), 8-aligned, `valueLength` = 8, in a kind-0 list with lookup
entries. Since bug-781 that payload is an owned pointer edge, like a non-flat nested
collection's: a function type is not `type_is_memcpy_copyable`, so a `List`/`Map` of
function values is not a flat block. An owning insert stores a closure the collection
owns (a fresh one, or a copy of an aliased one), `get` hands back a copy, copying the
collection copies every closure in it, and dropping it frees them
(`_mfb_rt_closure_drop`). A record **field** of function type is likewise a pointer edge
of its record. [[src/codegen/collection/layout/builder_collection_layout.rs:type_is_memcpy_copyable]] [[src/codegen/memory/arena/builder_arena_transfer/collection_transfer.rs:payload_edge_shape]] [[src/codegen/collection/layout/builder_collection_layout.rs:record_field_is_pointer]] [[src/codegen/collection/layout/builder_collection_layout.rs:emit_payload_length_to_stack]]

### Capacity Headroom and Growth

`capacity` and `dataCapacity` carry **real headroom**: the codegen append grow
path over-allocates so `capacity > count` and `dataCapacity > dataLength`, and a
later append into the same uniquely-owned `MUT` buffer writes into the spare slot
and bumps `count`/`dataLength` in place — amortized **O(1)** append instead of a
realloc-and-copy per item. The growth shape (an implementation tuning detail, not
an observable contract): lookup slots start at 4, double until 1024, then ×1.5.
For a fixed-width payload — a fixed-width list element, or a `Map`/`Set` entry
whose key and value are both fixed-width — the data region is sized from the
lookup capacity, `dataCapacity = capacity × stride`, where the stride is the
element width, or for a map entry the key and value each padded to their
alignment. A variable-width payload's data bytes grow on their own step instead:
start at 32, double until 64 KiB, then ×1.5. Either way data grows to at least
what the operation needs.
[[src/codegen/collection/buffer/collection_buffer.rs:emit_fixed_width_data_capacity]]

Headroom is retained by working buffers and bounded-output builders; ordinary
value copies tighten the destination:

- Because `LookupEntry[capacity]` precedes `Data`, the data region base is
  `header + capacity * entryStride` — **always derive it from `capacity`, never
  from `count`**. With headroom present a `count`-based base reads the spare
  slots as payload bytes.
- Literals and known-size builders (`transform`/`filter` outputs aside) allocate
  exactly (`capacity == count`, `dataCapacity == dataLength`) — no headroom where
  the final size is known up front.
- A builder whose input only bounds its result allocates once at that bound:
  native `toSet` (fixed-width elements) reserves `capacity = len(value)`, so a
  list with duplicates leaves `count < capacity` in the result.
  [[src/codegen/builtins/collections/func_to_set.rs:to_set_fast_path]]
- A builder that splits its input sizes each piece from what it holds, never
  from a requested size. The native constant-size `chunks` and `window` builds
  size every inner list from `min(size, remaining)` at run time and pack the
  inner blocks back to back in the outer's data region (`dataLength =
  dataCapacity = Σ (header + held × 8)`), so a `size` larger than the input
  reserves nothing for the unused remainder; `size` and `stride` are
  materialized in registers, never folded into a compile-time byte count or an
  add immediate.
  [[src/codegen/builtins/collections/func_chunks.rs:lower_collection_chunks_call]]
  [[src/codegen/builtins/collections/func_window.rs:lower_collection_window_call]]
  [[src/codegen/builtins/collections/func_window.rs:lower_collection_window_string_call]]
- A collection copied through `copy_flat_block` is **shrink-to-fit**:
  the destination has `capacity == count`
  and `dataCapacity == dataLength`. Entries and data are copied separately
  because their bases change when capacity changes. Maps and sets also reserve
  destination buckets, which are rebuilt lazily.
- One copy is a working buffer, not a value: a `List` that `get`/`getOr` binds
  to a `MUT` local whose very next statement is the in-place self-update
  `x = collections::append(x, item)` of a single element. That copy is made with
  the headroom the append's grow would give it — `capacity = step(count)`, and
  `dataCapacity = capacity × width` for a fixed-width element or
  `step(dataLength)` otherwise — so the append writes into the spare slot and the
  copy and the grow are one allocation. It is still an independent copy, and after
  the append the block has the shape the tight copy plus the grow would have
  produced. Any other statement after the bind (the append under an `IF`, an
  intervening read, a bulk append) keeps the tight copy.
  [[src/codegen/engine/analysis/append_headroom.rs:collect_append_headroom_binds]]
  [[src/codegen/collection/layout/builder_collection_layout.rs:copy_list_with_append_headroom]]

Shrink-to-fit applies to the collection being copied, not recursively to every
collection inside it. A whole-block copy of a record, union or `Result`, or
verbatim copying of nested collection payload bytes, preserves those embedded
headers and capacities. Likewise, embedding an existing block sizes it using
its stored capacities. Such copies remain independent, but need not be tight.
[[src/codegen/collection/layout/builder_collection_layout.rs:copy_flat_block]]
[[src/codegen/collection/layout/builder_collection_layout.rs:emit_inlined_block_size_from_ptr_slot]]

### Payload Order

Where there is a lookup table, it — not the data region — defines the sequence.
Element `i`'s payload is located by `entry[i].valueOffset` and
`entry[i].valueLength`, relative to the capacity-derived data base.

For a **fixed-width element type** — `Nothing`, `Boolean`, `Byte`, `Scalar`, `Integer`,
`Float`, `Fixed`, `Money` — payloads are additionally packed in **index order**:
`entry[i].valueOffset == i * payloadSize` after every operation. This is the
invariant that lets those types drop the table entirely (kind 2, above): an
entry that always equals the index carries no information. The rule is stated
in terms of the entry because it is what a kind-0 implementation must maintain,
and because it remains the definition a kind-2 layout is derived from — the
identity mapping made implicit. So walking the
data region linearly visits the elements in index order, and a vectorized kernel
or a `memcpy` to a native API may take the data base and stride it. That is what
makes the `math::` array overloads and the `fs`/`net`/`audio` byte-list writers
correct. `prepend` and `insert` move payload bytes to maintain this, which for
these types is *cheaper* than the alternative: the payload is 1–8 bytes where the
lookup entry it would otherwise splice is 40.

A fixed-width list (`kind = 2`) stores elements in index order without an
entry table. A variable-width list (`kind = 0`) requires lookup entries to
recover element order.

For a **variable-width element type** — `String`, records, unions, nested
collections — payloads are **not necessarily in index order**, and a list
updated by an in-place `set` is **not necessarily dense** either: the data region
may hold unreferenced bytes between payloads. A reader **must not** assume
element `i` begins at `dataBase + i * payloadSize`, must not assume a linear walk
visits elements in index order, and must not take `dataLength` as the size of the
live data. Splicing the lookup table and appending new payloads to the data
tail preserves existing payload offsets. Length-changing `set` can leave holes.
`collections::sort` on a `List OF String` relies on the permutation directly — it
swaps the fixed-size entry records and leaves the data region untouched.

Order is a property of the **value**, not of a moment: it survives every copy.
A value copy copies the entry table and the data region as two verbatim block
copies, so a permuted list stays permuted across assignment, argument passing,
record embedding, and thread transfer. Nothing in the value-copy path normalizes
it.

A consumer that requires a densely-ordered buffer of a variable-width type must
therefore **establish** that order rather than assume it, by one of the two
idioms already in the tree: **probe and repack** — scan the entries against a
running expected offset and take a normalizing fallback when they diverge, as
`collections::mid` does; or **rebuild** — construct a fresh list by appending in
index order, which normalizes as a side effect, as `transform`, `filter` and the
range-index `slice` do.

An alignment of 1 requires no alignment padding; it guarantees neither
index order nor the absence of dead bytes from prior mutations.
[[src/codegen/collection/layout/builder_collection_layout.rs:list_element_is_fixed_width]]

## List Examples

`List OF Integer = [10, 20]` — `Integer` is fixed-width, so this is a kind-2
list with no lookup array. Its logical block size is 56 bytes; the arena
rounds the allocation to 64 bytes.

```text
Header:
  kind = 2
  keyType = 0
  valueType = Integer
  count = 2
  capacity = 2
  dataLength = 16

Data:                     ; based at block + 40
  Integer(10)             ; element 0, at 0 * 8
  Integer(20)             ; element 1, at 1 * 8
```

`List OF String = ["hi", "bye"]`:

```text
Header:
  kind = 0
  keyType = 0
  valueType = String
  count = 2
  capacity = 2
  dataLength = 5

Lookup[0]:
  flags = used
  valueOffset = 0
  valueLength = 2

Lookup[1]:
  flags = used
  valueOffset = 2
  valueLength = 3

Data:
  h i b y e
```

## Map Example

`Map OF String TO Integer { "Ada" := 36 }`:

```text
Header:
  kind = 1
  keyType = String
  valueType = Integer
  count = 1
  capacity = 1
  dataLength = 16

Lookup[0]:
  flags = used
  keyOffset = 0
  keyLength = 3
  valueOffset = 8
  valueLength = 8

Data:
  A d a
  <5 padding bytes>
  Integer(36)
```

The three `String` key bytes occupy offsets `0` through `2`. The `Integer`
value requires 8-byte alignment, so it begins at the next valid offset, `8`,
leaving five unobservable padding bytes at offsets `3` through `7`.

## Operations

### Copy

A collection copy is **shrink-to-fit**: `copy_flat_block` routes a collection to
`copy_collection_tight`, which allocates exactly [[src/codegen/collection/layout/builder_collection_layout.rs:copy_collection_tight]]

```text
List kind 0: CollectionHeader + LookupEntry[count] + Data[dataLength]
List kind 2: CollectionHeader + Data[dataLength]
Map or Set:  CollectionHeader + LookupEntry[count] + Data[dataLength]
             + Buckets[2 * count]
```

The new header sets `capacity == count` and `dataCapacity == dataLength`.
The implementation copies used lookup entries and `dataLength` bytes in separate
passes, deriving the source data base from source capacity and the destination
base from count. Maps and sets start with `bucketsReady = 0`; bucket contents
are not copied. Dead data bytes and payload permutations are preserved. For
recursive element types, owning copies additionally copy their graph edges.
[[src/codegen/collection/layout/builder_collection_layout.rs:copy_collection_tight]]
[[src/codegen/memory/arena/builder_arena_transfer/collection_transfer.rs:own_collection_payload_edges]]

### `get`

For a `List`, `collections::get(value, index)` validates `0 <= index < count`,
then reads `Data + index * payloadSize` for a kind-2 list. For a kind-0
list it reads `LookupEntry[index]` and the payload at `Data + valueOffset`
with `valueLength`. An out-of-range or negative index fails with `ErrIndexOutOfRange`.

For a `Map`, `collections::get(value, key)` looks the key up through the *Map
Hash Index* — an O(1)-average bucket probe that lazily builds the index on
first use — then reads the matching entry's payload at `Data + valueOffset`;
missing keys fail with `ErrNotFound`. The probe covers
`String`, `Integer`, `Float`, `Fixed`, `Byte`, `Boolean`, and `Nothing`;
other admitted key types fall back to a generic linear scan over the live
lookup entries. [[src/codegen/builtins/collections/gen_map.rs:lower_map_get]]

The result is an owned value (`mfb spec language memory-semantics` §14.6): an
element held inline — a record, a data union, a nested collection — is copied
into a fresh arena block (or, for a scratch copy below, into the function's
record-temp scratch), and an element whose type reaches a type cycle is
deep-copied. [[src/codegen/memory/owned.rs:materialize_owned_element]] A `List`
copy bound to a `MUT` local that the next statement appends to in place carries
growth headroom (see *Capacity Headroom and Growth*).

The copy is skipped — the result is the address of the element inside the
container — when no program can tell the two apart. For a record or data-union
element (freeable-flat, not a `String`) that is:

- **A scalar field read straight off the call.** `get(xs, i).f`, through any
  chain of record fields, when the field it finally yields is a numeric,
  `Boolean`, `Byte` or enum value. The field is loaded before anything else
  runs, so the address is dead when the chain ends. Any container, `get` or
  `getOr`. [[src/codegen/memory/value/builder_value_semantics/field_access.rs:scalar_field_of_borrowable_get]]
- **A binding read only as a borrow.** `LET e = get(xs, i)` whose every read is
  a field access or a `MATCH` over it (with that match's variant extracts), and
  which is never stored, returned, passed, captured or reassigned whole — and
  whose container cannot change while it is live:
  - *pinned*: the container is a local bound once, never reassigned and not
    address-taken, and for `getOr` the default is such a local too. The
    function's move and hand-over analyses then leave everything the
    initializer reads alone;
  - *windowed* (`get` only): every read of `e` lies in the statements from its
    bind to the last statement of the same block that reads it, and none of
    those statements rebinds or reassigns a local container or reads it other
    than as the first argument of `get`/`getOr`/`len` or a `FOR EACH`
    iterable, and none can reach a store to a module-level container.
  [[src/codegen/engine/analysis/borrow_get.rs:collect_borrow_gets]]
- **An element-bound update** (see *Self-updates*).

Each binding is classified on its own. A local is never redeclared while it is
live, so a name bound again — in a later loop, in the other arm of an `IF`, or
after a nested block that bound it has ended — names a different binding, and
every rule above is applied to each binding with the reads in its own scope; one
binding's verdict never extends to another of the same name.
[[src/codegen/engine/analysis/borrow_get.rs:collect_borrow_gets]]

The binding registers no scope-drop free: the container owns the element.
A `String` element is always copied — its `get` builds the `String` block, so
there is no element block to point at — and so is a `getOr` over a default
the statement built (on a miss the result would be that temporary).

A copy that cannot be skipped is still not always a block of its own. The
function's **record-temp scratch** is one arena block per activation, shaped like
the self-update scratch, reserved at its first use and freed at every exit;
each value below has a region of its own in it, at an offset fixed at compile
time, so two live at once never overlap. The block is only ever asked for the
sum of its regions, so it never moves while a binding holds an address into it
— which is why it is not the self-update scratch, which is freed and
reallocated when an arm needs more room. It holds, for a record whose byte size
is fixed at compile time (every field a scalar or such a record, `vector::Float3`
or a record of `Float`s):

- **A scratch copy.** A `get` (not `getOr`) binding, every bind of the name a
  `get` of the same type, never reassigned, address-taken or captured by
  reference, whose every read is a field access or the target of a `WITH` that
  is the item of a `set`/`append`. Its `get` borrows the element and copies it
  into the region at once, so it is a copy exactly as the block would be: the
  two elements of an n-body pair step stay independent (`bj` never sees the
  write to slot `i`, even when `i = j`), and neither allocates.
  [[src/codegen/engine/analysis/borrow_get.rs:collect_borrow_gets]]
- **The `WITH` item of `x = set(x, k, WITH e { … })` or
  `x = append(x, WITH e { … })`.** The target and every update value are
  evaluated first, in source order; only then are `e`'s bytes copied into the
  region and the updated fields stored over them — a literal of an all-scalar
  record (`vector::Float3[…]`) field by field, with no block of its own — and
  the arm copies the region into the container. A failing update value writes
  nothing, and `e` may be the element being replaced.
  [[src/codegen/engine/control/builder_control.rs:lower_with_update_in_record_temp]]

A region is rewritten only by the next evaluation of its own site, after which
nothing reads the previous value; a recursive call or a callback has a block of
its own. [[src/codegen/collection/assign/self_update.rs:install_record_temps]]

### Self-updates

A self-update — `x = f(x, …)` of a `List`, `Map` or `Set` for a builtin `f`, and
`s = s & t` or `s = f(s, …)` of a `String` — mutates `x`'s own block instead of
building a new value, wherever `x` is bound: a function local, a module-level
global, a local inside a `FOR EACH` over itself, or a `MUT` captured by reference
in a `collections::forEach` lambda. The operation's result and every other value
are exactly what the copying form would give; the difference is that no second
copy of `x` exists. [[src/codegen/collection/assign/self_update.rs:try_inplace_self_update]]

The same mutation happens at a `RETURN`, which is what lets a collection be
threaded through a helper without copying:

- `RETURN f(x, …)` where `x` is an owned local read for the last time updates
  `x`'s block and then moves that block out as the return value, exactly as
  `RETURN x` already moves it.
  [[src/codegen/engine/control/builder_exits.rs:try_returned_self_update]]
- The same `RETURN` inside an owned-parameter variant (see the calling
  convention) updates the parameter's block, so
  `acc = helper(acc, …)` costs what the inline self-update costs.
- `RETURN WITH r { f := g(r.f, …) }` updates the field inside `r`'s own block and
  moves `r` out, which is the same mutation for a record accumulator.
  [[src/codegen/engine/control/builder_exits.rs:try_returned_field_self_update]]
- `RETURN WITH r { … }` whose every update is a field `r = WITH r { … }` would
  store in `r`'s own block — a scalar, a pointer field, a fixed-size inlined
  record, or the last variable-width inlined field (replaced in place when its
  16-byte granule is unchanged, else `r` is rebuilt) — stores those fields and
  moves `r` out. Every new value is computed before the first store, so a failing
  one leaves `r` as it was. Inside an owned-parameter variant this is what makes
  `w = step(w, …)` over a record of scalars cost no allocation per call.
  [[src/codegen/engine/control/builder_exits.rs:try_returned_field_store]]
  [[src/codegen/engine/analysis/handover.rs:returned_field_store_local]]

A `String` carrying a capacity shadow is the exception at a `RETURN`: its
block must be tight to leave the frame that tracks its capacity, so that return
copies its exact length. A fresh tight String without a capacity shadow may
move through the ordinary owned-return path.
[[src/codegen/engine/control/builder_exits.rs:returned_move_admits]] For the same reason there is nothing to
gain by handing a bare `String` argument over, and it is never handed over — see the
calling convention, which also gives the soundness reason.

`acc = helper(acc, …)` on a `String` therefore still costs a copy per call, while
`acc = acc & piece` in the function that owns `acc` is amortized O(1).

For a `String` the builtins fall into four families, and every one of them writes
into `s`'s own block:

- **identity** — `s = toString(s)` emits nothing at all;
- **window** — `strings::left`, `right`, `mid`, `stripPrefix`, `stripSuffix`,
  `trim`, `trimStart`, `trimEnd`, `trimChars`, `graphemeAt`, `fs::pathBaseName`,
  `pathDirName`, `pathExtension`: the result is a run of `s`'s own bytes, moved
  down to the start of the block — or left where it is when it already starts
  there (`stripSuffix`, a `stripPrefix` that matches nothing, `trimEnd`, `left`
  and `mid` from 0);
- **grow** — `strings::padLeft`, `padRight`, `padLeftToWidth`, `padRightToWidth`,
  `repeat`, `os::appResourcePath`: the added bytes are written into the block's spare
  capacity, which grows geometrically, so a loop that grows `s` allocates
  `O(log n)` times;
- **rewrite** — `strings::upper`, `lower`, `caseFold`, `normalizeNfc`,
  `strings::replace`, `fs::pathNormalize`: the result is built in the function's
  self-update scratch (a writer would otherwise overtake the reader — `ß`
  uppercases to `SS`) and copied back.
  [[src/codegen/collection/assign/string_self_update/window.rs:try_inplace_string_window_assign]]

`fs::readText`, `fs::canonicalPath`, `io::input`, `os::getEnv`, `os::getEnvOr`,
the `encoding` codecs, `net::percentDecode` and `regex::replace` have no in-place
form: their result is not built from `s`'s bytes. They read `s` without copying
it (the host reads the block's own NUL-terminated bytes), so there is no copy to
avoid.

A `String` binding that a self-update leaves shorter than its block keeps the
bytes it gave up as **spare capacity**, tracked by the compiler beside the
binding — in a frame slot for a local, a hidden global beside a global, and, for a
`MUT` captured by reference, in the OWNER's slot, whose address the closure
environment carries so the lambda and its creator always agree.
[[src/codegen/collection/assign/string_self_update/mod.rs:string_shadow_captures]]
The spare capacity is never observable: `len` and every copy, return and transfer
read `byteLength` bytes and hand on the tight form.

- **Failure atomicity.** Every error the operation can raise — a callback's, an
  index or domain error, `ErrOutOfMemory` — is raised before `x` is written, so a
  failed self-update leaves `x` unchanged.
- **Snapshots stay snapshots.** A `FOR EACH` over `x` whose body writes `x` walks
  the value `x` held when the loop began (a copy made once at entry); a call
  handed `x` while a callee or callback can write it receives the value at the
  call. [[src/codegen/engine/control/builder_control.rs:lower_for_each]]
  A value the loop cannot copy (a type that reaches a type cycle) is walked in
  `x`'s own block. A reassignment of `x` in the body does not free that block,
  which the loop still reads: it hands it to the loop, which frees it once it is
  done, on every exit (`EXIT FOR`, `RETURN`, an error, an outer loop's `EXIT FOR`
  or `CONTINUE FOR`). Every other block a reassignment replaces is freed at the
  reassignment, so the loop leaks nothing.
  [[src/codegen/engine/control/builder_control.rs:emit_defer_walked_block]]
- **No per-statement allocation.** State an operation needs per element lives in a
  per-function scratch block, grown geometrically; a lambda borrows the scratch of
  the function that created it. A global `String`'s spare capacity is kept in a
  hidden global beside it. [[src/codegen/collection/assign/self_update.rs:add_global_string_capacities]]
- **Cost.** A self-update costs the elements it adds, removes or moves, or, for
  an operation that tests or rewrites every element (`filter`, `distinct`,
  `replace`, `transform`, `sort`, `sortBy`, the element-wise `math` functions,
  `intersection`, `mapValues`, the `String` rewrite family), the elements it
  tests. It never costs the size of the part of `x` it leaves alone, at any
  optimization level. A lookup costs what that key type's lookup costs anywhere:
  an O(1)-average hash probe for a probe-eligible key type (see *Map Hash
  Index*), an entry scan otherwise.
  [[src/codegen/collection/assign/self_update.rs:try_inplace_self_update]]
  - `take`, `drop` and `mid` compact `x` in place. A range that removes
    nothing (`take(x, len(x))`, `drop(x, 0)`, `mid(x, 0, len(x))`) is O(1). A
    range that starts at 0 is a truncation: the kept elements do not move, so
    it costs the removed elements. On a variable-width list `dataLength`
    retracts over each removed payload that is the data tail (always so for a
    list built by `append`); a removed payload that is not — after a `sort`,
    `prepend`, `insert` or relocating `set` — becomes dead bytes. A range that
    starts past 0 moves every kept element and costs that. The dropped-element
    pass frees only the removed elements' graphs.
    [[src/codegen/collection/list/list_compact.rs:emit_compact_range_truncate]]
    [[src/codegen/collection/list/list_compact.rs:lower_list_compact_in_place]]
  - `x = append(x, mid(x, start, count))` on a fixed-width list (`Byte`,
    `Boolean`, `Scalar`, `Integer`, `Float`, `Fixed`, `Money`), with `start` and
    `count` built from literals, other locals and `+`/`-`, copies the slice
    straight from `x`'s own data into its tail: it costs the `count` elements,
    plus at most one grow, and builds no temporary list. `mid`'s range checks run
    first and raise exactly what `mid` raises.
    [[src/codegen/collection/list/list_mutate.rs:lower_list_self_slice_append_in_place]]
  - `repeat` costs the `count` positions it writes, plus at most one grow — which,
    like any grow, copies `x` — when the run passes the end or the tail cannot
    take `count` more payloads. For an element type that reaches a type cycle it
    also costs one graph copy of `item` per position, built before the first
    write. See [`repeat`](#repeat).
    [[src/codegen/collection/assign/builder_inplace_assign/mod.rs:try_inplace_repeat_assign]]
  - `removeAt` shifts following payloads on a fixed-width list. On a
    variable-width list it shifts following entries without moving surviving
    payloads; the removed payload is retracted when it is the data tail and
    otherwise becomes dead bytes. Its cost is linear in the following elements.
    [[src/codegen/collection/list/list_mutate.rs:lower_list_remove_at_in_place]]
  - `removeKey` and a `Set`'s `remove` on a local, global or by-reference
    binding, or on a record or `STATE` field, cost the lookup plus O(1)
    amortized: the entry becomes a tombstone, or is truncated at the back, and
    the tombstones are dropped in one pass once they outnumber the live entries
    (see *Map Hash Index*, "Tombstones"). For a key type without a hash probe they
    cost the lookup plus the entries after the removed one, which slide down with
    their buckets renumbered. The hash index stays built either way.
    [[src/codegen/collection/map/map_mutate/mod.rs:lower_map_remove_key_in_place]]
  - `difference(x, t)` looks each element of `t` up in `x` and retires each hit as
    `remove` does: it costs `t`'s elements, at a binding or a field alike. Without
    a hash probe it drops the hits in one pass from the first of them instead,
    costing `t`'s elements plus the entries of `x` after the first hit.
    `symmetricDifference(x, t)` is the same pass, then appends `t`'s misses. [[src/codegen/collection/assign/builder_inplace_setmap/set_ops.rs:try_inplace_difference_assign]]
    [[src/codegen/collection/assign/builder_inplace_setmap/set_ops.rs:try_inplace_symmetric_difference_assign]]
  - A `String` window costs the bytes it gives up plus the bytes it moves: none
    move when the window already starts at the block start. `trimEnd` and the
    end of `trim` step back over the trailing whitespace only.
    [[src/codegen/collection/assign/string_self_update/window.rs:try_inplace_string_window_assign]]
    [[src/codegen/builtins/strings/gen_trim.rs:lower_strings_trim_window]]
  - Dead bytes left by an in-place removal cost nothing until the list's data
    region next grows: every in-place grow of a variable-width list first totals
    the live bytes (the grow already copies every entry) and repacks over them
    (see *Compaction* for the exact rule), so a loop that keeps removing and adding stays
    within a constant factor of its live bytes. A `Map`/`Set` grow does the same
    for dead key and value bytes. See *Compaction*.
    [[src/codegen/collection/list/list_mutate.rs:emit_list_reclaim_before_grow]]
- **Functions with no in-place form** (`reduce`, `reduceRight`, the `compress`
  codecs, `crypto::argon2id`, `crypto::shake256`) read `x` without copying it; the
  result is a new value.

A **field** of a record or of a `RES … STATE` payload is a site too:
`r = WITH r { f := op(r.f, …) }`, `h.state.f = op(h.state.f, …)`, and the same
through a module-level record, a nested record field (`o.inner.f`), a `FOR EACH`
over the field, or a record captured by reference. The field is updated where it
lies in its owner's block; the owner is not rebuilt.
[[src/codegen/engine/control/builder_control.rs:field_self_update_site]]

- **Scalar and fixed-size fields are stored.** A `WITH` whose every update is a
  scalar field, a pointer field (a `json::Json`) or a record of compile-time fixed
  size (`vector::Float3`, `color::Color`, the `datetime` types) evaluates every new
  value first, in source order, then writes each into its slot or over its
  sub-block. A failing value therefore leaves the owner unchanged.
  [[src/codegen/engine/control/builder_control.rs:try_inplace_scalar_fields]]
- **A nested `WITH` of scalars is stored too.** An update
  `inner := WITH r.inner { … }` of an inlined record field whose every update is
  a scalar field of `inner`, beside any of the fields above, stores those
  scalars into `inner`'s sub-block. A scalar slot is 8 bytes whatever its value,
  so the sub-block keeps its size and no other field moves; this holds at any
  inlined position and over any owner. Its values are evaluated with the others,
  in source order, before the first store.
  [[src/codegen/engine/control/builder_control.rs:nested_scalar_with]]
- **One collection field beside scalars.** A `WITH` updating one collection field
  with a self-update and other fields with scalars runs the collection's
  operation in place and stores the scalars. `WITH` evaluates its updates in source
  order against the old record: a scalar is computed before the operation mutates
  anything, and a scalar written after the operation in source order is moved
  ahead only when it has no effect and neither it nor the operation can fail with
  a different error; otherwise the whole `WITH` is rebuilt.
  [[src/codegen/engine/control/builder_control.rs:try_inplace_mixed_with]]
- **Growth needs the last field.** An operation that can reallocate the
  collection (a growing insert, a repack) grows the owner's block, so it runs in
  place only when the field — and every record on a nested path — is its owner's
  last inlined field; elsewhere the statement is rebuilt. An operation that only
  shrinks or rewrites at the same width runs at any field.
  [[src/codegen/collection/assign/inplace_dest.rs:field_realloc_admitted]] A
  `set` of a variable-width list element (`List OF String`) changes length in the
  list's own block — overwrite, tail extend, or a write at the data tail — and
  grows the owner only on the overflow repack, so it runs in place at the last
  inlined field. Its item may read the owner only through an operation that builds
  a new value (`set(r.xs, j, get(r.xs, j) & "!")`); an item that could still point
  into the owner (`get(r.xs, k)` itself) rebuilds, because the repack frees the
  old owner before the item is copied.
  [[src/codegen/collection/assign/builder_inplace_assign/field.rs:lower_field_set]]
  The same holds for every growing field operation — `append` (one element or a
  whole list), `prepend`, `insert`, a `Map` field's `set` (key and value) and a
  `Set` field's `add`: the grow reallocates the owner and frees the old block
  before the item is copied in, so an item that reads the owner (`r.s`, an
  inlined record `r.p`, a forwarded copy of either) and is not a scalar and not
  an operation that builds a new value rebuilds the statement.
  [[src/codegen/collection/assign/inplace_dest.rs:field_item_may_view_owner]]
- **A variable-size last field keeps the block when its granule does.** A `WITH`
  that replaces a `String` or variable-size record field — alone or beside
  scalar and fixed-size fields — that is the last inlined field of a flat,
  pointer-free owner decides at run time. A record has no size word: every free
  and copy walks it, and the walk ends at the last inlined sub-block (`./mfb spec
  memory heap-values`). When the owner's walked size before and after the update rounds up to
  the same 16-byte arena granule, the new bytes are written over the old
  sub-block and the chunk `arena_alloc` handed out is still exactly the one every
  later walk frees. Otherwise the owner is rebuilt: a fresh block of the new
  walked size, the old one freed at its own walked size. A `RES … STATE`
  payload, a nested path, an `Error`, and an owner a live `FOR EACH` walks are
  always rebuilt; an element-bound owner follows the list-element rule below.
  [[src/codegen/engine/control/builder_control.rs:replace_field_admitted]]
  [[src/codegen/engine/control/builder_control.rs:emit_field_replace_or_rebuild]]
- **Rebuilt by design.** Any other field whose size depends on its value — a
  `String` or variable-size record that is not the last inlined field, a
  collection updated by an operation with no in-place form, a data union — is
  rebuilt: its new value's size is known only after it is built.
  [[src/codegen/engine/control/builder_control.rs:record_field_is_inlined_fixed]]
- **A copy of the field feeds the same site at the default level.**
  `LET cur = r.f` followed by `r = WITH r { f := op(cur, …) }` (or
  `h.state.f = op(cur, …)`, or through a module-level record) reaches the site as
  if `r.f` were written directly: at `-O1` and above, the "Place copy forwarding"
  optimizer row deletes `cur` and reads `r.f` in its place when every read of `cur`
  comes before the owner's next write. Where it cannot prove that — the owner is
  written before a later read, the write is nested in the statement holding the
  last read, a read is in a `TRAP` handler, or (for a global or `STATE` owner) the
  statements in between can run user code or pass the handle on — `cur` stays a
  copy and the owner is rebuilt. At `-O0` it is always a copy.
  [[src/optimizer/opt1/placefwd.rs:forward]]

A **list element** is a field site too. Both spellings update element `i` where
it lies in `xs`'s data region:

```text
MUT p = collections::get(xs, i)       ' xs = collections::set(xs, i,
…reads of p's fields…                 '   WITH collections::get(xs, i) { … })
p = WITH p { … }
xs = collections::set(xs, i, p)
```

`p` holds the element's address, the `WITH` is lowered by the field routes above
with the element as the owner, and the `set` writes nothing. This is exact
because the `WITH` and the `set` are adjacent statements (nothing can observe
`xs` between them), `i` is built from constants and locals none of which is
written before the `set` (so the `set` names the slot the `get` read), `p` is
read only through its fields before the `WITH` and never after the `set`, and
`xs` is a local that is neither rebound nor read other than by a lookup, or a
global nothing in between can store to. The `get` raises a bad index before
anything is written, and the field routes are failure-atomic, so a failed
update leaves `xs` unchanged. [[src/codegen/engine/analysis/borrow_get.rs:ElementBinding]]

No route may grow the element: its block is a span of the list's data region,
not an allocation. Every growing field route asks
`field_is_last_inlined`, which answers no for an element owner. An update no
route serves — a growing field, a second collection field — is lowered as
`xs = collections::set(xs, i, WITH p { … })`: the `WITH` builds a new record
from the element and `set` replaces it, resizing the payload when it must.
[[src/codegen/engine/control/builder_control.rs:lower_element_update]]
[[src/codegen/collection/assign/inplace_dest.rs:field_is_last_inlined]]

A `WITH` that replaces one `String` or variable-size record field of the element
— at any inlined position, beside scalar fields and nested `WITH`s of scalars —
decides at run time. The element's entry records its exact byte length, so the
new bytes are written over the old sub-block only when the new value's size
equals the old one's: no offset and no length changes. Otherwise the statement
is that same `set` of a `WITH`, built from the values already computed — none is
evaluated twice. So `xs = set(xs, i, WITH p { inner := WITH p.inner { n := … } })`
allocates nothing, and neither does `inner := step(p.inner, …)` while `inner`'s
size is unchanged: `step`'s destination variant writes the result into
`p.inner`'s bytes (`./mfb spec memory native-calling-convention`, *Destination
variants*).
[[src/codegen/engine/control/builder_control.rs:element_replace_admitted]]
[[src/codegen/engine/control/builder_control.rs:emit_element_replace_or_write_back]]

### `append`

`List` `append` has two paths:

- **In-place (`MUT`, amortized O(1)).** When the buffer is a uniquely-owned `MUT`
  working buffer with headroom (`capacity > count` and enough `dataCapacity`),
  `lower_list_append_in_place` writes the new item payload at `Data + dataLength`, [[src/codegen/collection/list/list_mutate.rs:lower_list_append_in_place]]
  fills the next spare lookup entry, and bumps `count`/`dataLength` in place — no
  reallocation. If headroom is insufficient it first grows the buffer using the
  geometric shape in *Capacity Headroom and Growth* (reallocate-and-copy once,
  with fresh headroom).
- **Snapshot/value path.** A value-semantic append produces a fresh allocation
  holding the existing items plus the new one.

### `set`

`collections::set(name, index/key, item)` on a uniquely-owned `MUT` local (the
`name = collections::set(name, …)` self-assignment idiom) mutates the live buffer
in place, like `append`. Inside a `FOR EACH` over the binding the loop walks its
own copy (see *Self-updates*), so the overwrite is not observable to it.
[[src/codegen/collection/list/list_mutate.rs:lower_list_set_in_place]]

- **`List`.** When the replacement payload is the **same size**
  (`newValueLength == oldValueLength` — always true for fixed-width elements and
  same-size records/strings) the value bytes are overwritten at the entry's
  `valueOffset` in place: no allocation, no copy, offsets unchanged. A size
  change on a variable-width element moves no other element's bytes or offset:
  - a **shorter** payload is written at the entry's `valueOffset`; the bytes it
    no longer uses become a hole and `dataLength` is unchanged;
  - a **longer** payload whose old span ends at `dataLength` grows where it lies,
    and `dataLength` becomes `valueOffset + newValueLength`;
  - any other **longer** payload is written at `dataLength` rounded up to the
    element alignment, the entry's `valueOffset` is repointed there, and its old
    span becomes a hole.

  When the growth or the tail write would pass `dataCapacity`, the element's
  `valueLength` is zeroed and `emit_repack_list_data` repacks the live payloads
  into a fresh block with geometric headroom (see *Compaction*), after which the
  payload is written at the new tail. Kind-2 (fixed-width) lists never change
  size. An out-of-range index fails with `ErrIndexOutOfRange`, like the value
  path.

  The value path checks the index against the argument before it allocates, so
  a failing call leaves no result allocation. A fixed-width element takes a
  tight copy (`copy_collection_tight`) and overwrites in place on the copy,
  after `emit_list_index_range_check`. Any other element runs `removeAt` then
  `insert`, and `removeAt` checks before it allocates.
  [[src/codegen/builtins/collections/func_set.rs:lower_set]]
  [[src/codegen/builtins/collections/gen_mutate.rs:emit_list_index_range_check]]
- **`Map`.** `lower_map_set_in_place` locates the key with the same hash probe as
  `get` (linear-scan fallback for non-probe key types). A hit whose new
  value fits overwrites in place. A larger value is appended to the data tail
  and its entry repointed, leaving the old span dead; insufficient room first
  triggers reclaim and, if still needed, geometric buffer growth. The key's
  entry index and insertion position are retained. A miss writes the key+value into a spare lookup slot and
  the spare data tail — the entry packed exactly like a literal entry (key then
  value, each aligned to its payload alignment) — and bumps `count`/`dataLength`,
  growing the buffer geometrically (capacity stepped, `dataCapacity` sized from it
  for a fixed-width key and value and stepped independently otherwise — see
  *Capacity Headroom and Growth* — entries and data copied verbatim against the
  capacity-based base)
  when full. Insertion order is preserved, and the new key is folded into the hash
  index per *Map Hash Index* (into the probe's empty end slot when built, or
  `bucketsReady = 0` when a grow moved the bucket region).
  [[src/codegen/collection/map/map_mutate/set_in_place.rs:lower_map_set_in_place]]

The source `collections::sort` is an insertion sort built on `set`, so its
per-swap `items = collections::set(items, j, …)` overwrites run in place:
the sort is a stable in-place O(n²)-comparison / O(1)-swap pass over a copy of the
argument (the argument itself is never modified). That body is the
fallback: a copying `sort`/`sortBy` whose shape the 8-byte word merge declines
(any record element, a `Float`/`Byte`/`Scalar` element, any key type the word merge
does not take) copies the argument once — a tight copy, or the graph copy for a
type-cycle element — and sorts that copy with the self-update arm's stable
index-permutation merge, so it costs what `MUT s = xs` / `s = sortBy(s, keyFn)`
costs. The source body stays for a function that cannot take the
self-update scratch.
[[src/codegen/collection/assign/builder_inplace_sort.rs:lower_copying_sort_by]]
[[src/codegen/collection/assign/builder_inplace_sort.rs:lower_copying_sort]]

### `insert`

`List` `insert` is **not** an in-place shift. `lower_list_insert_collection`
allocates a fresh **tight** buffer sized for `count + insertedCount` entries and [[src/codegen/collection/list/list_mutate.rs:lower_list_insert_collection]]
`dataLength + insertedDataLength` bytes, copies the pre-insertion data region then
the inserted data region verbatim, splices the lookup table (head, inserted,
tail), and writes a tight header (`capacity == count`, `dataCapacity ==
dataLength`). When the source list's dead bytes outnumber its live ones, only its
live payloads are copied, packed in entry order (see *Compaction*). No existing
entry is shifted in place. The index is range-checked (`0 <= index <= count`,
`ErrIndexOutOfRange`) before the inserted argument is normalized to a
singleton list, so an invalid index allocates no result or singleton buffer.
[[src/codegen/builtins/collections/func_insert.rs:lower_insert]] (A self-update `x = insert(x, …)` is the in-place form, under
*Self-updates*.)

### `removeAt`

The value form of `List` `removeAt` allocates a new block. It removes the
selected payload's span but can retain dead bytes already present in the source. `lower_list_remove_at` validates the index (`ErrIndexOutOfRange` on a bad
one) and allocates a fresh tight buffer sized for `count - 1` entries and
`dataLength - removedValueLength` bytes. Removing one entry punches a **single
contiguous hole** in the data region — the removed payload
`[removedValueOffset, removedValueOffset + removedValueLength)` — so the copy is
four block moves, not a per-payload re-pack: the entry table copies as two
verbatim spans (prefix `[0..index)`, suffix `[index+1..count)`), and the data
region copies as two verbatim blocks (the bytes before the hole, then the bytes
after it shifted left by `removedValueLength`). A final cheap pass over the
survivors subtracts `removedValueLength` from every `valueOffset` that lay **past**
the hole (`valueOffset > removedValueOffset`) — testing each entry's own offset,
not its list index, so it is correct whatever order the data region is in (a list
built with `insert`/`prepend`/`set` packs the spliced payload at the data tail, so
`entry[0]` can point past the hole and shift while a later entry does not). The
data region keeps its existing order minus the hole rather than being re-packed
into list order; the observable value and tight sizing are unchanged.
[[src/codegen/collection/list/list_mutate.rs:lower_list_remove_at]]

### `repeat`

`collections::repeat(value, item, count, start = 0)` stores `item` at list
positions `[start, start + count)`: positions below `len(value)` are overwritten,
the rest are appended, so the result's length is `max(len(value), start +
count)`. `Map` and `Set` have no overload.

- **Validation, before anything is written or copied.** `count < 0` raises
  `ErrInvalidArgument`; then `start < 0` or `start > len(value)` raises
  `ErrIndexOutOfRange` (`start == len(value)` is a pure extend). The count check
  runs first, so a call wrong in both raises `ErrInvalidArgument`.
  [[src/codegen/collection/list/list_repeat.rs:emit_list_repeat_validate]]
- **Value path.** `lower_repeat` takes a tight copy of `value`
  (`copy_collection_tight`) and runs the in-place core on the copy, the `set`
  shape. [[src/codegen/builtins/collections/func_repeat.rs:lower_repeat]]
- **One grow.** The core reserves for the whole call before its first write:
  `target = max(len(value), start + count)` entries, and a data region of `target × width` for
  a fixed-width element or `align(dataLength) + count × align(itemLength)` for a
  variable-width one — the most tail bytes the writes can take, since each
  append and each relocating `set` writes at most one aligned payload at the
  tail. When either does not fit it grows once: a variable-width list first
  tries the dead-byte reclaim (`emit_list_reclaim_before_grow`, see
  *Compaction*), then `newCapacity = max(step(capacity), target)` and the data
  capacity follows *Capacity Headroom and Growth* (tied to the capacity for a
  fixed-width element), entries and data copied verbatim. For a non-recursive payload the reservation is the only allocation in
  this core; recursive payload ownership can require additional graph copies
  as described below.
  [[src/codegen/collection/list/list_repeat.rs:lower_list_repeat_in_place]]
- **In-place path.** `x = collections::repeat(x, …)` runs the same core on `x`'s
  own block (see *Self-updates*): the operands are evaluated in source order
  (`item`, `count`, `start`), then the validation and the one reservation, all
  before the first write, so a failing call leaves `x` unchanged. An overwritten
  element's graph is dropped (`x` owns it). For an element type that reaches a
  type cycle every position needs a graph of its own, and a graph copy can fail
  with `ErrOutOfMemory`, so the `count` copies are made first: the value path's
  shape builds them as a private list (on an empty list, then
  `own_collection_payload_edges`), the write loop takes element `i - start` of
  that list instead of `item`, and afterwards only the private list's block is
  freed. The call can grow, so at a record or `STATE` field it runs in place
  only at the owner's last inlined field with no operand reading the owner
  (`field_realloc_admitted`), growing the owner's block through `InlineGrow`;
  at any other field the statement is rebuilt.
  [[src/codegen/collection/assign/builder_inplace_assign/mod.rs:try_inplace_repeat_assign]]
  [[src/codegen/collection/assign/inplace_dest.rs:field_realloc_admitted]]
- **Writes.** Position by position: an index below the current `count` goes
  through the `set` in-place primitive (same-size overwrite, or the shorter /
  longer rules under [`set`](#set)); an index at `count` through the `append`
  in-place primitive. Neither grows, because the reservation covers both.
- **Element graphs.** Every position holds its own copy of `item`. On the value
  path the tight copy's payloads are byte copies whose graph edges (a type that
  reaches a type cycle) still belong to `value`, so the overwrite does not drop
  the replaced element's graph (`lower_list_set_in_place_with`,
  `drop_replaced = false`); the abi-inline dispatch then deep-copies every
  payload's edges on the result (`own_collection_payload_edges`), which gives
  each repeated position a graph of its own.
  [[src/codegen/collection/list/list_mutate.rs:lower_list_set_in_place_with]]
  [[src/codegen/memory/arena/builder_arena_transfer/collection_transfer.rs:own_collection_payload_edges]]

### Map Updates

For `Map`, setting or removing keys updates lookup entries and packed key/value
payloads. Key lookup goes through the *Map Hash Index* (O(1)-average hash probe,
with a linear-scan fallback for any non-probe key type). Missing removed keys are
ignored. In-place `set` (insert into spare headroom, or value overwrite) is
described under [`set`](#set) above. The copying `removeKey` builds a tight
buffer and leaves `bucketsReady = 0` for a lazy rebuild; the in-place one (a
self-update) deletes the entry — a tombstone at a binding — and keeps the index
built (see *Self-updates*, "Cost", and *Map Hash Index*). The removed key and
value bytes stay behind in the data region as dead bytes until the map's next
grow reclaims them (see *Compaction*).

## Compaction

The value-semantic update operations (`insert`, `removeAt`, and the value-path
`append`) **always** produce a fresh, tight buffer with no slack capacity. The
data region is copied verbatim, so a source's holes are carried with it, unless
they outnumber its live bytes (below).

Dead space enters a buffer only through in-place self-updates:

- a length-changing `set` on a variable-width list leaves the replaced payload's
  old span unreferenced (see [`set`](#set));
- an in-place removal from a variable-width list — `removeAt`, a truncating
  `take`/`drop`/`mid`, and the out-of-order form of the other shrinking
  compactions — leaves each removed payload that is not the data tail;
- an in-place `Map`/`Set` removal leaves the removed key and value bytes.

A variable-width list's dead space is reclaimed by the one compactor in the
codegen, `emit_repack_list_data`. It copies each live payload, by its own entry,
packed in entry order into a fresh block whose data capacity is the geometric
step of the live bytes plus the pending write plus `count`. It runs when a `set`'s
tail write does not fit `dataCapacity`, when an out-of-order shrinking
compaction leaves more dead bytes than live ones, and at the start of every other
in-place grow of the data region (`append`, bulk `append`, `insert`, `prepend`,
and their record-field forms) — the grow first totals the live bytes, an
O(count) walk its own entry copy already pays for, and repacks when
`dataLength > 2 * live`, or, while the entry table still has a free slot, when
`dataLength > live` (any dead bytes at all). The second rule matters because a
verbatim grow steps entry `capacity` along with `dataCapacity`, while a repack
keeps entry capacity and sizes data capacity from live bytes. A data-only
overflow over dead bytes therefore repacks; `capacity` grows only when the
entries are full, and the data capacity stays within a constant factor of the
live bytes. Both tests compare, never subtract (`live` counts entry-order
alignment pads, which an out-of-order data region can undercut), and right after
a repack neither holds, so the retried grow cannot repack again. [[src/codegen/collection/list/list_mutate.rs:emit_repack_list_data]]
[[src/codegen/collection/list/list_mutate.rs:emit_list_reclaim_before_grow]]

The copying `insert`/`append`/`prepend` (`lower_list_insert_collection`, also
the rebuild a growing update takes at a field that is not its record's last
inlined field) totals the source's live bytes the same way and, when the dead
bytes outnumber them, copies only the live payloads, packed in entry order. So a
loop that removes in place and grows by copying cannot carry dead bytes forward
either. [[src/codegen/collection/list/list_mutate.rs:lower_list_insert_collection]]

A `Map`/`Set`'s dead key and value bytes are reclaimed the same way: every
in-place grow of a map or set — a `set`/`add` that needs a new entry or data
room, a `set` whose value outgrew its slot, and the one-time reserve of a batch
update (`union`, `symmetricDifference`, `merge`) — first calls
`_mfb_rt_map_reclaim`. That first drops the map's tombstones, if any, and
reports progress for that alone: a grow writes a fresh header whose
`deadEntries` is `0`, so it must never copy a tombstone, and the freed entries
may be all the room it needed. A `set` whose value outgrew its slot holds its
entry's index across the call, so it passes the index's address and gets it
rewritten. Then, when `dataLength > 2 * live`, it slides the
live keys and values, each aligned as the entry writer aligns it, down to the
front of the data region in place — eight bytes at a time, a payload already in
place not at all — rewrites their offsets, and the grow's room check is
retried. It allocates nothing and cannot fail. The block, its capacity
and its data base stay where they are. Moving in place needs the payloads
visited in increasing offset: keys are always in entry order, and a value is too
unless a `set` moved it to the data tail when it outgrew its slot. Such values
are heap-sorted by offset in the bucket region and merged in; the index is then
marked not-ready and rebuilt by the next probe. When nothing was out of order,
the bucket region is untouched and, since the buckets hold entry indices, the
index stays valid. The repack costs O(count + live), plus O(r log r) for `r`
moved values, and runs only once as many bytes have died as are live. The test
compares rather than subtracting `dataLength - live`: a packed map whose moved
values sit last needs fewer alignment pads than the entry-order `live` counts, so
after a repack `dataLength` can be below `live`, so subtracting would underflow. The packed length is at most `2 * live`, so a
repack is never followed by another. Unlike a list repack the map reclaim does
not shrink `dataCapacity`, so a map grow that follows it cannot cycle.
[[src/codegen/collection/map/map_bucket_remove.rs:lower_map_reclaim_helper]]
[[src/codegen/collection/map/map_mutate/mod.rs:emit_map_reclaim_before_grow]]

A plain value copy (`copy_collection_tight`) copies `dataLength` bytes verbatim,
so it carries whatever dead bytes its source holds at that moment; it does not
repack, and it does not grow.
[[src/codegen/collection/layout/builder_collection_layout.rs:copy_collection_tight]]

`dataCapacity`/`capacity` slack comes from growing working buffers and
builders that reserve an upper bound (see *Capacity Headroom and Growth*).
An ordinary collection value copy tightens those capacities. The spare-byte invariant — derive the data-region base
from `capacity`, never `count` — is what keeps that headroom unobservable.

## See Also

* ./mfb spec language collections — the source-level `List`/`Map` operations over this layout
* ./mfb spec memory heap-values — the uniform heap value representation
* ./mfb spec memory arenas — where collection storage is allocated and freed
* ./mfb man collections — collection built-in help
