# environ

Snapshot every environment variable as a map

## Synopsis

```
os::environ() AS Map OF String TO String
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

`os::environ` returns a `Map OF String TO String` holding every variable in the
live process environment, keyed by name. It walks the host environment array,
splitting each `NAME=VALUE` entry at its **first** `=`: the text before it is the
key and everything after it — including any further `=` — is the value. An entry
with no `=` maps its whole text to an empty-string value. The snapshot reflects
variables written earlier by `os::setEnv` and omits those removed by
`os::unsetEnv`. [[src/target/shared/code/os.rs:lower_environ]]

The returned map is an ordinary owned value captured at the moment of the call;
later `os::setEnv`/`os::unsetEnv` calls do not change it, so re-read the
environment with a fresh `os::environ()` to observe subsequent mutations. The map
is unordered, like any `Map`. On the rare host that lists a name twice, the map
retains one entry for that key.

`os::environ` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| — | — | `os::environ` takes no arguments. [[src/builtins/os.rs:arity]] |

## Return value

| Type | Description |
| --- | --- |
| `Map OF String TO String` | A snapshot of the environment, mapping each variable name to its value. [[src/builtins/os.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate the returned map. |

## Examples

Read a value out of the environment snapshot:

```
IMPORT os
IMPORT io
IMPORT collections

LET env AS Map OF String TO String = os::environ()
io::print(collections::getOr(env, "PATH", ""))
```

## See also

- `mfb man os getEnv`
- `mfb man os hasEnv`
- `mfb man os setEnv`
