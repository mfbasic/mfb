# Control Block

The native thread handle points to a runtime control block. The current native
layout is an implementation ABI between helper lowering and generated code. The
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
[[src/target/shared/code/runtime_helpers.rs:THREAD_BLOCK_SIZE]]

`state = 0` means running (`THREAD_STATE_RUNNING`). `state = 1` means complete
with an unretrieved result (`THREAD_STATE_COMPLETED`). `state = 2` means the
parent `Thread` handle is closed because the result was retrieved or the handle
was dropped (`THREAD_STATE_CLOSED`). [[src/target/shared/code/runtime_helpers.rs:THREAD_STATE_RUNNING]]

The `result tag`, `result value`, and `result error` fields describe the
completed `Result OF Out`. `result error source` (offset 96) holds the
`ErrorLoc` origin pointer of a worker's terminal error, captured by the
trampoline so `thread::waitFor` can recover the worker's source location (see
`error-propagation`). Heap-backed success or error payloads stored through these
fields must either be runtime-owned transfer values, values materialized into a
receiver-valid arena, or values whose producer arena is kept live by the control
block until the one result retrieval materializes its receiver-owned copy.

`worker arena state` and `parent arena state` let either side materialize a
boundary value into the *receiving* side's arena: a worker→parent send loads the
parent arena state from offset 88; every other copy uses the worker arena state at
offset 80 (see `queue-semantics`).

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
[[src/target/shared/code/runtime_helpers.rs:THREAD_QUEUE_BLOCK_SIZE]]

The mutex and both condition variables are `pthread_*_init`-ed when the queue is
allocated in `thread::start`. The values pointer is a separately arena-allocated
ring buffer of `capacity` eight-byte slots.

Queue storage must preserve enough type metadata to drop or close queued values
without receiving them. For queued resource handles, the runtime uses the
resource close function recorded in package metadata. For queued composite
values, the runtime uses the type metadata table to walk owned fields or payloads
that require cleanup.
