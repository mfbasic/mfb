# accept

Take a transferred resource from a thread's resource plane.

## Synopsis

```
thread::accept(t AS Thread OF Msg RES Res TO Out) AS Res
thread::accept(t AS Thread OF Msg RES Res TO Out, timeoutMs AS Integer) AS Res
thread::accept(t AS ThreadWorker OF Msg RES Res TO Out) AS Res
thread::accept(t AS ThreadWorker OF Msg RES Res TO Out, timeoutMs AS Integer) AS Res
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

`thread::accept` is the receiving side of the resource plane. It dequeues one
resource that a matching `thread::transfer` moved across the thread boundary and
hands ownership to the caller, which binds it with `RES`. A `ThreadWorker` handle
reads the worker's inbound resource queue (what the parent transferred); a parent
`Thread` handle reads the outbound resource queue (what the worker transferred
back). The direction is chosen during lowering from the static handle type, so a
thread never re-reads its own transfer.
[[src/ir/lower.rs:thread_resource_plane_target]] [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

The return type is the plane's element type, taken structurally from the handle's
`RES` clause. Where the plane declares a state
(`Thread OF RES File STATE Cursor TO Out`), `accept` returns `File STATE Cursor`,
so the receiver binds `RES f AS File STATE Cursor` by agreement and a different
`STATE` is rejected; on a bare plane it returns a bare resource and attaching a
`STATE` to that binding is the ordinary attach. The `STATE` payload arrives with
the resource, deep-copied into the receiving thread's arena, so the accepted
handle owns an independent copy with no cross-thread lifetime coupling. Calling
`accept` on a data-only thread type — one with no `RES` clause — fails to resolve.
[[src/builtins/thread.rs:thread_resource]] [[src/builtins/thread.rs:resolve_call]]

Dequeuing moves the resource out of the queue, frees a slot, and signals the
queue's *not-full* condition, so a sender blocked on a full resource queue can
proceed. The accepted resource is owned by the receiving side and is closed by
lexical drop at scope exit unless closed earlier.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_read_helper]]

There are two forms, sharing the read machinery with `thread::receive`. Omitting
`timeoutMs` **blocks**: lowering passes an unreachable sentinel timeout and the
helper waits indefinitely until a resource arrives, the queue closes, or
cancellation is observed. Supplying `timeoutMs` bounds the wait and it must be
`>= 0`: `0` does not wait, a positive value waits that many milliseconds against
an absolute deadline. A negative explicit `timeoutMs` is rejected with
`ErrInvalidArgument` — omit the argument to wait indefinitely.
[[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]] [[src/target/shared/code/runtime_helpers_thread.rs:ThreadReadMode]]

When no resource is available the reason decides the error, checked in this order:
on a worker handle a set cancelled flag gives `ErrInterrupted`; a queue marked
closed gives `ErrNotFound`; on a parent handle a closed thread state gives
`ErrResourceClosed` and a completed worker gives `ErrNotFound`; `timeoutMs = 0`
gives `ErrNotFound`; and an expired positive `timeoutMs` gives `ErrTimeout`. Both
resource queues are closed and broadcast by `thread::cancel`, by dropping a
running parent handle, and on worker exit, so a blocking `accept` always wakes
rather than hanging.
[[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_read_helper]] [[src/target/shared/code/runtime_helpers_thread.rs:emit_close_resource_queues]]

## Overloads

**`thread::accept(t) AS Res`**

Blocking accept on either handle kind: waits until a resource arrives, the queue
closes, or cancellation is observed.

**`thread::accept(t, timeoutMs AS Integer) AS Res`**

Bounded accept on either handle kind: `timeoutMs` must be `>= 0`; `0` polls once
and fails with `ErrNotFound` when the queue is empty, a positive value waits that
long and then fails with `ErrTimeout`. [[src/builtins/thread.rs:resolve_call]] [[src/builtins/thread.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg RES Res TO Out` or `ThreadWorker OF Msg RES Res TO Out` | The handle whose resource-plane queue is read. Must declare a `RES` plane. Borrowed, not consumed. [[src/builtins/thread.rs:call_param_names]] |
| `timeoutMs` | `Integer` | Optional. Omit to block until a resource arrives, the queue closes, or cancellation is observed. When supplied it must be `>= 0`: `0` does not wait, a positive value waits that many milliseconds. A negative value is rejected with `ErrInvalidArgument`. [[src/builtins/thread.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Res` | The dequeued resource, of the plane's element type including any `STATE` clause, moved to the caller under a `RES` binding and closed by lexical drop unless closed earlier. [[src/builtins/thread.rs:thread_resource]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | An explicit `timeoutMs` is negative. Omit the argument to wait indefinitely. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050004` | `ErrNotFound` | No resource is available and none will be waited for: `timeoutMs` is `0` and the queue is empty, the resource queue has been closed, or (parent-side) the worker has completed with an empty outbound resource queue. [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |
| `77050008` | `ErrTimeout` | The resource queue stayed empty until a positive `timeoutMs` elapsed. [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77050009` | `ErrInterrupted` | Worker-side only: cancellation has been requested for this worker. [[src/target/shared/code/error_constants.rs:ERR_INTERRUPTED_CODE]] |
| `77030004` | `ErrResourceClosed` | Parent-side only: the thread's state is closed while its outbound resource queue is not flagged closed. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Type checking

Generic over `Msg`, `Res`, and `Out`. `t` must be a `Thread` or `ThreadWorker`
carrying a `RES` plane; a data-only thread handle fails to resolve. The result is
the plane's element type, including its `STATE` clause when the plane declares
one, and the receiving binding must name the same `STATE`. `timeoutMs`, when
supplied, must be `Integer`. [[src/builtins/thread.rs:thread_resource]]

## Examples

Worker-side: accept a file the parent transferred, then use it:

```
IMPORT thread
IMPORT fs

ISOLATED FUNC sizeOfReceived(t AS ThreadWorker OF RES File TO Integer, seed AS String) AS Integer
  RES f AS File = thread::accept(t, 1000)
  RETURN len(fs::readAll(f))
END FUNC
```

Parent-side: accept a file the worker transferred back:

```
IMPORT thread
IMPORT fs
IMPORT xfer_bidi_worker

FUNC main AS Integer
  LET t AS Thread OF RES File TO Integer = thread::start(xfer_bidi_worker::exchange, "seed")
  RES pf AS File = fs::openFile("data/parent.txt")
  thread::transfer(t, pf)
  RES wf AS File = thread::accept(t)
  LET size AS Integer = len(fs::readAll(wf))
  fs::close(wf)
  RETURN thread::waitFor(t)
END FUNC
```

Accept a resource together with the `STATE` the plane declares:

```
IMPORT thread
IMPORT fs

TYPE Cursor
  pos AS Integer
END TYPE

ISOLATED FUNC takeCursor(t AS ThreadWorker OF RES File STATE Cursor TO Integer, seed AS String) AS Integer
  RES f AS File STATE Cursor = thread::accept(t, 1000)
  LET pos AS Integer = f.state.pos
  fs::close(f)
  RETURN pos
END FUNC
```

## See also

- `mfb man thread transfer`
- `mfb man thread receive`
- `mfb man thread send`
- `mfb man thread start`
- `mfb spec language resource-management`
