//! `os::setEnv` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Set or overwrite an environment variable"#;
const DESC: &str = r#"`os::setEnv` sets the environment variable named `name` to `value` in the live
process environment, overwriting any existing value. It is a SUB and returns
nothing. The change is visible to every later `os::getEnv`, `os::getEnvOr`,
`os::hasEnv`, and `os::environ` in the same process, and is inherited by child
processes spawned afterward. It maps to the host `setenv(name, value, 1)`.

`os::setEnv` mutates process-global state and is **not** synchronized against a
concurrent read in another `thread::` worker; avoid setting a variable while
another thread reads the environment. A `name` that is empty or contains `=` is
rejected with `ErrInvalidArgument`, since the host uses `=` to separate a name
from its value."#;
const EX: &str = r#"Set a variable and read it back:

```
IMPORT os
IMPORT io

SUB main()
  os::setEnv("GREETING", "hello")
  io::print(os::getEnv("GREETING"))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setEnv",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "name",
                    desc: "The variable name to set. Must be non-empty, free of embedded NUL bytes, and free of `=`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "value",
                    desc: "The value to store. Must be free of embedded NUL bytes.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(crate::codegen::builtins::os::gen_os_seam::lower_os_os_seam),
        }],
    });
}
