# MFB Memory Layouts

This document specifies runtime memory layouts used by the compiler and native
runtime. These layouts are implementation contracts, not source-level syntax.

## Goals

- Owned values have a clear, copyable memory representation.
- Collections are represented as one contiguous allocation.
- Copying a collection snapshot can be implemented as one contiguous memory copy.
- Collection mutation can minimize payload copying by moving lookup metadata
  instead of moving packed item bytes.
- The collection layout favors one uniform representation over collection-kind
  specialization.

## Scalar Storage

Primitive scalar payloads are stored in their native runtime value size inside a
collection data region.

| Type | Payload size |
|------|--------------|
| `Boolean` | 1 byte payload, stored as canonical `0` or `1` |
| `Byte` | 1 byte |
| `Integer` | 8 bytes |
| `Float` | 8 bytes |
| `Fixed` | 8 bytes |

Payloads in the data region are aligned so every payload begins at an offset
valid for that payload's type. Padding bytes are not observable.

## Fallible-Call Result ABI

A native fallible call returns its outcome in four registers:

```text
x0  tag       0 = success, 1 = error, 2 = program exit
x1  value     success: the result value (0 for Nothing); error: the Error code
x2  message   error: pointer to the error message string
x3  source    error: pointer to the origin ErrorLoc
```

On success only `x0`/`x1` are meaningful. On error all of `x1` (code), `x2`
(message), and `x3` (source) are set. A fresh error stamps `x3` with an
`ErrorLoc` built from the originating expression's `(file, line, char)`; a runtime
helper error is stamped at its call site; a propagated error forwards `x3`
unchanged so the origin is preserved. Trapping materializes a 3-field `Error`
record from `x1`/`x2`/`x3`, and `FAIL <error>` loads `x1`/`x2`/`x3` back from the
`Error` value's `code`/`message`/`source` fields.

## Native Heap Value Layouts

Native heap values use layout-specific compact object bodies. The arena
allocator may keep block-level bookkeeping, but values allocated inside the
arena do not share a universal per-object header.

Each package instance owns a distinct arena. Worker package instances therefore
allocate strings, records, unions, collections, errors, and other heap-backed
values in the worker arena by default. When such a value crosses a thread
boundary as start input, a queued message, or a completed result, the runtime
must materialize the value in transfer storage independent of the producer arena
or in the receiver's arena before the receiver observes it. A thread control
block or queue entry must not retain a bare layout handle into a worker arena
after that arena is eligible for reclamation.

The arena allocator entry point `arena_alloc(size, align)` validates that
alignment is a non-zero power of two, treats zero-size allocations as one byte,
rounds addresses with checked arithmetic, grows chained blocks when needed, and
uses a separate large-allocation block path for oversized requests. It reports
an invalid alignment or request as `ErrInvalidArgument` and exhaustion as
`ErrOutOfMemory`; both surface to source code as ordinary language-level errors
(see the language spec §14.3.1).

### Standalone String

Standalone and static `String` objects store the byte length first, followed by
UTF-8 bytes:

```text
StringObject
  U64 byteLength
  Byte[byteLength] utf8Bytes
  U8 nulTerminator
```

The trailing NUL byte is a native helper convenience and is not part of the
logical string length.

### Record

User-defined records store one 8-byte field slot per declared field, in
declaration order:

```text
RecordObject
  Slot[fieldCount] fields
```

Field `0` starts at offset `0`; field `n` starts at offset `8 * n`. A slot
stores the native scalar value or native handle for the field's type.

### `Error` and `ErrorLoc`

`Error` and `ErrorLoc` are read-only built-in records laid out exactly like any
other record (one 8-byte slot per field, in declaration order):

```text
Error
  +0  Integer  code
  +8  String   message      (pointer to a string object)
  +16 ErrorLoc source       (pointer to an ErrorLoc object)

ErrorLoc
  +0  String   filename     (pointer to a string object)
  +8  Integer  line
  +16 Integer  char
```

Both are 24-byte objects. Construction, field access, copy, thread-transfer, and
arena cleanup all reuse the generic record machinery; copying an `Error` deep-
copies its `message` string and its `source` `ErrorLoc` (which in turn deep-copies
its `filename`).

### Union

User-defined unions store the active member tag at offset `0`, followed by slot
space for the largest member payload:

```text
UnionObject
  U64 activeMemberTag
  Slot[maxMemberFieldCount] payloadFields
```

Payload field `0` starts at offset `8`; payload field `n` starts at offset
`8 * (n + 1)`. The active member tag is the compiler-assigned member index for
the expanded concrete union. Unused payload slots are not observable.

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
- `capacity` is the number of lookup entries allocated.
- `dataLength` is the number of used bytes in `Data`.
- `dataCapacity` is the number of bytes allocated for `Data`.

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

Version 1 packs primitive payloads, `String` bytes, user-defined record slots,
and user-defined union slots directly into the data region. Nested `List` and
`Map` values store one pointer-sized native collection handle in the data
region; the nested collection's own allocation stores its full header, lookup
table, and data region.

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

For a `List`, `get(value, index)` reads `LookupEntry[index]`, then reads the
payload at `Data + valueOffset` with `valueLength`.

For a `Map`, `get(value, key)` scans live lookup entries until the key payload
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
