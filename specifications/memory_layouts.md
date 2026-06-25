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

### `Error` and `ErrorLoc`

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

### `Result`

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

### Union

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

## Arenas

Every heap-backed value — strings, records, unions, errors, and collections —
is allocated from an *arena*. An arena owns a chain of OS-mapped blocks plus a
small fixed *arena-state* control structure, and manages the mapped bytes with a
single per-arena **address-ordered coalescing free-list**: a freshly mapped
block's usable region is added to the list as one big free chunk, `arena_alloc`
carves allocations out of it (first-fit + split), and `arena_free` returns a
chunk and merges it with address-adjacent free neighbors. The free-list subsumes
the historical bump pointer — bumping is just splitting the one big trailing free
chunk. Freeing is **internal reuse only**: a freed chunk goes back on the
free-list for the next allocation but is never returned to the OS; mapped blocks
are unmapped only by the bulk `arena_destroy` when the owning package instance
shuts down.

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
  +16  U64  fillRngLo        ; dedicated memory-fill PCG64 state, low 64 bits
  +24  U64  fillRngHi        ; dedicated memory-fill PCG64 state, high 64 bits
  +32  U64  exitStatus       ; pending exit/result code used during teardown
  +40  U64  arenaStartTime   ; arena init time in ns (diagnostics + fill-seed mix)
  +48  U64  freeListHead     ; lowest-address free chunk, 0 when the list is empty
  +56  U64  reserved
  +64  U64  cleanupFailCount ; count of cleanup errors (audit)
  +72  U64  cleanupFailCode  ; last cleanup failure error code
  +80  U64  cleanupFailMsg   ; pointer to last cleanup failure message
  +88  U64  rngStateLo       ; PCG64 RNG state, low 64 bits
  +96  U64  rngStateHi       ; PCG64 RNG state, high 64 bits
```

`blockHead` anchors the unmap walk and `freeListHead` anchors allocation. The
cleanup-failure triple records diagnostics if reclamation of a value fails during
teardown, and the two RNG words at 88/96 give each arena (hence each thread) an
independent `math::rand` stream seeded at startup. The `fillRngLo`/`fillRngHi`
words at 16/24 hold a **separate** dedicated memory-fill PCG64 stream (see Entropy
Fill below), seeded independently so it never perturbs the reproducible
`math::rand` sequence. The `ENTRY_*` argv/argc fields
the entry shim stores begin at offset `ARENA_STATE_SIZE`, immediately after this
structure on the entry stack. Because the main arena-state lives on the entry
stack (not zero-filled), the entry shim explicitly clears `freeListHead` before
the first allocation; worker arenas are zero-initialized at creation.

### Free-List Layout

A free chunk overlays an intrusive `FreeNode` in its own dead bytes; its start is
implicit (the node's own address):

```text
FreeNode (overlaid on a free chunk, start = node address)
  +0   U64  next            ; next free chunk in ascending address order, 0 = end
  +8   U64  size            ; size of this free chunk, in bytes (includes the node)
  ...                       ; remaining bytes are dead
```

The list is kept ascending by `start`, so one walk finds both the insertion slot
and the coalescing neighbors. **Allocated chunks carry no header or footer** —
only free chunks spend 16 bytes on a node, in their own dead space — which is
what keeps object layouts and copying untouched. The minimum granule is therefore
16 bytes; every request is rounded up to 16 and every allocation is at least
16-byte aligned, so each chunk start stays 16-aligned and each chunk size stays a
multiple of 16 (a split front/tail remainder is always 0 or a valid ≥16 node,
never sub-granule slack). Sizes passed to `arena_free` are supplied by the
compiler's drop glue from the static type, so no per-object size tag is needed to
recover a chunk's size at free time.

### Block Layout

Blocks are mapped on demand and chained head-first into a singly-linked list via
a 32-byte (`ARENA_BLOCK_HEADER_SIZE`) header:

```text
ArenaBlock
  +0   U64  prevBlock        ; previous block in the chain, 0 for the first
  +8   U64  blockSize        ; total mapped size of this block, in bytes
  +16  U64  usableCapacity   ; blockSize - 32 (bytes available after the header)
  +24  U64  bumpOffset       ; vestigial under the free-list (kept 0); see below
  +32  ...  payload          ; usableCapacity bytes managed by the free-list
```

`ArenaState.blockHead` always points at the newest block; older blocks are
reachable only through each block's `prevBlock` link, which is the chain
`arena_destroy` unmaps. The default block size is `ARENA_DEFAULT_BLOCK_SIZE` =
**4096 bytes**. Allocation no longer reads `bumpOffset` — it is written `0` at map
time and kept only so the block-header layout is unchanged; the free-list drives
all placement.

### `arena_alloc(size, align)`

`arena_alloc` (symbol `_mfb_arena_alloc`) takes a byte `size` in `x0` and a power-
of-two `align` in `x1`, and returns a fallible result: `x0` is `0` on success
with the aligned pointer in `x1`, or an error code in `x0` with `x1 = 0` on
failure. It clobbers **x9, x10, x14, x15, x20–x28** (the OS map is emitted inline,
so `arena_alloc` remains a leaf and never touches x11–x13/x17); callers must spill
any live values held in the clobbered registers across the call.

The algorithm:

1. **Validate alignment.** A zero `align`, or one that is not a power of two
   (`(align - 1) & align != 0`), returns `ErrInvalidArgument`.
2. **Normalize the request.** A zero `size` becomes `1`; `size` is then rounded up
   to the 16-byte granule, and `align` is raised to at least 16. This keeps every
   chunk 16-aligned and 16-sized.
3. **First-fit walk.** Walk the address-ordered free-list for the first chunk
   where the request fits after alignment: `aligned = align_up(start, align)` and
   `aligned + size <= start + chunkSize`. **Split** it — return `aligned`, push the
   front padding (`aligned - start`, if > 0) and the tail remainder
   (`chunkEnd - (aligned + size)`, if > 0) back as free chunks. All sums are
   overflow-checked; an overflow skips the chunk. First-fit over an ascending list
   reuses low-address holes before carving the trailing chunk, and carving the
   trailing chunk *is* the old bump pointer (O(1) while the list is short).
4. **Grow.** If no chunk fits, map a new block sized
   `max(4096, round_up(size + align + 32, 4096))`, write its header, link it at the
   head, insert its usable region as one free chunk (in address order), and retry
   the walk.

A failed mapping (the platform `mmap`/`VirtualAlloc` hook) reports
`ErrOutOfMemory`. Both `ErrInvalidArgument` and `ErrOutOfMemory` surface to
source as ordinary language-level errors (see the language spec §14.3.1).

### `arena_free(ptr, size)`

`arena_free` (symbol `_mfb_arena_free`) takes the chunk pointer in `x0` and its
byte `size` in `x1` and returns nothing; it clobbers **x9–x13**. `size` is
normalized exactly as `arena_alloc` normalizes it (zero → 1, rounded up to 16),
so the freed extent matches the live chunk that was handed out. The chunk is
inserted into the address-ordered free-list (`_mfb_arena_insert_free`, the same
ordered insert the grow path uses) and **coalesced** with the address-adjacent
free neighbor on either side:

- `prev.start + prev.size == ptr` → extend `prev` over the chunk;
- `ptr + size == next.start` → absorb `next`;
- both → merge all three into one chunk;
- neither → link a fresh `{next, size}` node at `ptr`.

Adjacency is pure arithmetic on `start`/`size`, so no boundary tags are read.
Chunks in different blocks are never contiguous (the 32-byte header always
separates blocks), so a merge never spans a block. `arena_free` **never unmaps**
— it only relinks; memory returns to the OS solely at `arena_destroy`. The `size`
is always supplied by the compiler's drop glue from the static type, so there is
no user-level free and no class of wrong-size/double-free bugs outside a codegen
error.

### Entropy Fill

Freed chunks and freshly mapped blocks are filled with pseudo-random bytes —
always on, in debug and release. This scrubs freed secrets so they
do not linger as plaintext and poisons memory so a use-after-free or
uninitialized read yields garbage instead of stale-but-plausible data. Because
fresh arena memory is no longer implicitly zero, every allocation site must fully
initialize the bytes it later reads (the language's allocators already do).

The fill source is a **dedicated per-arena PCG64** at arena-state offsets 16/24,
separate from the `math::rand` stream at 88/96 and seeded independently at arena
init (`arena_fill_seed`): the main thread mixes OS entropy (`getentropy`) with the
arena address and start time (offset 40); each worker mixes a draw from the
parent's fill stream with its own arena address. Its output is never observable —
filled bytes are always overwritten by a constructor before any read — so the
stream needs no reproducibility. `arena_fill_random(ptr, len)` streams PRNG words
(no syscall per fill); `arena_free` calls it before relinking a chunk, and
`arena_alloc` calls it on a freshly mapped block's usable region before first use.

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

### Scope-Drop Frees

Beyond the bulk `arena_destroy`, individual owned values are freed deterministically
at **scope-drop**, the same model resources already use. Because every non-resource
value is a flat, pointer-free block, freeing one is a single `arena_free(ptr, size)`
of its block — no per-type recursive drop glue — and the size is recomputed from the
static type at the drop point (`emit_inlined_block_size_from_ptr_slot`).

Soundness rests on the heap being an **ownership tree**, which **copy-insertion**
guarantees: every site that hands a value to a longer-lived owner — a `LET`/`MUT`
bind, a global store, an assignment, a closure capture, and a `RETURN` — deep-copies
(`copy_flat_block`) when the source is an *aliasing source* (a `Local`, `Global`,
`Capture`, field/`MemberAccess` read, `UnionExtract`, or `Result` payload — all of
which yield a borrow/pointer into an existing block) or a *static* `String` constant
(which lives in rodata, not the arena). Record/union/collection construction,
collection inserts, and `WITH` already byte-copy (inline) their flat payloads, so
they introduce no new aliases. After copy-insertion every owned local owns an
independent block, so freeing each exactly once at scope exit cannot double-free.

A free is emitted at **every** scope exit — the normal end-of-block drain,
`EXIT`/`CONTINUE` (only back to the loop's entry depth), `RETURN`, and `TRAP`
routing — reusing the resource cleanup stack (`ActiveCleanup::OwnedValue`). A value
that is **moved out** suppresses its free: a returned named local is moved (not
copied) and its cleanup deactivated; `thread::transfer` already deep-copies into the
receiver arena and deactivates the sender's cleanup. Binding slots are
zero-initialized before a (possibly trapping) initializer runs and the free is
null-guarded, so an initializer that traps before storing frees nothing.

Three classes of value are **excluded** from scope-drop frees because they are not
plain arena blocks this scope owns: **resources** (a move-only handle to the single
arena-global instance, reclaimed by its own close op); **runtime-managed thread
results** (`thread::receive`/`waitFor`/… yield values owned by the thread plumbing
and the worker arena, bulk-freed at teardown); and **recursive / non-flat composites**
(kept as pointer graphs, `type_is_flat` is false). Builtins that previously returned
a borrow into an argument now return an owned block instead (`collections::get`/`getOr`
materialize the element; `strings::replace`'s no-op path returns a fresh copy), so a
call result is always safe for the caller to own and free.

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
