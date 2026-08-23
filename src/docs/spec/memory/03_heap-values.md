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
(`emit_inlined_block_size_from_ptr_slot`). [[src/codegen/collection/layout/builder_collection_layout.rs:emit_inlined_block_size_from_ptr_slot]]

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
  is embedded inline. The field read recovers the interior pointer as
  `recordBase + offset`; the offset is relative to the record base, so a whole-
  block `memcpy` is a correct deep copy and the inlined `String` comes along.
- **flat composite** — a nested record, a data `Union`, a `List`/`Map`, or a
  `Result OF T` whose own payloads are all flat, plus the built-in
  flat records `Error`/`ErrorLoc`: inlined recursively as a `U64` block-relative
  offset into the data region, exactly like a `String`. The field read recovers
  `recordBase + offset`, and because the inlined block's own offsets are relative
  to that same base, a whole-block `memcpy` deep-copies the entire tree. A field is
  inlined into the data region iff it is a `String` or a flat composite. [[src/codegen/collection/layout/builder_collection_layout.rs:record_field_is_inlined]]
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
do not rebase. [[src/codegen/collection/layout/builder_collection_layout.rs:type_is_flat]]

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
fallible-call ABI registers. [[src/codegen/error/emission/builder_error_emission.rs:emit_load_error_fields]] Construction, field access, copy, and
thread-transfer reuse the generic flat-record machinery — copying an `Error` is
one `memcpy`.

The generic **size** walk honors the same sentinel: it reads each inlined field's
own offset word and skips the field when that word is 0, rather than assuming
every inlined sub-block is present at the running offset. An origin-less `Error`
is `{code, message}` with nothing written past the message, so the running-offset
assumption sized a phantom `ErrorLoc` out of whatever followed the block — and
freeing that error handed `arena_free` a garbage size (bug-371). [[src/codegen/collection/layout/builder_collection_layout.rs:emit_record_block_size_to_slot]]

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
inlined whole at `+16` and `size` covers it. Reading the value yields an interior
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
yields an interior pointer to the record at `+16`. The `size` word makes copy/free
generic (read the size, `memcpy`, then deep-copy only the active variant's
pointer fields). The union is variable-length, so a `List`/`Map` of a data union
stores each union block inline by its runtime `size`.

A **resource** union (all variants are resource handles; a union is all-data or
all-resource, never mixed — rule `TYPE_MIXED_RESOURCE_UNION`) is
**not** reshaped [[src/rules/table.rs:TYPE_MIXED_RESOURCE_UNION]] — it keeps the fixed
`{U64 activeMemberTag@0, resource-handle-ptr@8}` layout, and the handle is moved
(never deep-copied) so the resource is closed exactly once.

## Resource Record

Every resource value is a pointer to a **96-byte arena record**. The size is
uniform across resource kinds — `File`, `Socket`, `Listener`, `TlsSocket`,
`AudioInput`, a native `LINK` resource — so the generic thread-transfer copy and
the closed-default record stay one implementation. A kind that needs fewer words
carries the rest inertly.

Offsets `0..32` are a **single canonical header** shared by every built-in and
package resource (plan-80); the type-specific tail starts at `+32`. Before
plan-80 the header diverged after offset 8 and `STATE` lived at 16, which the
TLS/audio backends already used for `SSL*`/dispatch-queue/sample-rate fields — so
a union `STATE` over a `TlsSocket` clobbered a live field. The unified header
gives the generic `STATE` payload (plan-74) a free slot in *every* layout.

```text
ResourceRecord (96 bytes, alignment 8)
  ; --- canonical header (every resource, offsets 0..32) ---
  +0   U64  tag       ; resource type id, self-describing (plan-80); 0 = invalid
  +8   U64  handle    ; polymorphic: fd / connection ptr / audio kind / native ptr
  +16  U64  closed    ; flag SET, not a boolean — see below
  +24  U64  state     ; pointer to the STATE payload, 0 until initialized
  ; --- type-specific tail (offsets 32+; File shown) ---
  +32  U64  bufPtr    ; per-File output buffer (plan-14-B), 0 = unbuffered
  +40  U64  bufFilled ; bytes currently held in bufPtr
  +48  U64  bufEnabled; 0 on every freshly opened handle
  +56  U64  readPtr   ; per-File read buffer (plan-14-C), 0 until first read
  +64  U64  readPos   ; next unconsumed byte offset within readPtr
  +72  U64  readFill  ; valid bytes in readPtr
  +80  U64  readAtEof ; set once the underlying read() returned 0
```

The File tail is the widest, ending at offset 88; the record is rounded up to 96
(a 16-byte multiple, with one slot of headroom). `tag` at `+0` makes a record
self-describing, though close dispatch itself stays compile-time-resolved by the
static resource type. `0x00` is never a live record (uninitialized/invalid); a
value `< 0xFE` keys a built-in resource; an imported/native `LINK` resource
carries `RESOURCE_TAG_NATIVE`:

| Tag | Constant                    | Resource kind                       |
|-----|-----------------------------|-------------------------------------|
| 0   | *(none)*                    | uninitialized / invalid             |
| 1   | `RESOURCE_TAG_FILE`         | `File`                              |
| 2   | `RESOURCE_TAG_SOCKET`       | TCP `Socket`                        |
| 3   | `RESOURCE_TAG_UDP_SOCKET`   | UDP socket                          |
| 4   | `RESOURCE_TAG_LISTENER`     | TCP `Listener`                      |
| 5   | `RESOURCE_TAG_TLS_OPENSSL`  | `TlsSocket` (OpenSSL backend)       |
| 6   | `RESOURCE_TAG_TLS_MACOS`    | `TlsSocket` (Network.framework)     |
| 7   | `RESOURCE_TAG_TLS_SCHANNEL` | `TlsSocket` (Windows SChannel)      |
| 8   | `RESOURCE_TAG_TLS_LISTENER` | TLS listener                        |
| 9   | `RESOURCE_TAG_AUDIO`        | audio input/output                  |
| 10  | `RESOURCE_TAG_PROCESS`      | child `Process`                     |
| 255 | `RESOURCE_TAG_NATIVE`       | imported / native `LINK` resource   |

[[src/codegen/error/constants/error_constants.rs:RESOURCE_TAG_FILE]]

The type-specific tail (offsets 32+) differs per backend; every kind shares the
`0..32` header and the closed-default covers the widest layout. `—` is an inert
(zeroed) slot the kind never reads. `Socket`, `UdpSocket`, and `Listener` are
header-only. A `Native` record (imported and native `LINK` resources alike) is a
zeroed `File`-shaped record — its handle is a native `CPtr` and its tail carries
no live fields (close is resolved by the native thunk, not stored in the record).
The four TLS backends share the header but split into their own table below.

| Offset | File | Socket | Udp | Listener | Audio | Native |
|--------|------|--------|-----|----------|-------|--------|
| 0  | 0x01 | 0x02 | 0x03 | 0x04 | 0x09 | 0xFF |
| 8  | fd | fd | fd | fd | kind | CPtr |
| 16 | closed | closed | closed | closed | closed | closed |
| 24 | STATE | STATE | STATE | STATE | STATE | STATE |
| 32 | out-buf ptr | — | — | — | sampleRate | — |
| 40 | out-buf filled | — | — | — | channels | — |
| 48 | out-buf enabled | — | — | — | bytesPerFrame | — |
| 56 | read-buf ptr | — | — | — | bufferFrames | — |
| 64 | read-buf pos | — | — | — | AudioState ptr | — |
| 72 | read-buf fill | — | — | — | — | — |
| 80 | read-buf at-eof | — | — | — | — | — |

[[src/codegen/builtins/audio/gen_os_seam.rs:H_SAMPLE_RATE]]

The `TlsSocket` backend is platform-selected (OpenSSL on Linux, Network.framework
on macOS, SSPI/SChannel on Windows); `TLSListener` is the OpenSSL server listener.
Each still shares the `0..32` header:

| Offset | TLS ossl | TLS macOS | TLS schan | TLSListener |
|--------|----------|-----------|-----------|-------------|
| 0  | 0x05 | 0x06 | 0x07 | 0x08 |
| 8  | fd | conn ptr | fd | fd |
| 16 | closed | closed | closed | closed |
| 24 | STATE | STATE | STATE | STATE |
| 32 | SSL_CTX | conn CTX | — | SSL_CTX |
| 40 | SSL | dispatch queue | SSPI block ptr | — |

[[src/codegen/builtins/tls/gen_os_seam.rs:TLS_OFFSET_CTX]] [[src/codegen/builtins/tls/gen_os_seam.rs:TLS_SCHANNEL_OFFSET_BLOCK]]

`closed` at **offset 16 is a compiler-enforced invariant**, not a convention: it
is a u64 flag *set*, not a boolean — bit 0 is `closed`, bit 1 is `moved`, and 62
bits are spare. Every guard tests the word for *non-zero* rather than for `== 1`,
so a moved record already refuses every operation with no extra code; only a path
that must distinguish `ErrResourceMoved` from `ErrResourceClosed` reads the
individual bits (a moved record is flagged `moved|closed` = 3). A closed-default
record is 96 zeroed bytes with this word set to 1. Compile-time asserts tie every
per-backend resource layout to the header offsets, so a future resource whose
`closed`/`STATE` slot drifts fails to build.
[[src/codegen/error/constants/error_constants.rs:RESOURCE_RECORD_SIZE_BYTES]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_OFFSET_TAG]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_OFFSET_CLOSED]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_OFFSET_STATE]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_MOVED_BIT]]

Every pointer to a resource shares the one record, and therefore shares the `state`
pointer. Scope-drop reclaims the two buffers and the `STATE` payload but leaves
the 96-byte record itself as a tombstone carrying the flags — see
`./mfb spec memory arenas`.

## See Also

* ./mfb spec threading isolation — re-materializing a heap value across a thread boundary
* ./mfb spec memory arenas — where these values are allocated and freed
* ./mfb spec memory collections — the uniform `List`/`Map` layout
