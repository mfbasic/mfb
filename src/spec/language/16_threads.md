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
WEND

LET count = thread::waitFor(t)
io::print("Parsed " & toString(count) & " records")
```

Rules:

- A thread entry point must have type `ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out`. The worker handle is passed as the first argument by the runtime when the worker starts.
- A thread entry point must be an exported `ISOLATED FUNC` from an imported package. Starting a function from the current package is a compile error.
- A thread entry point must not be a `SUB`.
- A thread entry point must not be a closure or lambda. It must be a named package function.
- Each started thread receives its own fresh instance of the entry function's package, including a distinct worker arena. Starting isolated functions from the same package more than once creates independent package state for each thread.
- Thread arguments and messages are copied, moved, or frozen when they enter a thread. Values read from a thread are copied, moved, or frozen when they leave the thread. No sender and receiver can observe or mutate the same live value. (How heap-backed boundary values are materialized in receiver-valid storage is a runtime detail; see `./mfb spec threading queue-semantics`.)
- Thread boundary types must be thread-sendable. Primitive owned values, `String`, `Nothing`, records, unions, and immutable containers are sendable when every contained field, payload, element, key, or value type is sendable. Functions, lambdas, `Thread`, `ThreadWorker`, and opaque resource handles are not sendable by default. (A worker outcome — internally a fallible result — is sendable when its success type is.) [[src/typecheck/resources.rs:is_thread_sendable_type]]
- Concrete resource types opt in to thread sendability. Standard `File`, `Socket`, and `UdpSocket` handles are sendable; `Listener` and `TlsSocket` are not. A successful send of a non-copyable sendable resource moves ownership to the destination side immediately; a failed send leaves ownership with the sender.
- A thread's top-level `MUT` state is private to that thread's package instance.
- If the thread entry function succeeds with `v`, the thread's stored outcome carries the success value `v`. If it fails with `Error(e)`, including through auto-propagation, the stored outcome carries `e`. The runtime keeps any worker-arena-backed outcome valid until `thread::waitFor(t)` exposes a receiver-owned copy (runtime detail; see `./mfb spec threading queue-semantics`).
- The `Thread` value owns the completed outcome after the thread ends until it is retrieved. `thread::waitFor(t)` waits until completion, retrieves the outcome, auto-unwraps the `Out` value or auto-propagates the `Error` like any other function call, and consumes/closes the parent `Thread` handle. After retrieval, any further use of the same `Thread` handle fails with `ErrResourceClosed`.

The `thread` package exposes:

```basic
thread::start OF In, Msg, Out(f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, data AS In, inboundLimit AS Integer = 64, outboundLimit AS Integer = 64) AS Thread OF Msg TO Out
thread::isRunning OF Msg, Out(t AS Thread OF Msg TO Out) AS Boolean
thread::waitFor OF Msg, Out(t AS Thread OF Msg TO Out) AS Out
thread::cancel OF Msg, Out(t AS Thread OF Msg TO Out) AS Nothing
thread::send OF Msg, Out(t AS Thread OF Msg TO Out, data AS Msg, timeoutMs AS Integer = 0) AS Nothing
thread::poll OF Msg, Out(t AS Thread OF Msg TO Out, ms AS Integer) AS Boolean
thread::receive OF Msg, Out(t AS Thread OF Msg TO Out, timeoutMs AS Integer = 0) AS Msg
thread::send OF Msg, Out(t AS ThreadWorker OF Msg TO Out, data AS Msg, timeoutMs AS Integer = 0) AS Nothing
thread::receive OF Msg, Out(t AS ThreadWorker OF Msg TO Out, timeoutMs AS Integer = 0) AS Msg
thread::isCancelled OF Msg, Out(t AS ThreadWorker OF Msg TO Out) AS Boolean
thread::transfer OF Msg, Res, Out(t AS Thread OF Msg RES Res TO Out, res AS RES Res, timeoutMs AS Integer = 0) AS Nothing
thread::accept OF Msg, Res, Out(t AS Thread OF Msg RES Res TO Out, timeoutMs AS Integer = 0) AS RES Res
thread::transfer OF Msg, Res, Out(t AS ThreadWorker OF Msg RES Res TO Out, res AS RES Res, timeoutMs AS Integer = 0) AS Nothing
thread::accept OF Msg, Res, Out(t AS ThreadWorker OF Msg RES Res TO Out, timeoutMs AS Integer = 0) AS RES Res
```

**Two planes across a thread boundary.** A thread type carries an optional resource plane: `Thread OF Msg RES Res TO Out` (and `ThreadWorker OF …`), where `RES Res` is the resource channel and may be omitted for a data-only thread (`Thread OF Msg TO Out`). A thread with only a resource channel is spelled `Thread OF RES Res TO Out` (the message slot defaults to `Nothing`). The two planes use **separate per-thread queues**, so a thread may carry both at once. The message channel (`thread::send` / `thread::receive` / `thread::poll`) carries **copyable, resource-free data**: a resource in the `Msg` slot is rejected (`TYPE_THREAD_NOT_SENDABLE` — declare it on the `RES` plane). Resources cross on the **resource plane** (`thread::transfer` / `thread::accept`), typed by `Res`. `thread::transfer(t, res)` **moves** `res` to `t` (invalidation event #2, §15): the sender binding is consumed, with ownership returned to the sender on failure (a `TRAP` handler may reuse it). `thread::accept(t)` receives a transferred resource and binds it with `RES`; a resource's `STATE` is declared on that binding and moves with the resource. Only thread-sendable resource types may cross.

Thread functions are ordinary built-in templates. Their `Msg` and `Out` parameters are resolved by the template rules in §3 from argument types and expected result types. `thread::start` gets `Msg` and `Out` from the started function's first `ThreadWorker OF Msg TO Out` parameter, and gets `In` from the started function's second parameter and the `data` argument. If a thread does not exchange messages, `Msg` may be `Nothing`.

Each thread has a bounded inbound queue and bounded outbound queue. `thread::start` rejects limits less than `1` with `ErrInvalidArgument`. `thread::send(Thread, ...)` sends a value to the worker inbound queue. `thread::receive(ThreadWorker, ...)` reads from that inbound queue and is valid only inside the running worker. `thread::send(ThreadWorker, ...)` sends to the parent-visible outbound queue. `thread::poll` waits up to `ms` milliseconds for an outbound message from the worker and returns `TRUE` when `thread::receive(Thread, ...)` can read without blocking. `thread::receive(Thread, ...)` reads the next outbound message. Reading with no available message fails with `ErrNotFound`.

For queue operations, `timeoutMs = 0` means do not wait. A positive timeout waits up to that many milliseconds for space or data. Sending to a full queue or receiving from an empty queue after the timeout fails with `ErrTimeout`. Negative timeouts are invalid except where a specific overload documents an indefinite worker-side wait, such as `thread::receive(ThreadWorker, -1)`.

`thread::cancel` requests cooperative cancellation. It does not kill the worker immediately. The worker observes cancellation with `thread::isCancelled(t)` and should return or fail promptly. After cancellation is requested, new parent-side `thread::send` calls fail with `ErrInterrupted`; unread inbound messages may be discarded. Outbound messages already sent by the worker remain readable until drained. Runtime-managed worker queue cancellation points, including `thread::receive(ThreadWorker, ...)` and `thread::send(ThreadWorker, ...)`, wake and fail with `ErrInterrupted` when cancellation is requested. Other blocking built-ins that are implemented as runtime-managed waits, such as terminal input, blocking file reads, or network waits, must use the same cooperative error-return model when cancellation integration is provided. Cancellation points do not asynchronously kill the worker or interrupt arbitrary user/native code.

When a thread ends, its inbound queue is closed and further parent-side sends fail. Its outbound queue remains readable until drained; after it is empty, `thread::poll` returns `FALSE` and `thread::receive(Thread, ...)` fails with `ErrNotFound`. `thread::waitFor` may be used before or after draining messages; it retrieves the stored outcome exactly once and closes the parent `Thread` handle. Closing the handle drops any remaining queued outbound messages. Dropping a completed `Thread` handle releases all remaining queued messages. Dropping a running `Thread` handle requests cancellation and detaches the worker. (Worker-arena release timing and zombie-thread reclamation are runtime mechanics; see `./mfb spec threading queue-semantics`.)

`Thread` values are non-copyable owned handles and participate in lexical cleanup. Scope exit, `RETURN`, `FAIL`, `PROPAGATE`, auto-propagated errors, and trap routing drop live parent `Thread` handles in reverse declaration order together with other owned values. Reassigning a `MUT Thread` evaluates the right-hand side first; if that succeeds, the old handle is dropped before the binding stores the new handle. A `Thread` binding that has moved out through return or another consuming operation is not dropped by the source scope. `thread::waitFor(t)` closes the underlying handle but does not make the source binding syntactically moved; later user-visible operations fail with `ErrResourceClosed`, while compiler-generated lexical cleanup is idempotent for an already closed handle.

## See Also

* ./mfb spec threading queue-semantics — queue, cancellation, arena, and reclamation mechanics
* ./mfb spec threading source-model — impl enforcement of the thread source API
* ./mfb man thread — thread package function reference
