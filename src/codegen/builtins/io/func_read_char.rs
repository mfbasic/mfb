//! `io::readChar` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`crate::codegen::builtins::io::native::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Read one whole Unicode scalar value from standard input"#;
const DESC: &str = r#"`io::readChar` reads exactly one Unicode scalar value from standard input and
returns it as a one-character `String`. It reads the lead byte, derives the
sequence length from it, and reads the one to three continuation bytes that
complete the scalar. It takes no arguments and does not wait for a newline.

**On a terminal the read is a single keypress.** For the duration of the call,
standard input is switched out of canonical mode and echo is suppressed
(`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0`), so one key satisfies the read with
no Return and nothing is displayed; the previous line discipline is restored
before the call returns. When standard input is not a terminal the stream is read
as is with no mode change.

Before blocking, any pending standard-output buffer is drained, so a prompt
written with `io::write` appears before the program waits. Decoding is strict
UTF-8, not lenient: an ill-formed sequence raises `ErrEncoding` rather than
yielding a replacement character, and so does a sequence cut short by end of
input. This returns one *scalar value*, not one user-perceived character: a
grapheme cluster made of several scalars takes that many calls. Compare
`io::readByte`, which returns raw bytes with no decoding at all.

End of input is reported as an error, not as an empty result. Use `io::pollInput`
to test for readiness when the program must not block. Standard input is a
per-thread broadcast log; a thread other than the main thread must subscribe with
`thread::openStdIn` before reading, or the call raises `ErrInvalidContext`."#;
const EX: &str = r#"Wait for any keypress to continue:

```
IMPORT io

SUB main()
  io::write("Press any key to continue...")
  LET ignored AS String = io::readChar()
  io::print("")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readChar",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native_os_seam(
                Some(crate::codegen::builtins::io::native::lower_io_helper),
                Some(crate::codegen::builtins::io::native::lower_io_helper),
                &[],
            ),
        }],
    });
}
