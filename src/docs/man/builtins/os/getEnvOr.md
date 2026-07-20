# getEnvOr

Read an environment variable, or a fallback when it is unset

## Synopsis

```
os::getEnvOr(name AS String, fallback AS String) AS String
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

`os::getEnvOr` returns the value of the environment variable named `name` when it
is set, and otherwise returns `fallback`. It never raises for a missing variable,
mirroring `collections::getOr(map, key, fallback)`. The lookup reflects the live
environment, including values written earlier by `os::setEnv`.
[[src/target/shared/code/os.rs:lower_get_env]]

Both the found value and the fallback are returned as fresh owned `String`
values. Because absence yields `fallback` rather than a raised error, a variable
set to the empty string and an unset variable are indistinguishable through this
function; use `os::hasEnv` or `os::getEnv` when that distinction matters.

`os::getEnvOr` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `name` | `String` | The variable name to read. Must be non-empty and free of embedded NUL bytes. [[src/builtins/os.rs:call_param_names]] |
| `fallback` | `String` | The value returned when `name` is not set. [[src/builtins/os.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The variable's value when set, otherwise `fallback`. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate the returned `String`. |

## Examples

Read an optional variable with a default:

```
IMPORT os
IMPORT io

SUB main()
  LET level AS String = os::getEnvOr("LOG_LEVEL", "info")
  io::print(level)
END SUB
```

## See also

- `mfb man os getEnv`
- `mfb man os hasEnv`
- `mfb man os environ`
