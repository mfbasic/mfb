# send

Enqueue a data-plane message through a thread handle.

## Synopsis

```
thread::send(t AS Thread OF Msg TO Out, data AS Msg) AS Nothing
thread::send(t AS Thread OF Msg TO Out, data AS Msg, timeoutMs AS Integer) AS Nothing
thread::send(t AS ThreadWorker OF Msg TO Out, data AS Msg) AS Nothing
thread::send(t AS ThreadWorker OF Msg TO Out, data AS Msg, timeoutMs AS Integer) AS Nothing
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

`thread::send` enqueues `data` on the bounded data-plane queue selected by the
handle type. A parent `Thread` handle writes the worker's **inbound** queue, so
the message travels to the worker; a `ThreadWorker` handle writes the
parent-visible **outbound** queue, so the message travels back to the parent. The
matching reader is `thread::receive` on the opposite handle. Which of the two
queues is used is decided during lowering from the static handle type, not at
runtime. [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

The data plane carries values only and is deliberately resource-free: a message
type that is a resource is rejected at compile time with a pointer to
`thread::transfer`, which moves resources on the separate resource plane. `data`
must equal the thread's message type `Msg` (a handle whose message type is
`Unknown` accepts any value) and must be thread-sendable — every field, payload,
element, key, and value type must itself be sendable.
[[src/syntaxcheck/resources.rs:is_thread_sendable_type]] [[src/builtins/thread.rs:resolve_call]]

`data` is **moved** into the call rather than borrowed, and materialized in
storage the receiving thread owns, so no sender and receiver ever observe the same
live value. If the send fails after the copy was made, the orphaned copy is pushed
onto the destination queue's pending-free list and reclaimed by the destination
thread on its next read, rather than leaking.
[[src/syntaxcheck/types.rs:thread_argument_mode]] [[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_write_helper]]

`timeoutMs` bounds how long the call waits for space when the destination queue is
already full; it defaults to `0`, filled in during lowering. With `timeoutMs = 0`
the call does not wait: it enqueues if there is room and otherwise fails at once
with `ErrTimeout`. A positive value waits up to that many milliseconds on the
queue's *not-full* condition against an absolute deadline computed from the
monotonic-style clock, then fails with `ErrTimeout`. A negative value is rejected
with `ErrInvalidArgument` before anything else happens. When there is room, the
message is enqueued immediately regardless of `timeoutMs`.
[[src/target/shared/code/runtime_helpers_thread.rs:emit_thread_deadline]]

The call is a cancellation point. Before each attempt it re-checks the thread's
state: a parent-side send finds the handle *closed* and fails with
`ErrResourceClosed`, finds the worker *completed* or the cancelled flag set and
fails with `ErrInterrupted`; a worker-side send checks only the cancelled flag.
Either side also fails with `ErrInterrupted` when the destination queue has been
marked closed. Because these checks sit inside the wait loop, a send blocked on a
full queue wakes and fails when `thread::cancel` runs rather than hanging.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_write_helper]]

## Overloads

**`thread::send(t AS Thread OF Msg TO Out, data AS Msg[, timeoutMs AS Integer]) AS Nothing`**

Parent-side send onto the worker's inbound queue. Adds `ErrResourceClosed` when
the handle is already closed, and `ErrInterrupted` when the worker has completed
or cancellation was requested.

**`thread::send(t AS ThreadWorker OF Msg TO Out, data AS Msg[, timeoutMs AS Integer]) AS Nothing`**

Worker-side send onto the parent-visible outbound queue. Fails with
`ErrInterrupted` when this worker is cancelled or the queue is closed.
[[src/builtins/thread.rs:resolve_call]] [[src/builtins/thread.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` or `ThreadWorker OF Msg TO Out` | The handle whose data-plane queue receives the message. A parent handle targets the worker's inbound queue; a worker handle targets the outbound queue the parent reads. Borrowed, not consumed. [[src/builtins/thread.rs:call_param_names]] |
| `data` (also `value`) | `Msg` | The message to enqueue. Must equal the thread's `Msg` type (a handle typed `Unknown` accepts any value) and must be thread-sendable. Moved into the call. [[src/builtins/thread.rs:call_param_names]] |
| `timeoutMs` | `Integer` | Optional, default `0`. Milliseconds to wait for queue space when the destination queue is full. `0` does not wait and fails immediately with `ErrTimeout`; a positive value waits that long; a negative value is rejected. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | No value. Returns once the message has been enqueued and the queue's *not-empty* condition signalled. [[src/builtins/thread.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050008` | `ErrTimeout` | The destination queue is full and space did not become available before `timeoutMs` elapsed — immediately when `timeoutMs` is `0`. [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77050009` | `ErrInterrupted` | Cancellation was requested for the worker, the worker has completed (parent-side), or the destination queue has been marked closed. [[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_write_helper]] |
| `77030004` | `ErrResourceClosed` | Parent-side only: the `Thread` handle is already closed, for example after `thread::waitFor`. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Type checking

Generic over `Msg` and `Out`. `t` must be a `Thread` or `ThreadWorker` handle;
`data`'s static type must equal the handle's `Msg` unless that is `Unknown`, which
accepts any value; `timeoutMs`, when supplied, must be `Integer`. A resource-typed
`Msg` is rejected with `TYPE_THREAD_NOT_SENDABLE`. The result is always `Nothing`.
[[src/syntaxcheck/resources.rs:require_thread_sendable_type]]

## Examples

Send a message from the parent without waiting for queue space, then read the
worker's reply:

```
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::printReceived, "start")
  thread::send(t, "alpha")
  LET ack AS String = thread::receive(t)
  RETURN thread::waitFor(t)
END FUNC
```

Wait up to 10 milliseconds for room in the queue, from worker code:

```
IMPORT thread

ISOLATED FUNC emit(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  thread::send(t, "ready", 10)
  RETURN 0
END FUNC
```

## See also

- `mfb man thread receive`
- `mfb man thread poll`
- `mfb man thread transfer`
- `mfb man thread start`
- `mfb man thread cancel`
