# pid

The current process id

## Synopsis

```
os::pid() AS Integer
```

## Package

os

## Imports

```
IMPORT os
```

`os` is a built-in package, so no manifest dependency is required.
[[src/builtins/os.rs:is_os_call]]

## Description

`os::pid` returns the process id of the running program as an `Integer`, via the
host `getpid` call. The value is positive and stable for the life of the process.
[[src/target/shared/code/os/introspect.rs:lower_pid]]

`os::pid` is **not pure** in the sense that different processes see different
values, but within one process every call returns the same id. It reads process
state only and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::pid` takes no arguments. [[src/builtins/os.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The current process id (a positive value). [[src/builtins/os.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Print the process id:

```
IMPORT os
IMPORT io

SUB main()
  io::print(toString(os::pid()))
END SUB
```

## See also

- `mfb man os executablePath`
- `mfb man os args`
