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

A collection copy copies the full contiguous allocation:

```text
CollectionHeader + LookupEntry[capacity] + Data[dataCapacity]
```

This preserves snapshot ownership semantics with one memory copy.

### `get`

For a `List`, `collections::get(value, index)` reads `LookupEntry[index]`, then reads the
payload at `Data + valueOffset` with `valueLength`.

For a `Map`, `collections::get(value, key)` scans live lookup entries until the key payload
matches. Missing keys fail with `ErrNotFound`.

### `append`

For `List`, `append` writes the new item payload at `Data + dataLength`, appends
one lookup entry, and increments `count`. If capacity is insufficient, a new
larger contiguous allocation is created and the existing allocation is copied
once before writing the new item.

### `insert`

For `List`, `insert` appends the new item payload to `Data`, shifts lookup
entries right from the insertion point, writes the inserted lookup entry, and
increments `count`. Existing payload bytes are not moved unless compaction is
performed.

### `removeAt`

For `List`, `removeAt` shifts lookup entries left over the removed entry and
decrements `count`. Payload bytes may remain in `Data` as unreachable dead
space until compaction.

### Map Updates

For `Map`, setting or removing keys updates lookup entries and packed key/value
payloads. The first implementation may use linear lookup. Missing removed keys
are ignored.

## Compaction

Removal and replacement may leave unreachable bytes in `Data`. Implementations
may compact during an update when dead space crosses an implementation-defined
threshold or when growing into a new allocation.

Compaction rewrites live payload bytes into a new packed `Data` region and
updates lookup offsets. The observable collection value is unchanged.

## Open Questions

- Whether map hashing should be added in layout version 1 or deferred.
- Whether future layout versions should replace pointer-sized nested collection
  handles with fully inline nested collection payloads.
