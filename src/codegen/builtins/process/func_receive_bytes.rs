//! `process::receiveBytes` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/receiveBytes.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str = r#"Read one available chunk of raw bytes from a child's output."#;
const DESC: &str = r#"`process::receiveBytes` reads the next available chunk of raw bytes from a child's
output stream and returns it as a `List OF Byte`. It performs one underlying read,
so it returns as soon as any data is available rather than waiting to fill a fixed
size, and the returned list is frequently shorter than the amount the child will
eventually produce. It does no line framing, decoding, or newline translation, so
it is the right call for binary output; use `process::receive` for text lines.


Without a `from` argument it reads the child's standard output; pass a `Stream`
value to choose standard output or standard error. The call blocks until at least
one byte is available or the stream ends. A pipe read returns any buffered bytes
before signalling end of stream, so late output is drained; only a read that finds
end of stream with nothing buffered raises `ErrResourceClosed`. On success the
result always holds at least one byte — end of output is never an empty list."#;
const EX: &str = r#"Read one chunk of raw output and report its length:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  LET chunk = process::receiveBytes(child)
  io::print(toString(len(chunk)))
  RETURN 0
END FUNC
```"#;

pub(crate) const RECEIVE_BYTES: BuiltinFunction = BuiltinFunction::same(
    super::RECEIVE_BYTES,
    "receiveBytes",
    INTRO,
    DESC,
    &[],
    super::OV_RECEIVE_BYTES,
)
.with_example(EX);
