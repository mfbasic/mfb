//! `io::readRaw` — descriptor entry + authored docs (bug-699).
//!
//! Per-member file; the body is the shared keystroke-reader lowering in
//! [`super::gen_read_key_family`].

use super::gen_read_key_family::{lower_key_reader, KeyReader};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::error::constants::TIMEOUT_UNBOUNDED_SENTINEL;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn lower_read_raw(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_key_reader(builder, args, ctx, KeyReader::Raw)
}

const INTRO: &str = r#"Read the next byte of standard input exactly as it arrived"#;
const DESC: &str = r#"`io::readRaw` returns the next byte of standard input as a `Byte`, with no
decoding at all. It does not wait for a newline.

**The bytes a key produces differ by platform and terminal.** An arrow key is
three or more bytes, and which bytes depends on the terminal program, on whether
`term::on` is active, and on whether the program is a console or an `--app`
build: an `--app` window, and a terminal that speaks the kitty keyboard protocol
under `term::on`, report every key — letters included — as an escape sequence.
Use `io::readKey` to get the same answer for a key everywhere, and
`io::readScalar` or `io::readGrapheme` for text. `io::readRaw` is for binary
input and for protocols that frame their own bytes; on a pipe or a file it
returns the input byte for byte.

**On a terminal the read is a single keypress.** For the duration of the call,
standard input is switched out of line mode and echo is suppressed, so one key
satisfies the read with no Return and nothing is displayed; the previous mode is
restored before the call returns. In an `--app` build the window delivers each
key as it is pressed.

`timeoutMs` bounds the wait: omitted, the call waits for as long as it takes;
`0` makes one immediate attempt; a positive value waits up to that many
milliseconds. When the time runs out the call raises `ErrTimeout` and nothing is
read. A negative `timeoutMs` raises `ErrInvalidArgument`.

End of input raises `ErrEndOfFile`, never a sentinel byte, so all 256 byte values
are data. The readers share one queue of input, so `io::readRaw` can be mixed
with the other `io` readers without losing or reordering a byte. Standard input
is a per-thread broadcast log; a thread other than the main thread must subscribe
with `thread::openStdIn` before reading, or the call raises `ErrInvalidContext`."#;
const EX: &str = r#"Read one byte and report its value:

```
IMPORT io

SUB main()
  LET b AS Byte = io::readRaw()
  io::print(toString(b))
END SUB
```

Take a byte only if one is already waiting:

```
IMPORT errorCode
IMPORT io

SUB main()
  LET b AS Byte = io::readRaw(0) TRAP(e)
    IF e.code = errorCode::ErrTimeout THEN
      io::print("nothing waiting")
      EXIT SUB
    END IF
    FAIL e
  END TRAP
  io::print("got " & toString(b))
END SUB
```"#;

pub(crate) const TIMEOUT_DESC: &str = "Optional. How long to wait for input, in milliseconds. Omit to wait for as long as it takes; `0` makes one immediate attempt; a positive value waits up to that long, then raises `ErrTimeout`. Must not be negative.";

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readRaw",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "timeoutMs",
                desc: TIMEOUT_DESC,
                aliases: &[],
                ty: ParameterType::Integer,
                // An omitted timeout waits without bound: the convention's
                // sentinel, padded at the call.
                default: DefaultValue::Fill {
                    type_name: ParameterType::Integer,
                    expr: TIMEOUT_UNBOUNDED_SENTINEL,
                },
            }],
            return_type: ParameterType::Byte,
            errors: vec![
                "ErrTimeout",
                "ErrInvalidArgument",
                "ErrEndOfFile",
                "ErrInputFailed",
                "ErrInvalidContext",
                "ErrWrongMode",
            ],
            body: Body::abi_function(lower_read_raw),
        }],
    });
}
