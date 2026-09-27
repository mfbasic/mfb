//! `io::readLine` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101); the shared line-reader seam that `io::input`
//! also dispatches through lives in [`super::gen_read_line_family`], and the shared
//! stdin byte/UTF-8 read primitives in [`super::gen_read_family`].

use super::gen_read_line_family::{lower_read_line_family, timeout_parameter};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `abi_function` body for `io::readLine` — read a line from stdin with no prompt
/// (echo/canonical mode suppressed for the read on a console tty).
pub(crate) fn lower_read_line(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_read_line_family(builder, args, ctx, false, "io.readLine")
}

const INTRO: &str = r#"Read one line of UTF-8 text from standard input, with no prompt"#;
const DESC: &str = r#"`io::readLine` reads one line from standard input and returns it as a `String`
without its terminator. A line that is empty before its terminator returns an
empty `String`.

**From a terminal or an `--app` window it reads what was typed.** The line ends
at Enter; Backspace erases the last character; keys that type nothing — the
arrows, Home, End, the function keys — and Escape are ignored rather than
becoming part of the line. On a terminal, `io::readLine` suppresses echo for the
duration of the read: the user still edits the line normally and submits it
with Return, but the typed characters are not displayed, and the terminal's
usual input behavior is restored when the call finishes. This is the difference from
`io::input`, which echoes; use `io::readLine` for passphrases and `io::input`
when the user should see what they type.

**From a pipe or a file it reads the bytes as they are,** up to and including
the next line feed (LF, byte `0x0A`); a carriage return (CR) immediately before
the LF — a CRLF ending — is removed too. An escape byte in the data is just a
byte.

Either way the text must be valid UTF-8: an ill-formed sequence raises
`ErrEncoding` rather than being dropped or replaced, because a line with a byte
silently missing would look complete. (The keystroke readers — `io::readKey`,
`io::readScalar`, `io::readGrapheme` — skip such bytes instead.)

`timeoutMs` bounds the wait: omitted, the call waits for as long as it takes;
`0` makes one immediate attempt; a positive value waits up to that many
milliseconds. When the time runs out the call raises `ErrTimeout`, and **no part
of the line is lost**: whatever part of it had already been read is kept, and
the next read of standard input — of any kind — starts with it. On a terminal a
half-typed line simply stays where the user is typing it. A negative
`timeoutMs` raises `ErrInvalidArgument`.

Before blocking, any pending standard-output buffer is drained, so output already
produced — including a prompt written with `io::write` — appears before the
program waits. End of input is reported as an error, not as an empty result —
but only when it arrives before any character of the line. Input that ends
mid-line is not lost: those characters come back as the final, unterminated
line, with a trailing CR still removed, and the *next* call raises
`ErrEndOfFile`. A worker thread must subscribe with `thread::openStdIn` before
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
```

Give the user five seconds to answer:

```
IMPORT errorCode
IMPORT io

SUB main()
  io::write("Continue? ")
  LET answer AS String = io::readLine(5000) TRAP(e)
    IF e.code = errorCode::ErrTimeout THEN
      io::print("")
      io::print("no answer; continuing")
      EXIT SUB
    END IF
    FAIL e
  END TRAP
  io::print("you said " & answer)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readLine",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![timeout_parameter()],
            return_type: ParameterType::String,
            errors: vec![
                "ErrTimeout",
                "ErrInvalidArgument",
                "ErrEndOfFile",
                "ErrEncoding",
                "ErrInputFailed",
                "ErrInvalidContext",
                "ErrOutOfMemory",
            ],
            body: Body::abi_function(lower_read_line),
        }],
    });
}
