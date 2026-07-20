# waitFor

Block until a worker finishes, then take its outcome and close the handle.

## Synopsis

```
thread::waitFor(t AS Thread OF Msg TO Out) AS Out
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

`thread::waitFor` blocks the calling thread until the worker behind the parent
`Thread` handle has reached its terminal state, retrieves the stored outcome,
closes the handle, detaches the OS thread, and yields the worker's `Out` value.
The returned type is the handle's output type, read structurally from
`Thread OF Msg TO Out`. Only a parent `Thread` is accepted; a `ThreadWorker` is
rejected at compile time. [[src/builtins/thread.rs:parent_thread_output]] [[src/builtins/thread.rs:resolve_call]]

The wait is unbounded — there is no timeout parameter. Under the outbound queue
mutex the call re-reads the worker's state in a loop: if the worker has already
completed it returns at once, otherwise it sleeps on the outbound queue's
condition variable and rechecks each time it is signalled, so it never busy-waits
and cannot miss a completion that races the check.
[[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

Retrieval is one-shot and destructive. On success the call marks the thread state
*closed*, marks the outbound queue closed, and resets its entry count to zero, so
any outbound messages the worker sent and the parent never read are dropped at
this point — drain them with `thread::receive` **before** joining if they matter.
The OS thread is then detached. Any later user-visible operation on the same
handle fails with `ErrResourceClosed`; compiler-generated lexical cleanup is
idempotent for an already-closed handle, so an already-joined `Thread` is not
dropped twice at scope exit.
[[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

The worker's outcome is a fallible value. When the worker returned successfully,
`waitFor` yields that value. When the worker failed — by `FAIL`, by
auto-propagation, or by an unhandled built-in error — that error propagates out of
`waitFor` under the ordinary fallible-call rules, carrying the worker's original
code, message, and origin source location, which the runtime stores alongside the
outcome. Such an error is the *worker's* error, not one `waitFor` raises; the only
error `waitFor` itself raises is `ErrResourceClosed`.
[[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

There is no `t.result` member: worker outcomes are retrieved only through
`thread::waitFor`, and a `.result` member access is rejected before IR lowering.
[[src/ir/lower.rs:expression_type]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` | The parent handle whose worker outcome is awaited. Must not already be closed. The handle is closed by a successful retrieval, though the source binding is not marked syntactically moved. A `ThreadWorker` handle is rejected at compile time. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Out` | The worker's successful result value, of the thread's output type. If the worker terminated with an error, that error propagates instead of a value being returned. [[src/builtins/thread.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | The parent `Thread` handle is already closed — for example by an earlier `thread::waitFor` on the same handle, or because the handle was dropped. [[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]] |

## Type checking

Generic over `Msg` and `Out`. The single argument must be a parent
`Thread OF Msg TO Out`; the result type is that handle's `Out`. A `ThreadWorker`
handle has no `waitFor` and fails to resolve.
[[src/builtins/thread.rs:parent_thread_output]]

## Examples

Start a worker and join it:

```
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO String = thread::start(thread_runtime_workers::readTextPath, "data/input.txt")
  LET text AS String = thread::waitFor(t)
  RETURN len(text)
END FUNC
```

Drain the worker's messages before joining, so none are dropped:

```
IMPORT io
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::printReceived, "start")
  thread::send(t, "alpha")
  LET ack AS String = thread::receive(t)
  io::print(ack)
  LET result AS Integer = thread::waitFor(t)
  RETURN result
END FUNC
```

## See also

- `mfb man thread start`
- `mfb man thread isRunning`
- `mfb man thread cancel`
- `mfb man thread receive`
