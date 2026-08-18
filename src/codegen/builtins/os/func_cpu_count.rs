//! `os::cpuCount` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"The number of online logical CPUs"#;
const DESC: &str = r#"`os::cpuCount` returns the number of online logical CPUs as reported by the host
`sysconf(_SC_NPROCESSORS_ONLN)`. The result is clamped to a minimum of 1, so a
caller always gets a usable count even if the host cannot determine the true
value.

Use it to size a `thread::` worker pool. The value reflects CPUs online at the
moment of the call and may in principle change over a long-running process on a
host that hot-plugs CPUs."#;
const EX: &str = r#"Print the CPU count:

```
IMPORT os
IMPORT io

SUB main()
  io::print(toString(os::cpuCount()))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "cpuCount",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::native(
                Some(crate::codegen::builtins::os::native::lower_os_helper),
                Some(crate::codegen::builtins::os::native::lower_os_helper),
                None,
            ),
        }],
    });
}
