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
searches large size-class bins and sizes new mappings to the request. The "Arenas"
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
logical string length. A tight `String` block's logical size is therefore
`byteLength + 9` (the 8-byte length word plus the bytes plus the NUL); this same
`+9` formula sizes a `String` block inlined into a record or `Result`
(`emit_inlined_block_size_from_ptr_slot`). [[src/codegen/collection/layout/builder_collection_layout.rs:emit_inlined_block_size_from_ptr_slot]]

A `MUT` binding's own block can retain capacity, and only while it is bound: an
in-place self-update (`mfb spec memory collections`, *Self-updates*) may leave the
block larger than `byteLength + 9`, with the spare bytes recorded beside the
binding so its drop frees the block at its real size. The spare capacity never
escapes — every copy, return and transfer reads `byteLength` bytes and produces
the tight form above — so no reader of a `String` ever sees it.
[[src/codegen/collection/assign/string_self_update.rs:emit_string_set_len]]

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

- **scalar** (`Boolean`/`Byte`/`Integer`/`Float`/`Fixed`/`Money`/`Scalar`/enum): the value inline.
- **`String`**: a `U64` **block-relative offset** into the record's own data
  region, where the `String`'s flat block (`{U64 len, bytes, U8 nul}`, 8-aligned)
  is embedded inline. The field read recovers the interior pointer as
  `recordBase + offset`; the offset is relative to the record base, so a whole-
  block `memcpy` is a correct deep copy and the inlined `String` comes along.
- **flat composite** — a nested record, a data `Union`, a `List`/`Map`/`Set`, or a
  `Result OF T` whose own payloads are all flat, plus the built-in
  flat records `Error`/`ErrorLoc`: inlined recursively as a `U64` block-relative
  offset into the data region, exactly like a `String`. The field read recovers
  `recordBase + offset`. Each nested block preserves its own internal relative
  addressing, so a whole-block `memcpy` relocates the inlined tree. A field is inlined into the data region iff it is a `String` or a flat composite. [[src/codegen/collection/layout/builder_collection_layout.rs:record_field_is_inlined]]
- **non-flat composite** — a **resource** `Union`, a non-flat `List`/`Map`/`Set`,
  a non-flat `Result`, or a nested record that is
  not (or cannot be) flat (e.g. one on a type cycle): an 8-byte **pointer** to a
  separate allocation. For a value whose type reaches a type cycle, that separate
  allocation is an **owned child**: copying the record deep-copies it (the graph
  copy walker), and dropping the record frees it with the parent (the graph drop
  walker, `./mfb spec memory arenas` "Scope-Drop Frees"). [[src/codegen/memory/arena/graph_drop.rs:lower_graph_drop_walker]]

Because inlined fields are variable-length, a record's total byte size is
computed by walking the fixed slot region plus each inlined sub-block (a `String`
block, or recursively any inlined flat-composite block).
Construction, `WITH`-update, copy/transfer, equality, and collection embedding
all use that runtime size; copying a record whose fields are all scalar,
`String`, or flat-composite (i.e. a flat record) uses a block copy without per-field graph traversal. A recursive owned
pointer field needs its own graph copy; resource and function pointers retain
their separate ownership rules. Records a runtime helper constructs — `net::Address`,
`udp::Datagram`, `net::PingResult`, `audio::AudioDevice` — follow the same layout
exactly: the helper builds the flat image, and every read of their fields rebases. [[src/codegen/memory/marshal/record.rs:emit_build_inlined_record_sized]]

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
has the fixed three-slot prefix, an inlined message, and a zero source offset;
the size walk ends at the message rather than reading an absent `ErrorLoc`. [[src/codegen/collection/layout/builder_collection_layout.rs:emit_record_block_size_to_slot]]

## `Result`

`Result OF T` uses a `{tag, size, payload}` block — a two-variant data union
`Ok(T)` / `Err(Error)`:

```text
Result
  +0   U64   tag         ; 0 = Ok, otherwise Err
  +8   U64   size        ; total byte size of THIS object
  +16  payload           ; a scalar/pointer word, or a block inlined whole
```

A scalar success payload occupies the 8-byte word at `+16`; a block payload
(`String`, record, data union, collection, the `Err` `Error`, or a nested `Result`) is
inlined whole at `+16` and `size` covers it. Reading the value yields an interior
pointer into the block (`base + 16`) for a block payload, or the 8-byte value for
a scalar. A resource or function payload occupies the word at `+16` as a pointer;
a resource union likewise occupies that word as a pointer to its union box.
The result is memcpy-copyable only when its success type is: recursive
payloads require graph copying, and resource/thread payloads follow their
ownership and transfer rules.
[[src/codegen/memory/value/builder_value_semantics.rs:result_payload_is_block]]
[[src/codegen/collection/layout/builder_collection_layout.rs:type_is_memcpy_copyable]]

## Union

A **data** union (all variants are data records) is a self-describing
`{tag, size, data}` block sized to the **active** variant:

```text
DataUnionObject (flat)
  +0   U64 tag       ; active variant index
  +8   U64 size      ; total byte size of THIS object (16 + variant block)
  +16  data          ; the active variant's flat record block, inlined
```

`data` is the active variant's record laid out exactly as a standalone record
(scalar slots inline; `String`/flat-record fields inlined by block-relative
offset — relative to the variant record base (`unionBase + 16`)). Constructing a variant wraps its
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

Every resource value is a pointer to a **112-byte arena record**. The size is
uniform across resource kinds — `File`, `Socket`, `Listener`, `tls::Socket`,
`AudioInput`, a native `LINK` resource — so the generic thread-transfer copy and
the closed-default record stay one implementation. A kind that needs fewer words
carries the rest inertly.

Offsets `0..40` are the canonical header shared by built-in and package
resources; the type-specific tail starts at `+40` (`RESOURCE_TAIL_START`), and
every per-kind tail offset is defined as `RESOURCE_TAIL_START + n`. The `STATE`
payload pointer occupies `+24` and the owner word `+32`, independently of the
resource kind.
[[src/codegen/error/constants/error_constants.rs:RESOURCE_OFFSET_STATE]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_OFFSET_OWNER]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_TAIL_START]]

```text
ResourceRecord (112 bytes, alignment 8)
  ; --- canonical header (every resource, offsets 0..40) ---
  +0   U64  tag       ; resource type id, self-describing; 0 = invalid
  +8   U64  handle    ; polymorphic: fd / connection ptr / audio kind / native ptr
  +16  U64  closed    ; flag SET, not a boolean — see below
  +24  U64  state     ; pointer to the STATE payload, 0 until initialized
  +32  U64  owner     ; scope token of the one scope that closes it; 0 = unowned
  ; --- type-specific tail (offsets 40+; File shown) ---
  +40  U64  bufPtr    ; per-File output buffer, 0 = unbuffered
  +48  U64  bufFilled ; bytes currently held in bufPtr
  +56  U64  bufEnabled; 0 on every freshly opened handle
  +64  U64  readPtr   ; per-File read buffer, 0 until first read
  +72  U64  readPos   ; next unconsumed byte offset within readPtr
  +80  U64  readFill  ; valid bytes in readPtr
  +88  U64  readAtEof ; set once the underlying read() returned 0
```

The File tail ends at offset 96; the longest tail (`Process`) ends at 104; the
canonical record size is 112 bytes (a 16-byte multiple). Compile-time asserts
tie every tail's last word to `RESOURCE_RECORD_SIZE_BYTES`. `tag` at `+0` makes a record
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
| 5   | `RESOURCE_TAG_TLS_OPENSSL`  | `tls::Socket` (OpenSSL backend)     |
| 6   | `RESOURCE_TAG_TLS_MACOS`    | `tls::Socket` (Network.framework)   |
| 7   | `RESOURCE_TAG_TLS_SCHANNEL` | `tls::Socket` (Windows SChannel)    |
| 8   | `RESOURCE_TAG_TLS_LISTENER` | TLS listener                        |
| 9   | `RESOURCE_TAG_AUDIO`        | audio input/output                  |
| 10  | `RESOURCE_TAG_PROCESS`      | child `Process`                     |
| 11  | `RESOURCE_TAG_IMAGE`        | canvas `Image`                      |
| 12  | `RESOURCE_TAG_FONT`         | canvas `Font`                       |
| 13  | `RESOURCE_TAG_THREAD`       | `Thread` view                       |
| 14  | `RESOURCE_TAG_THREAD_WORKER` | `ThreadWorker` view                |
| 255 | `RESOURCE_TAG_NATIVE`       | imported / native `LINK` resource   |

[[src/codegen/error/constants/error_constants.rs:RESOURCE_TAG_FILE]]

The type-specific tail (offsets 40+) differs per backend; every kind shares the
`0..40` header and the closed-default covers the widest layout. `—` is an inert
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
| 32 | owner | owner | owner | owner | owner | owner |
| 40 | out-buf ptr | — | — | — | sampleRate | — |
| 48 | out-buf filled | — | — | — | channels | — |
| 56 | out-buf enabled | — | — | — | bytesPerFrame | — |
| 64 | read-buf ptr | — | — | — | bufferFrames | — |
| 72 | read-buf pos | — | — | — | AudioState ptr | — |
| 80 | read-buf fill | — | — | — | — | — |
| 88 | read-buf at-eof | — | — | — | — | — |

[[src/codegen/builtins/audio/gen_shared.rs:H_SAMPLE_RATE]]

The `tls::Socket` backend is platform-selected (OpenSSL on Linux, Network.framework
on macOS, SSPI/SChannel on Windows); `TLSListener` is the OpenSSL server listener.
Each still shares the `0..40` header:

| Offset | TLS ossl | TLS macOS | TLS schan | TLSListener |
|--------|----------|-----------|-----------|-------------|
| 0  | 0x05 | 0x06 | 0x07 | 0x08 |
| 8  | fd | conn ptr | fd | fd |
| 16 | closed | closed | closed | closed |
| 24 | STATE | STATE | STATE | STATE |
| 32 | owner | owner | owner | owner |
| 40 | SSL_CTX | conn CTX | — | SSL_CTX |
| 48 | SSL | dispatch queue | SSPI block ptr | — |

[[src/codegen/builtins/tls/gen_shared.rs:TLS_OFFSET_CTX]] [[src/codegen/builtins/tls/gen_shared.rs:TLS_SCHANNEL_OFFSET_BLOCK]]

`closed` at **offset 16 is a compiler-enforced invariant**, not a convention: it
is a u64 flag *set*, not a boolean — bit 0 is `closed`, bit 1 is `moved`, and 62
bits are spare. A moved record is flagged `moved|closed` = 3, so a test for
*non-zero* refuses it as well as a closed one; the drop and transfer paths, which
only need to refuse, test it that way. Every operation on a built-in handle that
can cross a thread (`File`, the `tcp`/`udp`/`tls` sockets and listeners, a
`Thread` view) instead opens with a guard that splits the two: a word of exactly
`1` raises `ErrResourceClosed`, and any other non-zero word raises
`ErrResourceMoved`. The handles that cannot cross (`Process`, the audio streams,
`Image`, `Font`) never carry the moved bit and keep the single non-zero test.
[[src/codegen/resource/mod.rs:closed_or_moved_guard]] A closed-default
record is 112 zeroed bytes with this word set to 1 (so its owner word is `0`). Compile-time asserts tie every
per-backend resource layout to the header offsets, so a future resource whose
`closed`/`STATE` slot drifts fails to build.
[[src/codegen/error/constants/error_constants.rs:RESOURCE_RECORD_SIZE_BYTES]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_OFFSET_TAG]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_OFFSET_CLOSED]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_OFFSET_STATE]] [[src/codegen/error/constants/error_constants.rs:RESOURCE_MOVED_BIT]]

Every reference to a resource shares the one record, and therefore shares the
`state` pointer and the owner word. Scope-drop reclaims File buffers and the `STATE`
payload when it owns them. Whether it also frees the resource record depends on
resource kind and binding ownership; the exact rule is in `./mfb spec memory arenas`.

### The owner word

`owner` at `+32` names the one scope that closes the resource
(`./mfb spec language resource-management` §15.1). `0` means **unowned**: freshly
produced and not yet bound, or released by a `RETURN` and not yet taken. Every
producer — the built-in open/connect/spawn/start helpers, a native `LINK`
constructor, the closed default, and the receiving copy `thread::accept` makes —
writes `0`. [[src/codegen/resource/owner.rs:emit_resource_claim]]

A scope's **token** is `((sp + token_slot) << 12) - depth`: `token_slot` is one
8-byte stack object per function, `depth` is the scope's nesting depth inside it
(the function body is 1). It is never `0`, and:

* two live scopes of one thread never share a token — different frames have
  different `token_slot` addresses, and one frame's live scopes have different
  depths; sibling scopes share one but are never live together;
* of two live tokens the **larger is the outer** scope. Every target's stack grows
  down and a callee's frame lies below its caller's `sp`, so every token of a
  callee is below every token of its caller, and a deeper scope of one frame
  subtracts more. Depth is bounded by `SCOPE_TOKEN_MAX_DEPTH` (4095); statement
  nesting stops at 256;
* `(owner >> 12) + 1 == sp + token_slot` holds exactly for the records a scope of
  the current frame owns.

Tokens need no per-thread state: each thread runs on its own stack, and a record
changes thread only through `thread::transfer`/`thread::accept`, whose copy starts
unowned. [[src/codegen/resource/owner.rs:SCOPE_TOKEN_DEPTH_BITS]]

## See Also

* ./mfb spec threading isolation — re-materializing a heap value across a thread boundary
* ./mfb spec memory arenas — where these values are allocated and freed
* ./mfb spec memory collections — the uniform `List`/`Map` layout
