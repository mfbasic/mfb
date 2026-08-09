# didSignal

Report which signal bucket a terminated child died on.

## Synopsis

```
process::didSignal(p AS Process) AS Signal
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

`process::didSignal` reports how a terminated child died, as one of the four
`Signal` buckets. It reads the raw wait status cached when the child was reaped —
by `process::waitFor` or by a `process::isRunning` that observed the exit — so it
returns `Signal.None` for a child that exited normally *or* that has not yet been
observed to terminate. Await or poll the child first if you need the death cause.
[[src/target/shared/code/process/unix.rs:lower_process_didsignal_helper]]
[[src/builtins/process.rs:SIGNAL_TYPE]]

On Unix it decodes the terminating signal (`WTERMSIG`): `SIGKILL` maps to
`Signal.Kill`; the fault signals `SIGILL`, `SIGABRT`, `SIGFPE`, `SIGBUS`, and
`SIGSEGV` map to `Signal.Error`; and every other terminating signal maps to
`Signal.Terminate`. On Windows exit codes carry no signal disposition, so
`didSignal` recovers only the fault case — an NTSTATUS "error"-severity exit code
(e.g. `0xC0000005` `STATUS_ACCESS_VIOLATION`) maps to `Signal.Error`, and every
other outcome maps to `Signal.None`; this is a documented Windows limitation. The
full platform mapping is tabulated in `mfb man process types`.
[[src/target/shared/code/process/windows.rs:lower_process_didsignal_helper]]

Reading a handle that has already been dropped or detached raises
`ErrResourceClosed`. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/builtins/process.rs:P_PROC]] |

## Return value

| Type | Description |
| --- | --- |
| `Signal` | The bucket the child died on: `Signal.Kill`, `Signal.Terminate`, or `Signal.Error` for a signal death, or `Signal.None` if it exited normally or has not yet been observed to terminate. [[src/target/shared/code/process/unix.rs:lower_process_didsignal_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `p` has already been dropped or detached. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Examples

Report how a child died after killing it:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["sleep", "30"])
  process::signal(child, Signal.Kill)
  LET code = process::waitFor(child)
  IF process::didSignal(child) = Signal.Kill THEN
    io::print("killed")
  END IF
  RETURN 0
END FUNC
```

## See also

- `mfb man process signal`
- `mfb man process waitFor`
- `mfb man process isRunning`
- `mfb man process types`
