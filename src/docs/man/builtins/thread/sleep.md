# sleep

Block the calling thread for a fixed number of milliseconds.

## Synopsis

```
thread::sleep(t AS Thread OF Msg TO Out, ms AS Integer) AS Nothing
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

`thread::sleep` pauses the *calling* thread for at least `ms` milliseconds, then
returns. It is a plain wall-clock delay: it reads nothing from the worker behind
the parent `Thread` handle, touches no queue, and has no effect on the worker,
which keeps running while the caller sleeps. The `Msg` and `Out` types do not
affect the behavior. [[src/target/shared/code/runtime_helpers.rs:lower_thread_sleep_helper]]

`ms = 0` returns immediately without sleeping, matching the `ms = 0` convention of
`thread::poll` and `thread::receive`. A positive `ms` blocks for at least that many
milliseconds of real time; the sleep may last marginally longer under scheduling
pressure but never returns early. On the native POSIX targets the delay is a libc
`nanosleep`, retried across signal interruption so a signal delivered mid-sleep
cannot cut it short; on Windows it is `Sleep(dwMilliseconds)`. A negative `ms` is
rejected with `ErrInvalidArgument` before any sleeping.
[[src/target/shared/code/runtime_helpers.rs:lower_thread_sleep_helper]]

This overload is a plain, uninterruptible delay on a parent `Thread` handle: it
does not observe cancellation, and `thread::cancel` on the handle does not wake it.
For parity with `thread::poll` it fails with `ErrResourceClosed` when the handle's
state is already closed — the state that `thread::waitFor` and dropping the handle
set. [[src/target/shared/code/runtime_helpers.rs:THREAD_STATE_CLOSED]]

`thread::sleep` accepts only a parent `Thread` handle here; a `ThreadWorker` is
rejected at compile time. [[src/builtins/thread.rs:THREAD]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` | The parent handle the sleep is issued against. Borrowed, not consumed; nothing is read from it. A `ThreadWorker` handle is rejected at compile time. [[src/builtins/thread.rs:call_param_names]] |
| `ms` | `Integer` | Milliseconds to block the calling thread. `0` returns immediately; a positive value blocks for at least that long; a negative value is rejected with `ErrInvalidArgument`. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns after the requested delay has elapsed (or immediately when `ms = 0`). [[src/builtins/thread.rs:THREAD]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `ms` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | The parent `Thread` handle's state is closed — after `thread::waitFor` retrieved the outcome, or after the handle was dropped. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Type checking

Generic over `Msg` and `Out`. Exactly two arguments: a parent
`Thread OF Msg TO Out` and an `Integer`. A `ThreadWorker` handle, a missing `ms`,
or a non-`Integer` `ms` fails to resolve. The result is always `Nothing`.
[[src/builtins/thread.rs:THREAD]]

## Examples

Pause the main thread briefly while a worker runs:

```
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::countWorker, "seed")
  thread::sleep(t, 50)
  RETURN thread::waitFor(t)
END FUNC
```

## See also

- `mfb man thread poll`
- `mfb man thread receive`
- `mfb man thread waitFor`
- `mfb man thread cancel`
