# getEnv

Read an environment variable, raising when it is unset

## Synopsis

```
os::getEnv(name AS String) AS String
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

`os::getEnv` returns the value of the environment variable named `name` as it
appears in the live process environment, including any value written earlier by
`os::setEnv`. The lookup is the host `getenv` call; the returned bytes are copied
into a fresh owned `String`. [[src/target/shared/code/os/env.rs:lower_get_env]]

If the variable is not set, `os::getEnv` raises `ErrNotFound` rather than
returning an empty string, so a program can distinguish an unset variable from
one deliberately set to the empty string. Use `os::getEnvOr` to supply a fallback
instead of raising, or `os::hasEnv` to test presence without reading the value.

`os::getEnv` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `name` | `String` | The variable name to read. Must be non-empty and free of embedded NUL bytes. [[src/builtins/os.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The variable's current value. May be the empty string if the variable is set to an empty value. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050004` | `ErrNotFound` | `name` is not set in the environment. |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate the returned `String`. |

## Examples

Read a variable that is expected to be present:

```
IMPORT os
IMPORT io

SUB main()
  LET home AS String = os::getEnv("HOME")
  io::print(home)
END SUB
```

Treat an unset variable as a recoverable condition:

```
IMPORT os
IMPORT io

SUB main()
  LET token = os::getEnv("API_TOKEN") TRAP(err)
    RECOVER ""
  END TRAP
  io::print(token)
END SUB
```

## See also

- `mfb man os getEnvOr`
- `mfb man os hasEnv`
- `mfb man os environ`
