# start

Start an imported isolated worker function on a new OS thread.

## Synopsis

```
thread::start(f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, data AS In) AS Thread OF Msg TO Out
thread::start(f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, data AS In, inboundLimit AS Integer) AS Thread OF Msg TO Out
thread::start(f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, data AS In, inboundLimit AS Integer, outboundLimit AS Integer) AS Thread OF Msg TO Out
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

`thread::start` runs the exported `ISOLATED FUNC` `f` on a fresh OS thread with
its own package instance and its own arena, and returns a parent-side `Thread`
handle immediately — it never waits for the worker to make progress or finish.
`f` is called with two arguments: the worker's own `ThreadWorker` handle, created
by the runtime, and `data` passed through as the second argument. The worker's
return value becomes the thread outcome, retrieved later in the parent with
`thread::waitFor`. [[src/builtins/thread.rs:matches_start]]

The returned handle's type is derived structurally from the worker parameter of
`f`, so a worker declared `ThreadWorker OF Msg RES Res TO Out` yields a
`Thread OF Msg RES Res TO Out` and the started thread carries both the data plane
and the resource plane. A worker with no data channel (`ThreadWorker OF RES Res
TO Out`) yields the resource-only spelling.
[[src/builtins/thread.rs:start_thread_type]] [[src/builtins/thread.rs:format_thread_type]]

`inboundLimit` and `outboundLimit` bound the queues in number of queued entries.
The call allocates **four** queues: the inbound and outbound data queues, and the
inbound and outbound resource-plane queues used by `thread::transfer` and
`thread::accept`. `inboundLimit` sizes both parent-to-worker queues (data and
resource); `outboundLimit` sizes both worker-to-parent queues. Each limit must be
at least `1` and at most `u64::MAX / 8`, the largest capacity whose backing array
size cannot wrap; either bound violated is `ErrInvalidArgument`. Omitting a limit
defaults it to `64`, filled in during lowering rather than by the runtime helper.
[[src/target/shared/code/runtime_helpers.rs:lower_thread_start_helper]] [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

Before the OS thread is spawned the parent allocates the thread control block and
a fresh arena state for the worker, zeroes that state, copies its own `Money`
rounding mode into it, and seeds the worker's own `math::rand` stream and
memory-fill stream from the parent's generators, so each worker gets an
independent random sequence. The worker is created with an explicit 8 MiB stack
because musl's 128 KiB default is far below what the main thread receives.
[[src/target/shared/code/runtime_helpers.rs:lower_thread_start_helper]]

`data` is **moved** into the call rather than borrowed, so a non-copyable value
handed to `thread::start` is consumed by it; the value must be thread-sendable.
[[src/syntaxcheck/types.rs:thread_argument_mode]] [[src/syntaxcheck/resources.rs:is_thread_sendable_type]]

`f` must be an exported `ISOLATED FUNC` from an imported package whose first
parameter is a `ThreadWorker` and whose declared output equals that worker's
output type. Current-package functions, `SUB`s, lambdas, closures, non-isolated
functions, and entry points without the leading `ThreadWorker` parameter are
rejected at compile time and never reach this call.
[[src/builtins/thread.rs:matches_start]] [[src/builtins/thread.rs:expected_arguments]]

The returned `Thread` is a non-copyable owned handle closed by lexical drop.
Dropping a still-running handle requests cancellation, closes and broadcasts all
four queues so nothing stays parked, and detaches the worker.
[[src/target/shared/code/runtime_helpers_thread.rs:simple_thread_handle_helper]]

## Overloads

**`thread::start(f, data) AS Thread OF Msg TO Out`**

Both queue limits default to `64`.

**`thread::start(f, data, inboundLimit) AS Thread OF Msg TO Out`**

Sets the parent-to-worker limit; `outboundLimit` still defaults to `64`.

**`thread::start(f, data, inboundLimit, outboundLimit) AS Thread OF Msg TO Out`**

Sets both limits explicitly. [[src/builtins/thread.rs:resolve_call]] [[src/builtins/thread.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `f` (also `entry`) | `ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out` | The exported isolated worker entry point to run. Receives the worker's `ThreadWorker` handle and `data`, and returns the thread outcome of type `Out`. [[src/builtins/thread.rs:call_param_names]] |
| `data` | `In` | The value handed to the worker as its second argument. Moved into the call, not borrowed, and must be thread-sendable. [[src/syntaxcheck/types.rs:thread_argument_mode]] |
| `inboundLimit` | `Integer` | Optional. Capacity of both parent-to-worker queues (data and resource plane), in entries. Must be in `1 ..= u64::MAX / 8`. Defaults to `64`. |
| `outboundLimit` | `Integer` | Optional. Capacity of both worker-to-parent queues (data and resource plane), in entries. Must be in `1 ..= u64::MAX / 8`. Defaults to `64`. |

## Return value

| Type | Description |
| --- | --- |
| `Thread OF Msg TO Out` | A live parent-side handle for the newly started worker, carrying the worker's `Msg`, optional `RES Res` plane, and `Out` types. Usable with `thread::send`, `thread::receive`, `thread::poll`, `thread::isRunning`, `thread::cancel`, `thread::transfer`, `thread::accept`, and `thread::waitFor`. [[src/builtins/thread.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `inboundLimit` or `outboundLimit` is below `1`, or above `u64::MAX / 8` (the cap that keeps the backing array size from wrapping). [[src/target/shared/code/runtime_helpers.rs:lower_thread_start_helper]] |
| `77010001` | `ErrOutOfMemory` | The thread control block, the worker's arena state, any of the four queue structures, or any queue's backing value array cannot be allocated — including when `capacity * 8` would overflow 64 bits. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |
| `77050009` | `ErrInterrupted` | The OS thread cannot be spawned (`pthread_create` fails), or a queue's mutex or condition variable cannot be initialized (`pthread_mutex_init` / `pthread_cond_init` fails). [[src/target/shared/code/runtime_helpers.rs:lower_thread_start_helper]] |

## Type checking

Generic over `In`, `Msg`, and `Out`. `Msg` and `Out` come from the first
(`ThreadWorker`) parameter of `f`; `In` must equal both the second parameter type
of `f` and the static type of `data`. The declared output of `f` must equal the
worker's `Out`. Optional limits must be `Integer`. Any mismatch fails to resolve
at compile time. [[src/builtins/thread.rs:matches_start]]

## Examples

Start a worker with the default queue limits and join it:

```
IMPORT thread
IMPORT thread_runtime_workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(thread_runtime_workers::printReceived, "start")
  thread::send(t, "alpha")
  LET ack AS String = thread::receive(t)
  LET result AS Integer = thread::waitFor(t)
  RETURN result
END FUNC
```

Start a worker whose type carries a resource plane:

```
IMPORT thread
IMPORT fs
IMPORT thread_file_sink

FUNC main AS Integer
  LET t AS Thread OF RES File TO Integer = thread::start(thread_file_sink::sizeOfReceived, "seed")
  RES f AS File = fs::openFile("data/input.txt")
  thread::transfer(t, f)
  LET size AS Integer = thread::waitFor(t)
  RETURN 0
END FUNC
```

## See also

- `mfb man thread waitFor`
- `mfb man thread send`
- `mfb man thread receive`
- `mfb man thread cancel`
- `mfb man thread isRunning`
- `mfb man thread transfer`
