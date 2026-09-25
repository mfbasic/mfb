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
is only ever touched under the block lock:

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

Timeouts differ between the writing side (`thread::send`, `thread::transfer`) and
the reading side (`thread::receive`, `thread::accept`).

All four waits obey the language **timeout convention** (see
`./mfb spec language builtin-functions` → "Timeout convention"): **omitting**
`timeoutMs` blocks unboundedly, `0` is one immediate attempt, a positive value
waits up to that many milliseconds, and a negative value is `ErrInvalidArgument`.

For `thread::send` and `thread::transfer` (producing calls), omitting `timeoutMs`
**blocks** until the destination queue has space or it/the thread closes; `0`
enqueues if there is room and otherwise fails at once with `ErrTimeout`; a positive
timeout waits up to that many milliseconds then fails with `ErrTimeout`.

`thread::receive` and `thread::accept` take **two forms** (identical on the parent
`Thread` and worker `ThreadWorker` handle). Omitting `timeoutMs` **blocks**: the
call waits until a message/resource arrives, the queue is closed, or the worker is
cancelled. Supplying `timeoutMs` requires a value `>= 0`: `0` is one immediate
attempt (fails with `ErrTimeout` when the still-open queue is empty) and a positive
value waits up to that many milliseconds (`ErrTimeout` on expiry). A negative
explicit `timeoutMs` is rejected with `ErrInvalidArgument` — to wait indefinitely,
omit the argument. A blocking read that observes cancellation of the worker fails
with `ErrInterrupted`; a blocking parent read wakes with `ErrNotFound` when the
worker completes or closes its outbound queue, so it never waits forever —
`ErrNotFound` marks a *terminally* empty queue, distinct from an unmet deadline's
`ErrTimeout`. (Internally the no-timeout form is lowered by padding the missing
`timeoutMs` with the unbounded block sentinel, `i64::MIN`; each queue helper waits
indefinitely on exactly that value and rejects every other negative timeout, so no
explicit value can request the indefinite wait.)

For `thread::send` and `thread::transfer`, ownership transfer is atomic with
enqueue success:

- If enqueue succeeds, the destination side owns the value immediately. While the
  value is queued, the destination queue owns its block-arena copy.
- If enqueue fails because the queue is full, closed, cancelled, timed out, or
  the timeout is invalid, ownership is not transferred and the sender still owns
  the value.
- Code may attach an inline `TRAP` to `thread::send(...)`/`thread::transfer(...)`
  to separate the success path, where the sent/transferred binding is moved, from
  the error handler, where it remains owned by the sender and can be released. The
  compiler treats the argument at index 1 of `thread.start`, `thread.send`, and
  `thread.transfer` as a move (`ExprMode::Transfer`). Any holder of a resource
  pointer may transfer it — a resource is owned by the outermost scope that touches
  it (§15.6) — and the transfer is tracked as a move, so a later use of the
  transferred name is a use-after-move. The receiving thread finds the record
  flagged `moved`, and any operation on it there reports `ErrResourceMoved` rather
  than acting on a handle that has left.
  [[src/ir/verify/resources.rs:check_resource_moves]]

Receiving a non-copyable value moves it out of the queue into the receiver's
binding. Receiving a copyable value may copy or move according to the normal
representation rules. In all cases, a heap-backed received value is materialized
in storage valid for the receiving thread before user code observes it.

Cancellation is cooperative:

- `thread::cancel` sets the cancellation flag.
- New sends fail after cancellation is requested.
- The worker observes cancellation with `thread::isCancelled(t)`.
- Runtime-managed blocking cancellation points wake and fail with
  `ErrInterrupted` when cancellation is requested for their worker thread.
- The runtime does not forcibly kill the worker as normal cancellation behavior.

Cancellation points are built-in operations whose implementations can safely
return an error without abandoning partially moved values or held runtime locks.
The runtime cancellation points are indefinitely blocking or timed waits
in the worker-side channel ops — `thread::receive`, `thread::send`,
`thread::accept`, and `thread::transfer` on a `ThreadWorker`. If cancellation is
already requested before a worker enters one of these operations, the operation
fails immediately with `ErrInterrupted`. If cancellation is requested while the
operation is blocked, the runtime wakes the wait and the operation fails with
`ErrInterrupted`. Other blocking built-ins that are implemented as
runtime-managed waits, such as terminal input, blocking file reads, or network
waits, must use the same cooperative error-return model when cancellation
integration is provided.
Normal `TRAP` and auto-propagation behavior then runs in the worker.

Cancellation does not interrupt arbitrary user code, does not asynchronously
terminate the OS thread, and does not unwind out of foreign/native code that has
not registered a cancellation point. A worker in non-blocking computation must
still check `thread::isCancelled(t)` or call a cancellation-point operation to
observe the request.

There is intentionally no `thread::stop()` operation. Asynchronous termination
can kill a worker while it owns a resource handle, holds a queue lock, is moving
a non-copyable value, is writing its result, or is inside package/native code.
That would make ownership and cleanup ambiguous and can leak resources, poison
queues, or deadlock other threads. Stopping work must happen at cooperative
cancellation points where the worker can return normally and the runtime can
close or transfer every owned value exactly once.

There is also no separate `thread::detach()` source API. Dropping a running
`Thread` already requests cancellation and detaches the OS worker for eventual
runtime cleanup. A public detach operation would need the same ownership and
cleanup guarantees as dropping the handle, while making it easier for user code
to abandon a worker that still owns resources or queued values.

The compiler lowers ordinary lexical ownership cleanup for every live parent
`Thread` handle. Scope exit, `RETURN`, `FAIL`, `PROPAGATE`, auto-propagated
errors, and trap routing run the same drop helper (`thread.drop`,
`_mfb_rt_thread_thread_drop`) in reverse declaration order. The drop helper marks
the control block `CLOSED` (state 2), sets the cancellation flag, closes and
clears both data queues (broadcasting their waiters) and wakes the resource plane.
A worker still running is then `pthread_detach`ed; a worker that had already
completed is `pthread_join`ed. Separately, each drop gives up its binding's owner
count, and only the drop that takes the count to 0 gives up the handle side's
reference on the control block, which is freed by whichever side lets go last
(`control-block` § Lifetime). Reassigning a `MUT Thread` evaluates the new value
first, then drops the old handle before storing the replacement. A handle returned
from a function moves to the caller's binding. A handle passed to a function is
closed by the callee's parameter but still released by the caller's binding, which
may read it afterwards. A trap route to the function's `TRAP` handler closes a
handle the handler can name without releasing it; the handler's own exit releases
it. The cleanup call nulls the binding's slot after a release, so a later exit edge
never releases the same binding twice.

The same scope-drop cleanup mechanism also frees ordinary owned **values** with
one `arena_free` each, in the same reverse order on the same exit paths — the
general lexical value-cleanup rule owned by `./mfb spec language memory-semantics`.
Two thread-specific exclusions apply, because those values are not plain blocks
this scope owns:

- **Thread-boundary results are the call site's copy.** A value produced by
  `thread::receive`, `thread::waitFor` or `thread::accept` is copied at the call
  site out of the block arena into the calling thread's arena, and that copy is
  owned and freed like any fresh value; the block-arena original stays the
  queue's (or the control block's) and is never freed by the caller.
  [[src/codegen/engine/value/builder_values.rs:runtime_call_result_is_copied_at_call_site]]
- **Resources** stay move-only handles closed by their own close op, never
  `arena_free`d, exactly as before.

When the worker completes:

- Inbound sends fail.
- The result is copied into the block arena and stored (a worker error's message
  and `ErrorLoc` are copied the same way), so it outlives the worker.
- `thread::isRunning` returns `FALSE`.
- `thread::waitFor` returns or propagates the stored result and closes the
  parent `Thread` handle.
- Remaining outbound messages stay readable until drained.

If a queued value is never received, it is dropped exactly once when the control
block is freed: the release runs each queue's recorded drop function on every
message still in its ring and on its last read's, before the block arena is
destroyed. A resource's drop closes a record that is still open, so a transfer
nobody accepted does not leak its OS handle. Until then — including after the handle
side drops a running thread — the messages stay owned by their queues.
[[src/codegen/runtime/thread/control_block.rs:lower_thread_block_release]]

A worker's own arena is not unmapped when the worker finishes; nothing the parent
can read points into it, because the result and every message are block-arena
copies.

## See Also

* ./mfb spec language memory-semantics — the general lexical scope-drop value-cleanup rule
* ./mfb spec threading control-block — the queue record and control-block layout
