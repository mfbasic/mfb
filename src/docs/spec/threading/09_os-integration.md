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
arena state, calls the worker export, stores the returned result in the control
block, keeps the worker arena live as needed for that result, marks the worker
complete, and returns `NULL` to pthread.

Raw Linux thread syscalls such as `clone`, `clone3`, `futex`, `set_tid_address`,
`gettid`, `tgkill`, and thread-local raw `exit` are not the threading ABI for
the Linux backend. They may be used by libc internally, but generated
thread helpers must call the libc/pthread interface. [[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_start_helper]]

## Arena Teardown at Process Exit

A threaded program does not explicitly destroy the main runtime arena during
process shutdown, on **any** platform. A worker may still be running when the main
function returns — dropping a handle whose worker is still running only cancels,
broadcasts and detaches it — and process exit reclaims every arena anyway, so the
runtime leaves the main arena to the OS rather than unmap memory a running worker
might still reach. The deferral is gated on the program embedding any `thread.`
runtime call, so a program that starts no worker still frees the arena at exit
exactly as before. [[src/codegen/engine/builder/mod.rs:lower_module_for_platform]]

The rule was made platform-independent when it was still load-bearing for the
control block: that block and the worker's arena-state block used to be carved from
the spawning thread's arena, so a running worker's pinned registers pointed into the
main arena, and the one backend that still destroyed it at exit (Windows) faulted
inside the trampoline. Both blocks now come from the process heap
(`./mfb spec threading control-block`).

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

The log is bounded by a fixed high-water mark: the reader refuses to advance past
`base + cap` and blocks on the condvar until a slow subscriber advances the
minimum cursor (`base`) or unsubscribes, so a stalled subscriber applies
backpressure rather than growing memory without limit. Worker teardown
auto-unsubscribes (the trampoline calls `_mfb_rt_stdin_unsubscribe` for the worker
arena when the module uses stdin), so an exited or crashed worker never permanently
pins `base`. On process shutdown a worker parked in a blocking stdin read is
terminated by process exit, exactly as for any other worker (§ arena teardown
above), so shutdown never hangs on a parked reader.
[[src/codegen/io/stdin/stdin_broadcast.rs]] [[src/codegen/builtins/thread/mod.rs:openStdIn]]

### The per-subscriber input buffer

Below the broadcast log (and behind the mouse decoder, which strips SGR reports
first) each subscriber has an **input buffer**: bytes already pulled from the
source for this thread that no reader has consumed yet, oldest first. It is an
arena-state region — a length word and up to 256 bytes — so each thread's buffer
is its own and needs no lock. A reader that must look ahead to classify input
(the key decoder below) leaves the bytes it did not consume there, and **every
reader takes buffered bytes before new ones**: the shared byte read checks the
buffer first, the loop head a line read returns to for each byte sits above that
check, and `io::pollInput` reports buffered bytes ready before it waits on the
log or `poll(fd 0)`, neither of which can see them. Mixing readers therefore
never loses or reorders input.
[[src/codegen/io/input/mod.rs:INPUT_STATE_SLOTS]]
[[src/codegen/io/input/buffer.rs:emit_take_buffered_byte]]
[[src/codegen/builtins/io/gen_read_family.rs:emit_stdin_byte_read]]
[[src/codegen/builtins/io/func_poll_input.rs:lower_poll_input]]

The region is reserved — appended past the mouse-state region, so no other
arena-state offset moves — only in a program that calls a reader which can leave
bytes in it (`INPUT_BUFFER_READERS`, the look-ahead readers bug-699 Phase 4
adds). No such reader exists yet, so today no program reserves it and every read
path is exactly the one described above.
[[src/codegen/io/input/mod.rs:INPUT_BUFFER_READERS]]
[[src/codegen/engine/builder/mod.rs]]

The decoder those readers use is `_mfb_rt_input_decode`, emitted with the region:
a pure function of a byte buffer, its length and three flags (`FINAL`: no more
input is coming for this decision; `EVENTS`: the source reports press/repeat/
release; `CLUSTER`: return a grapheme cluster, not one scalar) that writes one
token — need-more, skip `n` bytes, or a key with its event, modifiers, grapheme
and the text a text reader sees — into a 192-byte record. It accepts UTF-8 (an
ill-formed maximal subpart is skipped, not reported), the C0 keys, the legacy
xterm/VT220/rxvt/Linux-console sequences with xterm modifier parameters and
`modifyOtherKeys`, and kitty `CSI … u` with event types, modifiers and associated
text. It never waits: "need more" is the caller's cue to read more or, once the
one input delay (`INPUT_DELAY_MS`, 25 ms — the same delay the mouse decoder's
lone-`ESC` wait uses) has passed or the input has ended, to ask again with
`FINAL`.
[[src/codegen/io/input/decode.rs:lower_input_decode]]
[[src/codegen/io/input/keys.rs]]
[[src/codegen/io/input/mod.rs:INPUT_DELAY_MS]]

## See Also

* ./mfb spec linker linux-aarch64 — ELF interpreter paths and `DT_NEEDED` soname list
* ./mfb spec linker macos-aarch64 — libSystem branch-call and GOT relocation requirements
