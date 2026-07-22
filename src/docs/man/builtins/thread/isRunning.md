# isRunning

Report whether the worker behind a parent `Thread` handle is still executing.

## Synopsis

```
thread::isRunning(t AS Thread OF Msg TO Out) AS Boolean
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

`thread::isRunning` reads the worker's run state through the parent `Thread`
handle and reports whether the worker entry function is still executing. The call
locks the thread's outbound queue mutex, copies the state word out, and unlocks
before deciding, so it observes a coherent snapshot rather than a torn read. It
consumes no message, does not touch the stored worker outcome, does not block on
the queue, and does not change the worker's state in any way.
[[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

The state word has three values. While the entry function is still running the
call returns `TRUE`. Once the worker has finished — whether it returned an `Out`
value or failed with an error — the state is *completed* and the call returns
`FALSE`; the stored outcome is still available for a later `thread::waitFor`. If
the handle has been *closed*, the call raises `ErrResourceClosed` instead of
returning a value. A handle becomes closed when `thread::waitFor` retrieves the
one-shot outcome, or when the handle is dropped, so polling with `isRunning`
after joining fails rather than reporting `FALSE`.
[[src/target/shared/code/runtime_helpers.rs:THREAD_STATE_CLOSED]]

`isRunning` accepts only a parent `Thread` handle. A `ThreadWorker` is rejected at
compile time; worker code observes its own status through `thread::isCancelled`
instead. Note that `isRunning` reports execution, not cancellation: a worker for
which `thread::cancel` has been requested keeps reporting `TRUE` until it actually
finishes, because cancellation is cooperative and sets a separate flag.
[[src/builtins/thread.rs:resolve_call]] [[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` | The parent handle returned by `thread::start`. Borrowed, not consumed. Must not already be closed. A `ThreadWorker` handle is rejected at compile time. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` while the worker entry function is still running; `FALSE` once the worker has completed or failed and its outcome is stored but not yet retrieved. [[src/builtins/thread.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | The parent `Thread` handle is already closed — for example after `thread::waitFor` retrieved the worker outcome, or after the handle was dropped. [[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]] |

## Type checking

Generic over the message type `Msg` and output type `Out`. The single argument
must be a parent `Thread OF Msg TO Out` (a `RES Res` plane on the handle is
accepted and ignored); a `ThreadWorker` or any other type fails to resolve. The
result is always `Boolean`. [[src/builtins/thread.rs:is_parent_thread_type]]

## Examples

Poll a worker until it finishes, then collect its result:

```
IMPORT io
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::printNumbers, "seed")
  WHILE thread::isRunning(t)
    io::print("still working...")
  END WHILE
  LET count AS Integer = thread::waitFor(t)
  RETURN count
END FUNC
```

## See also

- `mfb man thread start`
- `mfb man thread waitFor`
- `mfb man thread poll`
- `mfb man thread isCancelled`
