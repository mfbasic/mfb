# MFBASIC Threading

Last updated: 2026-06-17

This document specifies how MFBASIC threads are compiled, linked, and executed.
It complements:

- `specifications/mfbasic.md`
- `specifications/standard_package.md`
- `specifications/package_format.md`
- `specifications/memory_layouts.md`

The source-level API is the built-in `thread` package. This document describes
the implementation contract behind that API.

## 1. Goals

MFBASIC threads provide isolated package workers with typed input, output, and
message channels.

The model has these requirements:

- A thread entry point is an exported `ISOLATED FUNC` from an imported package.
- The worker runs in a native OS thread.
- The worker receives its own thread handle and one input value.
- The parent communicates with the worker through bounded typed queues.
- All values that cross a thread boundary have thread-sendable types.
- Sendable resource handles move across thread queues without being copied.
- The worker outcome is stored internally and retrieved exactly once by `thread::waitFor(t)`, closing the parent `Thread` handle.
- Package imports used by the worker must work inside the worker thread exactly
  as they work outside a thread.
- Native code generation must resolve all worker and package calls at link time;
  no dynamic source-level package lookup occurs at runtime.

## 2. Source Model

Thread entry points have this shape:

```text
EXPORT ISOLATED FUNC worker(t AS ThreadWorker OF Msg TO Out, input AS In) AS Out
  ...
END FUNC
```

`thread::start` accepts a function reference to such an exported function:

```text
thread::start OF In, Msg, Out(
  f AS ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out,
  data AS In,
  inboundLimit AS Integer = 64,
  outboundLimit AS Integer = 64
) AS Thread OF Msg TO Out
```

The compiler rejects:

- Lambdas and closures as thread entry points.
- `SUB` thread entry points.
- Non-isolated functions.
- Current-package functions.
- Functions that are not exported from an imported package.
- Functions whose first parameter is not `ThreadWorker OF Msg TO Out`.
- Functions whose return type does not match `Out`.

`Msg` is the message type used by `thread::send`, `thread::receive`, and
`thread::poll`. `Out` is the worker success type. `In` is the input value type
passed to `thread::start`.

`In`, `Msg`, and `Out` must be thread-sendable. Thread sendability is a type
metadata property carried in package type metadata and available to the compiler,
verifier, and runtime. It is not stored as a per-value flag in every value's
memory block.

Thread sendability is derived by type:

- Primitive owned values, `String`, and `Nothing` are sendable.
- `List OF T` is sendable when `T` is sendable.
- `Map OF K TO V` is sendable when `K` and `V` are sendable.
- Records are sendable when every field type is sendable.
- Unions are sendable when every payload type is sendable; a worker outcome (internally a fallible result) is sendable when its success type is.
- Opaque handles are not sendable by default. Each concrete handle type opts in.
- Standard `File`, `Socket`, and `UdpSocket` handles are sendable.
- `Listener`, `Thread`, and `ThreadWorker` handles are not sendable.

The compiler rejects statically known non-sendable `In`, `Msg`, or `Out` types
before lowering. Runtime helpers and the verifier still consult type metadata so
queued values can be moved, dropped, or closed correctly.

## 3. Isolation

`ISOLATED` means the worker is callable from a separate runtime thread without
capturing current stack locals, closures, or current-package private state.

An isolated worker may still call:

- Built-in package functions such as `io::print`, `fs::readText`, and
  `strings::split`.
- Public exports from packages it imports.
- Other code that is reachable through package metadata and native linking.

The worker must not depend on the parent stack frame. Values passed to a thread
or through thread queues are transferred by the runtime representation rules for
their type. Immutable owned values may be shared or copied only when that is
safe for the value representation; mutable or unique resources must preserve
ownership rules.

For copyable sendable values, crossing a thread boundary copies or freezes the
value as required by the representation. The sender's original binding remains
usable.

For non-copyable sendable values, including sendable resource handles, crossing a
thread boundary is an ownership move. A successful `thread::start` or
`thread::send` consumes the source binding on that control-flow path. Later use
of the moved binding is an after-move error.

## 4. Package Requirements

Thread entry functions live in `.mfp` packages. The executable imports the worker
package, and the worker package may import additional packages.

For example:

```text
app
  imports thread_import_worker

thread_import_worker
  exports callImpPrint
  imports thread_imp_print

thread_imp_print
  exports impPrint
```

If `callImpPrint` starts in a thread and calls `thread_imp_print::impPrint`, that
call must be resolved from package metadata, not from app source text.

Package builds do not merge dependency IR into the generated `.mfp`.
Instead, a package build compiles against installed dependency ABI metadata and
records the dependency in the package's Binary IR:

- `IMPORT_TABLE` records imported packages.
- `IMPORT_TABLE.usedSymbols` records the imported public symbols used while
  compiling the package.
- `ABI_INDEX` records the ABI hashes for exported symbols and dependency used
  symbols.
- `FUNCTION_TABLE` describes the package's own functions; their bodies are the
  structured Binary IR carried in the `IR` section.
- `EXPORT_TABLE` maps exported source names to package-local function ids.

The package format remains architecture-independent. Native symbols are derived
later by the executable native backend.

## 5. Function Ids And Package Calls

In the Binary IR, calls reference functions by id. A call is an `IrValue::Call`
or `IrValue::CallResult` node naming the target; auto-unwrapping is the ordinary
`Result` desugaring (a `MATCH`/`PROPAGATE` over the `CallResult`), not an opcode
pair.

Inside one package, local function ids are package-local. They are not globally
unique across `.mfp` files.

When compiling a package against dependencies, imported exported functions are
referenced by their imported logical identity (import name plus the resolved ABI
identity recorded in `IMPORT_TABLE`/`ABI_INDEX`), not baked against another
package's local ids.

At consumption time, the executable decodes each imported package's Binary IR
back into IR functions and **merges** them into the project IR under each
package's deterministic identity prefix (`<id>.package.symbol`). The merge applies
that prefix as a consistent link-time rename of every definition and every
reference, driven by the resolved dependency graph, and resolves logical
inter-package references to concrete prefixed names. Identical content reached via
two dependency paths shares one prefix and de-duplicates.

The consumer must not assume that package-local function id `0` in two packages is
the same function. It resolves through package identity plus exported symbol
during the IR merge, before anything is lowered.

## 6. Worker And Package Functions In The Single Codegen

Worker functions are ordinary IR carried in the package's Binary IR. There is no
separate package bytecode-to-native bridge and no `lower_package_export_function`
path: once the consumer decodes and merges a package's IR, **every** package
function — including thread workers — is lowered through the same
`IR -> NIR -> native` path as the executable's own code.

Consequently package functions automatically get every language feature the
executable path has: full control flow (`IF`/`WHILE`/`FOREACH`/`MATCH`),
function-level and inline `TRAP`, all built-ins, and inline-`TRAP`-on-a-built-in.
A worker body's `CallResult` of a built-in is just an IR node; there is no flat
built-in dispatch to fail on.

Each merged package function still receives a stable internal native symbol so
the linker can resolve cross-package and worker entry points:

```text
_mfb_pkg_<package>_<export>
```

Characters outside ASCII letters, digits, and underscore are sanitized to
underscore. Cross-package calls and worker entry points resolve to these symbols
after the IR merge, with `Nothing` results initialized to the canonical zero
value, the same as for the executable's own functions.

## 7. Thread Runtime Helpers

Source calls to the `thread` package lower to runtime helper calls. The native
backend provides stable helper symbols such as:

```text
_mfb_rt_thread_thread_start
_mfb_rt_thread_thread_isRunning
_mfb_rt_thread_thread_waitFor
_mfb_rt_thread_thread_cancel
_mfb_rt_thread_thread_send
_mfb_rt_thread_thread_poll
_mfb_rt_thread_thread_receive
_mfb_rt_thread_thread_isCancelled
_mfb_rt_thread_trampoline
```

These helpers are compiler-owned runtime helpers. They are not source-level
`LINK` imports and do not appear as package dependencies.

`thread::start` stores:

- The worker function pointer.
- The input value as a transferred or frozen thread-boundary value.
- Queue state.
- Result state.
- Cancellation state.
- The native OS thread handle.
- The worker package instance's runtime arena state.

It then asks the OS to start `_mfb_rt_thread_trampoline`.

The trampoline restores the runtime state required by generated code, calls the
worker export with:

```text
x0 = thread handle
x1 = input value
```

and stores the returned `Result OF Out` in the thread control block before
marking the thread complete. If that stored result references worker-arena
storage, the worker arena remains owned by the control block until the result is
materialized for the parent or the completed thread is released.

## 8. Control Block

The native thread handle points to a runtime control block. The current native
layout is an implementation ABI between helper lowering and generated code:

```text
offset  field
0       state
8       cancelled
16      result tag
24      result value
32      result error
40      inbound queue handle
48      outbound queue handle
56      OS handle
64      entry function pointer
72      input data
80      worker arena state
88      parent arena state
```

`state = 0` means running. `state = 1` means complete with an unretrieved result. `state = 2` means the parent `Thread` handle is closed because the result was retrieved or the handle was dropped.

The `result tag`, `result value`, and `result error` fields describe the
completed `Result OF Out`. Heap-backed success or error payloads stored through
these fields must either be runtime-owned transfer values, values materialized
into a receiver-valid arena, or values whose producer arena is kept live by the
control block until the one result retrieval materializes its receiver-owned
copy.

The inbound and outbound queue handle fields point to runtime-owned bounded
queue records, not directly to a single queued message. A queue record stores
its requested capacity, current occupancy, synchronization state, and a backing
ring/buffer of value slots. The source-level contract is bounded queues with the
behavior specified in `standard_package.md`; implementation changes must
preserve that contract.

Queue storage must preserve enough type metadata to drop or close queued values
without receiving them. For queued resource handles, the runtime uses the
resource close function recorded in package metadata. For queued composite
values, the runtime uses the type metadata table to walk owned fields or payloads
that require cleanup.

## 9. Queue Semantics

Each thread has:

- An inbound queue: parent sends with `thread::send(Thread, ...)`; worker
  receives with `thread::receive(ThreadWorker, ...)`.
- An outbound queue: worker sends with `thread::send(ThreadWorker, ...)`;
  parent observes with `thread::poll` and reads with
  `thread::receive(Thread, ...)`.

`thread::start` rejects queue limits below `1`.

`timeoutMs = 0` means non-blocking. Positive timeouts wait up to that many
milliseconds. Negative timeouts are invalid except where a specific overload
documents an indefinite worker-side wait. `thread::receive(ThreadWorker, -1)`
waits until a message, queue closure, or cancellation; if cancellation is
requested before or during that wait, it fails with `ErrInterrupted`.

For `thread::send`, ownership transfer is atomic with enqueue success:

- If enqueue succeeds, the destination side owns `data` immediately. While the
  value is queued, the destination queue owns it in receiver-valid storage or
  runtime transfer storage independent of the sender arena.
- If enqueue fails because the queue is full, closed, cancelled, timed out, or
  the timeout is invalid, ownership is not transferred and the sender still owns
  `data`.
- Code may attach an inline `TRAP` to `thread::send(...)` to separate the
  success path, where a non-copyable sent binding is moved, from the error
  handler, where it remains owned by the sender and can be released.

Receiving a non-copyable value moves it out of the queue into the receiver's
binding. Receiving a copyable value may copy or move according to the normal
representation rules. In all cases, a heap-backed received value is materialized
in storage valid for the receiving thread before user code observes it.

Cancellation is cooperative:

- `thread::cancel` sets the cancellation flag.
- New sends fail after cancellation is requested.
- The worker observes cancellation with `thread::isCancelled(t)`.
- Runtime-managed blocking cancellation points wake and fail with
  `ErrInterrupted` when cancellation is requested for their worker thread.
- The runtime does not forcibly kill the worker as normal cancellation behavior.

Cancellation points are built-in operations whose implementations can safely
return an error without abandoning partially moved values or held runtime locks.
The current runtime cancellation points are indefinitely blocking or timed waits
in `thread::receive` and `thread::send` on a `ThreadWorker`. If cancellation is
already requested before a worker enters one of these operations, the operation
fails immediately with `ErrInterrupted`. If cancellation is requested while the
operation is blocked, the runtime wakes the wait and the operation fails with
`ErrInterrupted`. Other blocking built-ins that are implemented as
runtime-managed waits, such as terminal input, blocking file reads, or network
waits, must use the same cooperative error-return model when cancellation
integration is provided.
Normal `TRAP` and auto-propagation behavior then runs in the worker.

Cancellation does not interrupt arbitrary user code, does not asynchronously
terminate the OS thread, and does not unwind out of foreign/native code that has
not registered a cancellation point. A worker in non-blocking computation must
still check `thread::isCancelled(t)` or call a cancellation-point operation to
observe the request.

There is intentionally no `thread::stop()` operation. Asynchronous termination
can kill a worker while it owns a resource handle, holds a queue lock, is moving
a non-copyable value, is writing its result, or is inside package/native code.
That would make ownership and cleanup ambiguous and can leak resources, poison
queues, or deadlock other threads. Stopping work must happen at cooperative
cancellation points where the worker can return normally and the runtime can
close or transfer every owned value exactly once.

There is also no separate `thread::detach()` source API. Dropping a running
`Thread` already requests cancellation and detaches the OS worker for eventual
runtime cleanup. A public detach operation would need the same ownership and
cleanup guarantees as dropping the handle, while making it easier for user code
to abandon a worker that still owns resources or queued values.

The compiler lowers ordinary lexical ownership cleanup for every live parent
`Thread` handle. Scope exit, `RETURN`, `FAIL`, `PROPAGATE`, auto-propagated
errors, and trap routing run the same drop helper in reverse declaration order.
Reassigning a `MUT Thread` evaluates the new value first, then drops the old
handle before storing the replacement. Bindings that have moved out through
return or another consuming operation are removed from the cleanup set. Handles
closed by `thread::waitFor(t)` remain safe for compiler-generated cleanup; the
drop helper is idempotent for an already closed handle.

When the worker completes:

- Inbound sends fail.
- The result is stored, and the control block owns any worker-arena lifetime
  needed to materialize the one parent-visible result retrieval.
- `thread::isRunning` returns `FALSE`.
- `thread::waitFor` returns or propagates the stored result and closes the
  parent `Thread` handle.
- Remaining outbound messages stay readable until drained.

If a queued value is never received, the destination queue/runtime drops or
closes it exactly once:

- Unreceived inbound messages are cleaned up by the worker-side runtime when the
  worker exits or the thread is torn down.
- Unreceived outbound messages are cleaned up when the parent drains them,
  waits and lets lexical cleanup drop the completed `Thread`, or drops/detaches
  the thread handle according to the source-level `Thread` lifetime rules.
- Dropping a running `Thread` requests cancellation and detaches the worker; any
  remaining queued values are still owned by their destination queues until the
  responsible runtime cleanup path runs.
- The worker arena may be reclaimed only after the worker result has been
  transferred out of that arena or the result has otherwise been retrieved, and
  every worker-to-parent message has either been transferred into outbound queue
  storage or dropped by cleanup.

## 10. OS Integration

Threads are real native OS threads.

### macOS aarch64

The macOS backend starts MFBASIC workers through libSystem pthreads:

```text
pthread_create(&controlBlock.osHandle, NULL, _mfb_rt_thread_trampoline, controlBlock)
```

The trampoline is a normal pthread start routine. The runtime must not start
workers with raw Mach `thread_create_running`, because package imports used by a
worker may call libSystem facilities that require pthread registration,
including pthread TLS, `pthread_self`, errno storage, locale and stdio locks,
malloc internals, and other libc state. Mach thread APIs such as
`mach_thread_self` are reserved for introspection helpers only and are not the
thread creation ABI.

The linker must support both branch-call imports and any data or GOT-style
relocations required by libSystem integration. Missing linker support is not an
acceptable substitute for thread functionality.

### Linux aarch64

The Linux backend is cross-compiled and does not invoke an external system
linker. The compiler emits dynamic ELF executables directly.

```text
<project>-glibc.out
<project>-musl.out
```

The glibc executable uses:

```text
interpreter /lib/ld-linux-aarch64.so.1
DT_NEEDED libc.so.6
DT_NEEDED libpthread.so.0
```

The musl executable uses:

```text
interpreter /lib/ld-musl-aarch64.so.1
DT_NEEDED libc.musl-aarch64.so.1
```

Musl exposes pthread entry points from libc, so a separate musl pthread library
dependency is not required for the current backend.

`thread::start` calls `pthread_create` with:

```text
pthread_create(&controlBlock.osHandle, NULL, _mfb_rt_thread_trampoline, controlBlock)
```

The Linux trampoline is a normal pthread start routine. It preserves the
callee-saved runtime registers required by generated code, restores the worker
arena state, calls the worker export, stores the returned result in the control
block, keeps the worker arena live as needed for that result, marks the worker
complete, and returns `NULL` to pthread.

Linux threaded programs do not explicitly destroy the main runtime arena during
process shutdown. A worker may still be running when the main function returns,
and unmapping shared runtime memory would race that worker. Process exit lets
the OS reclaim the arena instead.

Raw Linux thread syscalls such as `clone`, `clone3`, `futex`, `set_tid_address`,
`gettid`, `tgkill`, and thread-local raw `exit` are not the threading ABI for
the current Linux backend. They may be used by libc internally, but generated
thread helpers must call the libc/pthread interface.

## 11. Linking Requirements

Executable native linking receives:

- The app's native module.
- The installed `.mfp` package files listed by the app manifest.
- Package exports from every installed package.
- Package import and ABI metadata from every installed package.
- Runtime helper symbols requested by app and package IR.

The native backend must:

1. Read all installed package exports.
2. Decode each installed package's Binary IR and merge its IR functions into the
   project IR under the package identity prefix.
3. Lower every merged package function through `IR -> NIR -> native`, deriving
   each package export symbol.
4. Add runtime helper imports for built-ins used inside package IR.
5. Resolve app calls to imported package exports.
6. Resolve package calls to other imported package exports by using the
   importing package's `IMPORT_TABLE` and the installed package set.
7. Validate ABI hashes before treating an installed package export as satisfying
   an imported used symbol.
8. Emit OS-specific runtime helper implementations or imports.
9. Link the final executable so worker calls, package calls, and runtime helpers
   all resolve to native symbols.

For Linux, runtime helpers used only inside package IR must still add the
same platform dynamic imports as helpers used by the app package. For example,
a worker package that calls `fs::readText`, `io::print`, or `thread::start`
must cause the final Linux executable to import the required libc, libm, or
libpthread symbols even if the app source does not call those helpers directly.

It is not valid to make a package-to-package call by preserving a raw
package-local function id and hoping the executable package order makes it
correct. Function ids are scoped to Binary IR payloads; the IR merge resolves
them through package identity plus exported symbol, and native symbols plus ABI
metadata define the executable-level call graph.

## 12. Error Propagation

Every MFBASIC function call returns a `Result` at the IR level. A source call
that is not directly matched is the ordinary `Result` auto-unwrap: a `CallResult`
whose `Ok` value flows on and whose `Error` propagates to the enclosing `TRAP`
(or the function's error result). This is structured IR — a `MATCH`/`PROPAGATE`
over the call — not an opcode pair.

The thread trampoline stores the worker's returned result tag and value/error in
the control block. `thread::waitFor` reads that stored result, materializes any
heap-backed payload into the caller's arena before user code observes it, and
closes the parent `Thread` handle before behaving like a normal fallible call:

- `Ok(value)` returns `value`.
- `Error(error)` propagates as the caller's error.

`thread::waitFor` is the only retrieval path; there is no `t.result` field. After
it retrieves the outcome, later use of the same `Thread` handle fails with
`ErrResourceClosed`.

Because worker and package functions ride the same `IR -> NIR -> native` path as
the executable's own code, this behavior is automatic for calls made inside a
worker. If a worker calls an imported package function and that function returns
`Err`, the worker returns or propagates the error according to the normal
structured `Result`/`TRAP` semantics — there is no separate bridge to keep in
sync.

## 13. Validation

Thread support is not validated by compiler output alone. A complete
implementation must include runtime tests that execute generated native
programs.

Required coverage includes:

- Starting a thread and printing from the worker.
- Sending parent-to-worker messages.
- Emitting worker-to-parent messages.
- Running two threads and cancelling them cooperatively.
- Using standard packages such as `strings`, `fs`, and `io` inside a worker.
- Passing data from main to a worker and returning structured data.
- Calling an imported package from inside a worker.
- Verifying the same runtime behavior on supported OS targets.

Acceptance tests should include `.run` goldens when behavior is observable
through stdout/stderr or exit code.
