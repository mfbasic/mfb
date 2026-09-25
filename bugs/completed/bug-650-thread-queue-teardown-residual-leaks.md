# bug-650: three thread-queue blocks that bug-646's reclaim still cannot reach

Last updated: 2026-09-19
Effort: medium (1h–2h) — **actual: large**, once case 2 turned out to be two leaks
Severity: LOW-MEDIUM (each is bounded per queue or per undelivered message, not per hand-over)
Class: Correctness (memory)

Status: Fixed
Regression Test: tests/runtime/rt_debug_soak.rs
(`an_undelivered_queued_message_is_freed_when_the_thread_is_released`,
`a_stateful_resource_transfer_frees_its_record_and_its_state`)

## STATUS: FIXED (48f2d55ff)

**Two of the three were real; the third is not a defect.** All three were reproduced
first, which is what the doc asked for and what changed the answer on case 3.

- **Case 1 — an undelivered message: REAL, fixed.** 96 B per iteration (three
  undelivered 32 B messages), `live_bytes` 4,800 → 9,600 at N=50/100.
  `emit_release_thread_plumbing` now walks the ring's live window — `count` entries
  from `head`, the same window `thread_queue_read_helper` dequeues from — and frees
  each entry, on the INBOUND queues only, which is the ownership rule the
  pending-free drain it sits inside already follows.
- **Case 2 — a stateful resource's STATE block: REAL, fixed, and it was TWO leaks,
  not one.** 128 B per transfer, `live_bytes` 6,400 → 12,800 at N=50/100. The doc
  named only the queued copy; fixing that took it to 16 B per transfer, not 0, and
  the residual was a second, unrelated mechanism. See "Case 2 was two leaks" below.
- **Case 3 — a message whose copy size is not computable: NOT A DEFECT.** The doc
  said "which types those are is not yet enumerated"; enumerating them is what
  closed it. See "Case 3: enumerated, and empty" below.

Every shape reports `live_bytes 0` with `free_calls == alloc_calls` exactly and
`double_free_skips 0`, and the bare-transfer control (bug-646's, which was already
flat) stays flat.

### Case 2 was two leaks

1. **The queued copy's record AND its STATE** — the mechanism this doc describes.
   `bare_resource_reclaimable` declined a stateful resource outright because the
   pending-free entry's single size could not describe record + STATE, so *both*
   leaked. The ring entry is now `{value, size, state_size, _}` (16 → 32 B; 32 not 24
   so the index-to-offset conversion stays one shift), and the send helper passes the
   STATE size as arg 4, sized by the same `emit_inlined_block_size_from_ptr_slot` the
   data plane uses — so a variable-length STATE (a record with an inlined `String`) is
   sized exactly rather than assumed constant per queue, which an earlier
   per-queue-constant design would have got wrong.

   Only the STATE **size** needs carrying. The **pointer** is read back from the
   record's own `RESOURCE_OFFSET_STATE` (+24), which a pending-free node's
   `{next, size, state_size}` words (+0, +8, +16) do not reach. The third word is
   written and read on the **resource planes only**: a data-plane block can be
   smaller than 24 B (a short `String` is `len + 9`), so +16 is not in range there.

2. **The SENDER's own STATE block** — not in this doc, found by measurement when the
   fix above left 16 B per transfer instead of 0.
   `emit_moved_resource_record_free` frees a moved tombstone's 96 B record but never
   what it points at. Its guarding comment assumed the receiver takes the sender's
   STATE payload; since bug-257 it does not — `copy_resource_to_current_arena`
   deep-copies the STATE into the RECEIVER's arena ("never an alias into the sender's
   arena"), so after a completed move the sender's block has no owner at all. Freed
   there now, before the record, while the pointer is still readable. Same class of
   stale-premise ownership bug as bug-629 Part A.

### Case 3: enumerated, and empty

The doc asked for the set of message types that make `size_computable` false. Measured
by instrumenting that predicate and compiling one `thread::send` per type
(`String`, a record, `List OF Integer`, `Map OF String TO Integer`, `Set OF String`, a
data union, `AttributedString`, `List OF Point`, `Boolean`, `Byte`, `Integer`, `Float`,
`Fixed`, `Money`). The compiler declined to size exactly six:

    Boolean  Byte  Fixed  Float  Integer  Money

— every one a **scalar**, which carves no block at all, so there is nothing to
reclaim and nothing leaks. Every message type that DOES allocate a block is sized.
`AttributedString` was the predicted counter-example and is not one: it is sized, and
measured flat (`live_bytes 13,520` at both N=50 and N=100).

So the size-0 fail-safe is correct and unreachable as a leak. **No test was added for
case 3** — there is no failing behaviour to pin — and the fail-safe stays exactly as
it is, per this doc's own non-goal.

### Found along the way, filed not fixed

The case-3 sweep compiled a program per message type and turned up two build failures
unrelated to queue teardown, both verified on a clean main-tip compiler: **bug-656**
(an `ENUM` thread message type fails codegen with "native inlined field size not
available") and **bug-657** (a `UNION` with a scalar member passes the verifier and
dies with "union wrap member is not a record", though the rule that should catch it
already exists).

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

- [x] A soak reproduction for each of the three; confirm RED. Cases 1 and 2 reproduced as
      documented; **case 3 did not reproduce**, and the enumeration below is why.
- [x] Enumerate which message types make `size_computable` false — exactly the six
      scalars, none of which carves a block.

Commit: 9562683f9 (the two RED cases; case 3 gets none, having none to pin)

### Phase 2 — the fix

- [x] Case 1: the release-time ring walk.
- [x] Case 2: the ring entry's third word + arg 4, and the moved tombstone's STATE free.
- [x] Case 3: no change — the fail-safe is correct and unreachable.

Commit: 48f2d55ff

### Phase 3 — full validation

- [x] Full suite; artifact gate (the ring layout moves the thread goldens — unlike
      bug-629's fix, which moved none).

Commit: (see the merge commit)

## Summary

The reclaim protocol reaches a block only if it is read and singly-sized. Two of these
three were genuinely outside it and are now reached; the third was never outside it —
the only unsized messages are scalars, which have no block.
