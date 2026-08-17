//! `io::write` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`super::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally). Docs migrated from
//! `src/docs/man/builtins/io/write.md`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Write a `String` to standard output with no trailing newline"#;
const DESC: &str = r#"`io::write` writes `value` to standard output exactly as stored and adds nothing.
The text is treated as UTF-8 and emitted byte for byte, with no escaping and no
newline translation. An empty `String` writes nothing at all. It is the
newline-free counterpart of `io::print`, which is the same call with a trailing
LF appended.

Only `String` is accepted, and exactly one argument; there is no implicit
conversion, so convert other values first — for example with `toString`.

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write is a failure and raises
`ErrOutput`.

With standard-output buffering enabled by `io::setBuffered(TRUE)` the text is
appended to a per-thread 4 KiB buffer rather than written immediately, so it may
not be visible to an external reader until drained. The buffer is drained when it
fills, on `io::flush`, before any standard-input read, and at program exit —
which is why a prompt written with `io::write` still appears before a following
`io::readLine` even under buffering. While the program is in `term::` TUI mode,
standard output is retained rather than printed and nothing reaches the terminal
until `term::sync` presents the frame. Output goes to whatever is bound to
standard output: file descriptor 1 in a console program, and the application
transcript window in app mode (`mfb build --app`)."#;
const EX: &str = r#"Write a prompt on the same line as the answer:

```
IMPORT io

SUB main()
  io::write("Name: ")
  LET name AS String = io::readLine()
END SUB
```

Build a line from several pieces:

```
IMPORT io

SUB main()
  io::write("x=")
  io::write(toString(3))
  io::print("")
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "write",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "The text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::native_os_seam(
                    Some(super::lower_io_helper),
                    Some(super::lower_io_helper),
                    &[],
                ),
            },
            Implementation {
                params: vec![Parameter {
                    name: "value",
                    desc: "The attributed text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.",
                    aliases: &[],
                    ty: ParameterType::Named("AttributedString"),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Nothing,
                errors: vec![],
                body: Body::native_os_seam(
                    Some(super::lower_io_helper),
                    Some(super::lower_io_helper),
                    &[],
                ),
            },
        ],
    });
}
