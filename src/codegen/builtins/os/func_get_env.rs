//! `os::getEnv` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). `os` is a native OS-seam package: the
//! member registers a `Body::native` whose per-family slots both hold the shared
//! [`crate::codegen::builtins::os::native::lower_os_helper`] dispatcher (which branches on `platform.family()`
//! and the runtime-call name internally).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Read an environment variable, raising when it is unset"#;
const DESC: &str = r#"`os::getEnv` returns the value of the environment variable named `name` as it
appears in the live process environment, including any value written earlier by
`os::setEnv`. The lookup is the host `getenv` call; the returned bytes are copied
into a fresh owned `String`.

If the variable is not set, `os::getEnv` raises `ErrNotFound` rather than
returning an empty string, so a program can distinguish an unset variable from
one deliberately set to the empty string. Use `os::getEnvOr` to supply a fallback
instead of raising, or `os::hasEnv` to test presence without reading the value.

`os::getEnv` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects."#;
const EX: &str = r#"Read a variable that is expected to be present:

```
IMPORT os
IMPORT io

SUB main()
  LET home AS String = os::getEnv("HOME")
  io::print(home)
END SUB
```

Treat an unset variable as a recoverable condition:

```
IMPORT os
IMPORT io

SUB main()
  LET token = os::getEnv("API_TOKEN") TRAP(err)
    RECOVER ""
  END TRAP
  io::print(token)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getEnv",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc:
                    "The variable name to read. Must be non-empty and free of embedded NUL bytes.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
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
