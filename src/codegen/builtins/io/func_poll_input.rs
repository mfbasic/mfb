//! `io::pollInput` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`crate::codegen::builtins::io::native::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Test whether standard input is ready to read, optionally waiting up to a timeout"#;
const DESC: &str = r#"`io::pollInput` reports whether a following read of standard input can proceed
without blocking. It returns `TRUE` when input is ready and `FALSE` when the wait
elapses first, and it **consumes nothing** — the bytes are still there for
`io::readLine`, `io::readChar`, `io::readByte`, or `io::input`.

`timeoutMs` bounds the wait, in milliseconds, following the language timeout
convention. When it is **omitted, `pollInput` blocks** until standard input
becomes ready and then returns `TRUE` (omit = unbounded). `0` is a non-blocking
check that returns immediately with the current readiness. A positive value waits
up to that long. A negative `timeoutMs` is rejected with `ErrInvalidArgument`.
Because the host `poll` takes a C `int`, a value above `2147483647` is clamped to
that, which is roughly 24 days.

Readiness is answered in two stages: a byte already staged in the per-thread
broadcast log reports `TRUE` at once with no system call, and only when the log
holds nothing for this thread does the call `poll` file descriptor 0. A thread
that has not subscribed simply defers to that `poll`; unlike the read calls,
`io::pollInput` does not raise `ErrInvalidContext`. **End of input counts as
ready**, so `io::pollInput` returns `TRUE` and the following read then raises
`ErrEof`; a `TRUE` result promises that the next read will not block, not that it
will succeed. A signal delivered while blocked is not an error: the `poll` is
re-armed and retried."#;
const EX: &str = r#"Read a line only when one is already pending (pass `0` for the immediate check —
omitting the timeout would instead block until input is ready):

```
IMPORT io

SUB main()
  IF io::pollInput(0) THEN
    io::print(io::readLine())
  END IF
END SUB
```

Wait up to a second for a keypress:

```
IMPORT io

SUB main()
  IF io::pollInput(1000) THEN
    io::print(io::readChar())
  ELSE
    io::print("timeout")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pollInput",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "timeoutMs",
                desc: "Optional. Omit to block until standard input is ready; `0` is an immediate non-blocking check; a positive value waits up to that many milliseconds, clamped to `2147483647`. Must not be negative.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::Optional,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::native_os_seam(
                Some(crate::codegen::builtins::io::native::lower_io_helper),
                Some(crate::codegen::builtins::io::native::lower_io_helper),
                &[],
            ),
        }],
    });
}
