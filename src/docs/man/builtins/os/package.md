# os

Process environment access

## Synopsis

```
IMPORT os
os::getEnv(name)
os::getEnvOr(name, fallback)
os::hasEnv(name)
os::setEnv(name, value)
os::unsetEnv(name)
os::environ()
```

## Description

The `os` package reaches the host process environment: it reads, tests, sets,
unsets, and enumerates environment variables. `os` is a built-in package, so
`IMPORT os` needs no manifest dependency. [[src/builtins/os.rs:is_os_call]]

Variable names and values are UTF-8 `String` values passed to and from the host
C library (`getenv`, `setenv`, `unsetenv`, and the platform environ accessor).
A name must be non-empty and, like a value, may not contain an embedded NUL byte
or, for a name, an `=` — the host requires NUL-terminated strings and uses `=`
to separate a name from its value. [[src/target/shared/code/os.rs:lower_os_helper]]

Reads observe the live environment: `os::getEnv`, `os::getEnvOr`, `os::hasEnv`,
and `os::environ` all reflect both variables inherited from the host and any
changes a prior `os::setEnv`/`os::unsetEnv` made earlier in the same process. A
missing variable is a first-class outcome: `os::getEnv` raises `ErrNotFound`,
while `os::getEnvOr` returns a caller-supplied fallback and `os::hasEnv` reports
presence as a `Boolean`, so a program can choose whether absence is an error.
[[src/target/shared/code/os.rs:lower_get_env]]

`os::environ` returns a `Map OF String TO String` snapshot built by walking the
process environment array and splitting each `NAME=VALUE` entry at its first `=`;
an `=` inside a value is preserved as part of the value. The map is an ordinary
owned value taken at the moment of the call and does not track later mutations.
[[src/target/shared/code/os.rs:lower_environ]]

`os::setEnv` and `os::unsetEnv` mutate process-global state. They are **not**
synchronized against a concurrent `os::getEnv`/`os::environ` running in another
`thread::` worker — this is the classic `getenv`/`setenv` data race and is the
caller's responsibility to avoid. All returned `String`, `Boolean`, and
`Map OF String TO String` values follow the ordinary owned-value rules; the
package holds no resource handles.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050004` | `ErrNotFound` | `os::getEnv` is given a name that is not set. |
| `77050002` | `ErrInvalidArgument` | `os::setEnv` is given a name that is empty or contains `=`. |
| `77010001` | `ErrOutOfMemory` | The host cannot allocate storage for a set variable or a returned value. |
