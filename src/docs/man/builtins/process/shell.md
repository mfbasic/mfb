# shell

Run a command line through the platform shell, returning a handle to the child.

## Synopsis

```
process::shell(cmd AS String) AS Process
```

## Package

`process`

## Imports

```
IMPORT process
```

`process` is a built-in package, so no manifest dependency is required.
[[src/codegen/builtins/process/mod.rs:register]]

## Description

`process::shell` runs `cmd` as a shell command line and returns an owned `Process`
handle to the resulting child. Unlike `process::spawn`, which execs a program
directly, `shell` hands the string to the platform shell — `/bin/sh -c` on Unix —
so shell features work: pipelines (`|`), redirection (`>`, `<`), globbing (`*`),
command sequencing (`;`, `&&`), quoting, and environment-variable expansion are all
interpreted by the shell. [[src/codegen/builtins/process/func_shell.rs:lower_process_shell_helper_posix]]

Because the string is parsed by a shell, values interpolated into `cmd` are subject
to shell word-splitting and metacharacter interpretation; build the command with
care when any part comes from untrusted input. When you do not need a shell — you
have a program and its arguments already separated — prefer `process::spawn`, which
avoids the shell entirely.

The child is wired to three pipes for its standard streams exactly as with
`process::spawn`, and the returned handle has the same ownership: it is closed by
lexical drop at scope exit, which force-kills and reaps a still-running child unless
it is first awaited with `process::waitFor` or released with `process::detach`.
[[src/codegen/builtins/process/native/unix.rs:emit_spawn_tail]]
[[src/codegen/registry/mod.rs:resource_close_function]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `cmd` | `String` | The command line, parsed and run by the platform shell (`/bin/sh -c` on Unix). Also accepts the alternate named-argument spelling `command`. [[src/codegen/builtins/process/func_shell.rs:register]] |

## Return value

| Type | Description |
| --- | --- |
| `Process` | An owned handle to the running shell child. Closed by lexical drop at scope exit (which kills and reaps it) unless first awaited with `process::waitFor` or released with `process::detach`. [[src/codegen/builtins/process/mod.rs:PROCESS_TYPE]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77080001` | `ErrSpawnFailed` | The shell child could not be created: `fork`/`pipe` failed, or the shell could not be `exec`'d. [[src/codegen/builtins/errorcode/mod.rs:ErrSpawnFailed]] |
| `77010001` | `ErrOutOfMemory` | The `argv` C strings or the `Process` handle record could not be allocated. [[src/codegen/builtins/errorcode/mod.rs:ErrOutOfMemory]] |

## Examples

Run a pipeline and read the result:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo hello | tr a-z A-Z")
  io::print(process::receive(sh))
  RETURN 0
END FUNC
```

Run a command and wait for its exit code:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("true")
  io::print(toString(process::waitFor(sh)))
  RETURN 0
END FUNC
```

## See also

- `mfb man process spawn`
- `mfb man process receive`
- `mfb man process waitFor`
- `mfb man process detach`
- `mfb man process types`
