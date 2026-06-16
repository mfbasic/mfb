# MFBASIC Threading

Last updated: 2026-06-14

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
- The worker result is stored as `Result OF Out`, with success exposed only through `MATCH` on the raw result and errors carried by public `Error` values.
- Package imports used by the worker must work inside the worker thread exactly
  as they work outside a thread.
- Native code generation must resolve all worker and package calls at link time;
  no dynamic source-level package lookup occurs at runtime.

## 2. Source Model

Thread entry points have this shape:

```text
EXPORT ISOLATED FUNC worker(t AS Thread OF Msg TO Out, input AS In) AS Out
  ...
END FUNC
```

`thread::start` accepts a function reference to such an exported function:

```text
thread::start OF In, Msg, Out(
  f AS ISOLATED FUNC(Thread OF Msg TO Out, In) AS Out,
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
- Functions whose first parameter is not `Thread OF Msg TO Out`.
- Functions whose return type does not match `Out`.

`Msg` is the message type used by `thread::send`, `thread::receive`,
`thread::emit`, `thread::poll`, and `thread::read`. `Out` is the worker success
type. `In` is the input value type passed to `thread::start`.

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

Package builds do not merge dependency bytecode into the generated `.mfp`.
Instead, a package build compiles against installed dependency ABI metadata and
records the dependency in the package bytecode:

- `IMPORT_TABLE` records imported packages.
- `IMPORT_TABLE.usedSymbols` records the imported public symbols used while
  compiling the package.
- `ABI_INDEX` records the ABI hashes for exported symbols and dependency used
  symbols.
- `FUNCTION_TABLE` stores the package's own bytecode functions.
- `EXPORT_TABLE` maps exported source names to package-local function ids.

The package format remains architecture-independent. Native symbols are derived
later by the executable native backend.

## 5. Function Ids And Package Calls

Bytecode calls use numeric function ids:

```text
CALL_RESULT   dstResult, functionId, argReg...
UNWRAP_RESULT dstValue, resultReg
```

Inside one package, local function ids are package-local. They are not globally
unique across `.mfp` files.

When compiling a package against dependencies, imported exported functions are
assigned deterministic temporary ids after the package's own function ids. The
order is:

```text
own package functions
then imported packages in IMPORT_TABLE order
then each imported package's exported function ids in EXPORT_TABLE order
```

Those ids are only a bytecode encoding device. At executable native-link time,
the backend reconstructs the same imported-function id mapping from the
importing package's `IMPORT_TABLE` and the installed package set, validates the
ABI through package metadata, and resolves each imported call to the concrete
native package export symbol.

The linker must not assume that package-local function id `0` in two packages is
the same function. It must resolve through package identity plus exported symbol.

## 6. Native Package Symbols

Each package export that is callable by native code receives a stable internal
symbol derived from package name and export name:

```text
_mfb_pkg_<package>_<export>
```

Characters outside ASCII letters, digits, and underscore are sanitized to
underscore.

Native package export lowering must support at least:

- Constants needed by the export body.
- Register moves and copies.
- Built-in runtime helper calls used by package bytecode.
- `CALL_RESULT` to another package export.
- `UNWRAP_RESULT` after a package call.
- Return of the success member carrying `value` or the error member carrying `error` using the native result ABI.

For package-to-package calls, native lowering loads bytecode argument registers
into ABI argument registers, branches to the resolved `_mfb_pkg_*` symbol,
checks the returned result tag, propagates the error member immediately, and stores the
returned value for the success member.

For `Nothing` results, the destination value is initialized to the canonical
zero value.

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
_mfb_rt_thread_thread_read
_mfb_rt_thread_thread_receive
_mfb_rt_thread_thread_emit
_mfb_rt_thread_thread_isCancelled
_mfb_rt_thread_trampoline
```

These helpers are compiler-owned runtime helpers. They are not source-level
`LINK` imports and do not appear as package dependencies.

`thread::start` stores:

- The worker function pointer.
- The input value.
- Queue state.
- Result state.
- Cancellation state.
- The native OS thread handle.
- The runtime arena state needed by the worker.

It then asks the OS to start `_mfb_rt_thread_trampoline`.

The trampoline restores the runtime state required by generated code, calls the
worker export with:

```text
x0 = thread handle
x1 = input value
```

and stores the returned `Result OF Out` in the thread control block before
marking the thread complete.

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
40      inbound count
48      inbound value
56      outbound count
64      outbound value
72      OS handle
80      entry function pointer
88      input data
96      stack pointer/base owned by the runtime
104     arena state
```

`state = 0` means running. `state = 1` means complete.

The current queue representation is intentionally minimal and may evolve into a
ring buffer. The source-level contract is bounded queues with the behavior
specified in `standard_package.md`; implementation changes must preserve that
contract.

## 9. Queue Semantics

Each thread has:

- An inbound queue: parent sends with `thread::send`; worker receives with
  `thread::receive`.
- An outbound queue: worker sends with `thread::emit`; parent observes with
  `thread::poll` and reads with `thread::read`.

`thread::start` rejects queue limits below `1`.

`timeoutMs = 0` means non-blocking. Positive timeouts wait up to that many
milliseconds. Negative timeouts are invalid.

Cancellation is cooperative:

- `thread::cancel` sets the cancellation flag.
- New sends fail after cancellation is requested.
- The worker observes cancellation with `thread::isCancelled`.
- The runtime does not forcibly kill the worker as normal cancellation behavior.

When the worker completes:

- Inbound sends fail.
- The result is stored.
- `thread::isRunning` returns `FALSE`.
- `thread::waitFor` returns or propagates the stored result.
- Remaining outbound messages stay readable until drained.

## 10. OS Integration

Threads are real native OS threads.

### macOS aarch64

The macOS backend uses libSystem/Mach thread primitives:

```text
thread_create
thread_create_running
thread_terminate
thread_suspend
thread_resume
thread_abort
thread_get_state
thread_set_state
thread_info
mach_thread_self
```

`thread_create_running` starts `_mfb_rt_thread_trampoline` on a runtime-owned
stack. Mach thread state is set so the trampoline receives the thread control
block and can restore the runtime arena state before calling the worker.

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
block, marks the worker complete, and returns `NULL` to pthread.

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
- Runtime helper symbols requested by app and package bytecode.

The native backend must:

1. Read all installed package exports.
2. Emit one native function for each reachable package export.
3. Derive each package export symbol.
4. Add runtime helper imports for built-ins used inside package bytecode.
5. Resolve app calls to imported package exports.
6. Resolve package calls to other imported package exports by using the
   importing package's `IMPORT_TABLE` and the installed package set.
7. Validate ABI hashes before treating an installed package export as satisfying
   an imported used symbol.
8. Emit OS-specific runtime helper implementations or imports.
9. Link the final executable so worker calls, package calls, and runtime helpers
   all resolve to native symbols.

For Linux, runtime helpers used only inside package bytecode must still add the
same platform dynamic imports as helpers used by the app package. For example,
a worker package that calls `fs::readText`, `io::print`, or `thread::start`
must cause the final Linux executable to import the required libc, libm, or
libpthread symbols even if the app source does not call those helpers directly.

It is not valid to make a package-to-package call by preserving a raw
package-local function id and hoping the executable package order makes it
correct. Function ids are scoped to bytecode payloads; native symbols and ABI
metadata define the executable-level call graph.

## 12. Error Propagation

Every MFBASIC function call returns a `Result` at bytecode level. A source call
that is not directly matched lowers to:

```text
CALL_RESULT
UNWRAP_RESULT
```

The thread trampoline stores the worker's returned result tag and value/error in
the control block. `thread::waitFor` reads that stored result and behaves like a
normal fallible call:

- `Ok(value)` returns `value`.
- `Error(error)` propagates as the caller's error.

Package bridge lowering must preserve the same behavior for calls made inside a
worker. If a worker calls an imported package function and that function returns
`Err`, the worker returns or propagates the error according to normal bytecode
semantics.

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
