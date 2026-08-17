# hostName

The host's network name

## Synopsis

```
os::hostName() AS String
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

`os::hostName` returns the host's network name via the host `gethostname` call,
copied into an owned `String`. The name is whatever the host is configured to
report (often the short hostname). [[src/codegen/builtins/os/native/introspect.rs:lower_host_name]]

If the host cannot supply the name, `os::hostName` raises `ErrUnsupported`. It
reads host state only and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::hostName` takes no arguments. [[src/codegen/builtins/os/func_host_name.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The host's network name. [[src/codegen/builtins/os/func_host_name.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050007` | `ErrUnsupported` | The host cannot supply its name. |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate the returned `String`. |

## Examples

Print the host name:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::hostName())
END SUB
```

## See also

- `mfb man os userName`
