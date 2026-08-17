//! `os::unsetEnv` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Remove an environment variable"#;
const DESC: &str = r#"`os::unsetEnv` removes the environment variable named `name` from the live
process environment. It is a SUB and returns nothing. Removing a variable that is
not set is a no-op, not an error, so the call is idempotent. After it returns,
`os::hasEnv(name)` reports `FALSE` and `os::getEnv(name)` raises `ErrNotFound`.
It maps to the host `unsetenv(name)`.

`os::unsetEnv` mutates process-global state and is **not** synchronized against a
concurrent read in another `thread::` worker."#;
const EX: &str = r#"Remove a variable and confirm it is gone:

```
IMPORT os
IMPORT io

SUB main()
  os::setEnv("TEMP_FLAG", "1")
  os::unsetEnv("TEMP_FLAG")
  io::print(toString(os::hasEnv("TEMP_FLAG")))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "unsetEnv",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc: "The variable name to remove. Must be non-empty and free of embedded NUL bytes.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(
                Some(super::lower_os_helper),
                Some(super::lower_os_helper),
                None,
            ),
        }],
    });
}
