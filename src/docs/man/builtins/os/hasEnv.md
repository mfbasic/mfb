# hasEnv

Test whether an environment variable is set

## Synopsis

```
os::hasEnv(name AS String) AS Boolean
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

`os::hasEnv` returns `TRUE` when the environment variable named `name` is
present in the live process environment and `FALSE` otherwise. It is the host
`getenv` call reduced to a non-NULL test, so it reflects both inherited variables
and any set earlier by `os::setEnv`. A variable set to the empty string still
counts as present. [[src/target/shared/code/os/env.rs:lower_has_env]]

`os::hasEnv` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects, and never raises.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `name` | `String` | The variable name to test. Must be non-empty and free of embedded NUL bytes. [[src/builtins/os.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` if `name` is set (even to an empty value), else `FALSE`. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Branch on the presence of a variable:

```
IMPORT os
IMPORT io

SUB main()
  IF os::hasEnv("CI") THEN
    io::print("running in CI")
  END IF
END SUB
```

## See also

- `mfb man os getEnv`
- `mfb man os getEnvOr`
- `mfb man os environ`
