# userName

The effective user's login name

## Synopsis

```
os::userName() AS String
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

`os::userName` returns the login name of the effective user, resolved through
`getpwuid(getuid())` and copied into an owned `String`. Using the passwd database
rather than the controlling terminal means it works without a login session (for
example under a service manager). [[src/target/shared/code/os.rs:lower_user_name]]

If the effective uid has no passwd entry (as on a bare container uid),
`os::userName` raises `ErrUnsupported`. It reads host state only and has no side
effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::userName` takes no arguments. [[src/builtins/os.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The effective user's login name. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050007` | `ErrUnsupported` | The effective uid has no passwd entry. |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate the returned `String`. |

## Examples

Print the user name:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::userName())
END SUB
```

## See also

- `mfb man os hostName`
- `mfb man os getEnv`
