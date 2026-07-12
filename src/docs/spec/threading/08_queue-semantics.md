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

A boundary value must end up in the *receiving* side's arena, since each side
allocates from its own arena. The runtime copies the value into the receiver's
arena at send time, then the reader just dequeues the already-materialized value:

- Worker→parent (`thread.emit`) loads the parent arena state from control-block
  offset 88 and copies the message into it.
- Parent→worker (`thread.send`) and all reads use the worker arena state at
  offset 80.

The message copy is emitted at the send site (the builder points the arena-state
register at the receiver's state, then copies); the queue-write helper only stores
the already-copied pointer into the queue slot. [[src/target/shared/code/builder_emit_helpers.rs:emit_thread_send_runtime_helper_call]] [[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_write_helper]]

Resource handles move as scalar handles through the resource queues without the
flat-block deep copy used for data-plane values.

`timeoutMs = 0` means non-blocking. Positive timeouts wait up to that many
milliseconds. Negative timeouts are invalid except where a specific overload
documents an indefinite worker-side wait. `thread::receive(ThreadWorker, -1)`
waits until a message, queue closure, or cancellation; if cancellation is
requested before or during that wait, it fails with `ErrInterrupted`.

For `thread::send` and `thread::transfer`, ownership transfer is atomic with
enqueue success:

- If enqueue succeeds, the destination side owns the value immediately. While the
  value is queued, the destination queue owns it in receiver-valid storage or
  runtime transfer storage independent of the sender arena.
- If enqueue fails because the queue is full, closed, cancelled, timed out, or
  the timeout is invalid, ownership is not transferred and the sender still owns
  the value.
- Code may attach an inline `TRAP` to `thread::send(...)`/`thread::transfer(...)`
  to separate the success path, where the sent/transferred binding is moved, from
  the error handler, where it remains owned by the sender and can be released. The
  syntaxchecker treats the argument at index 1 of `thread.start`, `thread.send`, and
  `thread.transfer` as a move (`ExprMode::Transfer`); a borrowed resource cannot be
  transferred, rejected on the IR with `TYPE_RESOURCE_BORROW_INVALIDATE`
  ("Binding `<name>` is a borrowed resource; only its owner may close, `RETURN`, or transfer it."). [[src/ir/verify/mod.rs:check_resource_moves]]

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
clears both data queues (broadcasting their waiters), and `pthread_detach`s the OS
thread so the runtime reclaims it on exit. Reassigning a `MUT Thread` evaluates the
new value first, then drops the old handle before storing the replacement.
Bindings that have moved out through return or another consuming operation are
removed from the cleanup set. Handles closed by `thread::waitFor(t)` remain safe
for compiler-generated cleanup; the drop helper is idempotent for an already
closed handle.

The same scope-drop cleanup mechanism also frees ordinary owned **values** with
one `arena_free` each, in the same reverse order on the same exit paths — the
general lexical value-cleanup rule owned by `./mfb spec language memory-semantics`.
Two thread-specific exclusions apply, because those values are not plain blocks
this scope owns:

- **Thread-boundary results are runtime-managed.** Values produced by
  `thread::receive`, `thread::waitFor`, and the data-plane reads live in the
  thread plumbing and the worker arena that the runtime bulk-reclaims at teardown;
  on a cancel/timeout path a result is not a clean ownable block at all. Such a
  binding is therefore *not* registered for a scope-drop value free — it follows
  the queue/control-block lifetime rules below, not lexical value cleanup. If such
  a value is re-bound or returned it is deep-copied first, so the copy is owned and
  freed normally while the original stays runtime-managed.
- **Resources** stay move-only handles closed by their own close op, never
  `arena_free`d, exactly as before.

When the worker completes:

- Inbound sends fail.
- The result is stored, and the control block owns any worker-arena lifetime
  needed to materialize the one parent-visible result retrieval.
- `thread::isRunning` returns `FALSE`.
- `thread::waitFor` returns or propagates the stored result and closes the
  parent `Thread` handle.
- Remaining outbound messages stay readable until drained.

If a queued value is never received, the destination queue/runtime drops or
closes it exactly once:

- Unreceived inbound messages are cleaned up by the worker-side runtime when the
  worker exits or the thread is torn down.
- Unreceived outbound messages are cleaned up when the parent drains them,
  waits and lets lexical cleanup drop the completed `Thread`, or drops/detaches
  the thread handle according to the source-level `Thread` lifetime rules.
- Dropping a running `Thread` requests cancellation and detaches the worker; any
  remaining queued values are still owned by their destination queues until the
  responsible runtime cleanup path runs.
- The worker arena may be reclaimed only after the worker result has been
  transferred out of that arena or the result has otherwise been retrieved, and
  every worker-to-parent message has either been transferred into outbound queue
  storage or dropped by cleanup.

## See Also

* ./mfb spec language memory-semantics — the general lexical scope-drop value-cleanup rule
* ./mfb spec threading control-block — the queue record and control-block layout
