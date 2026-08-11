//! `process::receive` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/receive.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Read one newline-terminated line of text from a child's output."#;
const DESC: &str = r#"`process::receive` reads one line from a child's output stream and returns it as a
`String`, **including** the trailing newline. It reads until it sees a `'\n'`,
never over-reading past the line boundary, so successive calls return successive
lines. Without a `from` argument it reads the child's standard output; pass a
`Stream` value to choose standard output or standard error explicitly.



The call blocks until a full line is available or the stream ends. At end of stream
it **drains before reporting closed**: any bytes accumulated since the last newline
are returned as a final (newline-less) line, and only a subsequent read that finds
end of stream with nothing buffered raises `ErrResourceClosed`. A consumer therefore
loops, reading lines until `ErrResourceClosed` marks the end of the output.


The returned line is validated as UTF-8; output that is not valid UTF-8 raises
`ErrEncoding`. Use `process::receiveBytes` for binary output or output whose
encoding is unknown. Very long lines are capped at 1 MiB: a line reaching that
length is returned as-is without waiting for a newline."#;
const EX: &str = r#"Read one line of a child's standard output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  io::print(process::receive(child))
  RETURN 0
END FUNC
```

Read a diagnostic line from the child's standard error:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo oops 1>&2")
  io::print(process::receive(sh, Stream.StdErr))
  RETURN 0
END FUNC
```"#;

pub(crate) const RECEIVE: BuiltinFunction = BuiltinFunction::same(
    super::RECEIVE,
    "receive",
    INTRO,
    DESC,
    &[],
    super::OV_RECEIVE,
)
.with_example(EX);
