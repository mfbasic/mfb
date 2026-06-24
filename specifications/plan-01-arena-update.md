# MFBASIC Arena Reuse + Entropy-Init Plan

Last updated: 2026-06-24

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

## 4. Sized Free + Free-List (runtime)

### 4.1 Sized free preserves the no-header invariant

`arena_free` takes the **size** as a parameter: `arena_free(ptr, size)`. The
caller (drop glue / runtime) always knows the size at the free site because it
knew the size at the allocation site. This is deliberate: it keeps the design's
"no universal per-object header" property — we do **not** add an 8-byte size
header to every object, which would bloat every record/string/collection and
violate the "no layout change" constraint.

`size` is normalized the same way `arena_alloc` normalizes it (zero → 1, then
rounded up to a size-class granule, see §4.3).

### 4.2 Free-list lives in the dead bytes

A freed chunk is itself the list node. We store a single `next` pointer in the
first 8 bytes of the freed region:

```text
FreeNode (overlaid on a freed chunk)
  +0  U64 next        ; next free chunk in this size class, 0 = end
  ...                 ; remaining bytes are dead (entropy-filled, see §6)
```

This requires every reusable allocation to be **≥ 16 bytes** (8 for `next`, plus
16-byte min alignment, see §4.4). Standalone arena allocations already clear
this: a `StringObject` is `8 + bytes + 1`, a record is `8 * fieldCount`, a
collection is `≥ 80`. Sub-16-byte requests round up to 16.

### 4.3 Segregated free-lists by size class

Rather than one sorted list with coalescing (which needs boundary tags ≈
per-object footers — a layout change we're avoiding), use **segregated
free-lists keyed on a rounded size class**. Round each request up to a granule
(proposal: 16-byte granules up to 256, then power-of-two classes above) and keep
one list head per class. Push/pop are O(1), exact-class fit, no coalescing.

Tradeoff to accept: without coalescing, N freed 32-byte chunks can never combine
to serve one 64-byte request. That fragmentation is bounded and is backstopped
by the bump path (a request that finds its class empty just bumps) and by bulk
arena teardown. This is the standard "simple, fast, slightly wasteful" choice
and is the right first version.

### 4.4 Where the free-list heads live — **per-arena, recommended**

The user's sketch put a free-list on each `ArenaBlock`. A per-block list forces
`arena_free(ptr, size)` to first find *which block* `ptr` belongs to (to reach
that block's list head) — which means either an O(blocks) address-range search
on every free, or a per-object back-pointer (a header we are avoiding).

Recommendation: keep the free-list **per-arena**. `arena_free` then never needs
to know the owning block; it just pushes onto the size-class head. Store the
class heads via one currently-reserved arena-state slot pointing at a small
`binHeads[]` array that is itself lazily allocated from the arena on first free.
Arena-state has reserved 8-byte slots at offsets 8/16/24/40/48/56 available for
this.

> Open decision (see §9): per-arena segregated lists (recommended) vs. the
> literal per-block list. The per-arena form is simpler and faster here; the
> per-block form only helps if we later want per-block reclamation, which the
> arena never does.

### 4.5 Allocation algorithm with reuse

`arena_alloc(size, align)` becomes:

1. Validate alignment (unchanged).
2. Normalize + round `size` to its size class.
3. If `align ≤ 16` and the class's free-list is non-empty → **pop and return**
   (O(1) reuse). Reused chunks already satisfy 16-byte alignment (§4.4).
4. Else fall through to the existing bump path (advance `bumpOffset`, growing a
   new block if needed).

Reuse-first keeps the live footprint small and slows block growth. Requests
needing alignment > 16 skip the free-list and bump (rare: scalars are ≤ 8-align,
records 8, collections 8, strings 1).

`arena_free(ptr, size)`:

1. Normalize + round `size` to its class.
2. Entropy-fill the chunk (§6), then write the class head into `[ptr]` as `next`
   and set the class head to `ptr`.

`arena_free` **never unmaps** and never touches a block header — it is pure
internal bookkeeping (fill + one push). Mapped memory is only ever returned to
the OS by `arena_destroy` at teardown. Both helpers document their clobber set
the way `arena_alloc` does today.

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

Introducing frees introduces the classic failure modes if drop emission is
wrong. Mitigations:

- Phase the rollout (see §8): start with **runtime-internal** free sites whose
  lifetimes are trivially correct and need *no* user-level analysis — the old
  buffer on collection grow/realloc, the discarded data region on compaction,
  throwaway string temporaries (number→string, interpolation) that are not
  returned. These reuse churned memory immediately with near-zero risk.
- Only then enable user value scope-drop frees, gated behind the existing
  ownership/escape analysis and a thorough test pass.
- Use entropy poisoning (§6) in debug builds as a use-after-free tripwire.

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

- `memory_layouts.md` Arena section: document the per-arena free-list (size
  classes, `FreeNode` overlay, the bin-heads slot in arena-state), the reuse-first
  branch in `arena_alloc`, and `arena_free(ptr, size)` with its result/clobber
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

1. **Zero-init audit.** Make every allocation site fully initialize what it
   reads; remove all reliance on `MAP_ANON` zeroing. Land independently — it is
   correct and valuable on its own.
2. **`arena_free` + per-arena segregated free-list + reuse-first `arena_alloc`.**
   No callers yet beyond tests.
3. **Runtime-internal free sites.** Collection grow/realloc, compaction,
   throwaway string temporaries call `arena_free`. First real reuse; minimal
   risk.
4. **Entropy fill.** `arena_fill_random` from the per-arena PCG64; fill on free
   and on new block; debug-default toggle. (Depends on Phase 1.)
5. **Scope-drop drop glue.** Per-type drop glue; emit frees at scope-drop for
   owned, non-escaping arena values, gated by existing escape analysis. The
   payoff phase and the riskiest — land behind heavy tests.

## 9. Decisions

Resolved:

- **Free-list placement:** per-arena segregated lists. (§4.4)
- **OS reuse:** none — freeing is internal reuse only; blocks unmap only at
  teardown. (§1, §4.5)
- **Fill source:** a dedicated per-arena PCG64 reusing reserved arena-state
  offsets 16/24, seeded independently so it never perturbs the `math::rand`
  stream at 88/96; `ARENA_STATE_SIZE` stays 104. (§6.1)
- **Fill default:** always on, debug and release. (§6.5)

Still open:

- **Size-class scheme:** 16-byte granules then power-of-two, vs. pure
  power-of-two, vs. exact-size lists. (§4.3)
- **Coalescing:** ship without it first (recommended); revisit if fragmentation
  shows up in real workloads.

## 10. Summary

The free-list, sized free, and entropy fill are straightforward, low-risk
runtime work and preserve every layout. The real engineering is (1) the
zero-init audit that entropy fill forces, and (2) deterministic scope-drop drop
glue with airtight escape analysis. The language, copying, and transfer
semantics are untouched throughout; this is purely "reuse freed memory and
poison/scrub it," delivered the deterministic, GC-free way the rest of the
language already cleans up resources.
