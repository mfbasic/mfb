# cancel

Request cooperative cancellation of a worker thread.

## Synopsis

```
thread::cancel(t AS Thread OF Msg TO Out) AS Nothing
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

`thread::cancel` asks the worker behind a parent `Thread` handle to stop. It is a
request, never a force stop: it sets the worker's cancelled flag, closes the
worker's queues, and wakes everything parked on them so the worker can observe the
request and unwind through ordinary control flow. It never asynchronously kills
user code or native code running inside the worker, and it does not wait for the
worker to terminate — use `thread::waitFor` to join.
[[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

The call closes and broadcasts **all four** of the thread's queues, not just the
data plane: under the inbound queue's mutex it sets the cancelled flag, marks the
inbound data queue closed, and broadcasts its *not-empty* and *not-full*
conditions; it then does the same for the outbound data queue; and finally it
closes and broadcasts both resource-plane queues. Waking the resource plane
matters — a worker parked in a blocking `thread::accept` would otherwise never
observe cancellation and would hang permanently.
[[src/target/shared/code/runtime_helpers_thread.rs:emit_close_resource_queues]]

Once cancellation is requested, the worker's runtime-managed queue waits become
cancellation points: `thread::receive` and `thread::accept` on a `ThreadWorker`
handle, and `thread::send` and `thread::transfer` on a `ThreadWorker` handle, wake
and fail with `ErrInterrupted`. Parent-side `thread::send` and `thread::transfer`
also fail with `ErrInterrupted` after cancellation, because the parent checks the
same cancelled flag. Worker code that is not parked in a queue operation observes
the request by polling `thread::isCancelled`.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_write_helper]] [[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_read_helper]]

`cancel` neither closes nor consumes the parent `Thread` handle: the handle stays
live so the caller can still join the worker and read its outcome. Note that
`thread::isRunning` keeps reporting `TRUE` after a successful `cancel` until the
worker actually finishes — cancellation and execution are separate pieces of
state. The only way `cancel` itself fails is when the handle is already closed.
A `ThreadWorker` handle is rejected at compile time.
[[src/builtins/thread.rs:resolve_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` | The parent handle whose worker should be cancelled. Borrowed, not consumed — the handle stays usable for `thread::waitFor`. Must not already be closed. A `ThreadWorker` handle is rejected at compile time. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | No value. On success the worker's cancelled flag is set and all four of its queues are closed and broadcast. [[src/builtins/thread.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | The parent `Thread` handle is already closed — for example by an earlier `thread::waitFor`, or because the handle was dropped. [[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]] |

## Type checking

Generic over `Msg` and `Out`. The single argument must be a parent
`Thread OF Msg TO Out`; a `ThreadWorker` or any other type fails to resolve. The
result is always `Nothing`. [[src/builtins/thread.rs:is_parent_thread_type]]

## Examples

Request cancellation and then join the worker:

```
IMPORT thread
IMPORT thread_cancel_worker

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_cancel_worker::echoWorker, "seed")
  thread::cancel(t)
  LET result AS Integer = thread::waitFor(t) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN result
END FUNC
```

A worker that winds down on the flag:

```
IMPORT thread

ISOLATED FUNC drain(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  MUT seen AS Integer = 0
  WHILE NOT thread::isCancelled(t)
    seen = seen + 1
  END WHILE
  RETURN seen
END FUNC
```

## See also

- `mfb man thread isCancelled`
- `mfb man thread waitFor`
- `mfb man thread start`
- `mfb man thread send`
- `mfb man thread receive`
