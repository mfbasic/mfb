//! `io::readGrapheme` — descriptor entry + authored docs (bug-699).
//!
//! Per-member file; the body is the shared keystroke-reader lowering in
//! [`super::gen_read_key_family`].

use super::func_read_raw::TIMEOUT_DESC;
use super::gen_read_key_family::{lower_key_reader, KeyReader};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::error::constants::TIMEOUT_UNBOUNDED_SENTINEL;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn lower_read_grapheme(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_key_reader(builder, args, ctx, KeyReader::Grapheme)
}

const INTRO: &str =
    r#"Read the next whole character — one grapheme cluster — of text from standard input"#;
const DESC: &str = r#"`io::readGrapheme` returns the next user-perceived character of text from
standard input as a `String` holding one extended grapheme cluster: a letter
with its combining accents, a flag, a family emoji. It does not wait for a
newline: on a terminal each key is read as it is pressed, with nothing echoed,
and the previous mode is restored before the call returns.

**It returns text, and only text** — the same rules as `io::readScalar`. Enter
arrives as `"\n"`, Tab as `"\t"`, Backspace as U+007F, Escape as U+001B and
the other Ctrl+letter keys as U+0001 through U+001A (Ctrl+H is Backspace and
Ctrl+M is Enter, as listed). Escape arrives from a keyboard, where a lone Escape
is decided after a short pause; in piped input an ESC byte followed by more
input starts a key sequence (ESC and a letter is Alt+letter), so only an ESC
that ends the input arrives as U+001B. A key with no Unicode value (the
arrows, Home, End, Page Up, Page Down, Insert, Delete, the function keys) and a
key release are read and skipped; bytes that are not valid UTF-8 are skipped.

**How it knows a cluster is complete.** A cluster can be typed as several keys,
so after a character the call waits a moment for the next one. From a keyboard
that moment is 25 milliseconds: a combining mark typed later than that starts a
new read. From a pipe or a file there is no timer — the call reads on until the
cluster is complete or the input ends, so the same input always splits the same
way — which also means a timed call on a pipe times out, with nothing read, while
the last character to arrive may still gain a combining mark. A cluster is never
longer than 128 bytes: a longer one is cut at the last scalar that fits, and the
rest — which then begins with a combining mark — is what the next read returns.

`timeoutMs` bounds the whole call: omitted, the call waits for as long as it
takes; `0` makes one immediate attempt; a positive value gives the call that
many milliseconds to return a complete character. When the time runs out the
call raises `ErrTimeout` and nothing is lost — a character that had already
begun to arrive is kept for the next read, together with any combining mark that
follows it. A negative `timeoutMs` raises `ErrInvalidArgument` (the most negative `Integer`
excepted; see the parameter). End of input
raises `ErrEndOfFile`.

Within a thread, the `io` readers share one queue of input, so they can be mixed without losing or reordering anything; each thread has its own view (see the package overview and `thread::openStdIn`). A thread other than the main thread must subscribe with
`thread::openStdIn` before reading, or the call raises `ErrInvalidContext`. In
an `--app` build the program must be in `app::Mode.Console` or
`app::Mode.Canvas`; in `app::Mode.None` there is no window to take keys from and
the call raises `ErrWrongMode`. A read of standard input that the operating
system refuses raises `ErrInputFailed`, and `ErrOutOfMemory` means the program
ran out of memory."#;
const EX: &str = r#"Read one character and echo it back:

```
IMPORT io

FUNC main AS Integer
  LET g AS String = io::readGrapheme()
  io::print("you typed: " & g)
  RETURN 0
END FUNC
```

Read characters until Enter:

```
IMPORT io

FUNC main AS Integer
  MUT word AS String = ""
  MUT g AS String = io::readGrapheme()
  WHILE g <> "\n"
    word = word & g
    g = io::readGrapheme()
  END WHILE
  io::print(word)
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readGrapheme",
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
            return_type: ParameterType::String,
            errors: vec![
                "ErrTimeout",
                "ErrInvalidArgument",
                "ErrEndOfFile",
                "ErrInputFailed",
                "ErrInvalidContext",
                "ErrOutOfMemory",
                "ErrWrongMode",
            ],
            body: Body::abi_function(lower_read_grapheme),
        }],
    });
}
