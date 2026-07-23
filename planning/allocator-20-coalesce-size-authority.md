# allocator-20 — ROBUSTNESS: coalescing trusts the caller's free size (compiler-drift canary)

Last updated: 2026-07-02
Effort: small (<1h) — debug-only assertion
Severity: LOW (defense-in-depth; not user-reachable on its own)
Category: 2 (latent robustness) — NOT a standalone security issue

**Scope caveat (why this is not a security finding).** `arena_free` /
`arena_insert_free` are **not user-callable**, and their `size` argument is
**never user-supplied** — it is always compiler-emitted from static type-size
helpers, and for collections it is read back from the object's own live header
(which mutation keeps in sync). So no user program input can hand the allocator a
disagreeing free size directly. For this to bite you need one of two things to
*already* be true: (1) a **compiler codegen bug** where an alloc site and its
free site emit different size formulas for the same object, or (2) **prior heap
corruption** of a header word that feeds a free size — precisely the primitives
`allocator-02` (size-overflow) and `allocator-03` (double-free) close. It is
therefore second-order amplification of a bug that must already exist, not a
reachable vulnerability. The original audit (`audit-1` MEM-06) rated it MEDIUM
security; that is overstated for this caller model and is corrected here.

`arena_free` derives the freed extent **entirely from the caller-passed size
argument** (`round_up(size,16)`, `entry_and_arena.rs:1191-1198`) — it never reads
an authoritative granted-size back from a block/allocation header (by design:
live allocations carry no metadata). `arena_insert_free` then coalesces from bare
address adjacency (`prev_end == ptr`, `ptr+size == cur`) with **no check that
`[ptr, ptr+size)` doesn't overlap a live allocation** (`:1108-1148`). If a free
site ever passes a size that disagrees with what the matching alloc granted (case
1 or 2 above), the free node overlaps the following live chunk and the next
first-fit `arena_alloc` walk hands the overlap out. The point of this plan is to
make such **compiler drift fail loudly in testing** rather than silently overlap
in release — not to defend a runtime attack surface (there isn't one).

It complements:

- `./mfb spec memory arenas` (`04_arenas.md` — the "frees are compiler-sized"
  contract and the coalescing algorithm).
- `planning/audit-1-codegen-memory.md` MEM-06 (allocator-scoped form).
- `planning/allocator-01-arena-alloc-quadratic.md` — **direct tension**: that
  plan's non-goal forbids a per-allocation header ("live allocations carry no
  metadata"). The full fix here (a granted-size redzone) conflicts with it; this
  plan records the conflict and proposes an interim guard that respects it.

## 2. Current State

- Every free size is generated from a *static* type-size helper at the call site
  (`emit_flat_block_size`, union `size@8`, `capacity*ENTRY + dataCapacity`) — see
  `builder_collection_layout.rs`, `builder_arena_transfer.rs`. There is no shared
  runtime authority; alloc and free each recompute the size from the type.
- `arena_free` (`:1191-1198`) normalizes the *passed* size and hands it to
  `arena_insert_free`, which merges purely on adjacency (`:1108-1112` prev-side,
  `:1135-1143` next-side) with no overlap validation.
- Contrast: `arena_alloc`'s grow path *does* overflow-guard its size math; the
  free path performs no correctness check on the extent at all.

Any drift between an alloc formula and its free formula (a data-union whose
runtime `size@8` exceeds its block; a collection whose alloc/free size
expressions diverge; corruption from allocator-03) makes the freed extent span a
neighbor → overlap that the allocator cannot detect.

## 3. Design Overview

Because this is not a reachable attack surface, the goal is a **cheap canary that
catches compiler size-drift in testing**, not a release-path runtime guard that
pays cost to defend against our own bugs. One recommended option, two rejected:

- **(recommended) Debug-only assertion.** In debug builds, before coalescing,
  assert that the freed `[ptr, ptr+size)` lies wholly within one mmap block and
  does not begin inside an existing free node; trap on violation. Zero release
  cost; surfaces an alloc/free size-formula divergence the moment a test frees a
  mis-sized chunk, instead of it silently overlapping. This is the whole value of
  the finding under the real caller model.
- **(rejected) Release-path block-bounds clamp.** Clamping every coalesce to the
  containing block at runtime adds per-free cost on the hot path to defend
  against a case that can only arise from a compiler bug or an already-existing
  corruption primitive — a bad trade. Note it as considered-and-declined.
- **(rejected) One-word granted-size redzone.** Reading the granted size back
  would make alloc/free extents provably agree, but it adds a per-allocation
  header — **directly conflicting with allocator-01's "no per-allocation header"
  non-goal** — and shifts every value's data offset by 8 (ABI/layout change,
  golden churn). Not worth an ABI change to guard our own codegen; declined
  unless a concrete size-drift bug is ever found that testing can't otherwise
  catch.

## Layout / ABI Impact

None. The debug assertion reads block headers already present at
`base+0/8/16/24`; it is compiled out of release builds, so release `.ncode`/`.run`
are unchanged. (The rejected redzone would have been an ABI change — explicitly
not pursued.)

## Phases

### Phase 1 — debug-build free-extent assertion (small)

- [ ] `arena_insert_free` (`:1108-1148`) or `arena_free` (`:1199-1205`): behind a
      debug-only gate, assert `[ptr, ptr+size)` is contained in one block and does
      not start inside an existing free node; trap on violation. Confirm it is
      fully elided in release (no `.ncode`/`.run` change).
- [ ] Test: a targeted debug-build `tests/` case that frees a deliberately
      over-large size through an internal path and asserts the debug guard traps
      (the canary fires), while release output is unaffected.
- [ ] Confirm release goldens **unchanged** (assertion elided); no accept-suite
      diff in release.

Acceptance: debug build traps on a mis-sized free; release build byte-identical
(assertion compiled out); accept suite green.
Commit: —

## Summary

Under the real caller model — only the runtime calls `arena_free`, with
compiler-emitted, header-derived sizes — this is not a security issue and the
audit's MEDIUM rating is corrected. A bad free size can only come from a compiler
size-formula divergence or from corruption another primitive (allocator-02/03)
must first create. The proportionate response is a **debug-only assertion** that
turns such drift into a loud test failure; the release-path clamp and the
granted-size redzone both pay real cost (the redzone an ABI change that collides
with allocator-01) to defend our own codegen and are declined.
