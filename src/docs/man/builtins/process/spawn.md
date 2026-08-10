# spawn

Run a program directly from an argument list, returning a handle to the child.

## Synopsis

```
process::spawn(args AS List OF String) AS Process
process::spawn(args AS List OF String, cwd AS String, env AS Map OF String TO String, envReplace AS Boolean) AS Process
```

## Package

`process`

## Imports

```
IMPORT process
```

`process` is a built-in package, so no manifest dependency is required.
[[src/builtins/process.rs:is_process_call]]

## Description

`process::spawn` starts a child process from an explicit argument vector and
returns an owned `Process` handle to it. `args[0]` is the executable and is
resolved on `PATH` (`execvp` on Unix); the remaining elements are passed as the
child's arguments verbatim. **No shell is involved** — quoting, globbing, pipes,
redirection, and environment-variable expansion are *not* interpreted, so an
argument that contains spaces or shell metacharacters reaches the program as one
literal argument. Use `process::shell` when you need a shell to parse a command
line. [[src/target/shared/code/process/unix.rs:lower_process_spawn_helper]]

The child is created with three pipes wired to its standard input, output, and
error, so the parent can `process::send` to it and `process::receive` from it.
Creation forks and execs; an exec failure in the child (for example a program that
is not found) is reported back to the parent over a close-on-exec self-pipe and
surfaces as `ErrSpawnFailed`, not as a silently running child.
[[src/target/shared/code/process/unix.rs:emit_spawn_tail]]

The returned `Process` is an owned, non-copyable resource handle. It is closed by
lexical drop when its binding leaves scope, which **force-kills and reaps** a
still-running child (`SIGKILL` + `waitpid` on Unix) so no runaway process or zombie
is left; call `process::waitFor` first if the child should be allowed to finish, or
`process::detach` to let it outlive the program. [[src/builtins/process.rs:resource_close_function]]

The empty argument list is rejected with `ErrInvalidArgument` — there is no program
to run. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]]

## Overloads

**`process::spawn(args AS List OF String) AS Process`**

Runs `args[0]` with `args[1..]`, inheriting the parent's working directory and
environment.

**`process::spawn(args AS List OF String, cwd AS String, env AS Map OF String TO String, envReplace AS Boolean) AS Process`**

The full form. `cwd` sets the child's working directory before it execs (an empty
string keeps the parent's directory). `env` supplies environment variables; when
`envReplace` is `TRUE` the child's environment is *only* the entries of `env`
(the inherited environment is cleared first), and when it is `FALSE` those entries
are merged over the inherited environment. [[src/target/shared/code/process/unix.rs:lower_process_spawnenv_helper]]
[[src/target/shared/code/process/unix.rs:emit_child_apply_env]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `args` | `List OF String` | The argument vector. `args[0]` is the executable, resolved on `PATH`; the rest are the child's arguments, passed literally with no shell interpretation. Must be non-empty. [[src/target/shared/code/process/unix.rs:lower_process_spawn_helper]] |
| `cwd` | `String` | (full form) The working directory to switch to before running the child. An empty string leaves the parent's working directory in effect. [[src/target/shared/code/process/unix.rs:lower_process_spawnenv_helper]] |
| `env` | `Map OF String TO String` | (full form) Environment variables for the child, each key/value set with `setenv`. [[src/target/shared/code/process/unix.rs:emit_child_apply_env]] |
| `envReplace` | `Boolean` | (full form) `TRUE` to run with *only* `env` (the inherited environment is cleared first); `FALSE` to merge `env` over the inherited environment. [[src/target/shared/code/process/unix.rs:emit_child_apply_env]] |

## Return value

| Type | Description |
| --- | --- |
| `Process` | An owned handle to the running child. Closed by lexical drop at scope exit (which kills and reaps it) unless it is first awaited with `process::waitFor` or released with `process::detach`. [[src/builtins/process.rs:PROCESS_TYPE]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `args` is empty — there is no program to run. [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77080001` | `ErrSpawnFailed` | The child could not be created: `fork`/`pipe` failed, or the program was not found or could not be `exec`'d. [[src/target/shared/code/error_constants.rs:ERR_SPAWN_FAILED_CODE]] |
| `77010001` | `ErrOutOfMemory` | The `argv`/environment C strings or the `Process` handle record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Run a program and read its first line of output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  io::print(process::receive(child))
  RETURN 0
END FUNC
```

Run a program in a chosen directory with an extra environment variable merged in:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  LET env AS Map OF String TO String = Map OF String TO String { "GREETING" := "hi" }
  RES child = process::spawn(["printenv", "GREETING"], "/tmp", env, FALSE)
  io::print(process::receive(child))
  RETURN 0
END FUNC
```

## See also

- `mfb man process shell`
- `mfb man process receive`
- `mfb man process send`
- `mfb man process waitFor`
- `mfb man process detach`
- `mfb man process types`
