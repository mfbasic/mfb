# openStdIn

Subscribe a thread to the process-global stdin broadcast log.

## Synopsis

```
thread::openStdIn() AS Nothing
thread::openStdIn(t AS Thread OF Msg TO Out) AS Nothing
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

`thread::openStdIn` subscribes a thread to the stdin broadcast log so that it may
read standard input. The runtime owns file descriptor 0 and reads it into a single
process-global append-only log; every *subscribed* thread holds its own cursor
over that log, so each subscriber sees the whole byte stream from where it joined
and a byte one subscriber consumes is never taken out from under another.
[[src/target/shared/code/stdin_broadcast.rs:lower_stdin_subscribe]]

The no-argument form subscribes the calling thread, using that thread's own arena
state as the registry key. The one-argument form takes a parent `Thread` handle
and subscribes the worker behind it, reading the worker's arena state out of the
thread control block — use it to grant a worker stdin access. Under the hood the
no-argument form is the same call with a null handle sentinel filled in during
lowering; the runtime branches on the sentinel.
[[src/target/shared/code/runtime_helpers.rs:lower_thread_stdin_subscription_helper]] [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

A subscriber joins at the current frontier — the next byte the runtime has yet to
read — and sees every byte that arrives after. There is no replay-from-the-start
form. A single-threaded program never needs this call: program entry subscribes
the main thread before any input is read and while the log's lazy setup is still
single-threaded, so main's cursor starts at offset `0` and it sees the entire
stream. Subscribing is idempotent per thread, so an explicit call from main is a
harmless no-op that documents intent.
[[src/target/shared/code/entry_and_arena.rs:STDIN_SUBSCRIBE_SYMBOL]]

A thread that reads stdin (`io::readLine`, `io::input`, `io::readChar`,
`io::readByte`) without a subscription raises `ErrInvalidContext` (`77050019`)
rather than silently observing an empty stream — that error comes from the read,
not from this call. Note also that the subscriber registry holds at most 128
concurrently-live subscribers; when it is full `openStdIn` still returns
successfully but the thread is *not* registered, and its later stdin reads raise
`ErrInvalidContext`. `openStdIn` returns `Nothing` and has no failure path of its
own. [[src/target/shared/code/error_constants.rs:STDIN_LOG_MAX_SUBSCRIBERS]] [[src/target/shared/code/stdin_broadcast.rs:lower_stdin_next_byte]]

The log's memory is bounded: a stalled subscriber applies backpressure to the
reader rather than letting the log grow without limit, so a subscribed thread that
never reads can throttle the producer. Release the subscription with
`thread::closeStdIn` when a thread is done reading; thread teardown also
unsubscribes automatically.
[[src/target/shared/code/stdin_broadcast.rs:lower_stdin_recompute_base]]

## Overloads

**`thread::openStdIn() AS Nothing`**

Subscribes the calling thread.

**`thread::openStdIn(t AS Thread OF Msg TO Out) AS Nothing`**

Subscribes the worker behind a parent `Thread` handle. A `ThreadWorker` handle is
not accepted. [[src/builtins/thread.rs:resolve_call]] [[src/builtins/thread.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg TO Out` | Optional. A parent `Thread` handle whose worker should be subscribed. When omitted, the calling thread is subscribed. Borrowed, not consumed; the call reads only the worker's arena-state pointer. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | No value. On return the target thread is subscribed at the current stream frontier, unless the 128-entry subscriber registry was full. [[src/builtins/thread.rs:resolve_call]] |

## Errors

No errors.

## Type checking

Either zero arguments, or exactly one parent `Thread OF Msg TO Out`. A
`ThreadWorker` handle or any other type fails to resolve. The result is always
`Nothing`. [[src/builtins/thread.rs:is_parent_thread_type]]

## Examples

Subscribe the calling thread explicitly, then release it:

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

Grant a worker access to stdin before it reads:

```
IMPORT thread
IMPORT stdin_workers

FUNC main AS Integer
  LET w AS Thread OF Integer TO Integer = thread::start(stdin_workers::doubleIt, 21, 1, 1)
  thread::openStdIn(w)
  thread::send(w, 1)
  RETURN thread::waitFor(w)
END FUNC
```

## See also

- `mfb man thread closeStdIn`
- `mfb man thread start`
- `mfb man io readLine`
- `mfb man io readChar`
- `mfb man io pollInput`
