# transfer

Move a resource across a thread boundary on the resource plane.

## Synopsis

```
thread::transfer(t AS Thread OF Msg RES Res TO Out, res AS Res) AS Nothing
thread::transfer(t AS Thread OF Msg RES Res TO Out, res AS Res, timeoutMs AS Integer) AS Nothing
thread::transfer(t AS ThreadWorker OF Msg RES Res TO Out, res AS Res) AS Nothing
thread::transfer(t AS ThreadWorker OF Msg RES Res TO Out, res AS Res, timeoutMs AS Integer) AS Nothing
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

`thread::transfer` moves a thread-sendable resource across the thread boundary on
the **resource plane** — a pair of queues separate from the data plane, so a
thread can carry values and resources at once and the message channel stays
resource-free. A parent `Thread` handle transfers onto the worker's inbound
resource queue; a `ThreadWorker` handle transfers onto the parent-visible outbound
resource queue. The matching reader is `thread::accept` on the opposite handle.
Source `thread::transfer` is rewritten during IR lowering to an internal
resource-plane target, then split by handle direction, so the two directions can
never re-read each other's queue.
[[src/ir/lower.rs:thread_resource_plane_target]] [[src/target/shared/code/builder_values.rs:lower_runtime_helper_call]]

A thread type spells its resource plane with a `RES` clause —
`Thread OF Msg RES Res TO Out`, or `Thread OF RES Res TO Out` when there is no
data channel. `transfer` requires the handle to declare a plane: calling it on a
data-only thread, or on a plane whose element is not a resource type, is rejected
at compile time. Only resource types marked thread-sendable may cross; `File`,
`Socket`, and `UdpSocket` are sendable, while `Listener`, `TlsSocket`,
`TlsListener`, `AudioInput`, and `AudioOutput` are not.
[[src/syntaxcheck/resources.rs:require_thread_sendable_type]] [[src/builtins/resource.rs:BUILTIN_RESOURCES]]

**`transfer` moves the resource.** The `res` argument is evaluated in transfer
mode, so the sender's binding is consumed and ownership passes to the receiving
side — this is resource invalidation event #2. On a failed transfer no move
happened and ownership stays with the sender, so a `TRAP` handler may still use
the binding. Where an untracked alias nonetheless reaches an operation after a
successful transfer, that operation is refused with `ErrResourceMoved` (`77030009`)
rather than `ErrResourceClosed`, because the handle is not closed — it belongs to
another thread now. [[src/syntaxcheck/types.rs:thread_argument_mode]]

**A stateful resource must agree with the plane.** The plane's `RES` element may
carry a `STATE T` clause (`Thread OF RES File STATE Cursor TO Out`). The front end
resolves `transfer` on the *base* resource name only; the `STATE` agreement itself
is checked by IR verification, which rejects a stateful resource on a bare plane,
a bare resource on a stateful plane, and two disagreeing states, all with
`TYPE_STATE_MISMATCH`. A transfer escapes the frame into a thread that re-types
the resource, so — unlike a `RES` parameter, which is an opaque non-escaping alias
— "bare" does not accept any state here. The `STATE` payload travels with the
resource and is deep-copied into the receiving thread's arena, so the accepted
handle owns an independent copy. [[src/ir/verify/mod.rs:check_thread_transfer_state]] [[src/builtins/thread.rs:resolve_call]]

`timeoutMs` bounds the wait for space on a full destination resource queue and
defaults to `0`, filled in during lowering. `0` does not wait and fails at once
with `ErrTimeout`; a positive value waits that many milliseconds against an
absolute deadline; a negative value is rejected with `ErrInvalidArgument`. The
call is a cancellation point on the same terms as `thread::send`: it re-checks the
thread state and cancelled flag inside the wait loop, so a blocked transfer wakes
and fails with `ErrInterrupted` when the worker is cancelled, completes, or the
queue is closed — and `ErrResourceClosed` on a parent handle that is already
closed. [[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_write_helper]]

## Overloads

**`thread::transfer(t, res[, timeoutMs])` on a `Thread` handle**

Parent-side transfer onto the worker's inbound resource queue. Adds
`ErrResourceClosed` for an already-closed handle and `ErrInterrupted` for a
completed or cancelled worker.

**`thread::transfer(t, res[, timeoutMs])` on a `ThreadWorker` handle**

Worker-side transfer onto the parent-visible outbound resource queue. Fails with
`ErrInterrupted` when this worker is cancelled or the queue is closed.
[[src/builtins/thread.rs:resolve_call]] [[src/builtins/thread.rs:arity]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `t` (also `thread`) | `Thread OF Msg RES Res TO Out` or `ThreadWorker OF Msg RES Res TO Out` | The handle whose resource-plane queue receives the resource. Must declare a `RES` plane. Borrowed, not consumed. [[src/builtins/thread.rs:call_param_names]] |
| `res` (also `resource`) | `Res` | The resource to move. Its base resource type must match the plane's, and its `STATE` must equal the plane's `STATE`. **Consumed** on success; still owned by the sender on failure. [[src/builtins/thread.rs:call_param_names]] |
| `timeoutMs` | `Integer` | Optional, default `0`. Milliseconds to wait for space on a full resource queue. `0` fails immediately with `ErrTimeout`; a positive value waits that long; a negative value is rejected. |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | No value. Returns once the resource has been enqueued on the resource plane and the queue's *not-empty* condition signalled. [[src/builtins/thread.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `timeoutMs` is negative. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050008` | `ErrTimeout` | The destination resource queue is full and space did not free up before `timeoutMs` elapsed — immediately when `timeoutMs` is `0`. [[src/target/shared/code/error_constants.rs:ERR_TIMEOUT_CODE]] |
| `77050009` | `ErrInterrupted` | Cancellation was requested for the worker, the worker has completed (parent-side), or the destination resource queue has been marked closed. [[src/target/shared/code/runtime_helpers_thread.rs:thread_queue_write_helper]] |
| `77030004` | `ErrResourceClosed` | Parent-side only: the `Thread` handle is already closed, for example after `thread::waitFor`. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Type checking

Generic over `Msg`, `Res`, and `Out`. `t` must be a `Thread` or `ThreadWorker`
carrying a `RES` plane; `res`'s base resource type must equal the plane's base
resource type (a plane typed `Unknown` accepts any resource), and its `STATE` must
equal the plane's `STATE` or IR verification rejects the call with
`TYPE_STATE_MISMATCH`. A non-resource plane element, a data-only thread handle, or
a non-sendable resource type is rejected with `TYPE_THREAD_NOT_SENDABLE`.
`timeoutMs`, when supplied, must be `Integer`. The result is always `Nothing`.
[[src/ir/verify/mod.rs:check_thread_transfer_state]] [[src/syntaxcheck/resources.rs:require_thread_sendable_type]]

## Examples

Move an open file to a worker and let the worker size it:

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

Transfer a resource whose `STATE` the plane declares:

```
IMPORT thread
IMPORT fs
IMPORT state_xfer_workers

TYPE Cursor
  pos AS Integer
END TYPE

FUNC main AS Integer
  LET t AS Thread OF RES File STATE Cursor TO Integer = thread::start(state_xfer_workers::takeCursor, "seed")
  RES f AS File STATE Cursor = fs::openFile("data/input.txt")
  f.state.pos = 99
  thread::transfer(t, f, 10)
  RETURN thread::waitFor(t)
END FUNC
```

## See also

- `mfb man thread accept`
- `mfb man thread send`
- `mfb man thread receive`
- `mfb man thread start`
- `mfb spec language resource-management`
