# detach

Relinquish ownership of a child so it keeps running after the program exits.

## Synopsis

```
process::detach(p AS Process) AS Nothing
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

`process::detach` relinquishes ownership of a child **without** killing it. It
closes the parent-side pipe ends, arranges for the operating system to auto-reap the
child when it eventually exits (on Unix, by setting `SIGCHLD` to be ignored so the
kernel reaps it and no zombie is left), and marks the handle closed. The child keeps
running on its own and survives the parent's exit. [[src/target/shared/code/process/unix.rs:lower_process_detach_helper]]

This is the counterpart to the default drop behavior. Normally letting a `Process`
go out of scope force-kills and reaps the child; `detach` is the deliberate opt-out
for a child that should outlive the program — a daemon, a background job, a handoff
to another process. [[src/builtins/process.rs:resource_close_function]]

Because `detach` marks the handle closed, it consumes the handle for all practical
purposes: every later `process::` call on it — including a second `detach` — raises
`ErrResourceClosed`, and the eventual scope-drop is a no-op rather than a kill.
[[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. After the call the handle is closed and unusable, but the child keeps running. Also accepts the alternate named-argument spelling `process`. [[src/builtins/process.rs:P_PROC]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `detach` returns no value. On return the parent-side pipes are closed, the handle is marked closed, and the child continues running independently. [[src/builtins/process.rs:PROCESS_TYPE]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `p` has already been dropped or detached. [[src/target/shared/code/error_constants.rs:ERR_RESOURCE_CLOSED_CODE]] |

## Examples

Start a background job and let it outlive the program:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES job = process::shell("sleep 5")
  process::detach(job)
  io::print("job detached")
  RETURN 0
END FUNC
```

## See also

- `mfb man process close`
- `mfb man process waitFor`
- `mfb man process signal`
- `mfb man process spawn`
- `mfb man process types`
