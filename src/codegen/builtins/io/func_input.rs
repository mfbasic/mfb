//! `io::input` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`super::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally). Docs migrated from
//! `src/docs/man/builtins/io/input.md`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Read one line of UTF-8 text from standard input, optionally writing a prompt first"#;
const DESC: &str = r#"`io::input` optionally writes a prompt to standard output, then reads bytes from
standard input up to and including the next line feed (LF, byte `0x0A`) and
returns the line as a `String` with its terminator removed. A preceding carriage
return (CR, byte `0x0D`, from a CRLF ending) is stripped as well. A line that is
empty before its terminator returns an empty `String`.

**`io::input` does not change the terminal mode**, so typed characters are echoed
by the terminal in the usual way and the line is submitted with Return. This is
the difference from `io::readLine`, which suppresses echo for the read; reach for
`io::input` when the user should see what they type.

The prompt is written verbatim — no trailing space or newline is added — and it
is written **directly**, bypassing the standard-output buffer, so it is on screen
before the program blocks. Any bytes already sitting in that buffer are drained
first, keeping the prompt in order with earlier output. An empty prompt writes
nothing at all and therefore cannot fail; `io::input()` with no argument is
exactly `io::input("")`. A genuine failure while writing the prompt raises
`ErrOutput` before any input is read.

Bytes are decoded as UTF-8 as they arrive, with the full validity check; an
ill-formed sequence fails rather than yielding a replacement character. End of
input is an error rather than an empty result, but only when it arrives before any
byte of the line: if input ends after some bytes were read, those bytes are
returned as the final unterminated line and the following call raises `ErrEof`.
Standard input is a per-thread broadcast log; a thread other than the main thread
must subscribe with `thread::openStdIn` before reading, or the call raises
`ErrInvalidContext`. In app mode the prompt goes to the application transcript and
the line is read from the window input pipe."#;
const EX: &str = r#"Prompt on the same line and greet the user:

```
IMPORT io

SUB main()
  LET name AS String = io::input("Name: ")
  io::print("Hello, " & name)
END SUB
```

Read a line without a prompt:

```
IMPORT io

SUB main()
  LET line AS String = io::input()
  io::print(line)
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "input",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "prompt",
                desc: "Optional. Text written to standard output before the read, verbatim and with nothing appended. An empty `String` (or omitting the argument) writes nothing.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::Optional,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native_os_seam(
                Some(super::lower_io_helper),
                Some(super::lower_io_helper),
                &[],
            ),
        }],
    });
}
