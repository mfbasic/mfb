# sleep

Block the calling thread for a fixed number of milliseconds.

## Synopsis

```
thread::sleep(t AS Thread OF Msg TO Out, ms AS Integer) AS Nothing
thread::sleep(t AS ThreadWorker OF Msg TO Out, ms AS Integer) AS Nothing
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
returns `Nothing`. It moves no message and mutates no queue; the `Msg` and `Out`
types do not affect the behavior. It accepts either handle side, and the handle
side selects one of two forms — a plain uninterruptible delay (parent `Thread`) or
a cancellation-aware delay (`ThreadWorker`) — described under Overloads.
[[src/target/shared/code/runtime_helpers.rs:lower_thread_sleep_helper]]

`ms = 0` returns immediately without sleeping, matching the `ms = 0` convention of
`thread::poll` and `thread::receive`. A positive `ms` blocks for at least that many
milliseconds of real time; the sleep may last marginally longer under scheduling
pressure but never returns early (except the worker form on cancellation). A
negative `ms` is rejected with `ErrInvalidArgument` before any sleeping.
[[src/target/shared/code/runtime_helpers.rs:lower_thread_sleep_helper]]

Which handle side you call it on decides whether the sleep is interruptible (see
Overloads). The direction is chosen from the static handle type during lowering,
not at runtime. [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

## Overloads

**`thread::sleep(t AS Thread OF Msg TO Out, ms AS Integer) AS Nothing`**

The parent-side form: a plain, **uninterruptible** wall-clock delay. It does not
observe cancellation, and `thread::cancel` on the handle does not wake it. For
parity with `thread::poll` it fails with `ErrResourceClosed` when the handle's
state is already closed — the state that `thread::waitFor` and dropping the handle
set. Implemented with libc `nanosleep` (Win32 `Sleep`).
[[src/target/shared/code/runtime_helpers.rs:lower_thread_sleep_helper]]

**`thread::sleep(t AS ThreadWorker OF Msg TO Out, ms AS Integer) AS Nothing`**

The worker-side form: a **cancellation-aware** delay, valid inside `ISOLATED FUNC`
worker code. It computes an absolute deadline and waits on the worker's inbound
queue, so if the parent requests cancellation (`thread::cancel`, or dropping the
parent handle) the sleep wakes promptly and fails with `ErrInterrupted` — the same
contract as a worker `thread::receive`/`send`. A parent `thread::send` arriving
mid-sleep does **not** shorten it: the deadline is absolute, so a spurious wake
re-enters the wait for the remaining time.
[[src/target/shared/code/runtime_helpers_thread.rs:lower_thread_sleep_worker_helper]]

## Cancellation

The two overloads differ only in cancellation behavior. The parent form is a raw
delay and ignores the cancel flag. The worker form is a cooperative cancellation
point: a worker that sleeps stays responsive to `thread::cancel`, waking with
`ErrInterrupted` (77050009) instead of running out the full duration. Neither form
is an asynchronous kill — the worker form only observes the cancel broadcast; it
does not interrupt package or native code. [[src/target/shared/code/runtime_helpers_thread.rs:lower_thread_sleep_worker_helper]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` or `ThreadWorker OF Msg TO Out` | The handle the sleep is issued against. Borrowed, not consumed; no message is read from it. A parent handle gives the plain delay, a worker handle the cancellation-aware delay. [[src/builtins/thread.rs:call_param_names]] |
| `ms` | `Integer` | Milliseconds to block the calling thread. `0` returns immediately; a positive value blocks for at least that long; a negative value is rejected with `ErrInvalidArgument`. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns after the requested delay has elapsed (or immediately when `ms = 0`). [[src/builtins/thread.rs:THREAD]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `ms` is negative (both overloads). [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77030004` | `ErrResourceClosed` | Parent overload only: the `Thread` handle's state is closed — after `thread::waitFor` retrieved the outcome, or after the handle was dropped. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |
| `77050009` | `ErrInterrupted` | Worker overload only: cancellation was requested (`thread::cancel`, or the parent handle was dropped) while the worker was sleeping. [[src/target/shared/code/error_constants.rs:ERR_INTERRUPTED_CODE]] |

## Type checking

Generic over `Msg` and `Out`. Exactly two arguments: a `Thread OF Msg TO Out` or
`ThreadWorker OF Msg TO Out` handle and an `Integer`. A missing `ms` or a
non-`Integer` `ms` fails to resolve. The result is always `Nothing`.
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

A worker that sleeps but stays cancellable — the sleep wakes with `ErrInterrupted`
if the parent cancels it:

```
IMPORT thread

EXPORT ISOLATED FUNC poller(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer
  thread::sleep(t, 5000)
  RETURN 0
  TRAP(err)
    RETURN 1
  END TRAP
END FUNC
```

## See also

- `mfb man thread poll`
- `mfb man thread receive`
- `mfb man thread waitFor`
- `mfb man thread cancel`
- `mfb man thread isCancelled`
