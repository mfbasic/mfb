# isRunning

Report whether a spawned child is still running, without blocking.

## Synopsis

```
process::isRunning(p AS Process) AS Boolean
```

## Package

`process`

## Imports

```
IMPORT process
```

`process` is a built-in package, so no manifest dependency is required.
[[src/builtins/process.rs:is_process_call]]

## Description

`process::isRunning` reports whether the child behind a `Process` handle is still
alive. It performs a non-blocking check (`waitpid` with `WNOHANG` on Unix) and
returns immediately: `TRUE` while the child is running, `FALSE` once it has exited.
[[src/target/shared/code/process/unix.rs:lower_process_isrunning_helper]]

When the check observes that the child has just exited, it decodes and **caches**
the exit code and raw wait status in the handle, so a later `process::waitFor`
returns without blocking and `process::didSignal` can report how the child died.
Once the exit has been cached, further `isRunning` calls answer `FALSE` from the
cache without another system call. [[src/target/shared/code/process/unix.rs:emit_decode_status]]

The handle is borrowed and left open. Calling `isRunning` on a handle that has
already been dropped or detached raises `ErrResourceClosed`.
[[src/builtins/errorcode.rs:ErrResourceClosed]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/builtins/process.rs:P_PROC]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` while the child is still running; `FALSE` once it has exited (at which point its exit status has been cached). [[src/target/shared/code/process/unix.rs:lower_process_isrunning_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `p` has already been dropped or detached. [[src/builtins/errorcode.rs:ErrResourceClosed]] |

## Examples

Poll a child until it finishes:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["true"])
  WHILE process::isRunning(child)
    ' still going
  END WHILE
  io::print("done")
  RETURN 0
END FUNC
```

## See also

- `mfb man process waitFor`
- `mfb man process pid`
- `mfb man process didSignal`
- `mfb man process spawn`
- `mfb man process types`
