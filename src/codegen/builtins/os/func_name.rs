//! `os::name` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `os::name` — the OS family string (`"macos"`/`"linux"`/`"windows"`) selected by the
/// build target, materialized as a fresh owned `String` via the shared
/// [`super::gen_introspect::lower_const_string`] (shared with `os::arch`).
pub(crate) fn lower_name(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_introspect::lower_const_string(
        &symbol,
        super::gen_shared::os_family(ctx.platform.family()),
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result("os.name"))
}

const INTRO: &str = r#"The operating-system family the program was built for"#;
const DESC: &str = r#"`os::name` returns the operating-system family of the build target: `"macos"` or
`"linux"`. It is a compile-time constant — the binary is built for exactly one
target, so the value is fixed at build time and materialized directly into an
`String`, with no host call.

Pair it with `os::arch` to identify the full platform. Because the value is
fixed per build, it is stable across runs of the same binary."#;
const EX: &str = r#"Print the platform:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::name() & "/" & os::arch())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "name",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_name),
        }],
    });
}
