//! `process::send` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/send.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Write a line of text to a child's standard input, appending a newline."#;
const DESC: &str = r#"`process::send` writes the UTF-8 bytes of `text` to the child's standard input and
then appends a single newline (`'\n'`), so each call delivers one complete line to
a line-oriented child. To write raw bytes with no trailing newline, use
`process::sendBytes`.

The whole payload is written before the call returns: it loops over the underlying
writes, advancing past whatever each accepted and retrying an interrupted write, so
a short write is resumed rather than mistaken for completion. Without a `timeoutMs`
the call blocks while the child's input pipe is full, waiting for the child to
consume enough to make room.

If the child has closed or is no longer reading its standard input — a broken pipe —
the write fails and `send` raises `ErrResourceClosed`, the same error raised when
the input was already closed with `process::close` or the handle was dropped or
detached.

`timeoutMs` bounds how long the call may wait for pipe space, in milliseconds;
when the deadline passes with the payload not fully written it raises `ErrTimeout`.
On Windows the timeout is best-effort: anonymous pipes have no write-readiness poll,
so a write to a draining reader returns immediately (the common case) but a write
that fills the pipe is not preempted at the deadline."#;
const EX: &str = r#"Send two lines to a filter and read its sorted output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sorter = process::spawn(["sort"])
  process::send(sorter, "banana")
  process::send(sorter, "apple")
  process::close(sorter)
  io::print(process::receive(sorter))
  RETURN 0
END FUNC
```

Bound the write with a one-second timeout:

```
IMPORT process

FUNC main AS Integer
  RES child = process::spawn(["cat"])
  process::send(child, "hello", 1000)
  RETURN 0
END FUNC
```"#;

pub(crate) const SEND: BuiltinFunction =
    BuiltinFunction::same(super::SEND, "send", INTRO, DESC, &[], super::OV_SEND).with_example(EX);
