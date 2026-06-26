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
  U8[4] reserved
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
order. The initial implementation may scan entries linearly. Future hash/probe
metadata may be added through a new layout version.

The data region packs every **flat** payload directly, addressed by the lookup
entry's `valueOffset`/`valueLength`: primitive
payloads, `String` bytes, inlined record blocks, inlined data-union blocks, and —
since Phase 5a — **nested flat collections** (a `List`/`Map` whose own payloads
are flat) as their full block (header + lookup table + data region) inlined by
offset, `valueLength` = the block byte size. Because a collection's internal
entry offsets are relative to its own base, an inlined nested collection
relocates correctly under the enclosing block's `memcpy`. The **only** payloads
that remain an 8-byte pointer handle are a **resource** and a **non-flat** nested
collection (one whose own payloads include a resource or a recursive type) — see
`is_pointer_collection_payload_type`.

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
`copy_collection_tight`, which allocates exactly

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

For a `Map`, `collections::get(value, key)` scans live lookup entries until the
key payload matches; missing keys fail with `ErrNotFound`. A `String`-keyed map
takes a dedicated comparison fast path (`lower_string_key_map_get`); other key
types use the generic linear scan.

### `append`

`List` `append` has two paths:

- **In-place (`MUT`, amortized O(1)).** When the buffer is a uniquely-owned `MUT`
  working buffer with headroom (`capacity > count` and enough `dataCapacity`),
  `lower_list_append_in_place` writes the new item payload at `Data + dataLength`,
  fills the next spare lookup entry, and bumps `count`/`dataLength` in place — no
  reallocation. If headroom is insufficient it first grows the buffer using the
  geometric shape in *Capacity Headroom and Growth* (reallocate-and-copy once,
  with fresh headroom).
- **Snapshot/value path.** A value-semantic append produces a fresh allocation
  holding the existing items plus the new one.

### `insert`

`List` `insert` is **not** an in-place shift. `lower_list_insert_collection`
allocates a fresh **tight** buffer sized for `count + insertedCount` entries and
`dataLength + insertedDataLength` bytes, copies the pre-insertion data region then
the inserted data region verbatim, splices the lookup table (head, inserted,
tail), and writes a tight header (`capacity == count`, `dataCapacity ==
dataLength`). No existing entry is shifted in place and no dead space is left. The
inserted argument is first normalized to a singleton list.

### `removeAt`

`List` `removeAt` is **not** an in-place shift either, and leaves **no** dead
space. `lower_list_remove_at` validates the index (`ErrIndexOutOfRange` on a bad
one), allocates a fresh tight buffer sized for `count - 1` entries and
`dataLength - removedValueLength` bytes, and copies the surviving entries through
`emit_copy_collection_entries`, which **re-packs each live payload at a running
destination offset and rewrites each entry's `valueOffset`** to its compacted
position.

### Map Updates

For `Map`, setting or removing keys updates lookup entries and packed key/value
payloads. The current implementation uses a linear scan. Missing removed keys are
ignored.

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

- Whether map lookup should gain hash/probe metadata (currently a linear scan)
  in layout version 1 or a future version.
- Whether future layout versions should also inline the **non-flat** nested
  collection payloads that still remain 8-byte pointer handles (flat nested
  collections are already inlined, since Phase 5a).
