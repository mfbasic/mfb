//! `os::isAdmin` — descriptor entry + native lowering.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

pub(crate) fn lower_is_admin(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let is_true = format!("{symbol}_true");
    let done = format!("{symbol}_done");
    match ctx.platform.family() {
        PlatformFamily::Windows => {
            ctx.platform.emit_external_call(
                "IsUserAnAdmin",
                &symbol,
                ctx.platform_imports,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_ne(&is_true),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::branch(&done),
            ]);
        }
        _ => {
            ctx.platform.emit_external_call(
                "geteuid",
                &symbol,
                ctx.platform_imports,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_eq(&is_true),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::branch(&done),
            ]);
        }
    }
    builder.instructions.extend([
        abi::label(&is_true),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::label(&done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(super::gen_shared::void_result("os.isAdmin"))
}

const INTRO: &str = r#"Whether the current process has administrator privileges"#;
const DESC: &str = r#"`os::isAdmin` returns `TRUE` when the process is running with administrator
privileges: effective uid 0 on POSIX hosts, or the Windows shell administrator
check on Windows. It reads process identity only and has no side effects."#;
const EX: &str = r#"Print whether the process is elevated:

```
IMPORT os
IMPORT io

SUB main()
  io::print(toString(os::isAdmin()))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isAdmin",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_is_admin),
        }],
    });
}
