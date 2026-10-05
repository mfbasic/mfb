# OS Integration

Threads are real native OS threads.

## macOS AArch64

The macOS backend starts MFBASIC workers through libSystem pthreads:

```text
pthread_create(&controlBlock.osHandle, attr, _mfb_rt_thread_trampoline, controlBlock)
  ; attr = a pthread_attr_t initialized with an 8 MiB stack (pthread_attr_setstacksize)
```

The trampoline is a pthread start routine, with libSystem managing the OS
thread's pthread registration and TLS. Before the worker runs, the trampoline sets
the worker arena's stack-limit word (`./mfb spec language functions`, recursion);
every platform's trampoline does the same through
`CodegenPlatform::emit_stack_limit`. Generated thread helpers call the
pthread interface, not raw Mach thread creation.
[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_start_helper]]
[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_trampoline]]

See `./mfb spec linker macos-aarch64` for libSystem imports and relocations.

## Linux AArch64, x86-64, and riscv64

The compiler emits dynamic ELF executables directly, including for cross-target
builds, without invoking an external system linker.

It emits both a glibc (`build/<project>-glibc.out`) and a musl
(`build/<project>-musl.out`) flavor into the project's output directory. Each carries the ELF interpreter path and `DT_NEEDED` list its libc
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
arena state, calls the worker function, stores the returned result in the control
block through `_mfb_rt_thread_finish`, and returns `NULL` to pthread. The result
is copied into the block arena; the worker destroys its own arena before returning.
[[src/codegen/runtime/thread/runtime_helpers_thread.rs:lower_thread_finish]]

Raw Linux thread syscalls such as `clone`, `clone3`, `futex`, `set_tid_address`,
`gettid`, `tgkill`, and thread-local raw `exit` are not the threading ABI for
the Linux backend. They may be used by libc internally, but generated
thread helpers must call the libc/pthread interface. [[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_start_helper]]

## Windows x86-64

The Windows backend starts a worker with
`CreateThread(NULL, 8 * 1024 * 1024, trampoline, controlBlock, 0, NULL)`.
The returned `HANDLE` is stored in the control block; zero reports
`ErrInterrupted`. The trampoline uses the Windows calling convention and returns
zero after the same worker initialization, result publication, and arena teardown
as the POSIX targets.
[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_start_helper]]
[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_trampoline]]

The common thread helpers express synchronization as pthread operations; the
Windows emitter translates them to SRW locks and condition variables:
`AcquireSRWLockExclusive` / `ReleaseSRWLockExclusive`,
`SleepConditionVariableSRW`, and `WakeConditionVariable` /
`WakeAllConditionVariable`. Joining uses `WaitForSingleObject` followed by
`CloseHandle`; detaching closes the OS handle without waiting for the worker.
Timed waits convert the absolute deadline to a remaining millisecond timeout.
[[src/codegen/runtime/thread/runtime_helpers.rs:emit_thread_external_call]]

## Arena Teardown at Process Exit

A threaded program does not explicitly destroy the main runtime arena during
process shutdown, on **any** platform. A worker may still be running when the main
function returns — dropping a handle whose worker is still running only cancels,
broadcasts and detaches it — and process exit reclaims every arena anyway, so the
runtime leaves the main arena to the OS rather than unmap memory a running worker
might still reach. The deferral is gated on the program embedding any `thread.`
runtime call. A program with no such call takes the ordinary arena-destruction
path; merely avoiding `thread::start` at runtime does not remove this compile-time
deferral. [[src/codegen/engine/builder/mod.rs:lower_module_for_platform]]

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
worker's arena through `CONTROL_OFFSET_WORKER_ARENA`). Subscription joins at the
current stream frontier — a subscriber sees every byte that arrives afterward, never
a replay of bytes already read. The compiler subscribes the **main** thread at
program entry whenever the module uses a stdin builtin, so a single-threaded program
is byte-identical to a direct per-byte reader with no source change. Any other
thread that reads stdin without a subscription raises `ErrInvalidContext`
(`77050019`).

The log is bounded by the manifest's `options.stdinLogCap`, baked into the executable
(see `./mfb spec tooling project-manifest`): the reader refuses to advance past
`base + cap` and blocks on the condvar until a slow subscriber advances the
minimum cursor (`base`) or unsubscribes, so a stalled subscriber applies
backpressure rather than growing memory without limit. Worker teardown
auto-unsubscribes (the trampoline calls `_mfb_rt_stdin_unsubscribe` for the worker
arena when the module uses stdin), so a worker returning normally or with a
language error releases its subscription. On process shutdown a worker parked in a blocking stdin read is
terminated by process exit, exactly as for any other worker (§ arena teardown
above), so shutdown never hangs on a parked reader.
[[src/codegen/io/stdin/stdin_broadcast.rs:lower_stdin_next_byte]]
[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_trampoline]]

Subscription and unsubscription are idempotent. The subscriber registry holds
128 entries, including the main thread. If it is full, subscription leaves the
thread unregistered without reporting an error; a later read reports
`ErrInvalidContext`. Unsubscription releases a registry slot and wakes readers
waiting for log capacity.
[[src/codegen/io/stdin/stdin_broadcast.rs:lower_stdin_subscribe]]
[[src/codegen/io/stdin/stdin_broadcast.rs:lower_stdin_unsubscribe]]

### The per-subscriber input buffer

Below the broadcast log (and behind the mouse decoder, which strips SGR reports
first) each subscriber has an **input buffer**: bytes already pulled from the
source for this thread that no reader has consumed yet, oldest first. It is an
arena-state region — a length word, the classification state and up to 4096
bytes — so each thread's buffer is its own and needs no lock. Every `io::` reader
(`readRaw`, `readScalar`, `readGrapheme`, `readKey`, `readLine`, `input`) reads
through it: the key decoder classifies what is buffered, the source is read one
byte at a time only when it cannot answer, and bytes a reader does not consume
stay for the next read — so mixing readers never loses or reorders input. The
timed wait before each source read asks this thread's log view first
(`emit_stdin_poll_ready_check`), because bytes staged there are invisible to
`poll(fd 0)`.
[[src/codegen/io/input/mod.rs:INPUT_STATE_SLOTS]]
[[src/codegen/io/input/reader.rs:emit_engine]]
[[src/codegen/io/input/reader.rs:emit_read_wait]]

The region is reserved — appended past the mouse-state region, so no other
arena-state offset moves — only in a program that calls one of the readers
(`INPUT_BUFFER_READERS`). The buffer's layout, the decoder, the read engine and
the readers' semantics are `./mfb spec stdlib io-input`.
[[src/codegen/io/input/mod.rs:INPUT_BUFFER_READERS]]
[[src/codegen/engine/builder/mod.rs]]

## See Also

* ./mfb spec linker linux-aarch64 — ELF interpreter paths and `DT_NEEDED` soname list
* ./mfb spec linker macos-aarch64 — libSystem branch-call and GOT relocation requirements
