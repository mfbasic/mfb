//! `fs::isBuffered` — descriptor + docs.
//!
//! Native syscall member: its `Body::abi_function` body delegates to the shared
//! family-generic OS-seam dispatcher `native::lower_fs_os_seam`.

use super::native::lower_fs_os_seam;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::isBuffered` — the shared OS-seam dispatcher
/// [`super::native::lower_fs_os_seam`], selected by runtime-call name (crypto/io's
/// clean-room shape); the `abi_function` wrapper finalizes it.
pub(crate) fn lower_fs_is_buffered(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_fs_os_seam(builder, ctx, "fs.isBuffered")
}

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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isBuffered",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
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
            body: Body::abi_function(lower_fs_is_buffered),
        }],
    });
}
