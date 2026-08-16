//! `io::isInputTerminal` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `io` is a native OS-seam package: the
//! member registers a `Body::native_os_seam` whose per-family slots both hold the
//! shared [`super::lower_io_helper`] dispatcher (which branches on
//! `platform.family()` and the runtime-call name internally). Docs migrated from
//! `src/docs/man/builtins/io/isInputTerminal.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether standard input is an interactive terminal"#;
const DESC: &str = r#"`io::isInputTerminal` returns `TRUE` when standard input is connected to a
terminal and `FALSE` when it is redirected from a file, a pipe, or any other
non-terminal source. It takes no arguments.

The answer comes from an `isatty` probe of file descriptor 0: a result greater
than zero yields `TRUE`, anything else — including an error return — yields
`FALSE`. Because a failure is folded into `FALSE`, the call never raises.

The probe inspects state only. It does not modify the stream, consume any input,
or block waiting for data, so it is safe to call before deciding whether to
prompt interactively, enable line editing, or read a piped stream straight
through. In app mode the program has no real standard streams — input is served by
the application window, which is treated as an interactive console — so this call
returns `TRUE` without probing a descriptor."#;
const EX: &str = r#"Prompt only when a human is attached, otherwise read the piped stream:

```
IMPORT io

SUB main()
  IF io::isInputTerminal() THEN
    io::print(io::input("Name: "))
  ELSE
    io::print(io::readLine())
  END IF
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isInputTerminal",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
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
