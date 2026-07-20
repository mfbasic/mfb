# poll

Wait up to a deadline for an outbound message to become readable.

## Synopsis

```
thread::poll(t AS Thread OF Msg TO Out, ms AS Integer) AS Boolean
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

`thread::poll` reports whether the worker behind a parent `Thread` handle has a
message waiting on its outbound data queue, waiting up to `ms` milliseconds for
one to arrive. It inspects the same queue `thread::receive` reads through a parent
handle, so a `TRUE` result means a following `thread::receive(t)` can take a
message without waiting. It never removes a message and has no effect on the queue
or the worker. [[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

`ms` is required — unlike `thread::send`'s `timeoutMs`, there is no default and no
one-argument form. With `ms = 0` the call returns immediately: `TRUE` if a message
is already queued, `FALSE` otherwise. A positive `ms` computes an absolute
deadline and sleeps on the outbound queue's *not-empty* condition variable,
rechecking on each signal, so it does not busy-wait; it returns `TRUE` as soon as a
message is queued and `FALSE` when the deadline passes with the queue still empty.
A negative `ms` is rejected with `ErrInvalidArgument` before the queue is touched.
[[src/target/shared/code/runtime_helpers_thread.rs:emit_thread_deadline]]

If the worker has already completed and left its outbound queue empty, `poll`
returns `FALSE` at once rather than waiting out the full timeout — the state check
runs on every pass of the wait loop, so a worker that exits mid-wait ends the wait
promptly. `poll` fails with `ErrResourceClosed` when the *thread's state* is
closed, which is what `thread::waitFor` and dropping the handle set. Note that this
is the handle's state, not the queue's closed flag: after `thread::cancel` the
outbound queue is marked closed but the thread state is not, so `poll` keeps
answering `TRUE`/`FALSE` from the queue contents rather than failing.
[[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]] [[src/target/shared/code/runtime_helpers.rs:THREAD_STATE_CLOSED]]

`poll` accepts only a parent `Thread` handle; a `ThreadWorker` is rejected at
compile time, and there is no worker-side poll of the inbound queue — worker code
uses `thread::receive(t, 0)` for a non-blocking read instead. The thread's `Msg`
and `Out` types do not affect the result. [[src/builtins/thread.rs:resolve_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` | The parent handle whose outbound queue is polled. Borrowed, not consumed. A `ThreadWorker` handle is rejected at compile time. [[src/builtins/thread.rs:call_param_names]] |
| `ms` | `Integer` | Required. Milliseconds to wait for a message. `0` returns immediately without waiting; a positive value waits up to that long; a negative value is rejected with `ErrInvalidArgument`. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when a message is queued on the outbound queue, so a following `thread::receive(t)` returns without waiting; `FALSE` when none arrived before the deadline, including when the worker has completed with an empty queue. [[src/builtins/thread.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `ms` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | The parent `Thread` handle's state is closed — after `thread::waitFor` retrieved the outcome, or after the handle was dropped. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Type checking

Generic over `Msg` and `Out`. Exactly two arguments: a parent
`Thread OF Msg TO Out` and an `Integer`. A `ThreadWorker` handle, a missing `ms`,
or a non-`Integer` `ms` fails to resolve. The result is always `Boolean`.
[[src/builtins/thread.rs:arity]]

## Examples

Check without waiting whether a message is ready:

```
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::countWorker, "seed")
  LET ready AS Boolean = thread::poll(t, 0)
  IF ready THEN
    LET message AS String = thread::receive(t)
  END IF
  RETURN thread::waitFor(t)
END FUNC
```

Wait up to 10 milliseconds for the worker to produce a message:

```
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::printReceived, "start")
  thread::send(t, "alpha")
  LET hasAck AS Boolean = thread::poll(t, 10)
  LET ack AS String = thread::receive(t)
  RETURN thread::waitFor(t)
END FUNC
```

## See also

- `mfb man thread receive`
- `mfb man thread send`
- `mfb man thread isRunning`
- `mfb man thread waitFor`
