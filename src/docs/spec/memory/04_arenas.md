# Arenas

Dynamically constructed strings, records, unions, errors and collections use
an *arena*. Static strings and non-capturing function descriptors live in the
program image; runtime-managed thread control blocks have separate storage.
See `./mfb spec memory closures` and `./mfb spec threading control-block`.

An arena owns a chain of OS-mapped blocks plus a
small fixed *arena-state* control structure, and manages the mapped bytes with
three cooperating structures (the classic deferred-coalescing
design): **128 per-size-class quick bins** (exact chunk sizes 16…2048 — a freed
small chunk parks on its bin in O(1) and the next same-class allocation pops it
in O(1)), a **designated-victim carve chunk** (one active chunk that bump-serves
small bin misses, so misses never splinter parked inventory), and a per-arena
**address-ordered coalescing free-list** holding block remainders, plus **64
large-bin size classes** for freed chunks over 2048 bytes (a large request takes
any parked chunk it fits, splitting it). A gated **flush** sorts and coalesces
every parked chunk before the arena maps more. Quick-bin and designated-victim
hits and bin pushes are O(1); large-bin
searches are bounded. Free-list walks and coalescing flushes have the costs
specified below. The designated victim subsumes the
bump pointer. Freeing is **internal reuse only**: a freed chunk goes
back to the allocator for the next allocation but is never returned to the OS;
mapped blocks are unmapped only by the bulk `arena_destroy` when the owning
package instance shuts down.

Each package instance owns a distinct arena. The main package's arena-state lives
on the entry stack and is pinned in `x19` (`ARENA_STATE_REGISTER`) for the life of
the program; its address is also published to the writable global
`_mfb_rt_main_arena` so signal handlers and shutdown code can reach it without
relying on the pinned register. [[src/codegen/error/constants/error_constants.rs:MAIN_ARENA_GLOBAL_SYMBOL]] Each worker package instance owns a separate
arena, referenced from its thread control block, so worker threads allocate and
reclaim independently of the main thread (see `./mfb spec threading`).

## Arena-State Layout

The arena-state structure is `ARENA_STATE_SIZE` = **3776 bytes**: [[src/codegen/error/constants/error_constants.rs:ARENA_STATE_SIZE]]

```text
ArenaState (at x19)
  +0    U64  blockHead        ; pointer to the current (most-recent) block, 0 if none
  +8    U64  workerThread     ; this thread's control block, 0 on the main thread
  +16   U64  fillRngLo        ; dedicated memory-fill PCG64 state, low 64 bits
  +24   U64  fillRngHi        ; dedicated memory-fill PCG64 state, high 64 bits
  +32   U64  exitStatus       ; pending exit/result code used during teardown
  +40   U64  arenaStartTime   ; arena init time in ns (diagnostics + fill-seed mix)
  +48   U64  freeListHead     ; lowest-address free chunk, 0 when the list is empty
  +56   U64  moneyRoundingMode ; per-thread Money arithmetic rounding: 0 = Commercial
                              ; (half-away, default), 1 = Banker (half-even); inherited
                              ; from the parent at thread spawn (money::setRounding)
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
  +1168 U64  largeBin[64]     ; large-block size-class heads, chunks > 2048: 4 classes
                              ; per power of two from 2 KiB, the last holding 112 MiB
                              ; and up (see arena_free); 0 = empty
  +1680 U128 v128Slots[127]   ; per-thread v128 scalarization region (2032 bytes);
                              ; reserved on every target, addressed only by rv64
                              ; codegen; kept low so its offset fits rv64's 12-bit addi
  +3712 U64  flagRhs          ; rv64 flag-emulation compare rhs snapshot (16 bytes,
                              ; carved from the v128 region); addressed only by rv64 codegen
  +3728 U64  currentError     ; in-flight owned Error block base for trap adoption
                              ; (0 when none); addressed by a computed register on rv64
  +3736 U64  stdinLocalBuf    ; stdin broadcast: per-thread 4 KiB copy buffer
                              ; (NULL until first stdin read)
  +3744 U64  stdinLocalFilled ; valid bytes in stdinLocalBuf
  +3752 U64  stdinLocalPos    ; next unread byte in stdinLocalBuf (lock-free fast path)
  +3760 U64  stdinSubscriber  ; pointer to this thread's stdin broadcast-log registry
                              ; entry (NULL when not subscribed)
  +3768 I64  largeFlushCredit ; large bytes freed since the last flush, minus 256 per
                              ; chunk that flush sorted; a big request flushes only
                              ; while it is positive; addressed by a computed register
```

`workerThread` is the back-pointer to this thread's control block, published by
the worker trampoline once its arena and control-block registers are both live.
Nothing writes it on the program's main thread, so the whole-`ARENA_STATE_SIZE`
zero-init leaves it `0` there — which makes it the reliable "am I a worker, and if
so which one" test on every thread, reachable through the pinned arena register
without a thread-local lookup. `os::sleep` reads it to choose between a plain delay
and the cancellation-aware worker wait (`./mfb spec threading`); the pinned
current-thread register cannot serve that role, because the entry stub reuses it as
scratch on the main thread.

`blockHead` anchors the unmap walk; `freeListHead`, the 128 `quickBin` heads,
the 64 `largeBin` heads, and the `carvePtr`/`carveSize` designated victim anchor
allocation (see the small- and large-request fast paths below), and
`largeFlushCredit` gates the flush a large request may run before it maps.
[[src/codegen/error/constants/error_constants.rs:ARENA_LARGE_FLUSH_CREDIT_OFFSET]] The
cleanup-failure triple records diagnostics if reclamation of a value fails during
teardown, and the two RNG words at 88/96 give each arena (hence each thread) an
independent `math::rand` stream seeded at startup. The `fillRngLo`/`fillRngHi`
words at 16/24 hold a **separate** dedicated memory-fill PCG64 stream (see Entropy
Fill below), seeded independently so it never perturbs the reproducible
`math::rand` sequence. The entry shim's RNG scratch begins at `ARENA_STATE_SIZE`; globals follow it,
and an argument-list entry reserves argc/argv materialization slots above the
globals (see `./mfb spec memory program-startup`). Because the main arena-state lives on the entry
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
a 32-byte (`ARENA_BLOCK_HEADER_SIZE`) header: [[src/codegen/error/constants/error_constants.rs:ARENA_BLOCK_HEADER_SIZE]]

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
**4096 bytes**. [[src/codegen/error/constants/error_constants.rs:ARENA_DEFAULT_BLOCK_SIZE]] Allocation does not read `bumpOffset` — it is written `0` at map
time and kept as a reserved field; the bins, carve chunk and free-list drive placement.

## `arena_alloc(size, align)`

`arena_alloc` (symbol `_mfb_arena_alloc`) takes a byte `size` in `x0` and a power-
of-two `align` in `x1`, and returns a fallible result: [[src/codegen/memory/arena/arena.rs:lower_arena_alloc]] `x0` is `0` on success
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
2d. **Large-bin size class** (large requests: `align ≤ 16` and normalized
   `size > 2048`). Look at the first 16 nodes of the request's size class and
   take an exact-size node at once, else the smallest node that fits; failing
   that, take the head of the lowest non-empty *higher* class — every chunk
   there is larger than any size in this one, so it fits without a look at its
   size. The chunk is split: the request is returned from its start and the
   remainder parks on its quick bin (≤ 2048) or its size class. This is O(1)
   per request (at most 16 nodes and 63 heads), and a freed block serves any
   later request it fits, not only one of its exact size. A miss falls through
   to step 3. [[src/codegen/memory/arena/arena.rs:emit_large_bin_slot]]
   [[src/codegen/error/constants/error_constants.rs:ARENA_LARGE_BIN_SCAN_LIMIT]]
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
4. **Flush-before-grow.** If no chunk fits, do not map yet.
   - A small request first takes the head of the lowest non-empty large-bin
     class as its new designated victim — at most 64 KiB of it, the rest
     re-parked on its class so one small request does not lock a whole large
     block away. [[src/codegen/error/constants/error_constants.rs:ARENA_LARGE_DV_MAX]]
     With the large bins empty, it flushes when the address-ordered list is
     non-empty and retries from step 2c once. On this path the bins are known
     to hold nothing of the request's class or larger, so coalescing adjacent
     parked chunks genuinely can produce a fit; with the list empty, the only
     free chunks are small ones a pure-small workload is recycling, and the
     flush would only destroy that reuse.
   - A large request flushes and retries from step 2d once, but only while
     `largeFlushCredit` is positive: every large free adds its bytes to it, and
     every flush resets it to minus 256 bytes per chunk the flush sorted. A
     flush is O(M log M) in the M chunks it sorts, and the credit must recover from `-256 * M` to a positive value before
     another large-request flush can run. [[src/codegen/error/constants/error_constants.rs:ARENA_LARGE_FLUSH_BYTES_PER_CHUNK]]
     Without the flush, a program whose large sizes vary splits its free
     memory into pieces too small for its next request and maps without bound.

   The flush gathers the list and every quick-bin and large-bin chain, merge
   sorts them by address, merges physically adjacent chunks in one pass, and
   re-parks each result on its quick bin (≤ 2048) or its large-bin size class;
   the list is left empty. [[src/codegen/memory/arena/arena.rs:lower_arena_flush_coalesce]]
5. **Grow.** If nothing fits after that, map a new block sized
   `max(4096, round_up(size + align + 32 + slack, 4096))`, write its header,
   link it at the head, and **carve the request directly from the fresh
   block**: one walk finds the block's address-ordered slot, then a small
   request takes the whole usable region as the new designated victim while a
   large request splits it as in step 3 — the fresh block is never
   whole-inserted and the list is never re-walked. `slack` is 0 except for a
   large request whose own size class, or the class below it, holds a parked
   chunk: that request is climbing past something it freed (a scene or copy a
   little longer each frame), and since mapped blocks never merge, an
   exact-size block would make every step of the climb map afresh. It gets
   `size / 4` of slack, which parks in the large bins and re-merges with the
   request's own chunk when that is freed, so a climb maps a geometric series
   instead of the sum of its steps. Every sum in the sizing is overflow-checked
   (including the 32-byte header add and the slack); a wrapped sum reports
   `ErrOutOfMemory` rather than mapping an undersized block.

A failed mapping (the platform `mmap`/`VirtualAlloc` hook) reports
`ErrOutOfMemory`. Both `ErrInvalidArgument` and `ErrOutOfMemory` surface to
source as ordinary language-level errors (see the language spec §14.3.1).

## `arena_free(ptr, size)`

`arena_free` (symbol `_mfb_arena_free`) takes the chunk pointer in `x0` and its
byte `size` in `x1` and returns nothing; [[src/codegen/memory/arena/arena.rs:lower_arena_free]] like every runtime helper it
clobbers all caller-saved integer registers (it carries a frame, saves the link
register, and calls `arena_fill_random`). `size` is
normalized exactly as `arena_alloc` normalizes it (zero → 1, rounded up to 16),
so the freed extent matches the live chunk that was handed out. A chunk of
2048 bytes or less then **parks on its exact-size quick bin** — an O(1) head
push, no list walk (`quickBin[size/16 - 1]`; bin nodes reuse the `FreeNode`
overlay, so a later flush hands them straight to the coalescing insert). A
larger chunk (> 2048) instead **parks on its large-bin size class**, also an
O(1) head push, and adds its bytes to `largeFlushCredit`. The 64 classes split
each power of two from 2 KiB into four: for `2^k ≤ size < 2^(k+1)` the class is
`(k - 11) * 4 + ((size >> (k - 2)) & 3)`, and every size from 112 MiB up shares
the last. [[src/codegen/memory/arena/arena.rs:emit_large_bin_slot]] In either bin path the chunk is then entropy-scrubbed (see *Entropy Fill*) over
`[ptr+16, ptr+size)` — every payload byte past the 16-byte `FreeNode` overlay
just written, so the scrub can never destroy live free-list metadata. An
**immediate repeated free of the same address is a no-op**: the push finds the
chunk already at its bin's head and returns without relinking it (a self-cycle
would hand it to the next two allocations). `arena_free` never coalesces; merging
happens in the flush (step 4 above) and in `arena_insert_free`, which the rare
retirement of a designated victim larger than 2048 bytes uses. Its walk also
detects a node already at `ptr` and returns without relinking. The insert's
coalescing cases:

- `prev.start + prev.size == ptr` → extend `prev` over the chunk;
- `ptr + size == next.start` → absorb `next`;
- both → merge all three into one chunk;
- neither → link a fresh `{next, size}` node at `ptr`.

Adjacency is pure arithmetic on `start`/`size`, so no boundary tags are read.
Chunks in different blocks are never contiguous (the 32-byte header always
separates blocks), so a merge never spans a block. `arena_free` **never unmaps**
— it only relinks; memory returns to the OS solely at `arena_destroy`. The `size`
is supplied by the compiler's type-directed drop glue; there is no user-level
free operation. This ABI requires the pointer and rounded extent to match the
allocation and does not provide general double-free detection.

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
parent's fill stream with its own arena address. [[src/codegen/builtins/math/gen_rng_pcg64.rs:lower_arena_fill_seed]] Its output is never observable —
filled bytes are always overwritten by a constructor before any read — so the
stream needs no reproducibility. `arena_fill_random(ptr, len)` streams PRNG words
(no syscall per fill); `arena_free` calls it after parking the chunk on its bin (over
the freed payload past the FreeNode words — see `arena_free` above), and
`arena_alloc` calls it on a freshly mapped block's usable region before first use.
It writes four words per loop iteration by PCG64 jump-ahead — word *j* is the
carried state advanced *j* steps in one 128-bit multiply-add, so the four
multiplies are independent — and single-steps the remaining `count mod 4` words;
the words and the state left at offsets 16/24 are exactly those of the one-step
stream. [[src/codegen/builtins/math/gen_rng_pcg64.rs:lower_arena_fill_random]]
A `--debug` build counts both kinds of fill for each arena in the debug report:
`arena.<n>.fill_grow_calls`/`.fill_grow_bytes` for fresh blocks and
`arena.<n>.fill_free_calls`/`.fill_free_bytes` for freed chunks (see
`tooling/09_debug-report.md`).

## Cleanup and Reclamation

An arena is reclaimed whole. `arena_destroy` (symbol `_mfb_arena_destroy`) walks
the block chain from `blockHead` through each `prevBlock`, [[src/codegen/memory/arena/arena.rs:lower_arena_destroy]] unmapping every block
with the platform `munmap`/`VirtualFree` hook, then clears **both list heads** —
`blockHead` and `freeListHead` — and every word from `ARENA_QUICK_BIN_BASE_OFFSET` through
`ARENA_STATE_SIZE` to `0`. This includes quick/large bins, the designated
victim, output/input buffer state, vector scratch, current error, and flush
credit. No allocator head retains an address into an unmapped block.
It frees no individual values; all memory returns to the OS at once. The helper
is idempotent — a second call sees `blockHead == 0` and does nothing.

At process teardown, `_mfb_shutdown` reads the arena-state address from
`_mfb_rt_main_arena`, clears that global first [[src/codegen/error/constants/error_constants.rs:SHUTDOWN_SYMBOL]] (so a signal arriving mid-teardown
re-enters as a no-op), restores the terminal if TUI mode was active, and then
calls `arena_destroy` on the main arena — except in a program that can still have a
live worker (one embedding any `thread.` runtime call), where the free is skipped
on every platform and the OS reclaims the blocks at process exit instead
(`./mfb spec threading os-integration`). A worker arena's chunks are unmapped by
the worker as the last step of its run; its arena-state block (a heap block) is
freed with the thread's control block. Nothing another thread reads points into a worker arena:
every value that crosses is copied into the control block's own block arena and
from there into the receiver's arena (`./mfb spec threading control-block`).

## Scope-Drop Frees

Beyond the bulk `arena_destroy`, individual owned values are freed deterministically
at **scope-drop**. A flat, pointer-free value is
freed by a single `arena_free(ptr, size)` of its block, the size recomputed from the
static type at the drop point (`emit_inlined_block_size_from_ptr_slot`). [[src/codegen/collection/layout/builder_collection_layout.rs:emit_inlined_block_size_from_ptr_slot]]
A value of a recursive type (one whose declaration reaches itself) is a graph of
separately allocated blocks. Its owner frees it through the module's one drop walker,
`_mfb_rt_graph_drop`, which frees one block at a time and keeps the pending edges on an
arena work stack rather than the native stack, so a graph of any depth is freed without
overflowing it. [[src/codegen/memory/arena/graph_drop.rs:lower_graph_drop_walker]]
Both walkers allocate that stack at their first push, and `_mfb_rt_graph_stack_grow`
doubles it from there, so walking a value with no cyclic edge to push (a leaf) allocates
and frees no stack. [[src/codegen/memory/arena/graph_copy.rs:emit_graph_copy_push]]

Soundness rests on the heap being an **ownership tree** — every owned local owns an
independent block, so freeing each exactly once at scope exit cannot double-free.
The source-level ownership/move/copy model that guarantees this is canonical in
`./mfb spec language memory-semantics`. The codegen enforces it by **copy-insertion**:
every site that hands a value to a longer-lived owner (a `LET`/`MUT` bind, a global
store, an assignment, a closure capture, a `RETURN`) deep-copies (`copy_flat_block`)
when the source is an *aliasing source* (a `Local`, `Global`, `Capture`,
field/`MemberAccess` read, `UnionExtract`, or `Result` payload — all of which yield a
pointer into an existing block) or a *static* `String` constant (which lives in rodata,
not the arena). Record/union/collection construction, collection inserts, and `WITH`
already byte-copy their flat payloads inline, so a flat payload introduces no new alias. A
payload of a recursive type (one whose declaration reaches itself) is a graph of separate
blocks that a byte copy would share, so those stores give an aliasing source its own graph
first — unless the store is the source's last read, which moves it — and a collection a
builtin builds out of another collection's elements copies those elements' graphs.

A free is emitted at **every** scope exit — the normal end-of-block drain,
`EXIT`/`CONTINUE` (only back to the loop's entry depth), `RETURN`, and `TRAP`
routing — reusing the resource cleanup stack (`ActiveCleanup::OwnedValue`). [[src/codegen/engine/builder/mod.rs:OwnedValueCleanup]] A value
that is **moved out** suppresses its free: a returned named local is moved (not
copied) and its cleanup deactivated; `thread::transfer` already deep-copies into the
receiver arena and deactivates the sender's cleanup. A recursive value handed to a new
owner at its source's last read is moved too: the source's slot, or the record field it
was read from, is zeroed, and every drop skips a null pointer. A store that byte-copies a
recursive record or union — into a union, or as a collection payload — frees only the
block it copied at the end of the statement, and leaves that block's children to the new
owner. Binding slots are
zero-initialized before a (possibly trapping) initializer runs and the free is
null-guarded, so an initializer that traps before storing frees nothing.

A **resource** is scope-dropped, but on its own terms: drop and close do different
jobs. *Close* releases the OS handle and sets the closed flag; *drop* reclaims the
memory — the drop path `arena_free`s the resource's output buffer,
read buffer, and `STATE` payload, nulling each pointer word as it goes.
The **owning binding's** drop also frees the 96-byte record for `fs.File`,
`tcp.Socket`, `tcp.Listener`, `udp.Socket`, `tls.Socket`, `tls.Listener`,
`audio.AudioInput`, `audio.AudioOutput`, and the parent `Thread` view. It frees
the record last, after close and block reclamation, and zeroes the slot. Other
kinds retain the record as a closed tombstone until arena teardown. The
owner, not `close`, frees it, because a `close` through a `RES` parameter leaves the
owner still reading the closed flag, and every non-owning holder lives in the owner's
scope. An explicit `close` on the owner therefore keeps the drop registered, and the
drop's re-close is the defined no-op. [[src/codegen/resource/cleanup/builder_resource_cleanup.rs:resource_record_freed_at_drop]]
Drop skips a record whose `RESOURCE_MOVED_BIT` is set: `thread::transfer`
copied the `STATE` pointer into the receiver's record, so freeing it here would hand
another thread a dangling payload. (`./mfb spec language resource-management` specifies the
close/drop split.) [[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_block_reclaim]]

A resource that **floats** into an owner collection (`./mfb spec language
resource-management` §15.6) has no binding of its own left to drop, so its memory is
reclaimed by that collection's owned-list drain instead. The drain always frees the
16-byte `{record, next}` node it allocated for the element, which nothing but the drain
can name. Everything the node POINTS at — the element's `STATE` block, its per-`File`
I/O buffers, its record for the record-freeing kinds above, a union element's
`{tag, record-ptr}` box, and the collection block itself — is freed only for a
collection proved to be their one owner: every store into it is a collection
literal, a self-receiving `collections::` mutator, or a call to a builtin body audited to
return a block nothing else holds (`collections::take`/`drop` over the native slice); it
is never named outside a call's argument position; no call keeps its block — it is never
a `collections::` member's non-receiver argument (`collections::append(out, piece)`
stores `piece`'s block inside `out`), nor an argument a callee's body keeps by the same
rules, a recursive callee counting as keeping; no `collections::get`/`getOr` (nor a
`*::poll`) reads an element back out of it; and every element that floats into it is a
fresh producer record that does not escape. A collection that fails any of those keeps a close-only drain, which leaks
those blocks rather than risking a double free.
[[src/codegen/resource/cleanup/record_ownership.rs:owning_collections]]
[[src/codegen/cleanup/owned/builder_owned_cleanup.rs:emit_owned_list_drain]]

Raw thread-helper results belong to the thread runtime's block arena and
must not be freed through the caller's arena. However, `thread::waitFor`,
`receive`/`read`, and `acceptResource`/`readResource` copy their returned values
into the calling thread's arena at the call site: those copies are owned and
dropped there. A runtime-managed value that has not been copied remains excluded
from ordinary scope-drop frees.
[[src/codegen/engine/value/builder_values.rs:value_is_runtime_managed]]
[[src/codegen/engine/value/builder_values.rs:runtime_call_result_is_copied_at_call_site]]

**Pointer graphs whose type holds a resource** are also excluded from ordinary
owned-value frees. A recursive or cycle-reaching value is not `type_is_memcpy_copyable`, so the
flat `arena_free` path never claims it. Instead its owner frees it through
`_mfb_rt_graph_drop` when the type has a graph-drop kind, which every resource-free type in
or reaching a cycle does. A resource-bearing type never gets a kind: it is move-only and
closed by its own op. [[src/codegen/engine/value/builder_values.rs:owns_graph]] [[src/codegen/memory/arena/graph_drop.rs:graph_drop_kind]] Builtins that could otherwise return
an interior pointer into an argument return an owned block instead (`collections::get`/`getOr`
materialize the element; `strings::replace`'s no-op path returns a fresh copy), so a
call result is always safe for the caller to own and free.

## Measuring an Arena

A program built with `mfb build --debug` counts the events described on this page
for every arena: blocks mapped and unmapped, allocation and free calls and bytes,
which `arena_alloc` path served each request, double-free skips, coalescing
flushes and inserts, and live and peak live bytes. It prints them in the `arena`
section of the debug report at exit. A normal build contains none of the counting
code. [[src/codegen/debug/arena.rs:ArenaFeature]]

## See Also

* ./mfb spec tooling debug-report — the `arena` report section and its counters

* ./mfb spec threading — per-worker arenas and thread isolation
* ./mfb spec language memory-semantics — the source-level ownership model
* ./mfb spec architecture native — arena helpers in native codegen
