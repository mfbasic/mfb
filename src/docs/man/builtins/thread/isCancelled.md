# isCancelled

Test whether cancellation has been requested for the running worker.

## Synopsis

```
thread::isCancelled(t AS ThreadWorker OF Msg TO Out) AS Boolean
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

`thread::isCancelled` is the worker-side cancellation check. It returns `TRUE`
once the parent has called `thread::cancel` for this worker, and `FALSE` until
then. The implementation is a plain flag read from the worker's thread control
block, reached through the pinned current-thread register rather than by
dereferencing the handle argument — so the call always reports the *calling*
worker's flag. It takes no lock, never blocks, consumes no message, does not touch
the worker outcome, and changes no state, which makes it cheap enough to poll
every iteration of a worker loop.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_is_cancelled_helper]]

Cancellation in MFBASIC is cooperative. `thread::cancel` sets this flag, closes
the worker's data and resource queues, and broadcasts their condition variables;
`isCancelled` is how worker code that is *not* parked in a queue operation learns
about the request and decides to wind down. Worker code that *is* parked resumes
by failing instead: `thread::receive` and `thread::accept` fail with
`ErrInterrupted`, as do worker-side `thread::send` and `thread::transfer`. The
polled flag and those failures are two views of the same bit.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_read_helper]] [[src/target/shared/code/runtime_helpers_thread.rs:emit_close_resource_queues]]

Only a `ThreadWorker` handle is accepted; a parent `Thread` handle is rejected at
compile time, and the parent observes a worker's liveness with
`thread::isRunning` instead. The flag is set once and never cleared for the life
of the worker, so `isCancelled` never reverts from `TRUE` to `FALSE`. Dropping a
still-running parent `Thread` handle also sets the flag, so a worker whose parent
handle went out of scope sees cancellation just as it would after an explicit
`thread::cancel`. [[src/builtins/thread.rs:resolve_call]] [[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `ThreadWorker OF Msg TO Out` | The running worker's own handle. Borrowed, not consumed. Used to type the call and to prove the caller is worker code; the flag itself is read from the calling thread's control block. A parent `Thread` handle is rejected at compile time. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` if cancellation has been requested for this worker (by `thread::cancel` or by the parent handle being dropped while the worker still runs); `FALSE` otherwise. [[src/builtins/thread.rs:resolve_call]] |

## Errors

No errors.

## Type checking

Generic over the worker's message type `Msg` and output type `Out`. The single
argument must be a `ThreadWorker OF Msg TO Out`; any other type, including a
parent `Thread` handle, fails to resolve. The result is always `Boolean`.
[[src/builtins/thread.rs:is_worker_thread_type]]

## Examples

Wind a worker loop down when cancellation is requested:

```
IMPORT thread

ISOLATED FUNC drain(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  MUT seen AS Integer = 0
  WHILE NOT thread::isCancelled(t)
    seen = seen + 1
  WEND
  RETURN seen
END FUNC
```

Read the flag into a variable:

```
IMPORT thread

ISOLATED FUNC probe(t AS ThreadWorker OF String TO Boolean, seed AS String) AS Boolean
  LET cancelled AS Boolean = thread::isCancelled(t)
  RETURN cancelled
END FUNC
```

## See also

- `mfb man thread cancel`
- `mfb man thread isRunning`
- `mfb man thread receive`
- `mfb man thread accept`
- `mfb man thread waitFor`
