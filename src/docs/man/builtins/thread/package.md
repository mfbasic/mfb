# thread

Start isolated workers and exchange bounded queue messages

## Synopsis

```
IMPORT thread
LET t = thread::start(worker, data)
thread::send(t, value)
LET reply = thread::receive(t)
LET result = thread::waitFor(t)
```

## Description

The `thread` package starts exported `ISOLATED FUNC` entry points from imported
packages, each in a fresh package instance. Parent code holds a `Thread` value;
worker code holds a `ThreadWorker` value. Together the pair carries an inbound
message queue, an outbound message queue, an optional resource plane,
cancellation state, and the worker's result. Retrieving the result with
`thread::waitFor(t)` (or the `t.result` field) is one-shot and closes the parent
`Thread` handle. [[src/builtins/thread.rs:is_thread_call]]

A worker entry point must have the shape
`ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out` and must be an exported
function from an imported package. Lambdas, closures, `SUB`s, non-isolated
functions, current-package functions, and functions without the leading
`ThreadWorker` parameter are rejected at compile time. [[src/builtins/thread.rs:matches_start]]

Threads are spelled `Thread OF Msg TO Out` (parent handle) and
`ThreadWorker OF Msg TO Out` (worker handle), where `Msg` is the message type
carried on the data plane and `Out` is the worker's result type. A thread may
also carry a resource plane, spelled with a `RES Res` clause
(`Thread OF Msg RES Res TO Out`, or `Thread OF RES Res TO Out` for a
resource-only thread). The data plane moves plain values with `thread::send` and
`thread::receive`; the separate resource plane moves owned resource handles with
`thread::transfer` and `thread::accept`, keeping the data channel resource-free.
A handle whose message or resource type is `Unknown` accepts any value and yields
a value of type `Unknown`. [[src/builtins/thread.rs:thread_parts_full]] [[src/builtins/thread.rs:format_thread_type]]

Both queues are bounded; their capacities are set when the thread starts
(`inboundLimit` and `outboundLimit`, each at least 1). Sending into a full queue
blocks, and receiving from an empty queue blocks, subject to a timeout. Timeouts
are `Integer` milliseconds: `timeoutMs = 0` does not wait (it acts at once and
otherwise fails immediately), a positive `timeoutMs` bounds the wait, and on a
worker-side wait a `timeoutMs` of `-1` waits indefinitely. A poll or send timeout
must not be negative; a worker-side accept or receive timeout must not be below
`-1`. [[src/builtins/thread.rs:call_param_names]]

`Thread` values are non-copyable owned handles. A live parent `Thread` is cleaned
up automatically on scope exit, `RETURN`, `FAIL`, propagated errors, trap routing,
and successful reassignment of a `MUT Thread`. Dropping a running `Thread`
requests cancellation, wakes its queues, and detaches the worker. A `Thread`
moved out by `RETURN` or another consuming operation is not dropped by the source
scope. Compiler-generated cleanup is idempotent for handles already closed by
`thread::waitFor(t)` or `t.result`.

There is intentionally no `thread::stop()` and no separate `thread::detach()`.
Cancellation is cooperative: `thread::cancel(t)` sets a flag, and the worker
observes it with `thread::isCancelled(t)`. Runtime-managed worker queue waits —
including `thread::receive(ThreadWorker, ...)`, `thread::send(ThreadWorker, ...)`,
and `thread::accept(ThreadWorker, ...)` — wake and fail with `ErrInterrupted` when
cancellation is requested. Cancellation does not asynchronously kill a worker
while it owns a resource handle, holds a queue lock, moves a non-copyable value,
writes its result, or runs package/native code. [[src/target/shared/code/runtime_helpers_thread.rs]]

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | raised by `start` when `inboundLimit` or `outboundLimit` is below 1, by `poll` when `ms` is negative, by `send` and `transfer` when `timeoutMs` is negative, and by `receive` and `accept` when `timeoutMs` is out of range for the handle (negative on a parent `Thread`, below `-1` on a `ThreadWorker`) [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050004` | `ErrNotFound` | raised by `receive` and `accept` when nothing is available without waiting (`timeoutMs = 0` and the queue is empty), when the queue has been closed, or, for a parent `Thread`, when the worker has completed with an empty outbound queue [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |
| `77050008` | `ErrTimeout` | raised by `send`, `receive`, `transfer`, and `accept` when a positive `timeoutMs` elapses before space frees up or a message or resource arrives [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77050009` | `ErrInterrupted` | raised by `start` when the underlying OS thread cannot be spawned, and by `send`, `receive`, `transfer`, and `accept` when a wait observes that the thread has ended, the queue has been closed, or cancellation of the worker has been requested [[src/target/shared/code/error_constants.rs:ERR_INTERRUPTED_CODE]] |
| `77030004` | `ErrResourceClosed` | raised by `cancel`, `isRunning`, `waitFor`, and the parent `Thread` overloads of `poll`, `send`, `receive`, `transfer`, and `accept` when the parent `Thread` handle has already been closed, such as after its result was retrieved with `waitFor` or `t.result` [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77010001` | `ErrOutOfMemory` | raised by `start` when the thread control block, the worker's arena state, or any of its queue structures and backing message arrays cannot be allocated [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
