# Control Block

The native thread handle points to a runtime control block. The native layout
is an implementation ABI between helper lowering and generated code. The
block is 120 bytes (`THREAD_BLOCK_SIZE`):

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
records with their value rings out of the *spawning* thread's arena. The parent
`Thread` binding owns them all, and they are freed exactly once, by the parent's
`thread.drop`, and only after the worker has been **joined** — the trampoline still
unlocks the outbound mutex after publishing `COMPLETED`, and a live worker's pinned
arena and current-thread registers point into these blocks. The join happens in one
of two places:

- `thread::waitFor` joins the worker (instead of detaching it) once the result is
  ready, and zeroes `OS handle` (offset 56). A `CLOSED` block with a zero OS handle
  and non-null queues is therefore a joined, retrieved thread, and the drop that
  follows frees its plumbing. The `TRAP`-path closed handle (bug-479) has null
  queues, so it is never taken for one.
- `thread.drop` of a handle whose worker is already `COMPLETED` joins it itself and
  frees the same blocks.

A drop of a still-`RUNNING` worker cancels and detaches it; its plumbing stays live
for the rest of the process (see `os-integration`). The worker arena's own chunks
are never reclaimed by either path. The cleanup call nulls the binding's slot after
the drop, so no later exit edge hands the freed block over again.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:emit_release_thread_plumbing]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_thread_cleanup_call]]

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

Each queue record is 240 bytes (`THREAD_QUEUE_BLOCK_SIZE`):

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
232     values pointer                (ring buffer of capacity * 8-byte slots)
```
[[src/codegen/runtime/thread/runtime_helpers.rs:THREAD_QUEUE_BLOCK_SIZE]]

The mutex and both condition variables are `pthread_*_init`-ed when the queue is
allocated in `thread::start`. The values pointer is a separately arena-allocated
ring buffer of `capacity` eight-byte slots.

Queue storage must preserve enough type metadata to drop or close queued values
without receiving them. For queued resource handles, the runtime uses the
resource close function recorded in package metadata. For queued composite
values, the runtime uses the type metadata table to walk owned fields or payloads
that require cleanup.

## See Also

* ./mfb spec threading thread-runtime-helpers — the helpers that read and write this block
* ./mfb spec threading queue-semantics — the inbound and outbound queue handles it holds
* ./mfb spec threading error-propagation — the result tag/value/error fields it stores
