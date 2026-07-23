# arch

The CPU architecture the program was built for

## Synopsis

```
os::arch() AS String
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

`os::arch` returns the CPU architecture of the build target: `"aarch64"`,
`"x86_64"`, or `"riscv64"`. Like `os::name`, it is a compile-time constant fixed
at build time and materialized directly into an owned `String`, with no host
call. [[src/target/shared/code/os/introspect.rs:lower_const_string]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::arch` takes no arguments. [[src/builtins/os.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The CPU architecture: `"aarch64"`, `"x86_64"`, or `"riscv64"`. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Print the architecture:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::arch())
END SUB
```

## See also

- `mfb man os name`
- `mfb man os cpuCount`
