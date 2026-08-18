//! `io::isOutputTerminal` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`crate::codegen::builtins::io::native::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether standard output is an interactive terminal"#;
const DESC: &str = r#"`io::isOutputTerminal` returns `TRUE` when standard output is connected to a
terminal and `FALSE` when it is redirected to a file, a pipe, or any other
non-terminal destination. It takes no arguments.

The answer comes from an `isatty` probe of file descriptor 1: a result greater
than zero yields `TRUE`, anything else — including an error return — yields
`FALSE`. Because a failure is folded into `FALSE`, the call never raises.

The probe inspects state only: it writes nothing and changes nothing. Use it to
decide whether emitting ANSI colour, progress bars, or cursor tricks is
appropriate, and to fall back to plain text when output is being captured. The
answer says nothing about `io::setBuffered`: buffering is an MFBASIC-level setting
the program controls, not something inferred from whether standard output is a
terminal. In app mode the program has no real standard streams — output is
rendered by the application transcript window, which is treated as an interactive
console — so this call returns `TRUE` without probing a descriptor."#;
const EX: &str = r#"Colour the output only when a terminal is attached:

```
IMPORT io

SUB main()
  IF io::isOutputTerminal() THEN
    io::print("\u{1b}[32mStatus: OK\u{1b}[0m")
  ELSE
    io::print("Status: OK")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isOutputTerminal",
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
                Some(crate::codegen::builtins::io::native::lower_io_helper),
                Some(crate::codegen::builtins::io::native::lower_io_helper),
                &[],
            ),
        }],
    });
}
