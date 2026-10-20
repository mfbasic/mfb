# Queue Semantics

Each thread has four bounded queues, two per plane, split by direction.

Data plane (copyable values):

- Inbound: parent sends with `thread::send(Thread, ...)` (lowers to
  `thread.send`); worker receives with `thread::receive(ThreadWorker, ...)`
  (lowers to `thread.receive`).
- Outbound: worker sends with `thread::send(ThreadWorker, ...)` (lowers to
  `thread.emit`); parent observes with `thread::poll` and reads with
  `thread::receive(Thread, ...)` (lowers to `thread.read`).

Resource plane (move-only resource handles):

- Inbound: parent transfers with `thread::transfer(Thread, ...)` (lowers to
  `thread.transferResource`); worker takes with `thread::accept(ThreadWorker, ...)`
  (lowers to `thread.acceptResource`).
- Outbound: worker transfers with `thread::transfer(ThreadWorker, ...)` (lowers to
  `thread.emitResource`); parent takes with `thread::accept(Thread, ...)` (lowers
  to `thread.readResource`).

The resource plane carries resource handles only and the data plane is kept
resource-free; the two planes are independent queues so a thread can use both
concurrently.

`thread::start` rejects queue limits below `1`.

## Arena materialization

No thread allocates in another thread's arena — an arena's free-list pop is
unsynchronized. Every value that crosses is deep-copied into the thread's **block
arena** (`./mfb spec threading control-block`), which belongs to neither thread and
serializes allocations and drops under the block lock. Reads of retained
messages for call-site materialization happen after the helper unlocks:

- **Send** (`thread.send`/`emit`/`transferResource`/`emitResource`): the call site
  passes the sender's value and its type's copy and drop functions. The write helper
  waits for room, then — with the enqueue now certain — copies the value into the
  block arena and enqueues the copy, recording the drop function on the queue. A
  send that fails (full, closed, cancelled, timed out, invalid timeout) never made a
  copy, so it leaves nothing behind and the sender's value is untouched.
- **Read** (`thread.receive`/`read`/`acceptResource`/`readResource`): the helper
  dequeues the block-arena copy and returns it; the call site copies it into the
  reader's own arena, and that copy is the value user code sees. The queue keeps
  the block-arena message until the NEXT read on the same queue, which drops it
  with the drop function the call site passes — one read behind is the earliest
  safe point, because the call site's copy runs after the helper returns, and each
  queue has exactly one reader.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_queue_write_helper]]
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_queue_read_helper]]
[[src/codegen/cleanup/thread/builder_thread_cleanup.rs:emit_thread_send_runtime_helper_call]]

A resource's copy is its record (and a deep copy of its `STATE`); copying it flags
the source record `moved|closed`, so the sender's binding cannot act on a handle
that has left. Because the copy happens only when the enqueue is certain, the flag
is set exactly when the transfer succeeds.

## Timeouts

All four waits obey the language **timeout convention** (see
`./mfb spec language builtin-functions` → "Timeout convention"): **omitting**
`timeoutMs` blocks unboundedly, `0` is one immediate attempt, a positive value
waits up to that many milliseconds. Negative values are `ErrInvalidArgument`
except for the runtime's reserved `i64::MIN` sentinel described below.

For `thread::send` and `thread::transfer` (producing calls), omitting `timeoutMs`
**blocks** until the destination queue has space or it/the thread closes; `0`
enqueues if there is room and otherwise fails at once with `ErrTimeout`; a positive
timeout waits up to that many milliseconds then fails with `ErrTimeout`.

`thread::receive` and `thread::accept` take **two forms** (identical on the parent
`Thread` and worker `ThreadWorker` handle). Omitting `timeoutMs` **blocks**: the
call waits until a message/resource arrives, the queue is closed, or the worker is
cancelled. Supplying `timeoutMs` requires a value `>= 0`: `0` is one immediate
attempt (fails with `ErrTimeout` when the still-open queue is empty) and a positive
value waits up to that many milliseconds (`ErrTimeout` on expiry). The
source-level spelling for an indefinite wait is to omit the argument. An empty worker-side read that observes cancellation fails
with `ErrInterrupted`; a blocking parent read wakes with `ErrNotFound` when the
worker completes or closes its outbound queue, so it never waits forever —
`ErrNotFound` marks a *terminally* empty queue, distinct from an unmet deadline's
`ErrTimeout`. (Internally the no-timeout form is lowered by padding the missing
`timeoutMs` with the unbounded block sentinel, `i64::MIN`; each queue helper waits
indefinitely on exactly that value and rejects every other negative timeout.
The helper receives no argument-presence flag, so an explicitly supplied
`i64::MIN` also takes the indefinite path.)
[[src/codegen/engine/value/builder_values.rs:lower_runtime_helper_call]]
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_queue_read_helper]]
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_queue_write_helper]]

## Copy and move semantics

`thread::start` and data-plane `thread::send` retain the caller's original
value. The block arena owns the copy placed on the queue. `thread::transfer`
instead moves a resource: on success the source record is marked moved/closed;
on failure it remains the sender's. An inline `TRAP` can therefore use the
resource on the failure path. A subsequent operation through a moved alias
reports `ErrResourceMoved`.
[[src/codegen/cleanup/thread/builder_thread_cleanup.rs:claim_moved_thread_arg_temp]]
[[src/codegen/memory/arena/builder_arena_transfer/transfer.rs:copy_resource_to_current_arena]]

A read materializes the returned value in the receiver's arena. For a resource,
this copies the record and its `STATE` and marks the queued record moved/closed;
it does not duplicate ownership of the OS handle.
[[src/codegen/memory/arena/builder_arena_transfer/transfer.rs:copy_resource_to_current_arena]]

## Cancellation and readiness

Cancellation is cooperative:

- `thread::cancel` sets the cancellation flag.
- New sends fail after cancellation is requested.
- The worker observes cancellation with `thread::isCancelled(t)`.
- Runtime-managed blocking cancellation points wake and fail with
  `ErrInterrupted` when cancellation is requested for their worker thread.
- The runtime does not forcibly kill the worker as normal cancellation behavior.

Queue writes check cancellation before enqueueing, even when space is available.
Queue reads check occupancy first: a queued message or resource is returned even
if cancellation has been requested. Once a worker's queue is empty, cancellation
produces `ErrInterrupted`; an empty closed queue on the parent side produces
`ErrNotFound`. Cancellation closes all four queues and broadcasts their waiters,
but does not discard their queued entries.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_queue_write_helper]]
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:thread_queue_read_helper]]
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:simple_thread_handle_helper]]

A positive `os::sleep` in a worker is also a cancellation point; zero is a no-op.
The implementation is specified by `./mfb spec threading thread-runtime-helpers`.
Normal `TRAP` and auto-propagation behavior applies to `ErrInterrupted`.

`thread::poll(t, ms)` observes only the outbound data queue and never dequeues.
It requires `ms >= 0`, returns `TRUE` when data is queued, and returns `FALSE`
when an empty queue's worker is completed or the wait expires. Zero checks once.
It does not inspect the cancellation flag or queue-closed flag: cancelling a
still-running worker does not by itself end a positive poll's wait.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:simple_thread_handle_helper]]

Cancellation does not interrupt arbitrary user code, does not asynchronously
terminate the OS thread, and does not unwind out of foreign/native code that has
not registered a cancellation point. A worker in non-blocking computation must
still check `thread::isCancelled(t)` or call a cancellation-point operation to
observe the request.

There is no source `thread::stop` or `thread::detach` operation. Dropping a
running parent view requests cancellation and detaches its OS worker.

## Cleanup

A parent `Thread` view is a resource, so the compiler drops it through the
ordinary resource cleanup: scope exit, `RETURN`, `FAIL`, `PROPAGATE`,
auto-propagated errors, and trap routing run its close op, the drop helper
(`thread.drop`, `_mfb_rt_thread_thread_drop`), in reverse declaration order, and
then free the 112-byte view record. The drop helper closes the view first, then
marks the control block `CLOSED` (state 2), sets the cancellation flag,
closes all four queues and broadcasts their waiters. It also resets the outbound
data queue's count, head, and tail, discarding its unread data; those blocks
remain in the block arena until destruction. Other queued entries remain recorded.
A worker still running is then `pthread_detach`ed; a worker that had already
completed is `pthread_join`ed, unless a `thread::waitFor` already joined it. Last
it gives up the handle side's reference on the control block, which is freed by
whichever side lets go last (`control-block` § Lifetime). There is one parent view
per thread and one drop of it: a `RETURN` moves the view to the caller's binding,
`RES b = a` moves it to `b`, a successful `thread::transfer` moves it to the
receiving thread, and a view passed to a function is lent — the callee drops
nothing. A drop reached through a null slot (a binding its initializer never
wrote) or on a closed view does nothing.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:simple_thread_handle_helper]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:emit_resource_cleanup_call]]

A value produced by `thread::receive`, `thread::waitFor`, or `thread::accept`
is copied at the call site out of the block arena into the calling thread's
arena. That copy follows ordinary value/resource cleanup; the caller never frees
the block-arena original. Resource cleanup closes the handle and frees its owned
record and state, except for the runtime-owned embedded worker view.
[[src/codegen/engine/value/builder_values.rs:runtime_call_result_is_copied_at_call_site]]
[[src/codegen/resource/cleanup/builder_resource_cleanup.rs:resource_record_freed_at_drop]]

When the worker completes:

- Inbound sends fail.
- The result is copied into the block arena and stored (a worker error's message
  and `ErrorLoc` are copied the same way), so it outlives the worker.
- `thread::isRunning` returns `FALSE`.
- `thread::waitFor` returns or propagates the stored result, every time it is
  called; it closes nothing.
- Remaining outbound messages stay readable until drained.

On final release, entries still recorded in the queues are dropped: the release
runs each queue's recorded drop function on every message still in its ring and
its retained last-read value, before the block arena is destroyed. A resource's
drop closes a record that is still open, so a transfer nobody accepted does not
leak its OS handle. Discarded outbound data needs no resource close and is
reclaimed with the block arena. Resource queues retain their entries until receipt or final release.
[[src/codegen/runtime/thread/control_block.rs:lower_thread_block_release]]

A worker's own arena is unmapped by the worker itself as the last step of its run
(`_mfb_rt_thread_finish`, after it has published its result): nothing another
thread can read points into it, because the result and every message are
block-arena copies, and the worker's own scope drops have already run. Its buffered
standard output is drained on exit even without an explicit `io::flush`; this
is best-effort, and a write failure cannot be reported after the worker returns. [[src/codegen/runtime/thread/runtime_helpers_thread.rs:lower_thread_finish]]

## See Also

* ./mfb spec language memory-semantics — the general lexical scope-drop value-cleanup rule
* ./mfb spec threading control-block — the queue record and control-block layout
