//! `io::readScalar` — descriptor entry + authored docs (bug-699).
//!
//! Per-member file; the body is the shared keystroke-reader lowering in
//! [`super::gen_read_key_family`].

use super::func_read_raw::TIMEOUT_DESC;
use super::gen_read_key_family::{lower_key_reader, KeyReader};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::types::type_utils::scalar_type;
use crate::codegen::error::constants::TIMEOUT_UNBOUNDED_SENTINEL;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn lower_read_scalar(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_key_reader(builder, args, ctx, KeyReader::Scalar)
}

const INTRO: &str = r#"Read the next Unicode scalar of text typed or piped to standard input"#;
const DESC: &str = r#"`io::readScalar` returns the next Unicode scalar of text from standard input as
a `Scalar`. It does not wait for a newline: on a terminal each key is read as it
is pressed, with nothing echoed, and the previous mode is restored before the
call returns.

**It returns text, and only text.** A key that types something returns it,
whatever platform or terminal it came from. The keys that have a control
character arrive as that scalar: Enter as U+000A (`\n`), Tab as U+0009,
Backspace as U+007F, Escape as U+001B, and Ctrl+A through Ctrl+Z as U+0001
through U+001A. A key with no Unicode value — the arrows, Home, End, Page Up,
Page Down, Insert, Delete, the function keys — is read and skipped, as is a key
release; use `io::readKey` to see those. Bytes that are not valid UTF-8 are
skipped too, never reported.

A character made of several scalars — `é` typed as `e` plus a combining accent,
or an emoji sequence — arrives one scalar per call. Use `io::readGrapheme` to
read it whole.

`timeoutMs` bounds the wait: omitted, the call waits for as long as it takes;
`0` makes one immediate attempt; a positive value waits up to that many
milliseconds. When the time runs out the call raises `ErrTimeout` and nothing is
read. A negative `timeoutMs` raises `ErrInvalidArgument`. End of input raises
`ErrEndOfFile`.

The `io` readers share one queue of input, so they can be mixed without losing
or reordering anything. A thread other than the main thread must subscribe with
`thread::openStdIn` before reading, or the call raises `ErrInvalidContext`. `ErrOutOfMemory` means the program ran out of memory."#;
const EX: &str = r#"Read one character and show its code point:

```
IMPORT io

FUNC main AS Integer
  LET c AS Scalar = io::readScalar()
  io::print(toString(c) & " is U+" & toString(toInt(c)))
  RETURN 0
END FUNC
```

Wait up to half a second for a key:

```
IMPORT errorCode
IMPORT io

FUNC main AS Integer
  LET c AS Scalar = io::readScalar(500) TRAP(e)
    IF e.code = errorCode::ErrTimeout THEN
      io::print("no key")
      RETURN 0
    END IF
    FAIL e
  END TRAP
  io::print("key: " & toString(c))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readScalar",
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
            return_type: scalar_type(),
            errors: vec![
                "ErrTimeout",
                "ErrInvalidArgument",
                "ErrEndOfFile",
                "ErrInputFailed",
                "ErrInvalidContext",
                "ErrOutOfMemory",
                "ErrWrongMode",
            ],
            body: Body::abi_function(lower_read_scalar),
        }],
    });
}
