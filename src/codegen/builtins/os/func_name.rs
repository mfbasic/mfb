//! `os::name` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"The operating-system family the program was built for"#;
const DESC: &str = r#"`os::name` returns the operating-system family of the build target: `"macos"` or
`"linux"`. It is a compile-time constant — the binary is built for exactly one
target, so the value is fixed at build time and materialized directly into an
owned `String`, with no host call.

Pair it with `os::arch` to identify the full platform. Because the value is
fixed per build, it is stable across runs of the same binary."#;
const EX: &str = r#"Print the platform:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::name() & "/" & os::arch())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "name",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(
                Some(crate::codegen::builtins::os::native::lower_os_helper),
                Some(crate::codegen::builtins::os::native::lower_os_helper),
                None,
            ),
        }],
    });
}
