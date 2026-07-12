# Collections

One uniform contiguous layout represents both `List` and `Map`: a header, a
lookup table, and a packed data region.

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

Collections use one uniform layout for `List` and `Map`.

```text
CollectionHeader
  U8 kind            ; 0 = List, 1 = Map
  U8 keyType         ; 0 for List
  U8 valueType
  U8 flagsVersion
  U8 bucketsReady    ; Map hash index built? 0 = no (rebuild lazily), 1 = yes
  U8[3] reserved
  U64 count          ; logical live item count
  U64 capacity       ; lookup entry capacity
  U64 dataLength     ; used bytes in data region
  U64 dataCapacity   ; allocated bytes in data region

LookupEntry[capacity]
  U8 flags           ; used/deleted/etc.
  U8[7] reserved
  U64 keyOffset      ; Map only, 0 for List
  U64 keyLength      ; Map only, 0 for List
  U64 valueOffset
  U64 valueLength

Data[dataCapacity]
  packed key and value payload bytes

Buckets[2 * capacity]   ; Map only (List reserves none); see Map Hash Index
  U64                   ; entryIndex + 1, or 0 = empty
```

Header and lookup entries have fixed aligned sizes. Version 1 uses a 40-byte
`CollectionHeader` and 40-byte `LookupEntry`. Implementations must not derive
the runtime entry stride from the sum of field sizes without accounting for
padding and alignment.

### Header Fields

- `kind` identifies whether the allocation is a `List` or `Map`.
- `keyType` identifies the map key payload type. It is `0` for `List`.
- `valueType` identifies the list item type or map value type.
- `flagsVersion` identifies the layout version and collection-level flags.
- `bucketsReady` is `1` when a `Map`'s hash index (the `Buckets` array) is built
  and `0` otherwise; it is `0` on every fresh, copied, or grown map and set to `1`
  the first time the index is probed (see *Map Hash Index*). It is unused (`0`)
  for a `List`.
- `count` is the number of live logical entries.
- `capacity` is the number of lookup entries allocated. It may exceed `count`:
  the spare slots are working-buffer headroom (see *Capacity Headroom*).
- `dataLength` is the number of used bytes in `Data`.
- `dataCapacity` is the number of bytes allocated for `Data`. It may exceed
  `dataLength` for the same reason.

`keyType` and `valueType` are compact runtime type identifiers.

| Type | Identifier |
|------|------------|
| none/list key | 0 |
| `Boolean` | 2 |
| `Integer` | 3 |
| `Float` | 4 |
| `Fixed` | 5 |
| `String` | 6 |
| `Byte` | 7 |
| `List` | 20 |
| `Map` | 21 |
| user-defined object | 22 |

User-defined, generic, nested, or future extended types may require an extended
type table; that extension must preserve the fixed header size through
`flagsVersion` or an explicitly versioned layout.

### Lookup Entry Fields

- `flags` includes at least a used/deleted state.
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
insertion-ordered entries. [[src/target/shared/code/mod.rs:lower_map_probe_helper]]

- **Size and addressing.** `Buckets` has `2 * capacity` `U64` slots (load factor
  ≤ 0.5), based at `header + capacity*entryStride + dataCapacity`. Each slot holds
  `entryIndex + 1`, or `0` for empty. A key maps to a bucket by
  `FNV-1a(keyBytes) mod (2*capacity)` with linear probing; equality is the
  byte-wise comparison over `keyLength` bytes — identical to the linear scan, so
  `Float` keys still compare bitwise (`+0.0`≠`-0.0`, `NaN`=`NaN`).
- **Derived, lazy, and recomputed — never authoritative.** The entries + data are
  the source of truth; the buckets are rebuildable from them. `bucketsReady` is
  `0` on every fresh allocation (and a fresh allocation's bucket bytes are
  uninitialized), so the index is built lazily on the first probe and `bucketsReady`
  set to `1`. **Copy and thread transfer never copy the buckets verbatim**: a
  shrink-to-fit copy/transfer reserves the (count-sized) bucket region and leaves
  `bucketsReady = 0`, so the receiver recomputes against its own capacity — no
  stale offsets, deterministic across the boundary.
- **Incremental maintenance.** In-place `set` inserts a new key into the buckets
  in O(1) when they are already built (`_mfb_rt_map_bucket_put`) and invalidates
  (`bucketsReady = 0`) when a capacity grow moves/resizes the region, so building a
  map with repeated `set` stays O(n). An in-place value overwrite or value-grow
  leaves keys/indices unchanged and so leaves the index valid.

The data region packs every **flat** payload directly, addressed by the lookup
entry's `valueOffset`/`valueLength`: primitive
payloads, `String` bytes, inlined record blocks, inlined data-union blocks, and —
for **nested flat collections** (a `List`/`Map` whose own payloads
are flat) as their full block (header + lookup table + data region) inlined by
offset, `valueLength` = the block byte size. Because a collection's internal
entry offsets are relative to its own base, an inlined nested collection
relocates correctly under the enclosing block's `memcpy`. The **only** payloads
that remain an 8-byte pointer handle are a **resource** and a **non-flat** nested
collection (one whose own payloads include a resource or a recursive type) — see
`is_pointer_collection_payload_type`. [[src/target/shared/code/builder_collection_layout.rs:is_pointer_collection_payload_type]]

A **function value** (`FUNC(...) AS T`, list element or map value) is packed as a
single **8-byte pointer** to its arena-lifetime closure object (`./mfb spec memory
closures`), 8-aligned, `valueLength` = 8. It is a **reference** payload with the
same discipline as a scalar `Integer` word: the pointer is written verbatim on
insert, read back verbatim on `get`, and `memcpy`-copied when the collection is
copied — the closure object it points at is **never deep-copied on insert and
never freed when the collection is dropped**. A function value therefore
matches the `List OF Integer` flatness class (`type_is_flat` is true for a function
type), so a `List`/`Map` of function values is itself a flat block whose scope-drop
`arena_free` reclaims only the packed pointer array, leaving every referenced
closure object owned by the arena. A record **field** of function type is likewise
a bare 8-byte slot and is unaffected. [[src/target/shared/code/type_utils.rs:is_function_type]] [[src/target/shared/code/builder_collection_layout.rs:emit_payload_length_to_stack]]

### Capacity Headroom and Growth

`capacity` and `dataCapacity` carry **real headroom**: the codegen append grow
path over-allocates so `capacity > count` and `dataCapacity > dataLength`, and a
later append into the same uniquely-owned `MUT` buffer writes into the spare slot
and bumps `count`/`dataLength` in place — amortized **O(1)** append instead of a
realloc-and-copy per item. The growth shape (an implementation tuning detail, not
an observable contract): lookup slots start at 4, double until 1024, then ×1.5;
data bytes start at 32, double until 64 KiB, then ×1.5; each grows to at least
what the appended element needs. Fixed-width element lists grow lookup and data
in lockstep; variable-width lists grow them independently.

Headroom is a property of a **mutable working buffer, never of a value**:

- Because `LookupEntry[capacity]` precedes `Data`, the data region base is
  `header + capacity * entryStride` — **always derive it from `capacity`, never
  from `count`**. With headroom present a `count`-based base reads the spare
  slots as payload bytes.
- Literals and known-size builders (`transform`/`filter` outputs aside) allocate
  exactly (`capacity == count`, `dataCapacity == dataLength`) — no headroom where
  the final size is known up front.
- Copying a collection value (pass-by-value, binding, embedding, thread
  transfer) is **shrink-to-fit**: the copy is re-tightened to `capacity == count`
  and `dataCapacity == dataLength`, so headroom never leaks into a snapshot or
  across a thread boundary, and the "one contiguous `memcpy`" property holds over
  the used prefix.

## List Examples

`List OF Integer = [10, 20]`:

```text
Header:
  kind = 0
  keyType = 0
  valueType = Integer
  count = 2
  capacity = 2
  dataLength = 16

Lookup[0]:
  flags = used
  keyOffset = 0
  keyLength = 0
  valueOffset = 0
  valueLength = 8

Lookup[1]:
  flags = used
  keyOffset = 0
  keyLength = 0
  valueOffset = 8
  valueLength = 8

Data:
  Integer(10)
  Integer(20)
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
`copy_collection_tight`, which allocates exactly [[src/target/shared/code/builder_collection_layout.rs:copy_collection_tight]]

```text
CollectionHeader + LookupEntry[count] + Data[dataLength]
```

(`capacity == count`, `dataCapacity == dataLength`) and copies the used prefix.
Working-buffer headroom never leaks into a snapshot. The copy preserves snapshot
ownership semantics, and over the tight used prefix it is a single contiguous
memory copy.

### `get`

For a `List`, `collections::get(value, index)` validates `0 <= index < count`,
reads `LookupEntry[index]`, then reads the payload at `Data + valueOffset` with
`valueLength`. An out-of-range or negative index fails with `ErrIndexOutOfRange`.

For a `Map`, `collections::get(value, key)` looks the key up through the *Map
Hash Index* — an O(1)-average FNV-1a bucket probe that lazily builds the index on
first use — then reads the matching entry's payload at `Data + valueOffset`;
missing keys fail with `ErrNotFound`. The probe covers every scalar key type
(`String`, `Integer`, `Float`, `Fixed`, `Byte`, `Boolean`, i.e. all valid map key
types); any other key type falls back to a generic linear scan over the live
lookup entries. [[src/target/shared/code/builder_collection_query.rs:lower_map_get]]

### `append`

`List` `append` has two paths:

- **In-place (`MUT`, amortized O(1)).** When the buffer is a uniquely-owned `MUT`
  working buffer with headroom (`capacity > count` and enough `dataCapacity`),
  `lower_list_append_in_place` writes the new item payload at `Data + dataLength`, [[src/target/shared/code/builder_collection_mutate.rs:lower_list_append_in_place]]
  fills the next spare lookup entry, and bumps `count`/`dataLength` in place — no
  reallocation. If headroom is insufficient it first grows the buffer using the
  geometric shape in *Capacity Headroom and Growth* (reallocate-and-copy once,
  with fresh headroom).
- **Snapshot/value path.** A value-semantic append produces a fresh allocation
  holding the existing items plus the new one.

### `set`

`collections::set(name, index/key, item)` on a uniquely-owned `MUT` local (the
`name = collections::set(name, …)` self-assignment idiom) mutates the live buffer
in place, like `append`. It is excluded while the binding is an active `FOR EACH`
iterable — an overwrite of an existing entry is observable to the snapshotting
iterator, unlike a beyond-`count` append, so that case takes the value path.
[[src/target/shared/code/builder_collection_mutate.rs:lower_list_set_in_place]]

- **`List`.** When the replacement payload is the **same size**
  (`newValueLength == oldValueLength` — always true for fixed-width elements and
  same-size records/strings) the value bytes are overwritten at the entry's
  `valueOffset` in place: no allocation, no copy, offsets unchanged. **Any** size
  change — grow *or* shrink — falls back to the value-semantic rebuild
  (`removeAt` + `insert`), which produces a tight buffer; a shrink that overwrote
  in place would leave dead space between payloads. An out-of-range index fails
  with `ErrIndexOutOfRange`, like the value path.
- **`Map`.** `lower_map_set_in_place` locates the key with the same hash probe as
  `get` (linear-scan fallback for non-probe key types), which also lazily builds
  the bucket index so a build-via-`set` loop stays O(n). A hit whose new
  value fits overwrites in place; a hit whose value grew rebuilds
  (`removeKey` + concat). A miss writes the key+value into a spare lookup slot and
  the spare data tail — the entry packed exactly like a literal entry (key then
  value, each aligned to its payload alignment) — and bumps `count`/`dataLength`,
  growing the buffer geometrically (capacity and `dataCapacity` stepped
  independently, entries and data copied verbatim against the capacity-based base)
  when full. Insertion order is preserved, and the new key is folded into the hash
  index per *Map Hash Index* (incremental `_mfb_rt_map_bucket_put` when built, or
  `bucketsReady = 0` when a grow moved the bucket region).
  [[src/target/shared/code/builder_collection_mutate.rs:lower_map_set_in_place]]

The source `collections::sort` is an insertion sort built on `set`, so its
per-swap `items = collections::set(items, j, …)` overwrites run in place:
the sort is a stable in-place O(n²)-comparison / O(1)-swap pass over a copy of the
argument (the argument itself is never modified).

### `insert`

`List` `insert` is **not** an in-place shift. `lower_list_insert_collection`
allocates a fresh **tight** buffer sized for `count + insertedCount` entries and [[src/target/shared/code/builder_collection_mutate.rs:lower_list_insert_collection]]
`dataLength + insertedDataLength` bytes, copies the pre-insertion data region then
the inserted data region verbatim, splices the lookup table (head, inserted,
tail), and writes a tight header (`capacity == count`, `dataCapacity ==
dataLength`). No existing entry is shifted in place and no dead space is left. The
inserted argument is first normalized to a singleton list.

### `removeAt`

`List` `removeAt` is **not** an in-place shift either, and leaves **no** dead
space. `lower_list_remove_at` validates the index (`ErrIndexOutOfRange` on a bad
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
[[src/target/shared/code/builder_collection_mutate.rs:lower_list_remove_at]]

### Map Updates

For `Map`, setting or removing keys updates lookup entries and packed key/value
payloads. Key lookup goes through the *Map Hash Index* (O(1)-average FNV-1a probe,
with a linear-scan fallback for any non-probe key type). Missing removed keys are
ignored. In-place `set` (insert into spare headroom, or value overwrite) is
described under [`set`](#set) above; `removeKey` takes the value path, which
re-tightens the buffer and leaves `bucketsReady = 0` for a lazy rebuild.

## Compaction

The value-semantic update operations (`insert`, `removeAt`, and the value-path
`append`) **always** produce a fresh, fully-packed, tight buffer with no dead
space, so there is never accumulated garbage to reclaim. There is no deferred,
threshold-triggered dead-space compactor in the codegen.

The only `dataCapacity`/`capacity` slack the layout ever carries is the
**intentional** headroom of an in-place `MUT` append working buffer (see *Capacity
Headroom and Growth*) — not garbage, and tightened away the moment the value is
copied (shrink-to-fit). The spare-byte invariant — derive the data-region base
from `capacity`, never `count` — is what keeps that headroom unobservable.

## Open Questions

- Whether future layout versions should also inline the **non-flat** nested
  collection payloads that still remain 8-byte pointer handles (flat nested
  collections are already inlined).
