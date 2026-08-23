//! `os::arch` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md).

use crate::codegen::engine::builder::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `os::arch` — the CPU architecture string (`"aarch64"`/`"x86_64"`/`"riscv64"`)
/// selected by the build target, materialized as a fresh owned `String` via the
/// shared [`super::gen_introspect::lower_const_string`] (shared with `os::name`).
pub(crate) fn lower_arch(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_introspect::lower_const_string(
        &symbol,
        super::gen_shared::os_arch(ctx.platform.target()),
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result("os.arch"))
}

const INTRO: &str = r#"The CPU architecture the program was built for"#;
const DESC: &str = r#"`os::arch` returns the CPU architecture of the build target: `"aarch64"`,
`"x86_64"`, or `"riscv64"`. Like `os::name`, it is a compile-time constant fixed
at build time and materialized directly into an owned `String`, with no host
call."#;
const EX: &str = r#"Print the architecture:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::arch())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "arch",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_arch),
        }],
    });
}
