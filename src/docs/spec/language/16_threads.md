# 16. Threads

Threads are isolated execution contexts created from `ISOLATED FUNC` entry points. They do not share lexical scope, package state, mutable collections, or resources with their parent thread or with each other.

```basic
IMPORT workers
IMPORT thread

' workers/jobs.mfb
' EXPORT ISOLATED FUNC parseFile(worker AS ThreadWorker OF String TO Integer, path AS String) AS Integer

LET t = thread::start(workers::parseFile, "data.csv")

WHILE thread::isRunning(t)
  IF thread::poll(t, 10) THEN
    LET message = thread::receive(t)
    io::print(message)
  END IF
END WHILE

LET count = thread::waitFor(t)
io::print("Parsed " & toString(count) & " records")
```

Rules:

- A thread entry point must have type `ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out`. The worker handle is passed as the first argument by the runtime when the worker starts.
- A thread entry point must name an `ISOLATED FUNC` — that is the whole rule. It may be one declared in the current project, at any visibility (`PRIVATE`, `PUBLIC` or `EXPORT`) and in a project of any `kind`, named by its bare identifier (`thread::start(worker, …)`); or an `EXPORT ISOLATED FUNC` of an imported package, named through its import binding (`thread::start(pkg::worker, …)`). Inside a `kind: "package"` project the reserved `IMPORT self` specifier (`thread::start(self::worker, …)`) remains accepted as a spelling of the bare name. Nothing about *how* the entry is reached matters: `ISOLATED` is the sole marker. See `./mfb spec language modules-and-packages` for the `self` specifier.
- A thread entry point must not be a `SUB`.
- A thread entry point must not be a closure or lambda. It must be a named top-level function.
- Each started thread receives its own fresh instance of the project that *declares* the entry — an imported package, or the current project when the entry is a local `ISOLATED FUNC` — including a distinct worker arena. Its top-level bindings are initialized from their declarations, not zeroed. Starting isolated functions from the same project more than once creates independent state for each thread, and the parent's own copy is never mutated by a worker.
- Thread arguments and messages are copied, moved, or frozen when they enter a thread. Values read from a thread are copied, moved, or frozen when they leave the thread. No sender and receiver can observe or mutate the same live value. (How heap-backed boundary values are materialized in receiver-valid storage is a runtime detail; see `./mfb spec threading queue-semantics`.)
- Thread boundary types must be thread-sendable. Primitive owned values, `String`, `Nothing`, records, unions, and immutable containers are sendable when every contained field, payload, element, key, or value type is sendable. Functions, lambdas, `Thread`, `ThreadWorker`, and opaque resource handles are not sendable by default. (A worker outcome — internally a fallible result — is sendable when its success type is.) [[src/ir/verify/resources.rs:is_thread_sendable]]
- Concrete resource types opt in to thread sendability. Every standard transport and file handle is sendable — `fs::File`, `tcp::Socket`, `tcp::Listener`, `udp::Socket`, `tls::Socket` and `tls::Listener`. `process::Process`, the `audio` streams and `canvas::Image` are not: each is driven by a device or child-process thread of its own. A successful send of a non-copyable sendable resource moves ownership to the destination side immediately; a failed send leaves ownership with the sender.
- A thread's top-level `MUT` state is private to that thread's package instance.
- If the thread entry function succeeds with `v`, the thread's stored outcome carries the success value `v`. If it fails with `Error(e)`, including through auto-propagation, the stored outcome carries `e`. The runtime keeps any worker-arena-backed outcome valid until `thread::waitFor(t)` exposes a receiver-owned copy (runtime detail; see `./mfb spec threading queue-semantics`).
- The `Thread` value owns the completed outcome after the thread ends until it is retrieved. `thread::waitFor(t)` waits until completion, retrieves the outcome, auto-unwraps the `Out` value or auto-propagates the `Error` like any other function call, and consumes/closes the parent `Thread` handle. After retrieval, any further use of the same `Thread` handle fails with `ErrResourceClosed`.

The `thread` package exposes:

```basic
thread::start OF In, Msg, Out(f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, data AS In, inboundLimit AS Integer = 64, outboundLimit AS Integer = 64) AS Thread OF Msg TO Out
thread::isRunning OF Msg, Out(t AS Thread OF Msg TO Out) AS Boolean
thread::waitFor OF Msg, Out(t AS Thread OF Msg TO Out) AS Out
thread::cancel OF Msg, Out(t AS Thread OF Msg TO Out) AS Nothing
thread::send OF Msg, Out(t AS Thread OF Msg TO Out, data AS Msg, timeoutMs AS Integer) AS Nothing
thread::poll OF Msg, Out(t AS Thread OF Msg TO Out, ms AS Integer) AS Boolean
thread::receive OF Msg, Out(t AS Thread OF Msg TO Out, timeoutMs AS Integer) AS Msg
thread::send OF Msg, Out(t AS ThreadWorker OF Msg TO Out, data AS Msg, timeoutMs AS Integer) AS Nothing
thread::receive OF Msg, Out(t AS ThreadWorker OF Msg TO Out, timeoutMs AS Integer) AS Msg
thread::isCancelled OF Msg, Out(t AS ThreadWorker OF Msg TO Out) AS Boolean
thread::transfer OF Msg, Res, Out(t AS Thread OF Msg RES Res TO Out, res AS RES Res, timeoutMs AS Integer) AS Nothing
thread::accept OF Msg, Res, Out(t AS Thread OF Msg RES Res TO Out, timeoutMs AS Integer) AS RES Res
thread::transfer OF Msg, Res, Out(t AS ThreadWorker OF Msg RES Res TO Out, res AS RES Res, timeoutMs AS Integer) AS Nothing
thread::accept OF Msg, Res, Out(t AS ThreadWorker OF Msg RES Res TO Out, timeoutMs AS Integer) AS RES Res
```

**Two planes across a thread boundary.** A thread type carries an optional resource plane: `Thread OF Msg RES Res TO Out` (and `ThreadWorker OF …`), where `RES Res` is the resource channel and may be omitted for a data-only thread (`Thread OF Msg TO Out`). A thread with only a resource channel is spelled `Thread OF RES Res TO Out` (the message slot defaults to `Nothing`). The two planes use **separate per-thread queues**, so a thread may carry both at once. The message channel (`thread::send` / `thread::receive` / `thread::poll`) carries **copyable, resource-free data**: a resource in the `Msg` slot is rejected with `TYPE_THREAD_RESOURCE_PLANE_REQUIRED`, which names the resource and the remedy — declare it on the `RES` plane. [[src/rules/table.rs:TYPE_THREAD_RESOURCE_PLANE_REQUIRED]] The same rule covers every **data** plane: the message, output and input slots, and a resource plane's deep-copied `STATE` payload, whether the resource is named directly or is reached through a collection element, a map value, or a record field. Resources cross on the **resource plane** (`thread::transfer` / `thread::accept`), typed by `Res`. `thread::transfer(t, res)` **moves** `res` to `t` (invalidation event #2, §15): the sender binding is consumed, with ownership returned to the sender on failure (a `TRAP` handler may reuse it). Only thread-sendable resource types may cross; a `Res` slot naming a resource type that is not thread-sendable is rejected with `TYPE_THREAD_NOT_SENDABLE`, which also remains the rule for a plane carrying a `FUNC` or a thread handle — neither has a resource plane to be moved to.

**A stateful resource names its `STATE` on the plane.** The plane's `RES` element may carry a `STATE T` clause — `Thread OF RES fs::File STATE Cursor TO Out` (and `ThreadWorker OF …`) — declaring the state the transferred resource carries. A transfer is a **move to a re-typer**: the accepting thread re-declares the resource type, and the `STATE` payload carries no runtime type tag, so the plane must name the state for both sides to agree (this is the escape rule of §15.5 at the thread boundary). Enforcement, decidable statically from the two type strings:

- `thread::transfer(t, res)` requires `res`'s `STATE` to **equal** the plane's element `STATE`. A stateful resource on a bare plane, a bare resource on a stateful plane, or two disagreeing states are each rejected at the `transfer` call with `TYPE_STATE_MISMATCH`. Unlike a `RES` parameter — a non-escaping alias, opaque to any state — a transfer escapes the frame, so bare does **not** accept any state here.
- `thread::accept(t)` on a `STATE T` plane returns `RES Res STATE T`, so the receiver binds `RES f AS Res STATE T` by agreement (a different `STATE` is rejected); on a bare plane it returns a bare `RES Res`, and binding a `STATE` onto that is the ordinary attach.

The `STATE` moves with the resource and is deep-copied into the receiving thread's arena, so the accepted handle owns an independent payload (no cross-thread lifetime coupling). The plane `STATE` rides an exported worker signature, so it holds across a package boundary.

Thread functions are ordinary built-in templates. Their `Msg` and `Out` parameters are resolved by the template rules in §3 from argument types and expected result types. `thread::start` gets `Msg` and `Out` from the started function's first `ThreadWorker OF Msg TO Out` parameter, and gets `In` from the started function's second parameter and the `data` argument. If a thread does not exchange messages, `Msg` may be `Nothing`.

Each thread has a bounded inbound queue and bounded outbound queue. `thread::start` rejects limits less than `1` with `ErrInvalidArgument`. `thread::send(Thread, ...)` sends a value to the worker inbound queue. `thread::receive(ThreadWorker, ...)` reads from that inbound queue and is valid only inside the running worker. `thread::send(ThreadWorker, ...)` sends to the parent-visible outbound queue. `thread::poll` waits up to `ms` milliseconds for an outbound message from the worker and returns `TRUE` when the next `thread::receive(Thread, ...)` can read without waiting. `thread::receive(Thread, ...)` reads the next outbound message; with no timeout it blocks until a message is available or the worker completes, and the immediate form `thread::receive(Thread, 0)` instead fails at once with `ErrTimeout` when no message is available on a still-open queue.

To sleep, call `os::sleep(ms)` (`./mfb spec stdlib os`) — there is no `thread::sleep`. It blocks the *calling* thread, whichever that is, and takes no handle: following the `ms` convention, `0` returns immediately, a positive `ms` blocks for at least that long, and a negative `ms` is `ErrInvalidArgument`. Where it runs selects the behavior, and the compiler does not need to know which: on the program's main thread it is a plain, uninterruptible wall-clock delay that does not observe cancellation; inside a worker it is a cancellation point — a sleep in progress wakes early and fails with `ErrInterrupted` when the parent requests cancellation (`thread::cancel`, or dropping the parent handle), and because the deadline is absolute a parent `send` arriving mid-sleep does not shorten it. That makes the worker case one of the runtime-managed worker waits governed by the cancellation contract below. `ErrInterrupted` is declared on every `os::sleep` — a shared `FUNC` can be called from both sides — but can only be raised in a worker.

The four waiting thread built-ins (`thread::send`, `thread::receive`, `thread::transfer`, `thread::accept`) obey the language **timeout convention** (`./mfb spec language builtin-functions` → "Timeout convention"): the trailing `timeoutMs AS Integer` is optional and, when **omitted, blocks unboundedly**; `0` is one immediate attempt; a positive value waits up to that many milliseconds; a negative value is `ErrInvalidArgument`. On the writing side (`thread::send`, `thread::transfer`) an unmet deadline — including `0` when the queue is full — fails with `ErrTimeout`, and omitting blocks until space frees or the queue/thread closes. On the reading side (`thread::receive`, `thread::accept`) `0` on a still-open empty queue and an expired positive timeout both fail with `ErrTimeout`; omitting blocks until a message/resource arrives, the queue closes, or the worker is cancelled. A *terminally* empty queue (closed, or a completed worker) fails with `ErrNotFound`, which is distinct from a deadline's `ErrTimeout`.

`thread::cancel` requests cooperative cancellation. It does not kill the worker immediately. The worker observes cancellation with `thread::isCancelled(t)` and should return or fail promptly. After cancellation is requested, new parent-side `thread::send` calls fail with `ErrInterrupted`; unread inbound messages may be discarded. Outbound messages already sent by the worker remain readable until drained. Runtime-managed worker cancellation points, including `thread::receive(ThreadWorker, ...)`, `thread::send(ThreadWorker, ...)`, and `os::sleep(...)` called inside a worker, wake and fail with `ErrInterrupted` when cancellation is requested. Other blocking built-ins that are implemented as runtime-managed waits, such as terminal input, blocking file reads, or network waits, must use the same cooperative error-return model when cancellation integration is provided. Cancellation points do not asynchronously kill the worker or interrupt arbitrary user/native code.

When a thread ends, its inbound queue is closed and further parent-side sends fail. Its outbound queue remains readable until drained; after it is empty, `thread::poll` returns `FALSE` and `thread::receive(Thread, ...)` fails with `ErrNotFound`. `thread::waitFor` may be used before or after draining messages; it retrieves the stored outcome exactly once and closes the parent `Thread` handle. Closing the handle drops any remaining queued outbound messages. Dropping a completed `Thread` handle releases all remaining queued messages. Dropping a running `Thread` handle requests cancellation and detaches the worker. (Worker-arena release timing and zombie-thread reclamation are runtime mechanics; see `./mfb spec threading queue-semantics`.)

`Thread` values are non-copyable owned handles and participate in lexical cleanup. Scope exit, `RETURN`, `FAIL`, `PROPAGATE`, auto-propagated errors, and trap routing drop live parent `Thread` handles in reverse declaration order together with other owned values. Reassigning a `MUT Thread` evaluates the right-hand side first; if that succeeds, the old handle is dropped before the binding stores the new handle. A `Thread` binding that has moved out through return or another consuming operation is not dropped by the source scope. `thread::waitFor(t)` closes the underlying handle but does not make the source binding syntactically moved; later user-visible operations fail with `ErrResourceClosed`, while compiler-generated lexical cleanup is idempotent for an already closed handle.

## See Also

* ./mfb spec threading queue-semantics — queue, cancellation, arena, and reclamation mechanics
* ./mfb spec threading source-model — impl enforcement of the thread source API
* ./mfb man thread — thread package function reference
