# name

The operating-system family the program was built for

## Synopsis

```
os::name() AS String
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

`os::name` returns the operating-system family of the build target: `"macos"` or
`"linux"`. It is a compile-time constant — the binary is built for exactly one
target, so the value is fixed at build time and materialized directly into an
owned `String`, with no host call. [[src/target/shared/code/os.rs:lower_const_string]]

Pair it with `os::arch` to identify the full platform. Because the value is
fixed per build, it is stable across runs of the same binary.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::name` takes no arguments. [[src/builtins/os.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The OS family: `"macos"` or `"linux"`. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Print the platform:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::name() & "/" & os::arch())
END SUB
```

## See also

- `mfb man os arch`
- `mfb man os executablePath`
