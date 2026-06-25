# MFBASIC Arena Reuse + Entropy-Init Plan

Last updated: 2026-06-24

> **Status.** Phases 1–5 are implemented, validated, and landed. Phases 1–4:
> coalescing free-list + sized `arena_free`, the zero-init audit (the entropy-fill
> forcing function showed it needs no code fixes), entropy fill, and the first
> runtime-internal free site. **Phase 5 (scope-drop frees) is DONE** — but via
> `plan-02`, not the per-type drop glue this document anticipated. `plan-02` made
> every non-resource value a flat, pointer-free block, so freeing a value is one
> generic `arena_free` of its block (no per-type recursive drop glue) and the
> hard "is this value aliased?" question is dissolved by **copy-insertion**: a
> deep `memcpy` (`copy_flat_block`) is inserted at every store/return site that
> takes an aliasing source, making every owned local hold an independent block.
> No general-value escape *analysis* was needed; only a syntactic copy-insertion
> rule plus the existing `RETURN`/`thread::transfer` move suppressions. The
> implementation is `plan-02` Phase 8 — see that document's status section for the
> full account (and the three entropy-surfaced subtleties: static `String`s in
> rodata, builtins that returned borrows, and runtime-managed thread results).

This document plans turning the arena from a pure bump allocator into a
deterministic, reuse-capable heap: freed memory is returned to a per-arena
free-list and handed back out to later allocations, values are freed at
scope-drop (no garbage collector, no funkiness), and memory is filled with
entropy when freed and when a fresh block is mapped from the OS.

It complements:

- `specifications/memory_layouts.md` (arena-state / block layout, `arena_alloc`)
- `specifications/mfbasic.md` (ownership, scope, deterministic cleanup)
- `specifications/threading.md` (per-worker arenas, thread transfer)

## 1. Goal

- Keep mapping memory from the OS in large blocks (the current block chain).
- Add a free-list to the arena so freed allocations can satisfy later
  allocations — i.e. make the arena a real heap, not just a bump pointer.
- **Internal reuse only.** Freeing never returns memory to the OS. A freed
  allocation goes onto the arena's free-list and stays mapped for the next
  allocation to reuse; blocks are unmapped only by the existing bulk
  `arena_destroy` at process/worker teardown.
- Free deterministically **on scope-drop**, exactly like resources already
  close on scope-drop. No GC, no reference counting, no background sweep.
- Fill memory with entropy when it is freed, and fill a block with entropy when
  it is first mapped from the OS. Entropy fill is a **requirement**, always on.

### Non-goals (explicit constraints)

This must **not** change the language, its surface syntax, value/copy/move/freeze
semantics, or thread-transfer rules. From a source-program point of view nothing
changes: the same programs compile, copy the same way, and observe the same
values. This plan only changes (a) what the allocator does with freed bytes and
(b) the initial byte pattern of fresh memory. Reuse and entropy-init are
invisible to correct programs.

## 2. Current State

Per `memory_layouts.md`:

- The arena is a bump allocator over a chain of OS-mapped blocks. Arena-state
  (104 bytes at `x19`) tracks only `blockHead`; each block header (32 bytes)
  holds `prevBlock`, `blockSize`, `usableCapacity`, `bumpOffset`.
- `arena_alloc(size, align)` either advances `bumpOffset` in the current block
  or maps a new block (`max(4096, round_up(size+align+32, 4096))`).
- **There is no per-object header and no free path.** Individual values are
  never freed. Reclamation is one bulk `arena_destroy` (unmap the chain) at
  teardown.
- Scope-drop emits **nothing** for arena-backed values (strings, records,
  unions, collections). Memory is implicitly zero because `MAP_ANON` pages are
  zero-filled.
- **Resources already free on scope-drop.** A resource's life ends on exactly
  four events — a registered close op, `thread::transfer`, `RETURN`, and
  scope-drop — and codegen emits the close at the scope-exit point
  (`typecheck.rs:5948`, `plan.rs:459`, `validate.rs:69`). This is the precedent
  we extend to arena values.

## 3. Design Overview

Three independent pieces, layered:

1. **`arena_free(ptr, size)` + per-arena free-list** — the runtime mechanism for
   reclaiming and reusing a single allocation.
2. **Drop emission at scope-drop** — codegen that calls `arena_free` (recursively,
   via per-type drop glue) when an owned arena value leaves scope, mirroring
   resource close emission.
3. **Entropy fill** — fill freed regions and freshly-mapped blocks with
   pseudo-random bytes.

Pieces 1 and 3 are pure runtime/allocator work. Piece 2 is codegen and is where
the correctness risk lives.

## 4. Coalescing Free-List (runtime)

The arena becomes a single **address-ordered, coalescing free-list** per arena:
allocation carves (splits) a free entry, and freeing returns the chunk and merges
it with adjacent free neighbors. This subsumes the old bump pointer — bumping is
just "split the one big trailing free entry" — so there is one mechanism, not a
bump path plus bins. It needs **no per-object headers** (see §4.1), packs memory
tightly via coalescing, and reuses a freed chunk for a request of any size.

### 4.1 Sized free, no headers — and the size is compiler-trusted

`arena_free(ptr, size)` is told the size by its caller. The caller is **always
the compiler's drop glue** (§5) — the language exposes no user-level free — so
the size is computed from the static type at codegen time and is as trustworthy
as any other emitted instruction. This is what lets the design skip per-object
size headers entirely: we never need to *recover* a chunk's size at free time
(the classic reason `malloc` carries a header), because the caller supplies it.

Consequently the free-list logic reads sizes only from (a) the `size` argument
and (b) the free entries it is already walking — never from a tag on an allocated
chunk. No allocation carries any header or footer; layouts and copying are
untouched.

`size` is normalized as `arena_alloc` normalizes it: zero → 1, then rounded up to
the 16-byte granule (§4.2).

### 4.2 Free-list node lives in the dead bytes

Each free entry is an intrusive node overlaid on the free chunk itself; its
**start is implicit** (the node's own address):

```text
FreeNode (overlaid on a free chunk, start = node address)
  +0  U64 next        ; next free chunk in address order, 0 = end
  +8  U64 size        ; size of this free chunk, in bytes
  ...                 ; remaining bytes are dead (entropy-filled, see §6)
```

The list is kept **address-ordered** (ascending `start`) so a single walk finds
the insertion point and both neighbors for coalescing. Allocated chunks carry
**nothing** — only free chunks spend 16 bytes on a node, in their own dead space.
Min chunk granule is therefore **16 bytes** (8 `next` + 8 `size`); sub-16 requests
round up to 16. Standalone arena allocations already clear this (`StringObject`
≥ `8 + bytes + 1`, record `8 * fieldCount`, collection ≥ 80).

### 4.3 Allocation — first-fit + split

`arena_alloc(size, align)`:

1. Validate alignment, normalize/round `size` (unchanged spirit).
2. Walk the address-ordered free-list for the first entry where the request fits
   after alignment: `aligned = align_up(entry.start, align)` and
   `aligned + size <= entry.start + entry.size`.
3. **Split** that entry: return `aligned`; push the front padding
   (`aligned - entry.start`, if ≥ 16) and the tail remainder
   (`entry.end - (aligned + size)`, if ≥ 16) back as free entries. Remainders
   below 16 bytes stay as unobservable slack inside the allocation.
4. If no entry fits, **map a new block** (`max(4096, round_up(size+align+32,
   4096))`, page-rounded as today), insert it as one big free entry, and retry.

First-fit on an address-ordered list does the right thing for free: low-address
reclaimed holes are reused **before** the big trailing entry is carved, and
carving the trailing entry *is* the bump pointer. Early on the list is one big
entry per block, so allocation is effectively an O(1) bump; it only lengthens
under real fragmentation.

### 4.4 Free — scan + coalesce

`arena_free(ptr, size)`:

1. Normalize/round `size`.
2. Entropy-fill the chunk (§6).
3. Walk the address-ordered list to `ptr`'s insertion slot, then:
   - if `prev.start + prev.size == ptr` → extend `prev` (absorb the chunk);
   - if `ptr + size == next.start` → absorb `next` into the chunk/`prev`;
   - both adjacent → merge all three into one entry;
   - neither → write the `{next, size}` node into `[ptr]` and link it in.

Adjacency is pure arithmetic on `start`/`size`; no neighbor tag is read, so no
headers are needed for coalescing — the scan replaces the O(1) boundary-tag
lookup. **Coalescing keeps the list short**: same-size churn (a loop that
allocs/frees the same shape each pass) merges straight back into its neighbor, so
the list stays ~1–2 entries and the "scan" is effectively O(1). The list only
grows — and scans only slow — under genuine fragmentation (many live objects
separated by holes), which is exactly when the alternative bin design would be
silently wasting memory instead.

**Never merges across blocks.** Chunks in different OS-mapped blocks are not
contiguous, so the `start + size == next.start` test fails between them with no
special-casing; a single chunk never spans a block.

`arena_free` **never unmaps** and never returns memory to the OS — it only fills
and relinks. Mapped memory is reclaimed only by `arena_destroy` at teardown. Both
helpers document their clobber set the way `arena_alloc` does today.

### 4.5 Free-list head — one pointer, per-arena

The list needs a single **head pointer** (lowest-address free entry), stored in
the reserved arena-state word at **offset 48** (`freeListHead`). That is the only
header state required — there is no bins array. After this plan the reserved
arena-state words at offsets 16/24/40 hold the fill RNG and start time, offset 48
holds `freeListHead`, and offsets 8 and 56 remain free. `ARENA_STATE_SIZE` stays
104.

A per-arena (not per-block) list is required for cheap coalescing: `arena_free`
must reach neighbors by address order without first resolving which block a
pointer belongs to. Across-block merges simply never occur (§4.4), so one
arena-wide ordered list is both correct and simplest.

## 5. Free on Scope-Drop (codegen) — the hard part

Reuse is worthless unless something calls `arena_free`. Per the follow-up
requirement, freeing is **deterministic at scope-drop**, the same model as
resources — not a GC.

### 5.1 Drop glue, mirroring copy glue

The compiler already emits **deep-copy glue** per type (value semantics deep-copy
strings, record slots, nested collection handles). We add the mirror image:
**drop glue** per type that frees an owned value and everything it owns:

- `String` → `arena_free(ptr, 8 + byteLength + 1)`.
- record → free each slot that holds an owned heap value (recursively), then
  `arena_free(record, 8 * fieldCount)`.
- union → drop the active member's payload by tag, then free the object.
- collection → drop each live entry's nested collection handle (string/scalar
  payloads are inline in the data region and need no separate free), then
  `arena_free` the single contiguous allocation.

Drop glue is the exact dual of the existing copy glue and reuses the same
type-directed traversal machinery.

### 5.2 When a drop is and isn't emitted (escape analysis already exists)

A value is freed at scope-drop **only when the scope owns it and it does not
escape**. The compiler already computes this to choose copy vs. move vs. borrow
(`ExprMode`), so the information is in hand. A drop is **suppressed** when:

- the value is **returned** (`RETURN` moves ownership to the caller),
- the value is **moved/transferred** (e.g. `thread::transfer`, or a move into a
  collection where ownership transfers rather than copies),
- the value was **copied** into a long-lived container — the *copy* is owned by
  the container and freed when the container drops; the *temporary* original is
  freed at its own drop point.

This is precisely the resource invalidation-event model (close op / transfer /
`RETURN` / scope-drop) generalized to all owned heap values. Because value
semantics deep-copy, **a dropped non-escaped value's bytes are provably
unaliased**, so freeing them is sound.

### 5.3 Thread-transfer interaction

A value transferred to a worker must never be freed in the producer arena. This
already falls out of escape analysis: `ExprMode::Transfer` consumes the sender
binding, so no drop is emitted for it. Cross-thread values continue to be
materialized in transfer storage / the receiver arena, untouched by this plan.

### 5.4 Risk: double-free / use-after-free

All frees are **compiler-emitted** — the language has no user-level free — so the
`(ptr, size)` passed to `arena_free` is always codegen output derived from the
static type, never user input. There is no class of "user passed the wrong size /
double-freed" bugs; the only way to corrupt the free-list is a compiler bug in
drop emission, the same trust boundary as any other emitted instruction.
Mitigations focus there:

- Phase the rollout (see §8): start with **runtime-internal** free sites whose
  lifetimes are trivially correct and need *no* user-level analysis — the old
  buffer on collection grow/realloc, the discarded data region on compaction,
  throwaway string temporaries (number→string, interpolation) that are not
  returned. These reuse churned memory immediately with near-zero risk.
- Only then enable user value scope-drop frees, gated behind the existing
  ownership/escape analysis and a thorough test pass.
- Entropy poisoning (§6) turns any drop-emission bug (premature free, missed
  escape) into a loud use-after-free rather than silent corruption.

### 5.5 Reality check: the heap is an **aliased graph**, not an ownership tree

The plan §5.2 assumed "value semantics deep-copy, so a dropped non-escaped
value's bytes are provably unaliased." **That assumption is false in the current
implementation.** Owned arena values (`String`, record, union, collection,
`Error`, `Result`) are shared by raw pointer at almost every store site — value
semantics survive only because *mutation* always allocates fresh, so the shared
copies are read-only. A naive "free every non-escaping owned local at scope-drop"
would therefore **double-free**: e.g. `LET b = a` makes `a` and `b` the same
allocation, and `MyRecord(s)` stores `s`'s pointer into the record.

To free user values soundly, Phase 5 must **first** make the heap an ownership
tree — insert `copy_value_to_current_arena` (the existing deep-copy glue,
`builder_misc.rs:1279`) at every site that stores an owned value into a
longer-lived or aliased location — **then** free every owned local at scope-drop
except those moved out by `RETURN`/`thread::transfer`. The insertion must be
**exhaustive**: any missed site leaves an alias the frees will double-free.

**Aliasing store sites** (owned value stored by pointer — must be deep-copied):

1. **Local bind / assignment** — `LET/MUT x = <aliasing source>` stores the
   loaded pointer (`builder_control.rs:24-30`; `builder_values.rs:43-63` returns
   the local's pointer). Aliasing sources: `Local`, `Global`, `Capture`,
   `MemberAccess`, `UnionExtract`. Fresh-producing RHS (`Call`, `Constructor`,
   literals, `Binary`/`Unary`, `UnionWrap`, `RuntimeCall`) yield a new
   allocation and are moves, not aliases.
2. **Record construction** — `Rec(a, b)` stores each arg pointer into
   `[record + 8*i]` (`builder_values.rs:681-684`).
3. **Union construction** — `Variant(a)` stores each arg pointer into
   `[union + 8*(i+1)]` (`builder_values.rs:750-753`).
4. **List/Map literals** — element results stored into the data region
   (`builder_collection_layout.rs lower_list_literal` / map literal). For
   **pointer-payload** element types (nested collections, resources) the element
   *pointer* is stored (alias); inline payloads (scalars, `String` bytes,
   record/union slot bytes) are byte-copied (`is_pointer_collection_payload_type`,
   `builder_collection_layout.rs:24-32`).
5. **Collection inserts** (`collections::append/set/insert/push/…`,
   `builder_collection_updates.rs`) — same pointer-vs-inline rule as (4): nested
   collection / resource payloads aliased; `String`/scalar/record/union payloads
   copied inline. **Note:** an inline record/union payload still embeds *its*
   field pointers, which alias the field values — recursion matters.
6. **Global store** — `globalVar = x` stores the value pointer into the global
   slot, no copy.
7. **Record `WITH` update** (`WithUpdate`) — copies the base record but stores
   the new field values by pointer (alias).
8. **Closure capture** — captured owned values stored into the closure env by
   pointer.

**Move / escape sites** (value leaves the scope — *suppress* the drop, do not
copy):

9. **`RETURN`** moves the value pointer to the caller (`builder_misc.rs:2994-3001`
   moves reference types; inline payloads are materialized). The returned local's
   scope-drop free must be deactivated, mirroring how resources deactivate their
   close on `RETURN` (`builder_misc.rs:3019-3041`).
10. **`thread::transfer` / `thread::send`** already deep-copy into the receiver
    arena and deactivate the sender's cleanup (`builder_misc.rs:2643-2675`).

**Non-issue:** function **arguments** are a transient borrow for the duration of
the call — the callee receives a pointer it must treat as borrowed and never
free, and cannot retain it past return except by `RETURN` (which copies/moves per
(9)). So args need no copy as long as parameters are borrows the callee never
drops. This is also why a sound *conservative* subset is possible without the
full copy-insertion: free only locals that never reach sites 1–8 or escape via
9–10 (pure transient temporaries), which needs no new copies and so cannot
corrupt.

Ownership/escape analysis for these owned values **does not exist yet**:
`escape.rs` and `ExprMode` (`typecheck.rs:110`) only track resource (`RES`)
bindings. Phase 5 must add a general owned-value pass.

## 6. Entropy Fill

### 6.1 Mechanism — a **dedicated** per-arena PCG64, separate from `math::rand`

Speed matters, so the fill uses a fast userspace PCG64, not a `getentropy`
syscall per region (a true OS-entropy fill of every 4 KB block and every free
would be far too expensive). OS entropy seeds the generator; the fill streams
PRNG output via `arena_fill_random(ptr, len)`.

**It must not reuse the language RNG state at arena-state offsets 88/96.** That
state backs `math::rand` / `math::seed`, whose sequence is reproducible and
user-observable; advancing it on every allocation would silently break that
reproducibility. Instead, give each arena a **separate fill-RNG state** seeded
independently, so memory fill and the language RNG never interfere.

Reuse two of the existing **reserved** arena-state words for the fill RNG rather
than growing the struct:

```text
  +16 U64 fillRngLo    ; dedicated memory-fill PCG64 state, low 64 bits
  +24 U64 fillRngHi    ; dedicated memory-fill PCG64 state, high 64 bits
```

Offsets 16 and 24 are currently only zero-initialized at arena init
(`mod.rs:1784-1786`) and never read anywhere, so repurposing them is free.
`ARENA_STATE_SIZE` stays **104** and `ENTRY_ARGC_OFFSET` is unchanged. The init
that zeros 16/24 is replaced by seeding the fill RNG into those slots.

The fill RNG is seeded from OS entropy at arena init (main and each worker
arena), independently of the language RNG. Its output is **never observable** —
filled bytes are always fully overwritten by a constructor before any read (§6.4)
— so the fill RNG needs no reproducibility and no parent-draw determinism; a
fresh OS-entropy seed per arena is sufficient and simplest.

### 6.1a Arena start time at offset 40

Capture the arena's start time once at arena init into the reserved arena-state
word at **offset 40** (`arenaStartTime`, nanoseconds via the already-wired
`clock_gettime` path — `timespec` ns at +8, see `mod.rs:4927`). Offset 40 is
currently never read or written, so it is free.

It serves two purposes: lightweight per-arena diagnostics (when this arena/worker
started), and a fast-varying value to **mix into the fill-RNG seed** so each
arena's poison stream differs even if two arenas seed in the same instant or a
`getentropy` call fails — the same defensive mixing `mod.rs:1863` already does
with the arena address. (The `CNTVCT_EL0` cycle counter is an even cheaper
single-`mrs` alternative if `clock_gettime`'s call cost matters at worker spawn.)

### 6.2 Fill on free

`arena_free` fills the chunk with PRNG bytes, *then* writes the `next` pointer
into `[ptr]`. Effect: a freed secret (key, password) does not linger as
plaintext, and any read of freed memory yields garbage that a debug build can
trap on.

### 6.3 Fill on new block

After `mmap` (which returns zeroed pages) for a freshly grown block, fill the
usable region with PRNG bytes before first use. Block growth is infrequent, so
the O(block) fill cost is amortized.

### 6.4 **Critical correctness consequence — zero-init audit**

Today fresh arena memory is implicitly **zero** (`MAP_ANON`). Entropy fill (and
reuse of freed chunks) **destroys that guarantee**: both reused chunks and
freshly-filled blocks now return non-zero bytes. Any allocation site that relies
on implicit zeroing — unset record slots, a union's unused payload slots,
reserved header bytes, partially-initialized collection headers — will read
garbage.

Therefore this plan **requires an audit + fix**: every allocation site must
fully initialize every byte it later reads, with no dependence on `MAP_ANON`
zeroing. This is real work, but entropy fill is also the *forcing function* that
surfaces these latent uninitialized-read bugs — turning a silent reliance on zero
into a loud, reproducible failure (cf. the existing macOS codegen latent-bug
notes). Do the audit first; turn entropy fill on second.

### 6.5 Always on

Entropy fill is a requirement, not a toggle: it is always on, in debug and
release alike. It both scrubs freed secrets and poisons memory to catch
use-after-free / uninitialized reads. The cost is accepted and kept low by using
the fast dedicated PCG64 (§6.1) rather than an OS-entropy syscall per fill.

## 7. Layout / ABI Impact

- `memory_layouts.md` Arena section: document the per-arena address-ordered
  coalescing free-list (the `{next, size}` `FreeNode` overlay, `freeListHead` at
  arena-state offset 48), `arena_alloc`'s first-fit + split path, and
  `arena_free(ptr, size)`'s scan + coalesce path, each with its result/clobber
  ABI.
- **Arena-state size unchanged (104 bytes).** The dedicated fill RNG reuses the
  reserved words at offsets 16/24 (`fillRngLo`/`fillRngHi`, §6.1), and the arena
  start time reuses reserved offset 40 (`arenaStartTime`, §6.1a) — all currently
  zero-only / untouched. The language RNG at 88/96 is unchanged. Only the
  arena-init/seed code changes — seed the fill RNG into 16/24 and stamp the start
  time into 40.
- Block header and object layouts are **unchanged** (no size headers, no footers,
  no per-object metadata) — this is what keeps copying/transfer untouched.
- `arena_destroy` is unchanged: bulk-unmap still reclaims everything, free-list or
  not. Freeing never unmaps; memory returns to the OS only at teardown.

## 8. Phases

1. **[DONE] Zero-init audit.** Drove the entropy-fill forcing function (Phase 4)
   across the full acceptance suite, native runtime integration tests, and
   targeted stress (maps, nested collections, fs listDirectory/readAllBytes/
   pathJoin, thread message queues, multi-block alloc). No allocation site relies
   on `MAP_ANON` zeroing in a way poisoning breaks — no code fixes required.
2. **[DONE] `arena_free` + per-arena coalescing free-list + first-fit/split
   `arena_alloc`.** Bump replaced by split-from-free-chunk; `arena_free` +
   `arena_insert_free` coalesce. `FreeListSim` reference-model unit tests.
3. **[DONE] Runtime-internal free sites.** The `io::readLine`/`io::input` line
   buffer grow returns the dead old buffer to the free-list — the first real
   reuse and first runtime exercise of `arena_free`/coalesce/scrub. (Further
   internal temporaries — fs path C-strings, string-concat intermediates — can
   follow the same `arena_free(ptr, size)` pattern.)
4. **[DONE] Entropy fill.** `arena_fill_random` from the dedicated per-arena
   PCG64 at offsets 16/24; fill on free and on new block; always on. Seeded from
   OS entropy + arena address + start time (offset 40), independent of
   `math::rand`.
5. **[DONE] Scope-drop frees.** Emit `arena_free` at scope-drop for owned,
   non-escaping arena values. Realized as `plan-02` Phase 8: because every value
   is now a flat block, the free is a single generic `arena_free` of the block
   (no per-type drop glue) and aliasing is removed up front by **copy-insertion**
   (`lower_value_owned` deep-copies an aliasing source at each bind/store/return),
   so no general escape analysis was required — only the existing
   `RETURN`/`thread::transfer` move suppressions, plus exclusions for static
   `String` constants (rodata, not arena), borrow-returning builtins
   (`collections::get`/`getOr`, `strings::replace` — fixed to return owned), and
   runtime-managed thread results (`thread::receive`/`waitFor`). Frees emit at
   every scope exit (drain, `break`/`continue`, `RETURN`, `TRAP`) via the resource
   cleanup machinery (`ActiveCleanup::OwnedValue`). Landed behind heavy tests
   (`plan-02` §7a / `tests/scope-drop-free*-rt`) under entropy poisoning; full
   acceptance green with identical runtime output.

## 9. Decisions

Resolved:

- **Allocator structure:** a single per-arena, address-ordered, coalescing
  free-list with first-fit + split on alloc and scan + merge on free. This
  subsumes the bump pointer (bump = split the trailing entry) and needs no
  per-object headers (sized, compiler-emitted free + arithmetic adjacency).
  (§4)
- **Free-list head:** one `freeListHead` pointer at arena-state offset 48; no
  bins array. (§4.5)
- **Splitting / coalescing:** both built in from the start — they are what give
  full cross-size reuse and keep the list short. Header-free because free is
  sized and the caller is always the compiler. (§4.3, §4.4)
- **Fit policy:** first-fit over the address-ordered list — reuses low-address
  holes before carving the trailing entry, and is O(1) while the list is short.
  (§4.3)
- **OS reuse:** none — freeing is internal reuse only; blocks unmap only at
  teardown. (§1, §4.4)
- **Fill source:** a dedicated per-arena PCG64 reusing reserved arena-state
  offsets 16/24, seeded independently so it never perturbs the `math::rand`
  stream at 88/96; `ARENA_STATE_SIZE` stays 104. (§6.1)
- **Fill default:** always on, debug and release. (§6.5)

Still open:

- **List length under pathological fragmentation:** if real workloads produce
  long free-lists, revisit with a hybrid (e.g. a small size-class index over the
  ordered list) — measure first. (§4.4)

## 10. Summary

The coalescing free-list, sized free, and entropy fill are straightforward,
low-risk runtime work and preserve every layout. The allocator is one mechanism —
an address-ordered list that splits on alloc and merges on free — with no
per-object headers, because free is sized and always compiler-emitted. The real
engineering is (1) the zero-init audit that entropy fill forces, and (2)
deterministic scope-drop drop glue with airtight escape analysis. The language,
copying, and transfer semantics are untouched throughout; this is purely "reuse
freed memory and poison/scrub it," delivered the deterministic, GC-free way the
rest of the language already cleans up resources.
