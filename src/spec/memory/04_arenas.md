# Arenas

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
relying on the pinned register. [[src/target/shared/code/mod.rs:MAIN_ARENA_GLOBAL_SYMBOL]] Each worker package instance owns a separate
arena, referenced from its thread control block, so worker threads allocate and
reclaim independently of the main thread (see `./mfb spec threading`).

## Arena-State Layout

The arena-state structure is `ARENA_STATE_SIZE` = **104 bytes**: [[src/target/shared/code/mod.rs:ARENA_STATE_SIZE]]

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

## Free-List Layout

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

## Block Layout

Blocks are mapped on demand and chained head-first into a singly-linked list via
a 32-byte (`ARENA_BLOCK_HEADER_SIZE`) header: [[src/target/shared/code/mod.rs:ARENA_BLOCK_HEADER_SIZE]]

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
**4096 bytes**. [[src/target/shared/code/mod.rs:ARENA_DEFAULT_BLOCK_SIZE]] Allocation no longer reads `bumpOffset` — it is written `0` at map
time and kept only so the block-header layout is unchanged; the free-list drives
all placement.

## `arena_alloc(size, align)`

`arena_alloc` (symbol `_mfb_arena_alloc`) takes a byte `size` in `x0` and a power-
of-two `align` in `x1`, and returns a fallible result: [[src/target/shared/code/mod.rs:lower_arena_alloc]] `x0` is `0` on success
with the aligned pointer in `x1`, or an error code in `x0` with `x1 = 0` on
failure. The caller-visible clobber set is **x9, x10, x14, x15, x20–x28**; callers
must spill any live values held in those registers across the call. The fast
(first-fit) path makes no call, but the rare block-grow path calls
`arena_fill_random` to poison the freshly mapped block, so `arena_alloc` is **not**
a leaf — it carries a 64-byte frame and saves the link register. The grow path
saves and restores x11–x13 around the fill call, so those registers (and x17) stay
out of the caller-visible clobber set, preserving the historical contract.

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

## `arena_free(ptr, size)`

`arena_free` (symbol `_mfb_arena_free`) takes the chunk pointer in `x0` and its
byte `size` in `x1` and returns nothing; [[src/target/shared/code/mod.rs:lower_arena_free]] it clobbers **x9–x16** (it carries a
32-byte frame, saves the link register, and calls both `arena_fill_random` and
`arena_insert_free`). `size` is
normalized exactly as `arena_alloc` normalizes it (zero → 1, rounded up to 16),
so the freed extent matches the live chunk that was handed out. The chunk is first
entropy-scrubbed (see *Entropy Fill*), then
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

## Entropy Fill

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
parent's fill stream with its own arena address. [[src/target/shared/code/mod.rs:lower_arena_fill_seed]] Its output is never observable —
filled bytes are always overwritten by a constructor before any read — so the
stream needs no reproducibility. `arena_fill_random(ptr, len)` streams PRNG words
(no syscall per fill); `arena_free` calls it before relinking a chunk, and
`arena_alloc` calls it on a freshly mapped block's usable region before first use.

## Cleanup and Reclamation

An arena is reclaimed whole. `arena_destroy` (symbol `_mfb_arena_destroy`) walks
the block chain from `blockHead` through each `prevBlock`, [[src/target/shared/code/mod.rs:lower_arena_destroy]] unmapping every block
with the platform `munmap`/`VirtualFree` hook, then clears `blockHead` to `0`. It
frees no individual values; all memory returns to the OS at once. The helper is
idempotent — a second call sees `blockHead == 0` and does nothing.

At process teardown, `_mfb_shutdown` reads the arena-state address from
`_mfb_rt_main_arena`, clears that global first [[src/target/shared/code/mod.rs:SHUTDOWN_SYMBOL]] (so a signal arriving mid-teardown
re-enters as a no-op), restores the terminal if TUI mode was active, and then
calls `arena_destroy` on the main arena. A worker arena is reclaimed the same way
when its package instance ends; the thread control block must not retain any bare
handle into a worker arena past the point that arena becomes eligible for
reclamation, so cross-thread values are first re-materialized in the receiver's
arena (see `./mfb spec threading isolation`).

## Scope-Drop Frees

Beyond the bulk `arena_destroy`, individual owned values are freed deterministically
at **scope-drop**, the same model resources already use. Because every non-resource
value is a flat, pointer-free block, freeing one is a single `arena_free(ptr, size)`
of its block — no per-type recursive drop glue — and the size is recomputed from the
static type at the drop point (`emit_inlined_block_size_from_ptr_slot`). [[src/target/shared/code/builder_collection_layout.rs:emit_inlined_block_size_from_ptr_slot]]

Soundness rests on the heap being an **ownership tree** — every owned local owns an
independent block, so freeing each exactly once at scope exit cannot double-free.
The source-level ownership/move/copy model that guarantees this is canonical in
`./mfb spec language memory-semantics`. The codegen enforces it by **copy-insertion**:
every site that hands a value to a longer-lived owner (a `LET`/`MUT` bind, a global
store, an assignment, a closure capture, a `RETURN`) deep-copies (`copy_flat_block`)
when the source is an *aliasing source* (a `Local`, `Global`, `Capture`,
field/`MemberAccess` read, `UnionExtract`, or `Result` payload — all of which yield a
borrow into an existing block) or a *static* `String` constant (which lives in rodata,
not the arena). Record/union/collection construction, collection inserts, and `WITH`
already byte-copy their flat payloads inline, so they introduce no new aliases.

A free is emitted at **every** scope exit — the normal end-of-block drain,
`EXIT`/`CONTINUE` (only back to the loop's entry depth), `RETURN`, and `TRAP`
routing — reusing the resource cleanup stack (`ActiveCleanup::OwnedValue`). [[src/target/shared/code/mod.rs:OwnedValueCleanup]] A value
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

## See Also

* ./mfb spec threading — per-worker arenas and thread isolation
* ./mfb spec language memory-semantics — the source-level ownership model
* ./mfb spec architecture native — arena helpers in native codegen
