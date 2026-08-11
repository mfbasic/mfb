# types

the process package resource handle and enum types

## Synopsis

```
process::Process
process::Stream
process::Signal
```

## Package

process

## Imports

```
IMPORT process
```

`process` is a built-in package, so `IMPORT process` needs no manifest
dependency. [[src/codegen/builtins/process/mod.rs:is_process_call]]

## Description

The `process` package exposes one resource type and two value enums.
[[src/codegen/builtins/process/mod.rs:PROCESS]]

`Process` is an opaque, owned, non-copyable **resource handle** to a spawned child
process. It carries no readable fields: a program never constructs one directly or
reads its parts, but obtains one from `process::spawn` or `process::shell` and
passes it to the other `process::` calls. Internally it is a native resource
(resource tag `10`) whose record caches the child pid, the three standard-stream
pipe file descriptors, and the exit/signal status once the child is reaped. Like
every resource handle it cannot be copied, stored as a collection element, or
carried in a record, and it is closed by lexical drop when its binding leaves
scope — a drop force-kills and reaps a still-running child.
[[src/codegen/builtins/process/mod.rs:PROCESS_TYPE]]
[[src/target/shared/code/error_constants.rs:RESOURCE_TAG_PROCESS]]

`Stream` and `Signal` are ordinary, flat, copyable value enums declared in the
package's source companion: they hold no resource and no hidden state, so they
copy freely and are thread-sendable. Both are recognized once `IMPORT process` is
in scope, and each member is addressed with the bare enum name and a dot
(`Stream.StdErr`, `Signal.Kill`), not a package prefix.
[[src/codegen/builtins/process/package.mfb:EXPORT ENUM]]

`Stream` selects which of the child's two output streams a read reads from. It is
the optional final argument of `process::receive`, `process::receiveBytes`, and
`process::poll`; when omitted, those calls read standard output.
[[src/codegen/builtins/process/mod.rs:STREAM_TYPE]]

`Signal` is a four-bucket cross-platform signal vocabulary used for two purposes:
`process::signal` **delivers** a bucket to a child, and `process::didSignal`
**observes** which bucket a terminated child died on. The buckets abstract over
platform signal numbers, and the two directions map to and from the host
differently, as tabulated below. [[src/codegen/builtins/process/mod.rs:SIGNAL_TYPE]]
[[src/codegen/builtins/process/native/unix.rs:lower_process_signal_helper]]

## Types

### process::Process

An opaque, owned handle to a spawned child process. Returned by `process::spawn`
and `process::shell`. It has no readable fields. [[src/codegen/builtins/process/mod.rs:PROCESS_TYPE]]

### process::Stream

Selects which of a child's output streams a read reads from. The member ordinals
are `StdOut = 0` and `StdErr = 1`; a read with no `Stream` argument reads
`StdOut`. [[src/codegen/builtins/process/package.mfb:Stream]]

| Variant | Description |
| --- | --- |
| `StdOut` | The child's standard output (the default when the `from` argument is omitted). |
| `StdErr` | The child's standard error. |

### process::Signal

A four-bucket cross-platform signal vocabulary, delivered by `process::signal` and
observed by `process::didSignal`. The member ordinals are `None = 0`, `Kill = 1`,
`Terminate = 2`, `Error = 3`. [[src/codegen/builtins/process/package.mfb:Signal]]

| Variant | Description |
| --- | --- |
| `None` | No signal. As an argument to `process::signal` it is a no-op; as a result of `process::didSignal` it means the child exited normally or has not terminated. |
| `Kill` | Forced, uncatchable termination. |
| `Terminate` | A polite, catchable "please stop". |
| `Error` | An abnormal-fault termination (an illegal instruction, an abort, a floating-point fault, a bad memory access, and the like). |

#### `process::signal` — bucket to host signal (deliver)

`process::signal` translates each bucket to a host mechanism.
[[src/codegen/builtins/process/native/unix.rs:lower_process_signal_helper]]
[[src/codegen/builtins/process/native/windows.rs:lower_process_signal_helper]]

| Bucket | Unix (`kill`) | Windows (`TerminateProcess`) |
| --- | --- | --- |
| `None` | no-op | no-op |
| `Kill` | `SIGKILL` | `TerminateProcess`, exit code `137` (`128 + 9`) |
| `Terminate` | `SIGTERM` | `TerminateProcess`, exit code `143` (`128 + 15`) |
| `Error` | `SIGABRT` | `TerminateProcess`, exit code `134` (`128 + 6`) |

Windows has no way to deliver an arbitrary signal to a child without a shared
console, so every terminating bucket is the same best-effort `TerminateProcess`.
The distinct POSIX-flavored exit codes (`128 + signo`) let a later
`process::waitFor` read back a recognizable value; there is no per-signal fidelity.
[[src/codegen/builtins/process/native/windows.rs:lower_process_signal_helper]]

#### `process::didSignal` — host status to bucket (observe)

`process::didSignal` decodes the cached status of a terminated child into a bucket.
[[src/codegen/builtins/process/native/unix.rs:lower_process_didsignal_helper]]
[[src/codegen/builtins/process/native/windows.rs:lower_process_didsignal_helper]]

| Bucket | Unix (`WTERMSIG` of the wait status) | Windows (exit code) |
| --- | --- | --- |
| `None` | exited normally, or not yet terminated | anything not below (normal exit, a `TerminateProcess` code, or not yet reaped) |
| `Kill` | `SIGKILL` | — (not recoverable) |
| `Error` | `SIGILL`, `SIGABRT`, `SIGFPE`, `SIGBUS`, or `SIGSEGV` | an NTSTATUS "error"-severity exit code (top two bits set, e.g. `0xC0000005` `STATUS_ACCESS_VIOLATION`) |
| `Terminate` | any other terminating signal | — (not recoverable) |

Windows exit codes carry no signal disposition, so `didSignal` there recovers only
the fault case (an NTSTATUS error-severity code maps to `Error`); every other
outcome maps to `None`. This is a documented Windows limitation.
[[src/codegen/builtins/process/native/windows.rs:lower_process_didsignal_helper]]

## See also

- `mfb man process`
- `mfb man process spawn`
- `mfb man process receive`
- `mfb man process signal`
- `mfb man process didSignal`
