//! `os::arch` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). Docs migrated from
//! `src/docs/man/builtins/os/arch.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"The CPU architecture the program was built for"#;
const DESC: &str = r#"`os::arch` returns the CPU architecture of the build target: `"aarch64"`,
`"x86_64"`, or `"riscv64"`. Like `os::name`, it is a compile-time constant fixed
at build time and materialized directly into an owned `String`, with no host
call."#;
const EX: &str = r#"Print the architecture:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::arch())
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "arch",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(
                Some(super::lower_os_helper),
                Some(super::lower_os_helper),
                None,
            ),
        }],
    });
}
