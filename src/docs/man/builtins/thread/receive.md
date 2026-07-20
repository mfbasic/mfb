# receive

Dequeue one data-plane message from a thread handle.

## Synopsis

```
thread::receive(t AS Thread OF Msg TO Out) AS Msg
thread::receive(t AS Thread OF Msg TO Out, timeoutMs AS Integer) AS Msg
thread::receive(t AS ThreadWorker OF Msg TO Out) AS Msg
thread::receive(t AS ThreadWorker OF Msg TO Out, timeoutMs AS Integer) AS Msg
```

## Package

`thread`

## Imports

```
IMPORT thread
```

`thread` is a built-in package, so no manifest dependency is required.
[[src/builtins/thread.rs:is_thread_call]]

## Description

`thread::receive` takes one message off the bounded data-plane queue selected by
the handle type and returns it by move. A parent `Thread` handle reads the
worker's **outbound** queue, taking something the worker sent; a `ThreadWorker`
handle reads its own **inbound** queue, taking something the parent sent. The
matching writer is `thread::send` on the opposite handle. The direction is chosen
during lowering from the static handle type.
[[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

The dequeued value has the thread's message type `Msg` (a handle typed `Unknown`
yields `Unknown`). Removing it frees a slot and signals the queue's *not-full*
condition, so a sender blocked on a full queue can proceed. The data plane carries
values only; resources cross on the resource plane and are taken with
`thread::accept`. Each read also drains the queue's pending-free list first,
reclaiming message copies that a failed send orphaned in this thread's arena.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_read_helper]]

There are two forms. Omitting `timeoutMs` **blocks**: lowering passes an
unreachable sentinel timeout and the helper waits indefinitely on the queue's
*not-empty* condition until a message arrives, the queue closes, or cancellation
is observed. Supplying `timeoutMs` bounds the wait and it must be `>= 0`: `0` does
not wait at all, and a positive value waits that many milliseconds against an
absolute deadline. A negative explicit `timeoutMs` is rejected with
`ErrInvalidArgument` — to wait indefinitely, omit the argument rather than passing
a negative number. [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]] [[src/target/shared/code/runtime_helpers_thread.rs:ThreadReadMode]]

An empty queue does not produce one single error. When a message is not available
the helper distinguishes the reasons, in this order: on a worker handle a set
cancelled flag gives `ErrInterrupted`; a queue marked closed gives `ErrNotFound`;
on a parent handle a closed thread state gives `ErrResourceClosed` and a completed
worker gives `ErrNotFound`; a `timeoutMs` of `0` gives `ErrNotFound`; and a
positive `timeoutMs` that expires gives `ErrTimeout`. Because the queue-closed
check precedes the thread-state check on a parent handle, a `receive` on a handle
that `thread::waitFor` already closed reports `ErrNotFound` — `thread::waitFor`
closes the outbound queue as well as the handle.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_read_helper]] [[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

The consequence worth remembering is that a blocking parent-side `receive` never
deadlocks on a worker that exits: the worker's exit path closes the queue and
broadcasts its condition, so the parked reader wakes and fails with `ErrNotFound`
instead of waiting forever. Likewise a blocking worker-side `receive` wakes with
`ErrInterrupted` when the parent cancels.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_read_helper]]

## Overloads

**`thread::receive(t) AS Msg`**

Blocking read on either handle kind: waits until a message arrives, the queue
closes, or cancellation is observed.

**`thread::receive(t, timeoutMs AS Integer) AS Msg`**

Bounded read on either handle kind: `timeoutMs` must be `>= 0`; `0` polls once and
fails with `ErrNotFound` when the queue is empty, a positive value waits that long
and then fails with `ErrTimeout`. [[src/builtins/thread.rs:resolve_call]] [[src/builtins/thread.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` or `ThreadWorker OF Msg TO Out` | The handle whose data-plane queue is read. A parent handle reads the worker's outbound queue; a worker handle reads its own inbound queue. Borrowed, not consumed. [[src/builtins/thread.rs:call_param_names]] |
| `timeoutMs` | `Integer` | Optional. Omit to block until a message arrives, the queue closes, or cancellation is observed. When supplied it must be `>= 0`: `0` does not wait, a positive value waits that many milliseconds. A negative value is rejected with `ErrInvalidArgument`. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Msg` | The dequeued message, of the thread's message type, removed from the queue and moved to the caller. A handle typed `Unknown` yields a value of type `Unknown`. [[src/builtins/thread.rs:thread_message]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | An explicit `timeoutMs` is negative. Omit the argument to wait indefinitely. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050004` | `ErrNotFound` | No message is available and none will be waited for: `timeoutMs` is `0` and the queue is empty, the queue has been closed, or (parent-side) the worker has completed with an empty outbound queue. [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |
| `77050008` | `ErrTimeout` | The queue stayed empty until a positive `timeoutMs` elapsed. [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77050009` | `ErrInterrupted` | Worker-side only: cancellation has been requested for this worker. [[src/target/shared/code/error_constants.rs:ERR_INTERRUPTED_CODE]] |
| `77030004` | `ErrResourceClosed` | Parent-side only: the thread's state is closed while its outbound queue is not flagged closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Type checking

Generic over `Msg` and `Out`. `t` must be a `Thread` or `ThreadWorker` handle; the
queue read is selected by which of the two it is, and the result type is the
handle's `Msg` (`Unknown` for a handle typed `Unknown`). `timeoutMs`, when
supplied, must be `Integer`. [[src/builtins/thread.rs:resolve_call]]

## Examples

Block until a message arrives — the common worker "wait for work" form:

```
IMPORT thread

ISOLATED FUNC echo(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  LET work AS String = thread::receive(t)
  thread::send(t, work)
  RETURN len(work)
END FUNC
```

Take a message with a bounded wait from the parent:

```
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::printReceived, "start")
  thread::send(t, "alpha")
  LET ack AS String = thread::receive(t, 10)
  RETURN thread::waitFor(t)
END FUNC
```

## See also

- `mfb man thread send`
- `mfb man thread poll`
- `mfb man thread accept`
- `mfb man thread start`
- `mfb man thread waitFor`
