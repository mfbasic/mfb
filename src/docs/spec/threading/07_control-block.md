# Control Block

The native thread handle points to a runtime control block. The native layout
is an implementation ABI between helper lowering and generated code. The
block is 128 bytes (`THREAD_BLOCK_SIZE`):

```text
offset  field
0       state
8       cancelled
16      result tag
24      result value
32      result error
40      inbound queue handle          (data plane, parent -> worker)
48      outbound queue handle         (data plane, worker -> parent)
56      OS handle                     (pthread_t)
64      entry function pointer        (closure)
72      input data
80      worker arena state
88      parent arena state
96      result error source           (ErrorLoc origin pointer)
104     resource inbound queue handle (resource plane, parent -> worker)
112     resource outbound queue handle (resource plane, worker -> parent)
120     owners                        (parent bindings holding the handle)
```
[[src/codegen/runtime/thread/runtime_helpers.rs:THREAD_BLOCK_SIZE]]

`state = 0` means running (`THREAD_STATE_RUNNING`). `state = 1` means complete
with an unretrieved result (`THREAD_STATE_COMPLETED`). `state = 2` means the
parent `Thread` handle is closed because the result was retrieved or the handle
was dropped (`THREAD_STATE_CLOSED`). [[src/codegen/runtime/thread/runtime_helpers.rs:THREAD_STATE_RUNNING]]

The `result tag`, `result value`, and `result error` fields describe the
completed `Result OF Out`. `result error source` (offset 96) holds the
`ErrorLoc` origin pointer of a worker's terminal error, captured by the
trampoline so `thread::waitFor` can recover the worker's source location (see
`error-propagation`). Heap-backed success or error payloads stored through these
fields must either be runtime-owned transfer values, values materialized into a
receiver-valid arena, or values whose producer arena is kept live by the control
block until the one result retrieval materializes its receiver-owned copy.

`worker arena state` (offset 80) is the arena the trampoline pins for the worker
and the one `thread::openStdIn` subscribes; `parent arena state` (offset 88) records
the spawning thread's arena. Neither is used to allocate a boundary value into the
*other* side's arena: a message is copied in the sender's own arena and handed
across (see `queue-semantics`, bug-498).

## Lifetime

`thread::start` carves the control block, the worker arena-state block (arena state
plus the program's writable globals, `worker_arena_state_size`) and the four queue
records with their value rings out of the *spawning* thread's arena. They are freed
exactly once, by `thread.drop`, and only when both of these hold:

- **No binding still holds the handle.** The language lets a handle be read after it
  was dropped or passed to a function (the op answers `ErrResourceClosed`), and lets
  one handle be bound under two names, so no single drop knows it is the last reader.
  `owners` (offset 120) counts the parent bindings holding it. `thread::start` sets 1.
  A binding or `MUT` assignment from another handle binding adds 1 (an assignment
  adds it before it drops the old handle), and a function parameter adds 1 on entry.
  Each binding's final drop gives one back (`THREAD_DROP_RELEASE`); a `RETURN` hands
  the callee's count to the caller's binding instead. A handle passed to a function
  is closed by the callee's parameter and released by both bindings. A trap route to
  the function's `TRAP` handler only closes (`THREAD_DROP_CLOSE`) a handle the
  handler can name; the handler's own exit releases it.
- **The worker has been joined.** The trampoline still unlocks the outbound mutex
  after publishing `COMPLETED`, and a live worker's pinned arena and current-thread
  registers point into these blocks. `thread::waitFor` joins the worker (instead of
  detaching it) and zeroes `OS handle` (offset 56); a closing drop that finds the
  worker `COMPLETED` joins it the same way.

A closing drop of a still-`RUNNING` worker cancels and detaches it; its plumbing then
stays live for the rest of the process (see `os-integration`). The `TRAP`-path closed
handle (bug-479) is a zeroed block with null queues and `owners` 1; it is counted like
any handle, and the release that takes it to 0 frees just the block. The worker arena's
own chunks are never reclaimed. The cleanup call nulls the binding's slot after a
release, so no later exit edge releases the same binding twice.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:emit_release_thread_plumbing]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_thread_cleanup_call]]
[[src/codegen/cleanup/thread/builder_thread_cleanup.rs:emit_thread_owner_increment]]

## Plane queues

There are four queue handle fields — two for the data plane (offsets 40/48) and
two for the resource plane (offsets 104/112). Each plane is split by direction so
a thread's own send is never re-read by its own receive: the inbound queue carries
parent→worker traffic and the outbound queue carries worker→parent traffic. The
resource plane is fully independent of the data plane, so a thread can carry both
at once.

The queue handle fields point to runtime-owned bounded queue records, not directly
to a single queued message. The source-level contract is bounded queues with the
behavior specified by the `queue-semantics` topic
(`./mfb spec threading queue-semantics`); implementation changes must preserve
that contract.

## Queue record layout

Each queue record is 272 bytes (`THREAD_QUEUE_BLOCK_SIZE`):

```text
offset  field
0       pthread_mutex_t               (guards the record)
64      pthread_cond_t not_empty      (signalled on enqueue / close)
128     pthread_cond_t not_full       (signalled on dequeue / close)
192     capacity                      (requested limit)
200     count                         (current occupancy)
208     head index
216     tail index
224     closed flag
232     values pointer                (ring buffer of capacity * 32-byte entries)
240     pending-free list head        (blocks the SENDER still has to reclaim)
248     last-read block pointer       (retired on this queue's next read)
256     last-read block size          (0 = not reclaimable)
264     last-read STATE block size    (0 = the message has no STATE block)
```
[[src/codegen/runtime/thread/runtime_helpers.rs:THREAD_QUEUE_BLOCK_SIZE]]

The mutex and both condition variables are `pthread_*_init`-ed when the queue is
allocated in `thread::start`. The values pointer is a separately arena-allocated
ring of `capacity` thirty-two-byte entries, each
`{value, size, state_size, _}`: the enqueued value (a pointer for a block-shaped
message, the scalar itself otherwise), the byte size the sender computed for it, and
the byte size of the STATE block hanging off it — so the reader can hand every block
back for reclamation without knowing its type. `size` is `0` when the message has no
reclaimable block of a size the sender computes; `state_size` is `0` for every
data-plane message and for a bare `RES`. The fourth word is padding: the entry is 32
rather than 24 bytes so an index-to-offset conversion stays a single shift.

The only message types whose `size` the sender declines to compute are the scalars
(`Boolean`, `Byte`, `Fixed`, `Float`, `Integer`, `Money`), and a scalar message
carves no block at all — so the `0` sentinel never strands a reclaimable block.

### Reclaiming a queued message copy

A send deep-copies its message into the SENDER's own arena and hands that block
across; `thread::receive` / `thread::accept` copy it AGAIN into the reader's arena,
so once the reader's copy is made the queued block has no owner. Two fields make
that block reclaimable without any thread allocating in, or freeing into, another
thread's arena:

* the reader stores each block it is handed in the **last-read** fields, and the
  NEXT read on that queue pushes it onto the **pending-free list** (under the queue
  mutex, reusing the dead block's own words as `{next, size, state_size}`). One read
  behind is the earliest safe point: the caller's copy of block *N* is complete
  before that thread asks for block *N+1*, and each queue has exactly one reader.
  The `state_size` word is written and read on the two **resource** queues only,
  where every block is one `RESOURCE_RECORD_SIZE` record and the third word is
  therefore in range; a data-plane block can be shorter than 24 bytes (a `String`
  block is `len + 9`). The STATE **pointer** is never stored — it is read back from
  the record's own `RESOURCE_OFFSET_STATE`, which those three words do not reach.
* the **sender** drains the pending-free list — at the top of its next write, and,
  for the queues the spawning thread sends into, when the handle's plumbing is
  released. The sender is the thread whose arena carved every block on the list, and
  a free pushes onto the FREEING thread's own bins, so draining on the reader would
  return the sender's memory to a heap that is never reused.

A failed send's orphaned copy joins the same list from the write helper's failure
path.

A message that is still in the ring when the handle's plumbing is released — sent but
never received — is not reachable through either field, because no read ever handed it
out. The release walks the ring's live window (`count` entries from `head`, the same
window a read dequeues from) and frees each entry directly, on the inbound queues
only, by the same ownership rule that governs the drain.

A transferred **stateful** resource has two blocks, the record and its STATE, and the
receiver gets a deep copy of both — so both of the sender's are reclaimed: the queued
pair through `state_size` above, and the sender's own tombstone STATE block at the
binding's drop, alongside the tombstone record.
[[src/codegen/runtime/thread/runtime_helpers.rs:THREAD_QUEUE_PENDING_FREE_OFFSET]]
[[src/codegen/runtime/thread/runtime_helpers.rs:THREAD_QUEUE_LAST_READ_PTR_OFFSET]]

Queue storage must preserve enough type metadata to drop or close queued values
without receiving them. For queued resource handles, the runtime uses the
resource close function recorded in package metadata. For queued composite
values, the runtime uses the type metadata table to walk owned fields or payloads
that require cleanup.

## See Also

* ./mfb spec threading thread-runtime-helpers — the helpers that read and write this block
* ./mfb spec threading queue-semantics — the inbound and outbound queue handles it holds
* ./mfb spec threading error-propagation — the result tag/value/error fields it stores
