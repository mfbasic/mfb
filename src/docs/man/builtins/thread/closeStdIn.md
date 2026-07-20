# closeStdIn

Unsubscribe a thread from the process-global stdin broadcast log.

## Synopsis

```
thread::closeStdIn() AS Nothing
thread::closeStdIn(t AS Thread OF Msg TO Out) AS Nothing
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

`thread::closeStdIn` releases a thread's subscription to the stdin broadcast log.
The no-argument form unsubscribes the calling thread; the one-argument form takes
a parent `Thread` handle and unsubscribes the worker behind it. As with
`thread::openStdIn`, the no-argument form is the same runtime call with a null
handle sentinel filled in during lowering, and the worker form reads the worker's
arena state out of the thread control block.
[[src/target/shared/code/runtime_helpers.rs:lower_thread_stdin_subscription_helper]] [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

Unsubscribing clears the thread's entry in the subscriber registry and recomputes
the log's reclamation point, so blocks that no remaining subscriber still needs can
be freed. A subscriber that had stalled and was throttling the reader stops
applying that backpressure once it unsubscribes, releasing a producer that was
waiting on the memory cap.
[[src/target/shared/code/stdin_broadcast.rs:lower_stdin_unsubscribe]] [[src/target/shared/code/stdin_broadcast.rs:lower_stdin_recompute_base]]

The call is a no-op when the thread is not subscribed or the log was never
initialized, so it is safe to call unconditionally and safe to call twice. Thread
teardown unsubscribes automatically, so an exited thread never permanently pins the
log's reclamation point; the explicit call is for releasing stdin *earlier*, while
the thread keeps running. `closeStdIn` returns `Nothing` and has no failure path.
[[src/target/shared/code/stdin_broadcast.rs:lower_stdin_unsubscribe]]

After unsubscribing, a stdin read from that thread raises `ErrInvalidContext`
(`77050019`) until it resubscribes with `thread::openStdIn`. That error is raised
by the read, not by this call, and it is produced without touching the OS, so the
behaviour is deterministic regardless of what stdin holds.
[[src/target/shared/code/stdin_broadcast.rs:lower_stdin_next_byte]]

## Overloads

**`thread::closeStdIn() AS Nothing`**

Unsubscribes the calling thread.

**`thread::closeStdIn(t AS Thread OF Msg TO Out) AS Nothing`**

Unsubscribes the worker behind a parent `Thread` handle. A `ThreadWorker` handle is
not accepted. [[src/builtins/thread.rs:resolve_call]] [[src/builtins/thread.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` | Optional. A parent `Thread` handle whose worker should be unsubscribed. When omitted, the calling thread is unsubscribed. Borrowed, not consumed; the call reads only the worker's arena-state pointer. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | No value. On return the target thread is not subscribed. [[src/builtins/thread.rs:resolve_call]] |

## Errors

No errors.

## Type checking

Either zero arguments, or exactly one parent `Thread OF Msg TO Out`. A
`ThreadWorker` handle or any other type fails to resolve. The result is always
`Nothing`. [[src/builtins/thread.rs:is_parent_thread_type]]

## Examples

Release stdin from the calling thread when done reading:

```
IMPORT io
IMPORT thread

FUNC main AS Integer
  thread::openStdIn()
  LET line AS String = io::readLine()
  io::print(line)
  thread::closeStdIn()
  RETURN 0
END FUNC
```

Reading after unsubscribing traps `ErrInvalidContext`:

```
IMPORT io
IMPORT thread

FUNC main AS Integer
  thread::openStdIn()
  thread::closeStdIn()
  MUT code AS Integer = 0
  LET line AS String = io::readLine() TRAP(e)
    code = e.code
    RECOVER ""
  END TRAP
  io::print("closed=" & toString(code))
  RETURN 0
END FUNC
```

## See also

- `mfb man thread openStdIn`
- `mfb man thread start`
- `mfb man io readLine`
