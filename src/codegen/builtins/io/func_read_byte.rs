//! `io::readByte` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`crate::codegen::builtins::io::native::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Read one raw byte from standard input"#;
const DESC: &str = r#"`io::readByte` reads exactly one byte from standard input and returns it as a
`Byte` in the range 0 through 255. It takes no arguments and does not wait for a
newline.

**On a terminal the read is a single keypress.** For the duration of the call,
standard input is switched out of canonical mode and echo is suppressed
(`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0`), so one key satisfies the read with
no Return and nothing is displayed; the previous line discipline is restored
before the call returns. When standard input is not a terminal the stream is read
as is with no mode change.

Before blocking, any pending standard-output buffer is drained, so a prompt
written with `io::write` appears before the program waits. No decoding happens:
the byte is transferred verbatim, so a multi-byte character such as an emoji
arrives one byte at a time across successive calls and there is no `ErrEncoding`
to raise — this is the difference from `io::readChar`, which always returns one
whole Unicode scalar value. Use `io::readByte` for binary input or protocol
framing, and `io::readChar` for text.

End of input is reported as an error, not as a sentinel value such as `0` or
`-1`, which keeps every one of the 256 byte values usable as data. Use
`io::pollInput` to test for readiness when the program must not block. Standard
input is a per-thread broadcast log; a thread other than the main thread must
subscribe with `thread::openStdIn` before reading, or the call raises
`ErrInvalidContext`."#;
const EX: &str = r#"Read one byte and report its value:

```
IMPORT io

SUB main()
  LET b AS Byte = io::readByte()
  io::print(toString(b))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readByte",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Byte,
            errors: vec![],
            body: Body::native_os_seam(
                Some(crate::codegen::builtins::io::native::lower_io_helper),
                Some(crate::codegen::builtins::io::native::lower_io_helper),
                &[],
            ),
        }],
    });
}
