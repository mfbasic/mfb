//! `os::hasEnv` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). Docs migrated from
//! `src/docs/man/builtins/os/hasEnv.md`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether an environment variable is set"#;
const DESC: &str = r#"`os::hasEnv` returns `TRUE` when the environment variable named `name` is
present in the live process environment and `FALSE` otherwise. It is the host
`getenv` call reduced to a non-NULL test, so it reflects both inherited variables
and any set earlier by `os::setEnv`. A variable set to the empty string still
counts as present.

`os::hasEnv` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects, and never raises."#;
const EX: &str = r#"Branch on the presence of a variable:

```
IMPORT os
IMPORT io

SUB main()
  IF os::hasEnv("CI") THEN
    io::print("running in CI")
  END IF
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hasEnv",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc:
                    "The variable name to test. Must be non-empty and free of embedded NUL bytes.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::native(
                Some(super::lower_os_helper),
                Some(super::lower_os_helper),
                None,
            ),
        }],
    });
}
