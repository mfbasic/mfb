//! `process::spawn` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs migrated from `src/docs/man/builtins/process/spawn.md`.

use std::collections::HashMap;

use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::code::{CodegenPlatform, HelperResult};

const INTRO: &str =
    r#"Run a program directly from an argument list, returning a handle to the child."#;
const DESC: &str = r#"`process::spawn` starts a child process from an explicit argument vector and
returns an owned `Process` handle to it. `args[0]` is the executable and is
resolved on `PATH` (`execvp` on Unix); the remaining elements are passed as the
child's arguments verbatim. **No shell is involved** — quoting, globbing, pipes,
redirection, and environment-variable expansion are *not* interpreted, so an
argument that contains spaces or shell metacharacters reaches the program as one
literal argument. Use `process::shell` when you need a shell to parse a command
line.

The child is created with three pipes wired to its standard input, output, and
error, so the parent can `process::send` to it and `process::receive` from it.
Creation forks and execs; an exec failure in the child (for example a program that
is not found) is reported back to the parent over a close-on-exec self-pipe and
surfaces as `ErrSpawnFailed`, not as a silently running child.


The returned `Process` is an owned, non-copyable resource handle. It is closed by
lexical drop when its binding leaves scope, which **force-kills and reaps** a
still-running child (`SIGKILL` + `waitpid` on Unix) so no runaway process or zombie
is left; call `process::waitFor` first if the child should be allowed to finish, or
`process::detach` to let it outlive the program.

The empty argument list is rejected with `ErrInvalidArgument` — there is no program
to run."#;
const EX: &str = r#"Run a program and read its first line of output:

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
```"#;

pub(crate) const SPAWN: BuiltinFunction = BuiltinFunction::os(
    super::SPAWN,
    "spawn",
    INTRO,
    DESC,
    &[],
    super::OV_SPAWN,
    lower_process_spawn_helper_posix,
    lower_process_spawn_helper_win,
    &["process.spawn", "process.spawnEnv"],
)
.with_example(EX);

pub(crate) fn lower_process_spawn_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    if call == "process.spawnEnv" {
        super::native::unix::lower_process_spawnenv_helper(symbol, platform_imports, platform)
    } else {
        super::native::unix::lower_process_spawn_helper(symbol, platform_imports, platform, false)
    }
}

pub(crate) fn lower_process_spawn_helper_win(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    if call == "process.spawnEnv" {
        super::native::windows::lower_process_spawnenv_helper(symbol, platform_imports, platform)
    } else {
        super::native::windows::lower_process_spawn_helper(
            symbol,
            platform_imports,
            platform,
            false,
        )
    }
}
