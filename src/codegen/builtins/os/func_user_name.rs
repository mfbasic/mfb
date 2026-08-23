//! `os::userName` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"The effective user's login name"#;
const DESC: &str = r#"`os::userName` returns the login name of the effective user, resolved through
`getpwuid(getuid())` and copied into an owned `String`. Using the passwd database
rather than the controlling terminal means it works without a login session (for
example under a service manager).

If the effective uid has no passwd entry (as on a bare container uid),
`os::userName` raises `ErrUnsupported`. It reads host state only and has no side
effects."#;
const EX: &str = r#"Print the user name:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::userName())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "userName",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(crate::codegen::builtins::os::gen_os_seam::lower_os_os_seam),
        }],
    });
}
