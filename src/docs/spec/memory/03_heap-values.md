# Native Heap Value Layouts

Native heap values use layout-specific compact object bodies. The arena
allocator may keep block-level bookkeeping, but values allocated inside the
arena do not share a universal per-object header.

Each package instance owns a distinct arena, so worker package instances
allocate strings, records, unions, collections, errors, and other heap-backed
values in the worker arena by default. When such a value crosses a thread
boundary it is re-materialized in the receiver's arena (not retained as a bare
handle into a soon-reclaimed worker arena) — see
`./mfb spec threading isolation`.

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
logical string length. A `String` object's total allocation size is therefore
`byteLength + 9` (the 8-byte length word plus the bytes plus the NUL); this same
`+9` formula sizes a `String` block inlined into a record or collection
(`emit_inlined_block_size_from_ptr_slot`). [[src/target/shared/code/builder_collection_layout.rs:emit_inlined_block_size_from_ptr_slot]]

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
- **flat composite** — a nested record, a data `Union`, a `List`/`Map`, or a
  `Result OF T` whose own payloads are all flat, plus the built-in
  flat records `Error`/`ErrorLoc`: inlined recursively as a `U64` block-relative
  offset into the data region, exactly like a `String`. The field read recovers
  `recordBase + offset`, and because the inlined block's own offsets are relative
  to that same base, a whole-block `memcpy` deep-copies the entire tree. A field is
  inlined into the data region iff it is a `String` or a flat composite. [[src/target/shared/code/builder_collection_layout.rs:record_field_is_inlined]]
- **non-flat composite** — a **resource** `Union`, a `List`/`Map` carrying a
  resource or recursive payload, a non-flat `Result`, or a nested record that is
  not (or cannot be) flat (e.g. one on a type cycle): an 8-byte **pointer** to a
  separate allocation.

Because inlined fields are variable-length, a record's total byte size is
computed by walking the fixed slot region plus each inlined sub-block (a `String`
block, or recursively any inlined flat-composite block).
Construction, `WITH`-update, copy/transfer, equality, and collection embedding
all use that runtime size; copying a record whose fields are all scalar,
`String`, or flat-composite (i.e. a flat record) is a single block `memcpy`
(no per-field deep copy) — only a non-flat pointer field needs a deep copy of its
separate allocation. The built-in helper-
constructed `net::` records `Address`, `Datagram`, and `DatagramText` are
**excluded**: their `String`/sub-record fields remain pointers to separate
allocations (the socket helpers build them that way), so reads of those records
do not rebase. [[src/target/shared/code/builder_collection_layout.rs:type_is_flat]]

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
fallible-call ABI registers. [[src/target/shared/code/builder_codegen_primitives.rs:emit_load_error_fields]] Construction, field access, copy, and
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
all-resource, never mixed — rule `TYPE_MIXED_RESOURCE_UNION`) is
**not** reshaped [[src/rules/table.rs:TYPE_MIXED_RESOURCE_UNION]] — it keeps the fixed
`{U64 activeMemberTag@0, resource-handle-ptr@8}` layout, and the handle is moved
(never deep-copied) so the resource is closed exactly once.

## Resource Record

Every resource value is a pointer to an **80-byte arena record**. The size is
uniform across resource kinds — `File`, `Socket`, `Listener`, `TlsSocket`,
`AudioInput`, a native `LINK` resource — so the generic thread-transfer copy and
the closed-default record stay one implementation. A kind that needs fewer words
carries the rest inertly.

```text
ResourceRecord (80 bytes, alignment 8)
  +0   U64  handle    ; the OS handle (fd, socket, or native pointer)
  +8   U64  flags     ; flag SET, not a boolean — see below
  +16  U64  state     ; pointer to the STATE payload, 0 until initialized
  +24  U64  bufPtr    ; per-File output buffer (plan-14-B), 0 = unbuffered
  +32  U64  bufFilled ; bytes currently held in bufPtr
  +40  U64  bufEnabled; 0 on every freshly opened handle
  +48  U64  readPtr   ; per-File read buffer (plan-14-C), 0 until first read
  +56  U64  readPos   ; next unconsumed byte offset within readPtr
  +64  U64  readFill  ; valid bytes in readPtr
  +72  U64  readAtEof ; set once the underlying read() returned 0
```

`flags` at **offset 8 is a compiler-enforced invariant**, not a convention: bit 0
is `closed`, bit 1 is `moved`, and 62 bits are spare. Every guard tests the word
for *non-zero* rather than for `== 1`, so a moved record already refuses every
operation with no extra code; only a path that must distinguish
`ErrResourceMoved` from `ErrResourceClosed` reads the individual bits. A
closed-default record is 80 zeroed bytes with this word set to 1. Compile-time
asserts tie every per-backend resource layout to this offset, so a future
resource whose closed flag drifts off offset 8 fails to build.
[[src/target/shared/code/error_constants.rs:RESOURCE_RECORD_SIZE_BYTES]] [[src/target/shared/code/error_constants.rs:RESOURCE_OFFSET_CLOSED]] [[src/target/shared/code/error_constants.rs:RESOURCE_MOVED_BIT]]

A borrow of a resource shares the record, and therefore shares the `state`
pointer. Scope-drop reclaims the two buffers and the `STATE` payload but leaves
the 80-byte record itself as a tombstone carrying the flags — see
`./mfb spec memory arenas`.

## See Also

* ./mfb spec threading isolation — re-materializing a heap value across a thread boundary
* ./mfb spec memory arenas — where these values are allocated and freed
* ./mfb spec memory collections — the uniform `List`/`Map` layout
