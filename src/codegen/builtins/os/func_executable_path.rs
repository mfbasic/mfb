//! `os::executablePath` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"The path to the running executable"#;
const DESC: &str = r#"`os::executablePath` returns the filesystem path of the running binary as an
owned `String`. On macOS it uses `_NSGetExecutablePath`; on Linux it reads the
`/proc/self/exe` symlink with `readlink`, which yields the absolute, symlink-
resolved path.

Use it to locate resources beside the executable, or to report the program's own
path. If the host cannot determine the path, `os::executablePath` raises
`ErrUnsupported`. It reads host state only and has no side effects."#;
const EX: &str = r#"Print the executable path:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::executablePath())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "executablePath",
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
