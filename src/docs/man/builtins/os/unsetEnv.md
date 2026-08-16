# unsetEnv

Remove an environment variable

## Synopsis

```
os::unsetEnv(name AS String)
```

## Package

os

## Imports

```
IMPORT os
```

`os` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/os/mod.rs:register]]

## Description

`os::unsetEnv` removes the environment variable named `name` from the live
process environment. It is a SUB and returns nothing. Removing a variable that is
not set is a no-op, not an error, so the call is idempotent. After it returns,
`os::hasEnv(name)` reports `FALSE` and `os::getEnv(name)` raises `ErrNotFound`.
It maps to the host `unsetenv(name)`. [[src/codegen/builtins/os/native/env.rs:lower_unset_env]]

`os::unsetEnv` mutates process-global state and is **not** synchronized against a
concurrent read in another `thread::` worker.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `name` | `String` | The variable name to remove. Must be non-empty and free of embedded NUL bytes. [[src/codegen/builtins/os/func_unset_env.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `os::unsetEnv` is a SUB and produces no value. [[src/codegen/builtins/os/func_unset_env.rs:register]] |

## Errors

No errors.

## Examples

Remove a variable and confirm it is gone:

```
IMPORT os
IMPORT io

SUB main()
  os::setEnv("TEMP_FLAG", "1")
  os::unsetEnv("TEMP_FLAG")
  io::print(toString(os::hasEnv("TEMP_FLAG")))
END SUB
```

## See also

- `mfb man os setEnv`
- `mfb man os hasEnv`
- `mfb man os getEnv`
