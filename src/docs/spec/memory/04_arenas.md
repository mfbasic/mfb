# Arenas

Every heap-backed value — strings, records, unions, errors, and collections —
is allocated from an *arena*. An arena owns a chain of OS-mapped blocks plus a
small fixed *arena-state* control structure, and manages the mapped bytes with
three cooperating structures (the classic deferred-coalescing
design): **128 per-size-class quick bins** (exact chunk sizes 16…2048 — a freed
small chunk parks on its bin in O(1) and the next same-class allocation pops it
in O(1)), a **designated-victim carve chunk** (one active chunk that bump-serves
small bin misses, so misses never splinter parked inventory), and a per-arena
**address-ordered coalescing free-list** as the backing store (large chunks,
block remainders, and re-coalesced bin drains). The common alloc/free cycle is
O(1) amortized regardless of the size mix. The designated victim subsumes the
bump pointer. Freeing is **internal reuse only**: a freed chunk goes
back to the allocator for the next allocation but is never returned to the OS;
mapped blocks are unmapped only by the bulk `arena_destroy` when the owning
package instance shuts down.

Each package instance owns a distinct arena. The main package's arena-state lives
on the entry stack and is pinned in `x19` (`ARENA_STATE_REGISTER`) for the life of
the program; its address is also published to the writable global
`_mfb_rt_main_arena` so signal handlers and shutdown code can reach it without
relying on the pinned register. [[src/target/shared/code/error_constants.rs:MAIN_ARENA_GLOBAL_SYMBOL]] Each worker package instance owns a separate
arena, referenced from its thread control block, so worker threads allocate and
reclaim independently of the main thread (see `./mfb spec threading`).

## Arena-State Layout

The arena-state structure is `ARENA_STATE_SIZE` = **3728 bytes**: [[src/target/shared/code/error_constants.rs:ARENA_STATE_SIZE]]

```text
ArenaState (at x19)
  +0    U64  blockHead        ; pointer to the current (most-recent) block, 0 if none
  +8    U64  reserved         ; zero-initialized
  +16   U64  fillRngLo        ; dedicated memory-fill PCG64 state, low 64 bits
  +24   U64  fillRngHi        ; dedicated memory-fill PCG64 state, high 64 bits
  +32   U64  exitStatus       ; pending exit/result code used during teardown
  +40   U64  arenaStartTime   ; arena init time in ns (diagnostics + fill-seed mix)
  +48   U64  freeListHead     ; lowest-address free chunk, 0 when the list is empty
  +56   U64  reserved
  +64   U64  cleanupFailCount ; count of cleanup errors (audit)
  +72   U64  cleanupFailCode  ; last cleanup failure error code
  +80   U64  cleanupFailMsg   ; pointer to last cleanup failure message
  +88   U64  rngStateLo       ; PCG64 RNG state, low 64 bits
  +96   U64  rngStateHi       ; PCG64 RNG state, high 64 bits
  +104  U64  quickBin[128]    ; per-size-class bin heads: exact chunk sizes
                              ; 16, 32, …, 2048 (class = size/16 - 1); 0 = empty
  +1128 U64  carvePtr         ; designated-victim carve chunk: current pointer
  +1136 U64  carveSize        ; remaining bytes in the carve chunk (0 = none)
  +1144 U64  outPtr           ; opt-in stdout buffer base, NULL until first use
  +1152 U64  outFilled        ; pending bytes held in the stdout buffer
  +1160 U64  outEnabled       ; io::setBuffered flag (0 = unbuffered default)
  +1168 U64  largeBin[64]     ; segregated large-block bin heads, hashed by exact
                              ; size (index = (size >> 4) & 63); chunks > 2048; 0 = empty
  +1680 U64  v128Slots[256]   ; per-thread v128 scalarization region (2048 bytes);
                              ; reserved on every target, addressed only by rv64
                              ; codegen; placed last so its offset stays layout-neutral
```

`blockHead` anchors the unmap walk; `freeListHead`, the 128 `quickBin` heads,
the 64 `largeBin` heads, and the `carvePtr`/`carveSize` designated victim anchor
allocation (see the small- and large-request fast paths below). The
cleanup-failure triple records diagnostics if reclamation of a value fails during
teardown, and the two RNG words at 88/96 give each arena (hence each thread) an
independent `math::rand` stream seeded at startup. The `fillRngLo`/`fillRngHi`
words at 16/24 hold a **separate** dedicated memory-fill PCG64 stream (see Entropy
Fill below), seeded independently so it never perturbs the reproducible
`math::rand` sequence. The `ENTRY_*` argv/argc fields
the entry shim stores begin at offset `ARENA_STATE_SIZE`, immediately after this
structure on the entry stack. Because the main arena-state lives on the entry
stack (not zero-filled), the entry shim zeroes the whole `ARENA_STATE_SIZE`
range with a loop before the first allocation; the thread-spawn path zeroes a
worker's freshly allocated arena state with the same size-derived loop, so the
two initializers can never fall out of lockstep when the state grows.

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
a 32-byte (`ARENA_BLOCK_HEADER_SIZE`) header: [[src/target/shared/code/error_constants.rs:ARENA_BLOCK_HEADER_SIZE]]

```text
ArenaBlock
  +0   U64  prevBlock        ; previous block in the chain, 0 for the first
  +8   U64  blockSize        ; total mapped size of this block, in bytes
  +16  U64  usableCapacity   ; blockSize - 32 (bytes available after the header)
  +24  U64  bumpOffset       ; reserved under the free-list (kept 0); see below
  +32  ...  payload          ; usableCapacity bytes managed by the free-list
```

`ArenaState.blockHead` always points at the newest block; older blocks are
reachable only through each block's `prevBlock` link, which is the chain
`arena_destroy` unmaps. The default block size is `ARENA_DEFAULT_BLOCK_SIZE` =
**4096 bytes**. [[src/target/shared/code/error_constants.rs:ARENA_DEFAULT_BLOCK_SIZE]] Allocation does not read `bumpOffset` — it is written `0` at map
time and kept only so the block-header layout is unchanged; the free-list drives
all placement.

## `arena_alloc(size, align)`

`arena_alloc` (symbol `_mfb_arena_alloc`) takes a byte `size` in `x0` and a power-
of-two `align` in `x1`, and returns a fallible result: [[src/target/shared/code/entry_and_arena.rs:lower_arena_alloc]] `x0` is `0` on success
with the aligned pointer in `x1`, or an error code in `x0` with `x1 = 0` on
failure. Its register contract is the standard runtime-helper one: **all
caller-saved integer registers (x0–x17) are clobbered**; callee-saved registers
(x19–x28) are preserved by its PCS frame. Callers must not hold a live value in
any caller-saved register across the call — spill to a stack slot instead. The
fast (first-fit) path makes no call, but the rare block-grow path calls
`arena_fill_random` to poison the freshly mapped block, so `arena_alloc` is **not**
a leaf — it carries a frame and saves the link register.

The algorithm:

1. **Validate alignment.** A zero `align`, or one that is not a power of two
   (`(align - 1) & align != 0`), returns `ErrInvalidArgument`.
2. **Normalize the request.** A request within `ARENA_MIN_CHUNK` (16) of
   `u64::MAX` is rejected as `ErrInvalidArgument` before normalization — the
   granule round-up would otherwise wrap it to a tiny size that allocates
   *small*, and no such request could ever be satisfied. A zero `size` becomes
   `1`; `size` is then rounded up to the 16-byte granule, and `align` is raised
   to at least 16. This keeps every chunk 16-aligned and 16-sized. No request
   can ever normalize to a value smaller than itself.
2a. **Quick-bin pop** (small requests: `align ≤ 16` and normalized
   `size ≤ 2048`; anything else skips to step 3). If `quickBin[size/16 - 1]`
   is non-empty, pop the bin head and return it — an exact-class O(1) hit, no
   walk and no splitting. This is sound because free and alloc normalize
   identically and every chunk the allocator ever hands out is 16-aligned, so
   any bin node satisfies any `align ≤ 16` request of its class.
2b. **Designated-victim bump.** On a bin miss, if the carve chunk holds at
   least `size` bytes, serve from `carvePtr` and advance it — an O(1) bump.
   Concentrating all small-miss carving in one chunk keeps parked bin
   inventory whole (splitting parked chunks per miss shaves them into
   sub-class fragments nothing ever requests).
2c. **Victim renewal.** When the carve chunk runs dry, its remnant parks on
   its exact-size bin (or joins the coalescing list if larger than 2048), and
   a new victim is acquired: the largest parked bin chunk that fits the
   request (a bounded top-down slot scan). If no bin fits, the walk (step 3)
   hands over a **whole** chunk — never a split — as the new victim; failing
   that, the flush retry (step 4) and finally a fresh block from the grow
   (step 5) supply it.
3. **First-fit walk.** Walk the address-ordered free-list for the first chunk
   where the request fits after alignment: `aligned = align_up(start, align)`
   and `aligned + size <= start + chunkSize`. A small request (per step 2a's
   bounds) takes the whole chunk as the new designated victim and bump-serves
   from it. A large request **splits** it — return `aligned`; the front
   padding (`aligned - start`, if > 0) and the tail remainder
   (`chunkEnd - (aligned + size)`, if > 0) each park on their exact-size bin
   when ≤ 2048 or relink into the list otherwise, so a split never leaves a
   sub-class fragment on the list. All sums are overflow-checked; an overflow
   skips the chunk.
4. **Flush-before-grow** (small requests only). If no chunk fits, do not map
   yet: drain every quick bin through the coalescing insert (restoring full
   adjacent-merge) and retry the walk once. On this path the bins are known to
   hold nothing of the request's class or larger (steps 2a–2c all missed), so
   the drain is cheap and coalescing adjacent parked chunks genuinely can
   produce a fit. A large request grows directly — draining a big parked-small
   inventory almost never coalesces past interleaved live objects into a
   large-enough run and can dominate whole workloads.
5. **Grow.** If the walk (and, for a small request, the flush retry) finds no
   fit, map a new block sized `max(4096, round_up(size + align + 32, 4096))`,
   write its header, link it at the head, and **carve the request directly
   from the fresh block**: one walk finds the block's address-ordered slot,
   then a small request takes the whole usable region as the new designated
   victim while a large request splits it as in step 3 — the fresh block is
   never whole-inserted and the list is never re-walked. Every sum in the
   sizing is overflow-checked (including the 32-byte header add); a wrapped
   sum reports `ErrOutOfMemory` rather than mapping an undersized block.

A failed mapping (the platform `mmap`/`VirtualAlloc` hook) reports
`ErrOutOfMemory`. Both `ErrInvalidArgument` and `ErrOutOfMemory` surface to
source as ordinary language-level errors (see the language spec §14.3.1).

## `arena_free(ptr, size)`

`arena_free` (symbol `_mfb_arena_free`) takes the chunk pointer in `x0` and its
byte `size` in `x1` and returns nothing; [[src/target/shared/code/entry_and_arena.rs:lower_arena_free]] like every runtime helper it
clobbers all caller-saved integer registers (it carries a frame, saves the link
register, and calls `arena_fill_random`). `size` is
normalized exactly as `arena_alloc` normalizes it (zero → 1, rounded up to 16),
so the freed extent matches the live chunk that was handed out. A chunk of
2048 bytes or less then **parks on its exact-size quick bin** — an O(1) head
push, no list walk (`quickBin[size/16 - 1]`; bin nodes reuse the `FreeNode`
overlay, so a later flush hands them straight to the coalescing insert). A
larger chunk (> 2048) instead **parks on its hashed large-block bin** —
`largeBin[(size >> 4) & 63]`, also an O(1) head push. Routing large
frees through the address-ordered `arena_insert_free` grew that list without
bound under heavy large-list churn (a 1000-element `List` frees ~40 KB per op),
so both the insert and every later first-fit walk went quadratic; the segregated
size bin keeps the general list short and makes a same-size large reuse O(1). A
large alloc scans its bin for an *exact*-size node before the first-fit walk;
a colliding different-size chunk is skipped (recovered when the bins drain
through the coalescing insert at flush-before-grow). Either
way the chunk is then entropy-scrubbed (see *Entropy Fill*) over
`[ptr+16, ptr+size)` — every payload byte past the 16-byte `FreeNode` overlay
just written, so the scrub can never destroy live free-list metadata. A
**repeated free of the same address is a no-op** on the coalescing path: the
insert walk detects a node already at `ptr` and returns without relinking, so a
double-free cannot corrupt the list into an overlapping structure. The
coalescing cases:

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
fresh arena memory is not implicitly zero, every allocation site must fully
initialize the bytes it later reads (the language's allocators already do).

The fill source is a **dedicated per-arena PCG64** at arena-state offsets 16/24,
separate from the `math::rand` stream at 88/96 and seeded independently at arena
init (`arena_fill_seed`): the main thread mixes OS entropy (`getentropy`) with the
arena address and start time (offset 40); each worker mixes a draw from the
parent's fill stream with its own arena address. [[src/target/shared/code/entry_and_arena.rs:lower_arena_fill_seed]] Its output is never observable —
filled bytes are always overwritten by a constructor before any read — so the
stream needs no reproducibility. `arena_fill_random(ptr, len)` streams PRNG words
(no syscall per fill); `arena_free` calls it after the coalescing insert (over
the freed payload past the FreeNode words — see `arena_free` above), and
`arena_alloc` calls it on a freshly mapped block's usable region before first use.

## Cleanup and Reclamation

An arena is reclaimed whole. `arena_destroy` (symbol `_mfb_arena_destroy`) walks
the block chain from `blockHead` through each `prevBlock`, [[src/target/shared/code/entry_and_arena.rs:lower_arena_destroy]] unmapping every block
with the platform `munmap`/`VirtualFree` hook, then clears **both list heads** —
`blockHead` and `freeListHead` — plus every quick-bin head and the
designated-victim words to `0`, leaving the arena fully inert (the heads would
otherwise dangle into the unmapped blocks).
It frees no individual values; all memory returns to the OS at once. The helper
is idempotent — a second call sees `blockHead == 0` and does nothing.

At process teardown, `_mfb_shutdown` reads the arena-state address from
`_mfb_rt_main_arena`, clears that global first [[src/target/shared/code/error_constants.rs:SHUTDOWN_SYMBOL]] (so a signal arriving mid-teardown
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
(kept as pointer graphs, `type_is_flat` is false). Builtins that could otherwise return
a borrow into an argument return an owned block instead (`collections::get`/`getOr`
materialize the element; `strings::replace`'s no-op path returns a fresh copy), so a
call result is always safe for the caller to own and free.

## See Also

* ./mfb spec threading — per-worker arenas and thread isolation
* ./mfb spec language memory-semantics — the source-level ownership model
* ./mfb spec architecture native — arena helpers in native codegen
