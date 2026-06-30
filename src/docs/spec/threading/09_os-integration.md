# OS Integration

Threads are real native OS threads.

## macOS aarch64

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

## Linux aarch64

The Linux backend is cross-compiled and does not invoke an external system
linker. The compiler emits dynamic ELF executables directly.

It emits both a glibc (`<project>-glibc.out`) and a musl (`<project>-musl.out`)
flavor. Each carries the ELF interpreter path and `DT_NEEDED` list its libc
requires; the glibc flavor names libpthread separately, while musl exposes pthread
entry points from libc, so no separate musl pthread dependency is needed. The
exact interpreter paths and soname list are owned by
`./mfb spec linker linux-aarch64`.

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
thread helpers must call the libc/pthread interface. [[src/target/shared/code/runtime_helpers.rs:lower_thread_start_helper]]

## See Also

* ./mfb spec linker linux-aarch64 — ELF interpreter paths and `DT_NEEDED` soname list
* ./mfb spec linker macos-aarch64 — libSystem branch-call and GOT relocation requirements
