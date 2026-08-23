//! `os::pid` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_pid`]) — `getpid()` as an `Integer`.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::pid` — `getpid()` as an `Integer` (a small positive value; the int return is
/// zero-extended by the W-register write, so no widening is needed). Windows has no
/// `getpid`; `GetCurrentProcessId` is the drop-in (plan-66-B).
pub(crate) fn lower_pid(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let getpid_fn = match ctx.platform.family() {
        PlatformFamily::Windows => "GetCurrentProcessId",
        _ => "getpid",
    };
    ctx.platform.emit_external_call(
        getpid_fn,
        &symbol,
        ctx.platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.instructions.extend([
        // plan-85: getpid's return is a C result (`rax`, `%retC`); read from the
        // C-return register (byte-identical `x0` on AArch64/RISC-V).
        abi::move_register(RESULT_VALUE_REGISTER, abi::c_return(0)),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(super::gen_shared::void_result("os.pid"))
}

const INTRO: &str = r#"The current process id"#;
const DESC: &str = r#"`os::pid` returns the process id of the running program as an `Integer`, via the
host `getpid` call. The value is positive and stable for the life of the process.

`os::pid` is **not pure** in the sense that different processes see different
values, but within one process every call returns the same id. It reads process
state only and has no side effects."#;
const EX: &str = r#"Print the process id:

```
IMPORT os
IMPORT io

SUB main()
  io::print(toString(os::pid()))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pid",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_pid),
        }],
    });
}
