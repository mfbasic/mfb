# signal

Deliver a cross-platform signal bucket to a child process.

## Synopsis

```
process::signal(p AS Process, sig AS Signal) AS Nothing
```

## Package

`process`

## Imports

```
IMPORT process
```

`process` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/process/mod.rs:is_process_call]]

## Description

`process::signal` delivers one of the four `Signal` buckets to the child behind a
`Process` handle. The bucket abstracts over platform signal numbers so the same
call works on Unix and Windows. `Signal.None` is a no-op. On Unix, `Signal.Kill`
sends `SIGKILL`, `Signal.Terminate` sends `SIGTERM`, and `Signal.Error` sends
`SIGABRT`. [[src/codegen/builtins/process/native/unix.rs:lower_process_signal_helper]]
[[src/codegen/builtins/process/mod.rs:SIGNAL_TYPE]]

On Windows there is no way to deliver an arbitrary signal to a child without a
shared console, so every terminating bucket maps to the same best-effort
`TerminateProcess`, with a POSIX-flavored exit code (`128 + signo`, so `137`/`143`/
`134` for `Kill`/`Terminate`/`Error`) that a later `process::waitFor` can read back;
there is no per-signal fidelity. The full platform mapping is tabulated in
`mfb man process types`. [[src/codegen/builtins/process/native/windows.rs:lower_process_signal_helper]]

Delivery does not wait for or reap the child; call `process::waitFor` afterward to
collect the exit status, or `process::didSignal` to read back which bucket a
terminated child died on. Signalling a handle that has already been dropped or
detached raises `ErrResourceClosed`.
[[src/builtins/errorcode.rs:ErrResourceClosed]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/codegen/builtins/process/mod.rs:P_SIGNAL]] |
| `sig` | `Signal` | The bucket to deliver: `Signal.None` (no-op), `Signal.Kill`, `Signal.Terminate`, or `Signal.Error`. Also accepts the alternate named-argument spelling `signal`. [[src/codegen/builtins/process/mod.rs:P_SIGNAL]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `signal` returns no value. On return the signal has been delivered (best-effort on Windows); it neither waits for nor reaps the child. [[src/codegen/builtins/process/mod.rs:PROCESS_TYPE]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `p` has already been dropped or detached. [[src/builtins/errorcode.rs:ErrResourceClosed]] |

## Examples

Ask a long-running child to stop, then wait for it:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["sleep", "30"])
  process::signal(child, Signal.Terminate)
  io::print(toString(process::waitFor(child)))
  RETURN 0
END FUNC
```

## See also

- `mfb man process didSignal`
- `mfb man process waitFor`
- `mfb man process detach`
- `mfb man process types`
