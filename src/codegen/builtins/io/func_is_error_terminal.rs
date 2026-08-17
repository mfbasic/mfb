//! `io::isErrorTerminal` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`super::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether standard error is an interactive terminal"#;
const DESC: &str = r#"`io::isErrorTerminal` returns `TRUE` when standard error is connected to a
terminal and `FALSE` when it is redirected to a file, a pipe, or any other
non-terminal destination. It takes no arguments.

The answer comes from an `isatty` probe of file descriptor 2: a result greater
than zero yields `TRUE`, anything else — including an error return — yields
`FALSE`. Because a failure is folded into `FALSE`, the call never raises.

Standard error is probed independently of standard output, which matters in the
common case where one is redirected and the other is not: a program run as
`prog > out.txt` should still colour its diagnostics, and `prog 2> log.txt`
should not. Ask this question about the stream you are about to write to. The
probe inspects state only: it writes nothing and changes nothing. In app mode the
program has no real standard streams — error output is rendered by the application
transcript, which is treated as an interactive console — so this call returns
`TRUE` without probing a descriptor."#;
const EX: &str = r#"Colour diagnostics only when the error stream is a terminal:

```
IMPORT io

SUB main()
  IF io::isErrorTerminal() THEN
    io::printError("\u{1b}[31mError\u{1b}[0m: build failed")
  ELSE
    io::printError("Error: build failed")
  END IF
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isErrorTerminal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::native_os_seam(
                Some(super::lower_io_helper),
                Some(super::lower_io_helper),
                &[],
            ),
        }],
    });
}
