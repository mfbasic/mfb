# close

Close a child's standard input, signalling end-of-input; the child keeps running.

## Synopsis

```
process::close(p AS Process) AS Nothing
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

`process::close` closes the child's standard input — the parent's write end of the
child's stdin pipe. It sends end-of-input to the child, so a filter that reads
until EOF (`sort`, `cat`, `wc`, `tr`, …) stops waiting for more input and produces
its output. After `close`, further `process::send`/`process::sendBytes` to the same
child raise `ErrResourceClosed`. [[src/codegen/builtins/process/func_close.rs:lower_process_close_helper_posix]]

`process::close` is **not** a handle-consuming close. Despite the name, it does not
release the `Process` resource: the child keeps running, its output stays readable
with `process::receive`, and the handle remains valid and owned. The resource is
still closed the usual way — by lexical drop at scope exit (which force-kills and
reaps the child) — because `close` is deliberately not treated as an ownership
transfer. [[src/codegen/builtins/process/mod.rs:DROP]]

Closing the input is idempotent with respect to the input pipe: once stdin is
closed the call is a harmless no-op. Only a handle that has already been dropped or
detached makes `close` raise `ErrResourceClosed`.
[[src/builtins/errorcode.rs:ErrResourceClosed]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `p` | `Process` | The child process handle. Borrowed, not consumed; the child keeps running. Also accepts the alternate named-argument spelling `process`. [[src/codegen/builtins/process/mod.rs:P_PROC]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `close` returns no value. On return the child's standard input has been closed and any queued input has reached the child. [[src/codegen/builtins/process/mod.rs:PROCESS_TYPE]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77030004` | `ErrResourceClosed` | `p` has already been dropped or detached. [[src/builtins/errorcode.rs:ErrResourceClosed]] |

## Examples

Feed a filter its input, then close stdin so it flushes its output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sorter = process::spawn(["sort"])
  process::send(sorter, "banana")
  process::send(sorter, "apple")
  process::close(sorter)
  io::print(process::receive(sorter))
  RETURN 0
END FUNC
```

## See also

- `mfb man process send`
- `mfb man process receive`
- `mfb man process detach`
- `mfb man process waitFor`
- `mfb man process types`
