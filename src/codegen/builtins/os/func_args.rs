//! `os::args` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"The command-line arguments after the program name"#;
const DESC: &str = r#"`os::args` returns the program's command-line arguments as a `List OF String`,
**excluding** the program name — element 0 is the first real argument, not the
executable. (The program name is available through `os::executablePath`.) A
program invoked with no arguments returns an empty list.

The arguments are captured at program startup from the values the OS passes in,
so `os::args` reflects the invocation regardless of where in the program it is
called. Each element is an owned `String` copied from the corresponding `argv`
entry."#;
const EX: &str = r#"Print each argument on its own line:

```
IMPORT os
IMPORT io
IMPORT collections

SUB main()
  LET a AS List OF String = os::args()
  FOR i = 0 TO len(a) - 1
    io::print(collections::get(a, i))
  NEXT
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "args",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec![],
            body: Body::abi_function(crate::codegen::builtins::os::gen_os_seam::lower_os_os_seam),
        }],
    });
}
