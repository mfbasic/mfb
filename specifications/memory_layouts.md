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
uses a separate large-allocation block path for oversized requests. The "Arenas"
section below specifies the arena-state and block layouts, the full allocation
algorithm, and how arenas are reclaimed.

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

## Arenas

Every heap-backed value — strings, records, unions, errors, and collections —
is allocated from an *arena*. An arena is a bump allocator over a chain of
OS-mapped blocks plus a small fixed *arena-state* control structure. Arenas are
not general-purpose heaps: individual values are never freed, and the entire
arena is reclaimed in one operation when its owning package instance shuts down.

Each package instance owns a distinct arena. The main package's arena-state lives
on the entry stack and is pinned in `x19` (`ARENA_STATE_REGISTER`) for the life of
the program; its address is also published to the writable global
`_mfb_rt_main_arena` so signal handlers and shutdown code can reach it without
relying on the pinned register. Each worker package instance owns a separate
arena, referenced from its thread control block, so worker threads allocate and
reclaim independently of the main thread (see `threading.md`).

### Arena-State Layout

The arena-state structure is `ARENA_STATE_SIZE` = **104 bytes**:

```text
ArenaState (at x19)
  +0   U64  blockHead        ; pointer to the current (most-recent) block, 0 if none
  +8   U64  reserved         ; zero-initialized
  +16  U64  reserved         ; zero-initialized
  +24  U64  reserved         ; zero-initialized
  +32  U64  exitStatus       ; pending exit/result code used during teardown
  +40  U64  reserved
  +48  U64  reserved
  +56  U64  reserved
  +64  U64  cleanupFailCount ; count of cleanup errors (audit)
  +72  U64  cleanupFailCode  ; last cleanup failure error code
  +80  U64  cleanupFailMsg   ; pointer to last cleanup failure message
  +88  U64  rngStateLo       ; PCG64 RNG state, low 64 bits
  +96  U64  rngStateHi       ; PCG64 RNG state, high 64 bits
```

Only `blockHead` participates in allocation. The cleanup-failure triple records
diagnostics if reclamation of a value fails during teardown, and the two RNG
words give each arena (hence each thread) an independent `math::rand` stream
seeded at startup. The `ENTRY_*` argv/argc fields the entry shim stores begin at
offset `ARENA_STATE_SIZE`, immediately after this structure on the entry stack.

### Block Layout

Blocks are mapped on demand and chained head-first into a singly-linked list via
a 32-byte (`ARENA_BLOCK_HEADER_SIZE`) header:

```text
ArenaBlock
  +0   U64  prevBlock        ; previous block in the chain, 0 for the first
  +8   U64  blockSize        ; total mapped size of this block, in bytes
  +16  U64  usableCapacity   ; blockSize - 32 (bytes available after the header)
  +24  U64  bumpOffset       ; bytes consumed in the usable region so far
  +32  ...  payload          ; usableCapacity bytes of allocations
```

`ArenaState.blockHead` always points at the newest block; older blocks are
reachable only through each block's `prevBlock` link. The default block size is
`ARENA_DEFAULT_BLOCK_SIZE` = **4096 bytes**.

### `arena_alloc(size, align)`

`arena_alloc` (symbol `_mfb_arena_alloc`) takes a byte `size` in `x0` and a power-
of-two `align` in `x1`, and returns a fallible result: `x0` is `0` on success
with the aligned pointer in `x1`, or an error code in `x0` with `x1 = 0` on
failure. It clobbers **x9, x10, x14, x15, x20–x28**; callers must spill any live
values held in those registers across the call.

The algorithm:

1. **Validate alignment.** A zero `align`, or one that is not a power of two
   (`(align - 1) & align != 0`), returns `ErrInvalidArgument`.
2. **Normalize size.** A zero `size` is treated as `1` byte so every allocation
   yields a distinct address.
3. **Try the current block.** Compute the aligned cursor
   `align_up(blockBase + bumpOffset, align)` and the end `cursor + size`. If the
   end fits within `usableCapacity`, store the new `bumpOffset` and return the
   cursor. Every intermediate addition is overflow-checked; an overflow reports
   `ErrOutOfMemory`.
4. **Grow.** If there is no current block, or the request does not fit, map a new
   block. Its size is `max(4096, round_up(size + align + 32, 4096))` — large
   requests get a dedicated, page-rounded block. The new block is linked at the
   head (its `prevBlock` = old `blockHead`, `bumpOffset` = 0) and becomes the
   current block; allocation then retries against it.

A failed mapping (the platform `mmap`/`VirtualAlloc` hook) reports
`ErrOutOfMemory`. Both `ErrInvalidArgument` and `ErrOutOfMemory` surface to
source as ordinary language-level errors (see the language spec §14.3.1).

Because allocation only ever advances `bumpOffset` or links a fresh block, there
is no per-allocation free path and no per-object header — allocated bytes carry
only the value's own layout.

### Cleanup and Reclamation

An arena is reclaimed whole. `arena_destroy` (symbol `_mfb_arena_destroy`) walks
the block chain from `blockHead` through each `prevBlock`, unmapping every block
with the platform `munmap`/`VirtualFree` hook, then clears `blockHead` to `0`. It
frees no individual values; all memory returns to the OS at once. The helper is
idempotent — a second call sees `blockHead == 0` and does nothing.

At process teardown, `_mfb_shutdown` reads the arena-state address from
`_mfb_rt_main_arena`, clears that global first (so a signal arriving mid-teardown
re-enters as a no-op), restores the terminal if TUI mode was active, and then
calls `arena_destroy` on the main arena. A worker arena is reclaimed the same way
when its package instance ends; the thread control block must not retain any bare
handle into a worker arena past the point that arena becomes eligible for
reclamation, so cross-thread values (start inputs, queued messages, results) are
first materialized in transfer storage or in the receiver's arena.

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
