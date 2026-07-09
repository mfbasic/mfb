# cpuCount

The number of online logical CPUs

## Synopsis

```
os::cpuCount() AS Integer
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

`os::cpuCount` returns the number of online logical CPUs as reported by the host
`sysconf(_SC_NPROCESSORS_ONLN)`. The result is clamped to a minimum of 1, so a
caller always gets a usable count even if the host cannot determine the true
value. [[src/target/shared/code/os.rs:lower_cpu_count]]

Use it to size a `thread::` worker pool. The value reflects CPUs online at the
moment of the call and may in principle change over a long-running process on a
host that hot-plugs CPUs.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::cpuCount` takes no arguments. [[src/builtins/os.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The number of online logical CPUs, at least 1. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Print the CPU count:

```
IMPORT os
IMPORT io

io::print(toString(os::cpuCount()))
```

## See also

- `mfb man os arch`
- `mfb man thread start`
