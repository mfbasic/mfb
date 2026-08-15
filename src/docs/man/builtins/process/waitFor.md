# waitFor

Block until a spawned child exits and return its exit code.

## Synopsis

```
process::waitFor(p AS Process) AS Integer
```

## Package

`process`

## Imports

```
IMPORT process
```

`process` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/process/mod.rs:register]]

## Description

`process::waitFor` blocks until the child behind a `Process` handle has exited, then
returns its exit code. A child that exited normally returns its exit status
(`0 .. 255` on Unix); a child killed by a signal returns `-1`.
[[src/codegen/builtins/process/func_wait_for.rs:lower_process_waitfor_helper_posix]]

`waitFor` is **idempotent**. The first call reaps the child (`waitpid` on Unix) and
caches its exit code and raw wait status in the handle; every later call — and a
call after `process::isRunning` already observed the exit — returns the cached code
without blocking again. Because reaping and caching happen here (or in
`isRunning`), a subsequent `process::didSignal` can report how the child died.
[[src/codegen/builtins/process/native/unix.rs:emit_decode_status]]

The handle is borrowed and left open; the child stays reaped, so letting the handle
drop afterward is a no-op rather than a second wait. Calling `waitFor` on a handle
that has already been dropped or detached raises `ErrResourceClosed`.
[[src/builtins/errorcode.rs:ErrResourceClosed]]

Standard output a child writes but the program never reads is discarded when the
pipe buffer fills, which can cause a child that keeps writing to block instead of
exiting; drain the child with `process::receive` (or close its input with
`process::close`) before `waitFor` when the child produces output.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/codegen/builtins/process/func_wait_for.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The child's exit code: its normal exit status, or `-1` if it was killed by a signal. Cached, so repeat calls return the same value without blocking. [[src/codegen/builtins/process/func_wait_for.rs:lower_process_waitfor_helper_posix]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `p` has already been dropped or detached. [[src/builtins/errorcode.rs:ErrResourceClosed]] |

## Examples

Run a command to completion and read its exit code:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["true"])
  LET code = process::waitFor(child)
  io::print(toString(code))
  RETURN 0
END FUNC
```

## See also

- `mfb man process isRunning`
- `mfb man process didSignal`
- `mfb man process receive`
- `mfb man process close`
- `mfb man process types`
