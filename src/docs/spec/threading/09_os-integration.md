# OS Integration

Threads are real native OS threads.

## macOS aarch64

The macOS backend starts MFBASIC workers through libSystem pthreads:

```text
pthread_create(&controlBlock.osHandle, attr, _mfb_rt_thread_trampoline, controlBlock)
  ; attr = a pthread_attr_t initialized with an 8 MiB stack (pthread_attr_setstacksize)
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

## Linux aarch64

The Linux backend is cross-compiled and does not invoke an external system
linker. The compiler emits dynamic ELF executables directly.

It emits both a glibc (`<project>/<project>-glibc.out`) and a musl
(`<project>/<project>-musl.out`) flavor into the project's output directory. Each carries the ELF interpreter path and `DT_NEEDED` list its libc
requires; the glibc flavor names libpthread separately, while musl exposes pthread
entry points from libc, so no separate musl pthread dependency is needed. The
exact interpreter paths and soname list are owned by
`./mfb spec linker linux-aarch64`.

`thread::start` calls `pthread_create` with:

```text
pthread_create(&controlBlock.osHandle, attr, _mfb_rt_thread_trampoline, controlBlock)
  ; attr = a pthread_attr_t initialized with an 8 MiB stack (pthread_attr_setstacksize)
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
the Linux backend. They may be used by libc internally, but generated
thread helpers must call the libc/pthread interface. [[src/target/shared/code/runtime_helpers.rs:lower_thread_start_helper]]

## Standard Input Broadcast

Standard input (file descriptor 0) is owned by the runtime and served through one
process-global append-only broadcast log, so it is safe to read from more than one
thread. The runtime reads `fd 0` in chunks into the log; each *subscribed* thread
holds its own cursor over the log and reads independently, so every subscriber sees
the whole stdin byte stream from its subscription point and a byte read by one
thread is never consumed out from under another. This is the only cross-thread
shared mutable state added for stdin; it is guarded by its own
`pthread_mutex_t`/`pthread_cond_t` (the same primitives the transfer queues use) and
never leaks a pointer across an arena boundary — log blocks are `malloc`/`free`d,
never per-arena. A cooperative reader (no dedicated thread) lets exactly one
subscriber at a time perform the blocking `read(0,…)`, and that syscall is never
issued while the data lock is held.

A thread subscribes with `thread::openStdIn` and unsubscribes with
`thread::closeStdIn` (the no-arg forms act on the calling thread; the one-argument
forms take a parent `Thread` handle and act on the worker behind it, reaching the
worker's arena through `THREAD_OFFSET_ARENA_STATE`). Subscription joins at the
current stream frontier — a subscriber sees every byte that arrives afterward, never
a replay of bytes already read. The compiler subscribes the **main** thread at
program entry whenever the module uses a stdin builtin, so a single-threaded program
is byte-identical to a direct per-byte reader with no source change. Any other
thread that reads stdin without a subscription raises `ErrInvalidContext`
(`77050019`).

The log is bounded by a fixed high-water mark: the reader refuses to advance past
`base + cap` and blocks on the condvar until a slow subscriber advances the
minimum cursor (`base`) or unsubscribes, so a stalled subscriber applies
backpressure rather than growing memory without limit. Worker teardown
auto-unsubscribes (the trampoline calls `_mfb_rt_stdin_unsubscribe` for the worker
arena when the module uses stdin), so an exited or crashed worker never permanently
pins `base`. On process shutdown a worker parked in a blocking stdin read is
terminated by process exit, exactly as for any other worker (§ arena teardown
above), so shutdown never hangs on a parked reader.
[[src/target/shared/code/stdin_broadcast.rs]] [[src/builtins/thread.rs:OPEN_STD_IN]]

## See Also

* ./mfb spec linker linux-aarch64 — ELF interpreter paths and `DT_NEEDED` soname list
* ./mfb spec linker macos-aarch64 — libSystem branch-call and GOT relocation requirements
