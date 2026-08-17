//! `os::environ` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Snapshot every environment variable as a map"#;
const DESC: &str = r#"`os::environ` returns a `Map OF String TO String` holding every variable in the
live process environment, keyed by name. It walks the host environment array,
splitting each `NAME=VALUE` entry at its **first** `=`: the text before it is the
key and everything after it — including any further `=` — is the value. An entry
with no `=` maps its whole text to an empty-string value. The snapshot reflects
variables written earlier by `os::setEnv` and omits those removed by
`os::unsetEnv`.

The returned map is an ordinary owned value captured at the moment of the call;
later `os::setEnv`/`os::unsetEnv` calls do not change it, so re-read the
environment with a fresh `os::environ()` to observe subsequent mutations. The map
is unordered, like any `Map`. On the rare host that lists a name twice, the map
retains one entry for that key.

`os::environ` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects."#;
const EX: &str = r#"Read a value out of the environment snapshot:

```
IMPORT os
IMPORT io
IMPORT collections

SUB main()
  LET env AS Map OF String TO String = os::environ()
  io::print(collections::getOr(env, "PATH", ""))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "environ",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::map_of(ParameterType::String, ParameterType::String),
            errors: vec![],
            body: Body::native(
                Some(super::lower_os_helper),
                Some(super::lower_os_helper),
                None,
            ),
        }],
    });
}
