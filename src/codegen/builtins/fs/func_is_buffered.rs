//! `fs::isBuffered` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper`. Docs migrated from
//! `src/docs/man/builtins/fs/isBuffered.md`.

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Report whether opt-in output buffering is enabled for an open `File`"#;
const DESC: &str = r#"`fs::isBuffered` reads the per-handle buffering flag on a single open `File` and
returns `TRUE` when output buffering is currently enabled for that handle and
`FALSE` otherwise. It only inspects the handle's state — it writes no data, drains
nothing, and has no side effect.

Buffering is a per-handle flag stored on the `File` resource itself, so this call
reflects only `file` and no other open handle; each `File` carries its own buffer
and its own enabled flag.

Buffering is **off by default**: a freshly opened `File` starts with its buffered
flag clear, so a program that never calls `fs::setBuffered` always observes
`FALSE` here. The flag becomes `TRUE` after `fs::setBuffered(file, TRUE)` and
returns to `FALSE` after `fs::setBuffered(file, FALSE)`. Transferring a buffered
handle to another thread resets it to unbuffered, so the receiving thread again
observes `FALSE`."#;
const EX: &str = r#"Enable buffering only when it is not already on:

```
IMPORT fs

SUB main()
  RES log = fs::openFile("events.log", "write")
  IF NOT fs::isBuffered(log) THEN
    fs::setBuffered(log, TRUE)
  END IF
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isBuffered",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "An open `File` resource whose buffering flag is being queried.",
                aliases: &[],
                ty: ParameterType::Named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
