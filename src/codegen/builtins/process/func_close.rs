//! `process::close` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). process members are
//! `Implementation::Same`: they lower via the `_mfb_rt_process_*` runtime-call
//! seam (emission in `../native/`), so this file carries only the descriptor +
//! docs migrated from `src/docs/man/builtins/process/close.md`.

use crate::codegen::registry::BuiltinFunction;

const INTRO: &str =
    r#"Close a child's standard input, signalling end-of-input; the child keeps running."#;
const DESC: &str = r#"`process::close` closes the child's standard input — the parent's write end of the
child's stdin pipe. It sends end-of-input to the child, so a filter that reads
until EOF (`sort`, `cat`, `wc`, `tr`, …) stops waiting for more input and produces
its output. After `close`, further `process::send`/`process::sendBytes` to the same
child raise `ErrResourceClosed`.

`process::close` is **not** a handle-consuming close. Despite the name, it does not
release the `Process` resource: the child keeps running, its output stays readable
with `process::receive`, and the handle remains valid and owned. The resource is
still closed the usual way — by lexical drop at scope exit (which force-kills and
reaps the child) — because `close` is deliberately not treated as an ownership
transfer.

Closing the input is idempotent with respect to the input pipe: once stdin is
closed the call is a harmless no-op. Only a handle that has already been dropped or
detached makes `close` raise `ErrResourceClosed`."#;
const EX: &str = r#"Feed a filter its input, then close stdin so it flushes its output:

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
```"#;

pub(crate) const CLOSE: BuiltinFunction = BuiltinFunction::same(
    super::CLOSE,
    "close",
    INTRO,
    DESC,
    &[],
    &[super::ov(super::P_PROC, "Nothing")],
)
.with_example(EX);
