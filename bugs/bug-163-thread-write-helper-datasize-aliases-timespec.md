# bug-163 — thread write-helper `DATA_SIZE_OFFSET` stack slot aliases the deadline timespec → wrong-size `arena_free` (arena corruption)

Last updated: 2026-07-12
Severity: HIGH — arena free-list corruption on a failed timed thread send with a heap message.
Class: Memory-safety.
Status: Open

## Finding

`src/target/shared/code/runtime_helpers_thread.rs` (`thread_queue_write_helper`).
The frame lays out `TIMESPEC_OFFSET = 40` (:750) and `DATA_SIZE_OFFSET = 48`
(:753). A `struct timespec` is 16 bytes, so the deadline occupies **[40, 56)**:
`emit_thread_deadline(.., TIMESPEC_OFFSET=40)` (invoked at :781-786) writes
tv_sec at sp+40 and tv_nsec at sp+**48** (see the helper at :54-55, storing at
`timespec_stack_offset` and `timespec_stack_offset + 8`), and `clock_gettime`
itself writes all 16 bytes. sp+48 is exactly `DATA_SIZE_OFFSET`. The message-copy
size stored at entry (:771, `store ARG[3] → [sp,48]`) is therefore overwritten by
the deadline's nanoseconds before it is used. On a failed send the orphan-push
path (bug-147.5b) reloads the size at :948 (`load %v10, [sp,DATA_SIZE_OFFSET]`)
and pushes the message copy onto the queue's pending-free list with this garbage
size (:955); the destination thread later `arena_free(copy, garbage_size)` in its
drain loop (`thread_queue_read_helper`, ~:1130) — a wrong-size free that corrupts
the arena free list. The read helper puts TIMESPEC at 56 with no size field
(:1048), so it does not collide; this is unique to the write helper.

## Trigger

A blocking `thread::send`/`emit`/`transfer` (any `thread_queue_write_helper`
path) with `timeoutMs > 0`, a heap message (non-zero copy size in ARG[3]), a full
destination queue, and a send that then FAILS (times out, or the peer
closes/cancels mid-wait). The orphaned message copy is freed with a garbage size
→ arena corruption (delayed, size-threshold-dependent crashes).

## Fix

Move `DATA_SIZE_OFFSET` off the timespec (e.g. to 56; `FRAME_SIZE = 80` has room
at 56/64/72) so the message-copy size survives the deadline computation. Add a
timed-send-to-full-queue-that-fails test with a heap message and assert no arena
corruption (e.g. via a subsequent alloc pattern).
