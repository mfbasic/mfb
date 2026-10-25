# Arenas

Dynamically constructed strings, records, unions, errors and collections use
an *arena*. Static strings and non-capturing function descriptors live in the
program image; runtime-managed thread control blocks have separate storage.
See `./mfb spec memory closures` and `./mfb spec threading control-block`.

An arena owns one **slice** of the address space the program reserves at startup
(`./mfb spec memory program-startup`, *The Arena Reservation*) plus a small fixed
*arena-state* control structure. It commits memory from its slice as it needs it,
in three regions (plan-183-C):

- **small** — requests of 2048 bytes or less: 64 KiB pages, each serving one of
  128 size classes (16, 32, …, 2048), with each page's slot bitmap in a side
  descriptor;
- **medium** — larger requests below 32 MiB: chunks carved upward from the
  region's base, free runs on 64 size-class lists;
- **huge** — requests of 32 MiB or more, and any request aligned above 16 bytes:
  16 KiB-page chunks carved downward from the slice top, free runs on one list.

No allocator metadata lives in free memory: run and page bookkeeping is in side
tables of the slice and in the arena state, so **every byte of an arena that is not
in use by the program is zero** — a freed chunk is zeroed whole, and committed
memory the arena never handed out is the OS's zero pages. A medium or huge free
merges with its free neighbours at once. Freed memory goes back to the allocator;
`arena_destroy` decommits the whole slice when the owning package instance shuts
down.

Each package instance owns a distinct arena. The main package's arena-state lives
on the entry stack and is pinned in `x19` (`ARENA_STATE_REGISTER`) for the life of
the program; its address is also published to the writable global
`_mfb_rt_main_arena` so signal handlers and shutdown code can reach it without
relying on the pinned register. [[src/codegen/error/constants/error_constants.rs:MAIN_ARENA_GLOBAL_SYMBOL]] Each worker package instance owns a separate
arena, referenced from its thread control block, so worker threads allocate and
reclaim independently of the main thread (see `./mfb spec threading`).

## Arena-State Layout

The arena-state structure is `ARENA_STATE_SIZE` = **3936 bytes**: [[src/codegen/error/constants/error_constants.rs:ARENA_STATE_SIZE]]

```text
ArenaState (at x19)
  +0    U64  reserved0        ; reserved: zero, read by nothing
  +8    U64  workerThread     ; this thread's control block, 0 on the main thread
  +16   U64  sliceBase        ; first byte of the arena's slice, 0 for none
  +24   U64  reserved24       ; reserved: zero, read by nothing
  +32   U64  exitStatus       ; pending exit/result code used during teardown
  +40   U64  arenaStartTime   ; arena init time in ns (diagnostics)
  +48   U64  reserved48       ; reserved: zero, read by nothing
  +56   U64  moneyRoundingMode ; per-thread Money arithmetic rounding: 0 = Commercial
                              ; (half-away, default), 1 = Banker (half-even); inherited
                              ; from the parent at thread spawn (money::setRounding)
  +64   U64  cleanupFailCount ; count of cleanup errors (audit)
  +72   U64  cleanupFailCode  ; last cleanup failure error code
  +80   U64  cleanupFailMsg   ; pointer to last cleanup failure message
  +88   U64  rngStateLo       ; PCG64 RNG state, low 64 bits
  +96   U64  rngStateHi       ; PCG64 RNG state, high 64 bits
  +104  U64  smallHead[128]   ; per-class page lists: index + 1 of the first small page
                              ; of class size/16 - 1 with a free slot; 0 = none
  +1128 U64  hugeHead         ; huge free-run list: a run-node index, 0 = empty
  +1136 U64  nodeFree         ; free list of the run-node pool, 0 = empty
  +1144 U64  outPtr           ; opt-in stdout buffer base, NULL until first use
  +1152 U64  outFilled        ; pending bytes held in the stdout buffer
  +1160 U64  outEnabled       ; io::setBuffered flag (0 = unbuffered default)
  +1168 U64  mediumHead[64]   ; medium free-run lists by size class (4 classes per power
                              ; of two from 2 KiB, the last holding 112 MiB and up);
                              ; run-node indices, 0 = empty
  +1680 U64  stackLimit       ; lowest sp a function body may start at (bug-789); 0 =
                              ; unchecked; low so one load reads it on every target
  +1688 U64  stackLimitTmp    ; scratch while a thread computes stackLimit
  +1696 U128 v128Slots[126]   ; per-thread v128 scalarization region (2016 bytes);
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
  +3768 U64  allocScalars[21] ; the allocator's slice geometry and frontiers (below);
                              ; addressed off a computed register on every target
```

The scalar block (`ARENA_ALLOC_SCALARS_OFFSET`, offsets `ARENA_SC_*` within it):
[[src/codegen/error/constants/error_constants.rs:ARENA_ALLOC_SCALARS_OFFSET]]

```text
  +0    smallPages    address of small page 0; 0 until the slice is laid out
  +8    smallEnd      end of the small region = start of the run tags
  +16   smallLimit    how many small pages fit
  +24   smallNext     next never-used small page index
  +32   smallEmpty    empty-page list, page index + 1 (0 = none)
  +40   smallCommit   end of the committed small pages
  +48   tableCommit   end of the committed page descriptors
  +56   tags          the run-tag array
  +64   mediumBase    first byte of medium data (tag block 0)
  +72   mediumFront   medium frontier: every byte below it was handed out once
  +80   mediumCommit  end of the committed medium data
  +88   hugeFront     huge frontier: everything from it to the slice top is huge
  +96   hugeCommit    start of the committed huge data
  +104  tagsLow       end of the tag records committed for medium
  +112  tagsHigh      start of the tag records committed for huge
  +120  nodes         the run-node pool
  +128  nodeNext      next never-used node index (index 0 is never used)
  +136  nodeLimit     how many nodes the pool holds
  +144  nodeCommit    end of the committed node pool
  +152  sliceTop      the slice's last byte + 1
  +160  committed     bytes this arena has committed and not decommitted
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

`sliceBase` is set where the arena is created: slice 0 by the startup reservation
for the main arena, slice 1 for a canvas program's graphics arena, and the two
slices `thread::start` takes for a worker and its control block. The cleanup-failure
triple records diagnostics if reclamation of a value fails during teardown, and the
two RNG words at 88/96 give each arena (hence each thread) an independent
`math::rand` stream seeded at startup. The words at 0, 24 and 48 are **reserved**:
every arena state is zero-initialized and nothing reads or writes them.
[[src/codegen/error/constants/error_constants.rs:ARENA_SLICE_BASE_OFFSET]] The entry
shim's RNG scratch begins at `ARENA_STATE_SIZE`; globals follow it, and an
argument-list entry reserves argc/argv materialization slots above the globals (see
`./mfb spec memory program-startup`). Because the main arena-state lives on the entry
stack (not zero-filled), the entry shim zeroes the whole `ARENA_STATE_SIZE`
range with a loop before the first allocation; the thread-spawn path zeroes a
worker's freshly allocated arena state with the same size-derived loop, so the
two initializers can never fall out of lockstep when the state grows.

## Slice Layout

An arena's slice is `[base, base + S)`, `S` the reservation's slice size (at least
256 MiB, a multiple of 2 MiB). The arena lays it out at its first allocation
(`_mfb_arena_geometry`), so an arena that never allocates touches none of it:
[[src/codegen/memory/arena/arena/layout.rs:lower_arena_geometry]]

```text
| small: S/8                    | tags | nodes | medium → grows up   ...   grows down ← huge |
| descriptors | 64 KiB pages    |  side        |                                            |
```

- **Small region**, the first `S/8` bytes: one 576-byte descriptor for every page
  the region could hold, then the pages from the next 64 KiB boundary. A
  descriptor is `{class, slots, free, hint, next, prev}` (class = size/16, slots =
  65536 / size, free = free slots, hint = the first bitmap word that may have a
  free slot, next/prev = the page's class-list links) and a 512-byte slot bitmap at
  +64 whose bit is 1 while the slot is handed out. Bits past the last slot are set.
- **Run tags**: one 16-byte record `{start, end}` per 2 KiB block of the rest of
  the slice, indexed by `(address - mediumBase) >> 11`. `start` describes what
  begins in the block — an allocated chunk, `size | (address >> 4 & 127) << 48 |
  1` (bits 55–63 zero; bit 55 is reserved for a huge chunk's growth headroom, and
  every reader masks bits 55 and up away), or a free run, `node << 2 | 2` — and `end` names the free run whose last byte
  lies in the block. Every medium and huge chunk and free run is longer than 2 KiB
  (a remainder shorter than the region's minimum run stays in the chunk it was cut
  from), so at most one thing starts, and at most one free run ends, in any block.
- **Run nodes**: half as many 32-byte nodes `{start, size, next, prev}` as there
  are blocks, the most free runs the span can hold.
- **Medium** data begins at the next 4 MiB boundary and grows up; **huge** data
  grows down from the slice top. Each region commits on demand — data in 4 MiB
  steps, side structures in 64 KiB granules (`_mfb_arena_commit`) — and a region
  never commits into the other's committed range.

## `arena_alloc(size, align)`

`arena_alloc` (symbol `_mfb_arena_alloc`) takes a byte `size` in `x0` and a power-
of-two `align` in `x1`, and returns a fallible result: [[src/codegen/memory/arena/arena/alloc.rs:lower_arena_alloc]] `x0` is `0` on success
with the aligned pointer in `x1`, or an error code in `x0` with `x1 = 0` on
failure. Its register contract is the standard runtime-helper one: **all
caller-saved integer registers (x0–x17) are clobbered**; callee-saved registers
(x19–x28) are preserved by its PCS frame. Callers must not hold a live value in
any caller-saved register across the call — spill to a stack slot instead. The
small path makes no call; a new page, a medium or huge request and the first
allocation call helpers, so `arena_alloc` carries a frame and saves the link
register.

1. **Validate.** A zero `align`, one that is not a power of two, or one above
   16 KiB returns `ErrInvalidArgument`. So does a `size` within 16 of `u64::MAX`,
   which the round-up would otherwise wrap to a small size.
2. **Normalize.** A zero `size` becomes 1; `size` is rounded up to 16.
3. **Lay the slice out** on the arena's first allocation; an arena with no slice
   reports `ErrOutOfMemory`.
4. **Small** (`size ≤ 2048`, `align ≤ 16`): the first page on class `size/16 - 1`'s
   list gives its lowest free slot — scan the bitmap from the hint for a word that
   is not all ones, take its lowest clear bit — and leaves the list when its last
   slot goes. An empty list first gets a page (`_mfb_arena_small_page`): from the
   empty-page list, else the next never-used page, committing its descriptor and
   page as needed. [[src/codegen/memory/arena/arena/small.rs:emit_small_alloc]]
5. **Medium** (`2048 < size < 32 MiB`, `align ≤ 16`): the best fit among the first
   16 runs of the size's class (an exact size at once), else the head of the next
   non-empty higher class, every run of which is larger; the run's low end becomes
   the chunk and the rest stays a run (a rest under 2064 bytes stays in the chunk).
   With no run, the chunk is carved at `mediumFront`, committing data and tag records
   as needed; it may not reach `hugeFront`. [[src/codegen/memory/arena/arena/span.rs:lower_arena_span_alloc]]
6. **Huge** (`size ≥ 32 MiB`, or `align > 16`): `size` is rounded up to 16 KiB;
   the best fit among the first 64 runs of the huge list, its low end taken (a rest
   under 16 KiB stays in the chunk), else a chunk carved down from `hugeFront`,
   committing as needed; it may not reach `mediumFront`. Huge chunks are 16 KiB
   aligned, which is how an alignment above 16 is honoured.
7. The chunk's start record gets its tag. Failure anywhere — the small region full,
   the medium and huge regions meeting, a refused commit, the run-node pool
   exhausted — reports `ErrOutOfMemory`.

## Process Memory Cap

The arena memory cap (`./mfb spec tooling project-manifest`, *Arena memory cap*)
bounds the bytes every arena of the process has committed — the main arena and
every worker's, counted together, because each thread has its own arena and a
per-arena cap would let N threads commit N times the cap. Three data objects hold it,
none of them in the arena state:

| symbol | what |
| --- | --- |
| `_mfb_rt_arena_map_limit` | u64 cap in bytes, stored by the startup check (`./mfb spec memory program-startup`, *The Arena Memory Cap Check*); `0` means no enforcement |
| `_mfb_rt_arena_committed_bytes` | u64 bytes the arenas have committed and not decommitted |
| `_mfb_rt_arena_map_lock` | 64-byte mutex guarding the counter, statically initialized (`PTHREAD_MUTEX_INITIALIZER`; an SRWLOCK on Windows) |

[[src/codegen/runtime/arena_cap/accounting.rs:ARENA_COMMITTED_BYTES_SYMBOL]]
[[src/codegen/runtime/arena_cap/accounting.rs:ARENA_MAP_LOCK_SYMBOL]]

- **Charge before a commit** (`_mfb_arena_commit`). The helper reads the limit; if it
  is 0 it commits without touching the counter. Otherwise, under the lock, it adds
  the commit's bytes — data steps and side-structure granules alike. A sum that
  wraps, or that is above the limit, is not stored: the lock is released and the
  request fails with `ErrOutOfMemory` without a commit. Charging under the lock makes
  the check and the add one step, so two threads cannot both pass a check made
  before either adds. [[src/codegen/runtime/arena_cap/accounting.rs:emit_commit_charge]]
- **Refund a refused commit.** When the OS refuses the commit after a charge, the
  helper takes the lock again and subtracts it before reporting `ErrOutOfMemory`.
  [[src/codegen/runtime/arena_cap/accounting.rs:emit_commit_refund]]
- **Refund on decommit.** `arena_destroy` subtracts the arena's `committed` word,
  when the limit is nonzero, as it decommits the slice. A finished worker's arena
  therefore gives its bytes back, and a program that churns through workers is capped
  on what its arenas hold, not on the total they ever took.
  [[src/codegen/runtime/arena_cap/accounting.rs:emit_decommit_refund]]
- **The fast path is untouched.** Allocation and free never take the lock; only a
  commit or a decommit does.
- **Turning accounting off.** Storing 0 into the limit stops all charging and
  refunding. `_mfb_shutdown` does it first (so a signal handler never waits on a lock
  its own thread holds), and the POSIX `process::spawn`/`process::shell` fork child
  does it before it allocates (so it never waits on a lock another parent thread
  held at `fork()`; the child's copy of that lock would stay locked forever).
  [[src/codegen/runtime/arena_cap/accounting.rs:emit_disable_accounting]]
- **Not counted:** the `--debug` perf region and the `--debug` arena registry,
  which map memory through their own platform hook, and every mapping that is not
  an arena's (thread stacks, audio and GPU buffers, native `LINK` libraries' own
  memory). The `--debug` counters `committed_bytes`/`committed_peak_bytes` are
  separate and per arena.

With `maxArenaMemoryDisable: true` none of this is emitted: no data objects, no
lock calls, no checks. Both `ErrInvalidArgument` and `ErrOutOfMemory` surface to
source as ordinary language-level errors (see the language spec §14.3.1).

## `arena_free(ptr, size)`

`arena_free` (symbol `_mfb_arena_free`) takes the chunk pointer in `x0` and its
byte `size` in `x1` and returns nothing; [[src/codegen/memory/arena/arena/free.rs:lower_arena_free]] like every runtime helper it
clobbers all caller-saved integer registers. The chunk's region follows from its
address; `size`, normalized as `arena_alloc` normalizes it, feeds only the `--debug`
counters.

- **Small**: the page is `(ptr - smallPages) >> 16`; its descriptor's class gives
  the slot size, and `ptr` must be a slot boundary whose bit is set. The bit is
  cleared, the hint lowered if needed, and the slot zeroed. A page whose first slot
  comes free rejoins its class list; a page whose last slot comes free leaves it for
  the empty-page list — unless it is the class's only page, which stays.
  [[src/codegen/memory/arena/arena/small.rs:emit_small_free]]
- **Medium or huge** (`_mfb_arena_span_free`): the start record of `ptr`'s block
  must be an allocated-chunk tag whose granule offset matches `ptr`; it gives the
  chunk's size. The record is cleared and the chunk zeroed whole. Then, within the
  chunk's own region, the free run whose end record lies in the block before `ptr`
  and ends at `ptr` (left), and the free run whose start record lies in the block of
  the chunk's end and starts there (right), are unlinked and merged with it; the
  result's tags are rewritten and it is listed by its size (or on the huge list).
  [[src/codegen/memory/arena/arena/span.rs:lower_arena_span_free]]

A pointer that is not a chunk this arena handed out — a repeated free, a pointer
into a chunk or a free run, an address outside the arena's slice, a free before the
arena ever allocated — changes nothing (counted as `double_free_skips` in a
`--debug` build). The `size` is supplied by the compiler's type-directed drop glue;
there is no user-level free operation.

## Zero Fill

Freed chunks are overwritten with zero — always on, in debug and release, the whole
chunk. The point is confidentiality: a freed value does not linger in the arena as
plaintext for a later bug, dump or reader to find. One overwrite is enough, because
RAM keeps no trace of the value it replaced, and zero is the cheapest value to write.
Committed memory is never filled by hand: never-touched committed pages read zero on
every target (anonymous memory on Linux and macOS, `MEM_COMMIT` on Windows), so no
byte another process or a previous mapping left can be read through it.

`arena_fill_zero(ptr, len)` takes an 8-aligned `ptr` and a `len` that is a multiple
of 8 (every chunk handed to it is 16-aligned and a multiple of 16). Its body is per
target: [[src/codegen/memory/arena/arena/mod.rs:lower_arena_fill_zero]]

- **AArch64** (macOS, Linux): the program entry reads `DCZID_EL0` once into the
  process-global word `_mfb_rt_dczva_bytes`: `4 << (DCZID_EL0 & 15)` bytes, or 0 when
  bit 4 (DZP) prohibits `dc zva` or the block size is outside 16–2048 bytes.
  [[src/codegen/error/constants/error_constants.rs:DCZVA_BYTES_SYMBOL]]
  [[src/codegen/engine/function/entry/mod.rs:lower_program_entry]] With a non-zero
  word and at least two blocks to clear, the fill stores zero words up to block
  alignment, issues one `dc zva` per whole block, and finishes the tail with the
  generic loop. Otherwise it runs the generic loop alone. Every AArch64 module carries
  the word; a module without an entry leaves it 0.
- **x86-64** (Linux, Windows): one `rep stosb`.
- **riscv64**: the generic loop — eight zero-word stores, 64 bytes an iteration,
  then one word at a time.

**Every chunk `arena_alloc` hands out reads zero**, since nothing but the program
writes into the arena and every free zeroes what it frees. Allocation sites still
fully initialize what they later read (the language's allocators do); a
use-after-free reads zeros, which can look like a valid value. The fill draws no
randomness: it never reads or advances the language RNG (`math::rand`, offsets
88/96).

A `--debug` build counts the fills for each arena in the debug report:
`arena.<n>.fill_free_calls`/`.fill_free_bytes` for freed chunks, and
`arena.<n>.fill_grow_calls`/`.fill_grow_bytes`, which stay 0. Run with
`MFB_ARENA_CHECK_ZERO` set, it also checks every chunk `arena_alloc` hands out and
counts the chunks that hold a non-zero byte in `arena.<n>.zero_violations` (see
`./mfb spec tooling debug-report`).

## Cleanup and Reclamation

An arena is reclaimed whole. `arena_destroy` (symbol `_mfb_arena_destroy`)
decommits the arena's whole slice in one call — `madvise(MADV_DONTNEED)` then
`mprotect(PROT_NONE)` on Linux, an `mmap(MAP_FIXED, PROT_NONE)` over the range on
macOS (guaranteed to drop the old contents, which `madvise` is not there),
`VirtualFree(MEM_DECOMMIT)` on Windows — so its pages go back to the OS and read zero
when committed again. [[src/codegen/memory/arena/arena/mod.rs:lower_arena_destroy]]
[[src/codegen/engine/types/types.rs:emit_arena_vm]] It refunds the arena's committed
bytes to the cap and clears the small and medium list heads, the huge head, the node
free list and the scalar block, so the arena is inert and lays its slice out afresh
if it allocates again. The slice stays the arena's (`thread::start`'s bookkeeping
hands worker and control-block slices back, `./mfb spec threading control-block`).
Nothing is unmapped: unmapping part of the reservation would leave a hole for an
unrelated `mmap` to land in. The helper is idempotent — an arena that never
allocated, or was already destroyed, has nothing laid out and does nothing.

At process teardown, `_mfb_shutdown` reads the arena-state address from
`_mfb_rt_main_arena`, clears that global first [[src/codegen/error/constants/error_constants.rs:SHUTDOWN_SYMBOL]] (so a signal arriving mid-teardown
re-enters as a no-op), restores the terminal if TUI mode was active, and then
calls `arena_destroy` on the main arena — except in a program that can still have a
live worker (one embedding any `thread.` runtime call), where the free is skipped
on every platform and the OS reclaims the memory at process exit instead
(`./mfb spec threading os-integration`). A worker arena's slice is decommitted by
the worker as the last step of its run; its arena-state block (a heap block) is
freed with the thread's control block. Nothing another thread reads points into a worker arena:
every value that crosses is copied into the control block's own block arena and
from there into the receiver's arena (`./mfb spec threading control-block`).

## Scope-Drop Frees

Beyond the bulk `arena_destroy`, individual owned values are freed deterministically
at **scope-drop**. A flat, pointer-free value is
freed by a single `arena_free(ptr, size)` of its block, the size recomputed from the
static type at the drop point (`emit_inlined_block_size_from_ptr_slot`). [[src/codegen/collection/layout/builder_collection_layout/records.rs:emit_inlined_block_size_from_ptr_slot]]
A value of a recursive type (one whose declaration reaches itself) is a graph of
separately allocated blocks. Its owner frees it through the module's one drop walker,
`_mfb_rt_graph_drop`, which frees one block at a time and keeps the pending edges on an
arena work stack rather than the native stack, so a graph of any depth is freed without
overflowing it. [[src/codegen/memory/arena/graph_drop.rs:lower_graph_drop_walker]]
A function value owns its closure the same way (bug-781): a bare function value is freed
by `_mfb_rt_closure_drop`, and a value that holds function values (a record or union with
a `FUNC` field, a collection or `Result` of them) is a graph-owning value whose walker
frees each closure it meets (`./mfb spec memory closures`). [[src/codegen/collection/layout/builder_collection_layout/type_predicates.rs:type_contains_func]]
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
payload of a recursive type (one whose declaration reaches itself), or one holding a
function value, is a graph of separate blocks that a byte copy would share, so those stores
give an aliasing source its own graph first — unless the store is the source's last read,
which moves it — and a collection a builtin builds out of another collection's elements
copies those elements' graphs. Two kinds of builtin result already own their graphs, so
their edges are not copied a second time: an element READ (`collections::get`/`getOr`,
`materialize_owned_element`'s copy) and `collections::transform`'s list, which takes over
each callback result (the callee copies anything it was lent before returning it) and
frees an inline record or union result's own block once its bytes are appended. [[src/codegen/builtins/collections/mod.rs:member_result_owns_its_graph]] [[src/codegen/builtins/collections/func_transform.rs:lower_transform]]

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
The `STATE` payload is freed by whichever cleanup owns the record, whoever attached
it: a binding that names another binding's record and declares the `STATE` (`RES b
AS fs::File STATE Cur = a`) records the state type on that owner's cleanup. The
inline-`TRAP` form depends on it — `RES v AS fs::File STATE Cur = open() TRAP …`
binds `v` from a delivery temporary typed by the producer, with no `STATE`, and the
temporary's cleanup is the one that runs. A resource union's binding does the same.
[[src/codegen/engine/control/builder_control/bind_op.rs:live_resource_aliases]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:resource_alias_root]]
The temporary an inline `TRAP` delivers a resource through is bound to a closed
default record of its own (`./mfb spec architecture ir`), and the delivery stores
over it. The store closes the value it replaces and frees no stored record (the
temporary's scope has not exited), but the default is the temporary's own. Its bind
remembers the default's address in a `trap_default_record` stack slot, zeroed with
the frame, and each drop of the temporary (the store's drop of the old value, or
the scope's when the handler leaves without storing) frees the record it holds
when that record is at the remembered address, then clears the slot. A resource
union's temporary does the same for the record its default box names, before it
frees the box.
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_free_trap_default_record]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:remember_trap_default_record]]
The assigned scope's eligible cleanup also frees the resource record for `fs.File`,
`tcp.Socket`, `tcp.Listener`, `udp.Socket`, `tls.Socket`, `tls.Listener`,
`audio.AudioInput`, `audio.AudioOutput`, and the parent `Thread` view. It frees
the record last, after close and block reclamation, and zeroes the slot. Other
kinds retain the record as a closed tombstone until arena teardown. The
owner, not `close`, frees it, because a `close` through a resource parameter leaves the
owner still reading the closed flag. Reclamation relies on every valid view being
bounded by the cleanup scope; the module-store gap is documented in
`./mfb spec language resource-management`. An explicit `close` for these
record-freeing kinds keeps the drop registered, and the drop's re-close is the
defined no-op. [[src/codegen/resource/cleanup/builder_resource_cleanup.rs:resource_record_freed_at_drop]]
Ordinary resource reclamation skips a record whose `RESOURCE_MOVED_BIT` is set;
it must not close the transferred handle. For an eligible owning cleanup retired
by a successful transfer, a separate moved-record cleanup frees the sender's
`STATE` and tombstone record without closing the handle. The receiver's `STATE`
is an independent deep copy, not a pointer into the sender's payload. Other
retired cleanups are removed and their remaining arena allocations survive until
teardown. See `./mfb spec language resource-management` for the source-visible
close/reclamation distinction.
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_block_reclaim]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:retire_moved_resource_cleanup]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_moved_resource_record_free]]
[[src/codegen/memory/arena/builder_arena_transfer/transfer.rs:copy_resource_to_current_arena]]

**What a call hands its caller's statement.** When a call to a function body
returns a value that reaches resources, the statement the call sits in takes, at
once, every record the callee's `RETURN` released (`./mfb spec language
resource-management`, "The owner of a resource"). The result is spilled to a
`resource_temp` slot and walked: a record whose owner word is below the statement's
token takes that token and gets a `{record, next}` node (`{tag, record, next}` for
a resource-union leaf) on a list of its leaf kind headed by a `resource_temp_owned`
slot; a record that already has an owner at the statement or further out is
skipped. The statement's token is the token of the scope depth one inside the
current scope, so every binding, container or `RETURN` of this frame that keeps
the record raises it past the statement by the ordinary float, and no callee can
take it. The slots are zero-initialized with the frame, so a path that never ran
the producer drops nothing. At the end of the statement, and on each of its other
exits (an error exit and a `RETURN` included), the lists are drained: a record the
statement still owns is closed and its buffers, `STATE` and record freed; a record
a taker raised no longer holds the statement's token and is skipped. The result's
own block is an ordinary flat temporary with its own free. Which expression the
call is written in is not consulted.
[[src/codegen/engine/value/builder_values/pending_temps.rs:adopt_statement_temp]]
[[src/codegen/engine/value/builder_values/pending_temps.rs:emit_statement_temp_drop]]
[[src/codegen/engine/builder/mod.rs:StatementTemps]]

**The scope that closes a resource frees its record.** That is the one rule for a
resource record's memory, and the owner word alone decides it
(`./mfb spec memory heap-values`, "The owner word"): no cleanup asks whether its
binding is the record's only holder. Every store that lets a reference outlive a
scope raises the owner word first (`./mfb spec language resource-management`
§15.6), so when the scope holding the token exits, every remaining reference is in
that scope or inside it. What one cleanup cannot know is whether another cleanup of
the same exit still reads the word, so the free is split from the close:

1. A cleanup that finds its scope's token in the record closes it, writes the
   **taken** owner word (`RESOURCE_OWNER_TAKEN`) into it, and parks it: an
   owned-list drain moves the node to a reclaim list headed by a second stack slot
   (`owned_list_reclaim_<name>`); a `RES` binding's drop stores the record in its
   `resource_reclaim_<name>` slot; a resource-union binding's drop stores its box
   in `resource_union_reclaim_<name>`. A record the scope does not own is left
   alone (a drain frees that record's node).
2. After the last cleanup of the exit, each parked record is freed: its `File`
   I/O buffers, its `STATE`, the record itself for the record-freeing kinds
   above, then the node or the box.

A second list, or a binding, that names the same record reads an owner word that is
not its token and skips it, having read nothing else; the record is still allocated
at that point. The reclaim follows the whole cleanup sequence of an exit: the end
of a block, an `EXIT`/`CONTINUE` branch, a `RETURN`, and an error exit. The
error-return landing pads drop one cleanup each and share their tail, so there the
reclaim is the last pad's, which walks every reclaim slot of the function. A
reclaim slot is zeroed with the frame and emptied by every walk, so a path that
took nothing costs one load and a branch. A leaf with nothing to reclaim (no record
free, no I/O buffers and no `STATE`: a `canvas` image or font) has no reclaim slot
and keeps the plain close. A value a store replaces in a binding (the inline-`TRAP`
temporary's closed default) is closed at the store and its default record freed by
its own rule; the binding's scope has not exited.
[[src/codegen/cleanup/owned/builder_owned_cleanup/drain.rs:emit_owned_list_drain]]
[[src/codegen/cleanup/owned/builder_owned_cleanup/drain.rs:emit_owned_list_reclaims]]
[[src/codegen/cleanup/owned/builder_owned_cleanup/mod.rs:OwnedListPass]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_reclaim]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_union_reclaim]]
[[src/codegen/engine/control/builder_exits.rs:append_error_return_pads]]
[[src/codegen/resource/owner.rs:RESOURCE_OWNER_TAKEN]]

An error routed to the function's own `TRAP` handler leaves the function-level
scope alive: the handler runs inside it. Nothing that scope owns is dropped on the
route, neither closed nor freed; the handler's own exit runs every function-level
cleanup and the reclaim, once. The scopes the error leaves on its way are dropped
as on any exit. [[src/codegen/engine/control/builder_exits.rs:trap_route_cleanups]]

A resource that **floats** into an owner collection (`./mfb spec language
resource-management` §15.6) is listed on that collection's owned-list: a 16-byte
`{record, next}` node, or for a resource-union element a 24-byte
`{tag, record, next}` node holding a copy of the element's box. A concrete resource
that floats into a container of a union it is a member of (`RES s AS fs::File`, then
`hs = collections::append(hs, s)` on a `List OF RES Handle`) is listed at its
binding, before the store wraps it, so it has no box yet: its node takes the record
itself and the member's own variant tag, the node a box of the member would have
given. [[src/codegen/cleanup/owned/builder_owned_cleanup/owned_list.rs:emit_owned_list_push_as]]
The drain frees no union box: a box is its binding's or its statement's, or a span
of the collection block that holds the element inline (`./mfb spec memory
heap-values`, "Union"). The collection's own block is not part of any of this: a
collection of resource aliases is a flat block (`./mfb spec memory collections`),
copied at every hand-off to a longer-lived owner and freed by its one owner through
the ordinary owned-value drop, the block a reassignment replaces and the
intermediates a `collections` member builds included.
[[src/codegen/collection/buffer/collection_buffer.rs:free_intermediate_collection]]

A **record** that reaches a resource — a record with a `RES` field, or one nesting
such a record — is an owner container too: it lists the records it takes, and its
drain closes and frees them by the same rule.

A binding or an assignment lists only the records it takes. A record whose owner
word is below the taker's token (unowned, owned by the taker's statement, or owned
by a scope inside the taker's) is raised to the taker's scope and listed. A record
already owned at that scope or further out — a handle the caller passed in and got
back, or one the container already lists — keeps its owner and is not listed again.
[[src/codegen/cleanup/owned/builder_owned_cleanup/owned_list.rs:emit_owned_list_push_node]]

A **store** raises what it stores (rule 4). The code is emitted where it can do
something, which is read off the declarations: a binding's value reaches nothing
owned inside the binding's own scope, so an assigned value is asked for the deepest
scope anything in it can be owned by — a local by the scope it was declared in, a
`FOR EACH` variable by what its iterable reaches, a call to a function body or a
function value by the statement itself — and the assignment floats when that is
deeper than the container's declaration. A builtin's view over outer values reaches
nothing deeper and floats nothing.
[[src/codegen/resource/cleanup/record_ownership.rs:StoreFloats]]
The float is done at the store, over what the store adds: an in-place append or set
walks the one item it stores, any other assignment walks the assigned container.
Each resource reached is pushed, adopting, onto the container's owned-list.
[[src/codegen/cleanup/owned/builder_owned_cleanup/mod.rs:emit_stored_item_float]]
[[src/codegen/cleanup/owned/builder_owned_cleanup/mod.rs:emit_assigned_container_float]]
A binding that a `LAMBDA` captures **by reference** lives in the frame that created
the lambda, so its stack object is a cell `{value, scope token, one owned-list head
per resource leaf kind}`. An assignment to it inside the lambda cannot compare
scopes statically, so it always raises: each resource the stored value reaches whose
owner is below the cell's token takes that token and is listed on the cell's head
for its kind, which the binding's own scope drains and reclaims like any other
owned-list. [[src/codegen/cleanup/owned/builder_owned_cleanup/mod.rs:setup_ref_cell]]
[[src/codegen/cleanup/owned/builder_owned_cleanup/mod.rs:emit_captured_store_float]]

A resource union's drop reclaims what the concrete drop reclaims: after the
tag-dispatched close it parks its box, and the reclaim frees a `File` variant's
output and read buffers, then the `STATE` block, then the record, then the box.
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_union_tag_dispatch_drop]]

A list of such collections (`List OF List OF RES T`) is a nested flat list: each
inner list is inlined in the outer block's data region, so one copy copies the
whole value and one free releases it.
[[src/codegen/collection/layout/builder_collection_layout/type_predicates.rs:type_is_memcpy_copyable]]

Raw thread-helper results belong to the thread runtime's block arena and
must not be freed through the caller's arena. However, `thread::waitFor`,
`receive`/`read`, and `acceptResource`/`readResource` copy their returned values
into the calling thread's arena at the call site: those copies are owned and
dropped there. A runtime-managed value that has not been copied remains excluded
from ordinary scope-drop frees.
[[src/codegen/engine/value/builder_values/ownership.rs:value_is_runtime_managed]]
[[src/codegen/engine/value/builder_values/ownership.rs:runtime_call_result_is_copied_at_call_site]]

**Pointer graphs whose type holds a resource** are also excluded from ordinary
owned-value frees. A recursive or cycle-reaching value is not `type_is_memcpy_copyable`, so the
flat `arena_free` path never claims it. Instead its owner frees it through
`_mfb_rt_graph_drop` when the type has a graph-drop kind, which every resource-free type in
or reaching a cycle does. A resource-bearing type never gets a kind: it is move-only and
closed by its own op. [[src/codegen/engine/value/builder_values/ownership.rs:owns_graph]] [[src/codegen/memory/arena/graph_drop.rs:graph_drop_kind]] Builtins that could otherwise return
an interior pointer into an argument return an owned block instead (`collections::get`/`getOr`
materialize the element; `strings::replace`'s no-op path returns a fresh copy), so a
call result is always safe for the caller to own and free.

## Measuring an Arena

A program built with `mfb build --debug` counts the events described on this page
for every arena: its slice's size, commits and decommits and their bytes, allocation
and free calls and bytes, which region and source served each request, small pages
set up, frontier extensions, double-free skips, fills, and live and peak live bytes.
It prints them in the `arena` section of the debug report at exit. A normal build
contains none of the counting code. [[src/codegen/debug/arena/mod.rs:ArenaFeature]]

## See Also

* ./mfb spec tooling debug-report — the `arena` report section and its counters

* ./mfb spec threading — per-worker arenas and thread isolation
* ./mfb spec language memory-semantics — the source-level ownership model
* ./mfb spec architecture native — arena helpers in native codegen
