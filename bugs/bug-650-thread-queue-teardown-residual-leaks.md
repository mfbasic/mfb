# bug-650: three thread-queue blocks that bug-646's reclaim still cannot reach

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: LOW-MEDIUM (each is bounded per queue or per undelivered message, not per hand-over)
Class: Correctness (memory)

Status: Open
Regression Test: none yet — see Phase 1

bug-646 made the sender reclaim the queue's copy of every message it hands over, which is the
per-operation leak. Three narrower cases are left, each measured or read out during that work
and deliberately out of its scope:

1. **An undelivered message.** A message still sitting in the ring when the queue closes is
   never freed — nothing walks the ring at release. Bounded by the queue's capacity, but a
   program that sends more than the worker reads leaks all of it.
2. **A stateful resource's STATE block.** `thread::transfer` of a `RES … STATE T` reclaims the
   record but not the separate STATE block: the pending-free entry carries ONE size (arg-3),
   which cannot describe record + STATE. Leaks bounded-until-teardown, as
   `.ai/resources-packages.md` already records.
3. **A message whose copy size is not computable.** When the send helper's
   `size_computable` is false it passes `0` as the size, and the reclaim skips the block
   rather than risk a wrong-size `arena_free`. Correct as a fail-safe, but the block leaks.

**The single correct behavior a fix produces:** each of the three shapes reports equal
`live_bytes` at N and 2N, with `double_free_skips 0` and no cross-arena free.

References:

- bug-646 (`e124b7eee`), whose reclaim protocol these fall outside of.
- `.ai/canvas-threading.md` §2 — a free lands on the FREEING thread's bins.
- `.ai/resources-packages.md`, "Thread resource plane split" and "Thread transfer move-flag is
  success-gated".

## Failing Reproduction

**Not yet written — none of the three is reproduced as a soak program yet.** Phase 1's first
job. Sketches:

1. Send more messages than the worker receives, then let the thread be released.
2. `thread::transfer` a `RES udp::Socket STATE Cursor` in a loop (bug-646's transfer soak with
   a STATE clause added).
3. Send a message type the helper declines to size; find one by instrumenting
   `size_computable`, since which types those are is not yet enumerated.

Each needs an N large enough that the growth clears `rt_debug_soak.rs`'s `BLOCK_BOUND` (4096).

## Root Cause

Read from the source during bug-646, to confirm per case. bug-646's protocol parks the block
a reader handed out on the queue's pending-free list one read behind, and the SENDER drains it
(inbound queues only — a parent draining outbound was measured to double-free,
`free_calls 884 > alloc_calls 725`). That protocol reaches only blocks that are (a) actually
read and (b) describable by a single size. Case 1 fails (a); cases 2 and 3 fail (b).

## Goal

- All three flat, with the sender still the only thread that frees its own blocks.

### Non-goals (must NOT change)

- bug-498: a sender must never allocate in the receiver's arena, nor a receiver free into the
  sender's.
- bug-646's inbound-only drain at release — the outbound variant was measured to double-free.
- The fail-safe skip when a size is not computable must stay a skip, not become a guess.

## Phases

### Phase 1 — failing tests + audit

- [ ] A soak reproduction for each of the three; confirm RED; enumerate which message types
      make `size_computable` false.

Commit: —

### Phase 2 — the fix

Commit: —

### Phase 3 — full validation

Commit: —

## Summary

The reclaim protocol reaches a block only if it is read and singly-sized; these three are
neither.
