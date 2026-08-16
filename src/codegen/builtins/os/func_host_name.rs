//! `os::hostName` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). Docs migrated from
//! `src/docs/man/builtins/os/hostName.md`.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"The host's network name"#;
const DESC: &str = r#"`os::hostName` returns the host's network name via the host `gethostname` call,
copied into an owned `String`. The name is whatever the host is configured to
report (often the short hostname).

If the host cannot supply the name, `os::hostName` raises `ErrUnsupported`. It
reads host state only and has no side effects."#;
const EX: &str = r#"Print the host name:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::hostName())
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hostName",
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
