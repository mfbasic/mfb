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
  U8 kind            ; 0 = List, 1 = Map, 2 = fixed-width List (no LookupEntry)
  U8 keyType         ; 0 for List
  U8 valueType
  U8 flagsVersion
  U8 bucketsReady    ; Map hash index built? 0 = no (rebuild lazily), 1 = yes
  U8[3] reserved
  U64 count          ; logical live item count
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

Buckets[2 * capacity]   ; Map only (List reserves none); see Map Hash Index
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

[[src/target/shared/code/error_constants.rs:COLLECTION_KIND_LIST_FIXED]]

| Element type | Payload width |
|---|---|
| `Boolean`, `Byte` | 1 |
| `Scalar` | 4 |
| `Integer`, `Float`, `Fixed`, `Money` | 8 |

Every other element type — `String`, a record, a union, a nested collection, a
resource pointer — is variable-width and keeps the kind-0 layout. A `Map` always
keeps its entries regardless of key or value width, because a map's entry
carries a key offset that is not derivable from the index.

The layouts differ only in the entry array:

| | kind 0 | kind 2 |
|---|---|---|
| element address | `Data + entry[i].valueOffset` | `Data + i * payloadSize` |
| data region base | `block + 40 + capacity*40` | `block + 40` |
| block size | `40 + capacity*40 + dataCapacity` | `40 + dataCapacity` |

The saving is the whole entry array: a `List OF Byte` costs `40 + N` bytes
rather than `40 + 41N`. Measured on a 16 MiB byte list, peak RSS falls from
674 MB to 33 MB.

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

### Header Fields

- `kind` identifies whether the allocation is a `List` (`0`), a `Map` (`1`), or a
  fixed-width `List` carrying no lookup entries (`2`; see *Fixed-Width Lists*).
- `keyType` identifies the map key payload type. It is `0` for `List`.
- `valueType` identifies the list item type or map value type.
- `flagsVersion` identifies the layout version and collection-level flags.
- `bucketsReady` is `1` when a `Map`'s hash index (the `Buckets` array) is built
  and `0` otherwise; it is `0` on every fresh, copied, or grown map and set to `1`
  the first time the index is probed (see *Map Hash Index*). It is unused (`0`)
  for a `List`.
- `count` is the number of live logical entries.
- `capacity` is the number of lookup entries allocated. It may exceed `count`:
  the spare slots are working-buffer headroom (see *Capacity Headroom*). For a
  kind-2 list no entries are allocated, and `capacity` is the number of element
  slots the `Data` region has room for — it still governs the data base and the
  block size, so it remains meaningful with an entry stride of zero.
- `dataLength` is the number of used bytes in `Data`.
- `dataCapacity` is the number of bytes allocated for `Data`. It may exceed
  `dataLength` for the same reason.

`keyType` and `valueType` are compact runtime type identifiers. The **payload
size and alignment** columns are what a decoder strides by; they are not implied
by the identifier's value. [[src/target/shared/code/error_constants.rs:COLLECTION_TYPE_SCALAR]] [[src/target/shared/code/builder_collection_layout.rs:collection_payload_alignment]]

| Type | Identifier | Payload | Align |
|------|------------|---------|-------|
| none/list key | 0 | — | — |
| `Boolean` | 2 | 1 byte | 1 |
| `Integer` | 3 | 8 bytes | 8 |
| `Float` | 4 | 8 bytes | 8 |
| `Fixed` | 5 | 8 bytes | 8 |
| `String` | 6 | 8-byte offset/pointer | 1 |
| `Byte` | 7 | 1 byte | 1 |
| `Money` | 8 | 8 bytes | 8 |
| `Scalar` | 9 | **4 bytes** | **4** |
| `List` | 20 | inlined block | 8 |
| `Map` | 21 | inlined block | 8 |
| user-defined object | 22 | inlined block | 8 |

> **`Scalar` is the only 4-byte element type.** Every other payload is either
> 1-byte or 8-byte aligned, so a decoder that infers the stride from the shape of
> this table rather than reading the alignment column mis-strides every element of
> a `List OF Scalar`.

> These identifiers are the **runtime** type space. The package wire format uses a
> *different*, independently numbered id space for the same types — see
> `./mfb spec package type-table`. The two must not be read into one another.

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

### Payload Order

Where there is a lookup table, it — not the data region — defines the sequence.
Element `i`'s payload is located by `entry[i].valueOffset` and
`entry[i].valueLength`, relative to the capacity-derived data base.

For a **fixed-width element type** — `Boolean`, `Byte`, `Scalar`, `Integer`,
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

The distinction is now structural, not merely a property to be maintained. A
fixed-width list is `kind = 2` and has no entry table to disagree with: index
order is guaranteed **by construction**, and a linear reader cannot be wrong. A
variable-width list is `kind = 0` and the hazard below applies in full. Half the
element types are safe; the other half are exactly as dangerous as before, and a
reader that handles "lists" uniformly is still wrong.

For a **variable-width element type** — `String`, records, unions, nested
collections — payloads are densely packed but **not necessarily in index order**.
A reader **must not** assume element `i` begins at `dataBase + i * payloadSize`,
and must not assume a linear walk visits elements in index order. The permutation
is a deliberate consequence of the offset-stable scheme (plan-01 §4.1): splicing
the lookup table and appending the new payload to the data tail avoids
recomputing every offset, which for a variable-width payload is the expensive
part. `collections::sort` on a `List OF String` relies on this directly — it
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

Note that `list_element_padding_alignment` returning 1 guarantees there are no
*gaps* between payloads, not that they are in *order*. The two are independent,
and conflating them is what made every linear reader wrong for permuted lists
before this rule was written down (bug-365).
[[src/target/shared/code/builder_collection_layout.rs:list_element_is_fixed_width]]

## List Examples

`List OF Integer = [10, 20]` — `Integer` is fixed-width, so this is a kind-2
list with no lookup array. The whole block is 56 bytes; the kind-0 form below it
would need 136.

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

The same list under the kind-0 layout, which is what a *variable*-width element
type gets and what every list looked like before kind 2 existed:

```text
Header:
  kind = 0
  ...
  capacity = 2

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

Data:                     ; based at block + 40 + 2*40
  Integer(10)
  Integer(20)
```

Both entries say only what the index already said — that is the redundancy
kind 2 removes.

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

or, for a kind-2 list, `CollectionHeader + Data[dataLength]` — the same formula
with an entry stride of zero, which is why the copy path needed no kind-specific
case.

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

## See Also

* ./mfb spec language collections — the source-level `List`/`Map` operations over this layout
* ./mfb spec memory heap-values — the uniform heap value representation
* ./mfb spec memory arenas — where collection storage is allocated and freed
* ./mfb man collections — collection built-in help
