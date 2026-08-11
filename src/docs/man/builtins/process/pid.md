# pid

Return the operating-system process ID of a spawned child.

## Synopsis

```
process::pid(p AS Process) AS Integer
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

`process::pid` reads the operating-system process identifier of the child behind a
`Process` handle. The value is the child pid captured when the process was spawned
and cached in the handle record, so `pid` performs no system call and never blocks;
it returns the same value for the life of the handle, even after the child has
exited (the pid is not re-checked for liveness — use `process::isRunning` for
that). [[src/target/shared/code/process/unix.rs:lower_process_pid_helper]]

The handle is borrowed and left open. Calling `pid` on a handle that has already
been dropped or detached raises `ErrResourceClosed`.
[[src/builtins/errorcode.rs:ErrResourceClosed]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed. Also accepts the alternate named-argument spelling `process`. [[src/codegen/builtins/process/mod.rs:P_PROC]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The child's operating-system process ID, as recorded at spawn. [[src/target/shared/code/process/unix.rs:lower_process_pid_helper]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `p` has already been dropped or detached. [[src/builtins/errorcode.rs:ErrResourceClosed]] |

## Examples

Print the child's process ID:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["sleep", "1"])
  io::print(toString(process::pid(child)))
  RETURN 0
END FUNC
```

## See also

- `mfb man process spawn`
- `mfb man process isRunning`
- `mfb man process waitFor`
- `mfb man process types`
