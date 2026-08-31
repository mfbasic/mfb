//! `io::readLine` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101); the shared line-reader seam that `io::input`
//! also dispatches through lives in [`super::gen_read_line_family`], and the shared
//! stdin byte/UTF-8 read primitives in [`super::gen_read_family`].

use super::gen_read_line_family::lower_read_line_family;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `abi_function` body for `io::readLine` — read a line from stdin with no prompt
/// (echo/canonical mode suppressed for the read on a console tty).
pub(crate) fn lower_read_line(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_read_line_family(builder, ctx, false, "io.readLine")
}

const INTRO: &str = r#"Read one line of UTF-8 text from standard input, with no prompt"#;
const DESC: &str = r#"`io::readLine` reads bytes from standard input up to and including the next line
feed (LF, byte `0x0A`) and returns the line as a `String` with its terminator
removed. If the byte immediately before the LF is a carriage return (CR, byte
`0x0D`) — a CRLF ending — that CR is stripped as well. A line that is empty before
its terminator returns an empty `String`, and the terminator is read too. It
takes no arguments.

**On a terminal, `io::readLine` suppresses echo for the duration of the read.**
It clears `ECHO` on standard input while leaving canonical (line) mode intact, so
the user still edits the line normally and submits it with Return, but the typed
characters are not displayed. The previous line discipline is restored before the
call returns. This is the difference from `io::input`, which leaves the terminal
untouched and therefore echoes; use `io::readLine` for passphrases and
`io::input` when the user should see what they type. When standard input is not a
terminal the stream is read as is with no mode change.

Before blocking, any pending standard-output buffer is drained, so output already
produced — including a prompt written with `io::write` — appears before the
program waits. Bytes are decoded as UTF-8 as they arrive, with the full validity
check; an ill-formed sequence fails rather than yielding a replacement character.
End of input is reported as an error, not as an empty result — but only when it
arrives before any byte of the line. Input that ends mid-line is not lost: those
bytes come back as the final, unterminated line, and the *next* call raises
`ErrEof`. Standard input is a per-thread broadcast log;
a thread other than the main thread must subscribe with `thread::openStdIn` before
reading, or the call raises `ErrInvalidContext`."#;
const EX: &str = r#"Read a line and echo it back:

```
IMPORT io

SUB main()
  LET line AS String = io::readLine()
  io::print(line)
END SUB
```

Prompt without echoing the answer:

```
IMPORT io

SUB main()
  io::write("Passphrase: ")
  LET secret AS String = io::readLine()
  io::print("")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readLine",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_read_line),
        }],
    });
}
