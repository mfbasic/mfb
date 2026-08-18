//! `io::print` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`crate::codegen::builtins::io::native::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally).

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Write a `String` to standard output followed by a newline"#;
const DESC: &str = r#"`io::print` writes `value` to standard output and then appends a single line feed
(LF, byte `0x0A`). The text is treated as UTF-8 and emitted byte for byte, with no
escaping and no newline translation beyond the one trailing newline this call
adds. An empty `String` emits nothing but that newline.

Only `String` is accepted, and exactly one argument. There is no implicit
conversion, so a non-string value must be converted first — for example with
`toString`.

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write is a failure and raises
`ErrOutput`.

With standard-output buffering enabled by `io::setBuffered(TRUE)` the text is
appended to a per-thread 4 KiB buffer instead of being written immediately; it is
drained when the buffer fills, on `io::flush`, before any standard-input read, and
at program exit. Buffering is off by default, in which case each call writes
straight through. While the program is in `term::` TUI mode, standard output is
retained rather than printed and nothing reaches the terminal until `term::sync`
presents the frame. Output goes to whatever is bound to standard output: file
descriptor 1 in a console program, and the application transcript window in app
mode (`mfb build --app`)."#;
const EX: &str = r#"Print a line of text:

```
IMPORT io

SUB main()
  io::print("Hello")
END SUB
```

Convert a non-string value before printing:

```
IMPORT io

SUB main()
  io::print(toString(42))
  io::print("total: " & toString(42))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "print",
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
                    Some(crate::codegen::builtins::io::native::lower_io_helper),
                    Some(crate::codegen::builtins::io::native::lower_io_helper),
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
                    Some(crate::codegen::builtins::io::native::lower_io_helper),
                    Some(crate::codegen::builtins::io::native::lower_io_helper),
                    &[],
                ),
            },
        ],
    });
}
