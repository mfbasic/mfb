# setEnv

Set or overwrite an environment variable

## Synopsis

```
os::setEnv(name AS String, value AS String)
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

`os::setEnv` sets the environment variable named `name` to `value` in the live
process environment, overwriting any existing value. It is a SUB and returns
nothing. The change is visible to every later `os::getEnv`, `os::getEnvOr`,
`os::hasEnv`, and `os::environ` in the same process, and is inherited by child
processes spawned afterward. It maps to the host `setenv(name, value, 1)`.
[[src/target/shared/code/os.rs:lower_set_env]]

`os::setEnv` mutates process-global state and is **not** synchronized against a
concurrent read in another `thread::` worker; avoid setting a variable while
another thread reads the environment. A `name` that is empty or contains `=` is
rejected with `ErrInvalidArgument`, since the host uses `=` to separate a name
from its value.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `name` | `String` | The variable name to set. Must be non-empty, free of embedded NUL bytes, and free of `=`. [[src/builtins/os.rs:call_param_names]] |
| `value` | `String` | The value to store. Must be free of embedded NUL bytes. [[src/builtins/os.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `os::setEnv` is a SUB and produces no value. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `name` is empty or contains `=`. |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate storage for the variable. |

## Examples

Set a variable and read it back:

```
IMPORT os
IMPORT io

os::setEnv("GREETING", "hello")
io::print(os::getEnv("GREETING"))
```

## See also

- `mfb man os unsetEnv`
- `mfb man os getEnv`
- `mfb man os environ`
