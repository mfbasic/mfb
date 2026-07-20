# executablePath

The path to the running executable

## Synopsis

```
os::executablePath() AS String
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

`os::executablePath` returns the filesystem path of the running binary as an
owned `String`. On macOS it uses `_NSGetExecutablePath`; on Linux it reads the
`/proc/self/exe` symlink with `readlink`, which yields the absolute, symlink-
resolved path. [[src/target/shared/code/os.rs:lower_executable_path]]

Use it to locate resources beside the executable, or to report the program's own
path. If the host cannot determine the path, `os::executablePath` raises
`ErrUnsupported`. It reads host state only and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::executablePath` takes no arguments. [[src/builtins/os.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The path to the running executable. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050007` | `ErrUnsupported` | The host cannot determine the executable path. |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate the returned `String`. |

## Examples

Print the executable path:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::executablePath())
END SUB
```

## See also

- `mfb man os args`
- `mfb man os name`
