//! `os::pid` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"The current process id"#;
const DESC: &str = r#"`os::pid` returns the process id of the running program as an `Integer`, via the
host `getpid` call. The value is positive and stable for the life of the process.

`os::pid` is **not pure** in the sense that different processes see different
values, but within one process every call returns the same id. It reads process
state only and has no side effects."#;
const EX: &str = r#"Print the process id:

```
IMPORT os
IMPORT io

SUB main()
  io::print(toString(os::pid()))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pid",
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
