//! `process::shell` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/shell.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str =
    r#"Run a command line through the platform shell, returning a handle to the child."#;
const DESC: &str = r#"`process::shell` runs `cmd` as a shell command line and returns an owned `Process`
handle to the resulting child. Unlike `process::spawn`, which execs a program
directly, `shell` hands the string to the platform shell — `/bin/sh -c` on Unix —
so shell features work: pipelines (`|`), redirection (`>`, `<`), globbing (`*`),
command sequencing (`;`, `&&`), quoting, and environment-variable expansion are all
interpreted by the shell.

Because the string is parsed by a shell, values interpolated into `cmd` are subject
to shell word-splitting and metacharacter interpretation; build the command with
care when any part comes from untrusted input. When you do not need a shell — you
have a program and its arguments already separated — prefer `process::spawn`, which
avoids the shell entirely.

The child is wired to three pipes for its standard streams exactly as with
`process::spawn`, and the returned handle has the same ownership: it is closed by
lexical drop at scope exit, which force-kills and reaps a still-running child unless
it is first awaited with `process::waitFor` or released with `process::detach`."#;
const EX: &str = r#"Run a pipeline and read the result:

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
```"#;

pub(crate) const SHELL: BuiltinFunction = BuiltinFunction::same(
    super::SHELL,
    "shell",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_SHELL, super::PROCESS_TYPE)],
)
.with_example(EX);
