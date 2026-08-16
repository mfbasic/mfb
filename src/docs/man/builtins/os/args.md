# args

The command-line arguments after the program name

## Synopsis

```
os::args() AS List OF String
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

`os::args` returns the program's command-line arguments as a `List OF String`,
**excluding** the program name — element 0 is the first real argument, not the
executable. (The program name is available through `os::executablePath`.) A
program invoked with no arguments returns an empty list.
[[src/codegen/builtins/os/native/introspect.rs:lower_args]]

The arguments are captured at program startup from the values the OS passes in,
so `os::args` reflects the invocation regardless of where in the program it is
called. Each element is an owned `String` copied from the corresponding `argv`
entry.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::args` takes no arguments. [[src/codegen/builtins/os/func_args.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF String` | The command-line arguments after the program name, in order; empty when none were given. [[src/codegen/builtins/os/func_args.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate the returned list. |

## Examples

Print each argument on its own line:

```
IMPORT os
IMPORT io
IMPORT collections

SUB main()
  LET a AS List OF String = os::args()
  FOR i = 0 TO len(a) - 1
    io::print(collections::get(a, i))
  NEXT
END SUB
```

## See also

- `mfb man os executablePath`
- `mfb man os pid`
