# Native Heap Value Layouts

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
topic specifies the arena-state and block layouts, the full allocation
algorithm, and how arenas are reclaimed.

## Standalone String

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

## Record

User-defined records store one 8-byte field slot per declared field, in
declaration order, followed by a trailing data region that inlines variable-
length sub-values:

```text
RecordObject (flat)
  Slot[fieldCount] fields        ; field n at offset 8 * n
  Byte[...] dataRegion           ; inlined String blocks, 8-aligned, in field order
```

Field `0` starts at offset `0`; field `n` starts at offset `8 * n`. A slot
stores, by field type:

- **scalar** (`Boolean`/`Byte`/`Integer`/`Float`/`Fixed`/enum): the value inline.
- **`String`**: a `U64` **block-relative offset** into the record's own data
  region, where the `String`'s flat block (`{U64 len, bytes, U8 nul}`, 8-aligned)
  is embedded inline. The field read recovers the borrow pointer as
  `recordBase + offset`; the offset is relative to the record base, so a whole-
  block `memcpy` is a correct deep copy and the inlined `String` comes along.
- **fully-flat nested record** (a record whose every field is scalar, inlined
  `String`, or another fully-flat record): inlined recursively as a `U64`
  block-relative offset into the data region, exactly like a `String` — the field
  read recovers `recordBase + offset` and the inlined record's own offsets are
  relative to that same base, so a whole-block `memcpy` deep-copies the tree.
- **other composite** (`Union`/`List`/`Map`/`Result`/`Error`, or a not-yet-flat
  nested record): a pointer to a separate allocation (inlined by later phases).

Because inlined fields are variable-length, a record's total byte size is
computed by walking the fixed slot region plus each inlined sub-block (a `String`
block, or recursively a nested-record block).
Construction, `WITH`-update, copy/transfer, equality, and collection embedding
all use that runtime size; copying a record with only scalar and `String` fields
is a single block `memcpy` (no per-field deep copy). The built-in helper-
constructed `net::` records `Address`, `Datagram`, and `DatagramText` are
**excluded**: their `String`/sub-record fields remain pointers to separate
allocations (the socket helpers build them that way), so reads of those records
do not rebase.

## `Error` and `ErrorLoc`

`Error` and `ErrorLoc` are flat built-in records: their
`String`/sub-record fields are inlined into the trailing data region by
block-relative offset, exactly like any other flat record, so the whole value is
a single pointer-free block.

```text
Error                              ErrorLoc
  +0  Integer  code                  +0  String    filename  (block-relative offset)
  +8  String   message  (offset)     +8  Integer   line
  +16 ErrorLoc source   (offset)     +16 Integer   char
  ...  inlined message + source        ...  inlined filename
```

`message` is always a valid (possibly empty) `String`. A null `source` (an
OOM-degraded error with no origin) is represented by an **offset-0 sentinel**
(offset 0 can never address a real inlined block, since the data region starts at
24); `emit_load_error_fields` maps it back to a null pointer when loading the
fallible-call ABI registers. Construction, field access, copy, and
thread-transfer reuse the generic flat-record machinery — copying an `Error` is
one `memcpy`.

## `Result`

`Result OF T` is a flat `{tag, size, payload}` value — a two-variant data union
`Ok(T)` / `Err(Error)`:

```text
Result
  +0   U64   tag         ; 0 = Ok, otherwise Err
  +8   U64   size        ; total byte size of THIS object
  +16  payload           ; a scalar value inline, or a flat block inlined whole
```

A scalar success payload occupies the 8-byte word at `+16`; a block payload
(`String`, record, union, collection, the `Err` `Error`, or a nested `Result`) is
inlined whole at `+16` and `size` covers it. Reading the value yields a borrow
pointer into the block (`base + 16`) for a block payload, or the 8-byte value for
a scalar. Copy/transfer is one generic `memcpy`.

## Union

A **data** union (all variants are data records) is a flat, self-describing
`{tag, size, data}` block sized to the **active** variant:

```text
DataUnionObject (flat)
  +0   U64 tag       ; active variant index
  +8   U64 size      ; total byte size of THIS object (16 + variant block)
  +16  data          ; the active variant's flat record block, inlined
```

`data` is the active variant's record laid out exactly as a standalone record
(scalar slots inline; `String`/flat-record fields inlined by block-relative
offset — relative to the union base at `+16`). Constructing a variant wraps its
built record block at `+16`; `MATCH` dispatches on `tag@0`; extracting a variant
yields a borrow pointer to the record at `+16`. The `size` word makes copy/free
generic (read the size, `memcpy`, then deep-copy only the active variant's
pointer fields). The union is variable-length, so a `List`/`Map` of a data union
stores each union block inline by its runtime `size`.

A **resource** union (all variants are resource handles; a union is all-data or
all-resource, `rules.rs:790`) is **not** reshaped — it keeps the fixed
`{U64 activeMemberTag@0, resource-handle-ptr@8}` layout, and the handle is moved
(never deep-copied) so the resource is closed exactly once.
