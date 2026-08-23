//! `os::unsetEnv` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_unset_env`]).

use super::gen_env::{emit_env_lock, emit_env_unlock_return};
use super::gen_shared::{marshal_cstring, push_alloc_error, void_result};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::unsetEnv(name)` — `unsetenv(name)`, holding the env lock across the call so a
/// concurrent reader never observes a half-relocated `environ` (bug-64). A no-op for
/// an absent variable; any return is treated as success.
pub(crate) fn lower_unset_env(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let name = vregs.next();
    let cname = vregs.next();
    let mut instructions = vec![abi::move_register(&name, abi::c_arg(0))];
    let mut relocations = Vec::new();
    emit_env_lock(&mut EmitCtx {
        symbol: symbol.as_str(),
        platform_imports: ctx.platform_imports,
        platform: ctx.platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    marshal_cstring(
        &symbol,
        &name,
        &cname,
        &alloc_error,
        &format!("{symbol}_name"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::move_register(abi::c_arg(0), &cname));
    // Windows: SetEnvironmentVariableW(name, NULL) deletes the variable; a NULL value
    // pointer in ARG[1] selects the delete path in emit_env_set (plan-66-B).
    if ctx.platform.family() == PlatformFamily::Windows {
        instructions.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
        ctx.platform.emit_env_set(
            &symbol,
            ctx.platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    } else {
        ctx.platform.emit_external_call(
            "unsetenv",
            &symbol,
            ctx.platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    // `unsetenv` is a no-op for an absent variable; treat any return as success.
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(&symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol: symbol.as_str(),
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = 0;
    Ok(void_result("os.unsetEnv"))
}

const INTRO: &str = r#"Remove an environment variable"#;
const DESC: &str = r#"`os::unsetEnv` removes the environment variable named `name` from the live
process environment. It is a SUB and returns nothing. Removing a variable that is
not set is a no-op, not an error, so the call is idempotent. After it returns,
`os::hasEnv(name)` reports `FALSE` and `os::getEnv(name)` raises `ErrNotFound`.
It maps to the host `unsetenv(name)`.

`os::unsetEnv` mutates process-global state and is **not** synchronized against a
concurrent read in another `thread::` worker."#;
const EX: &str = r#"Remove a variable and confirm it is gone:

```
IMPORT os
IMPORT io

SUB main()
  os::setEnv("TEMP_FLAG", "1")
  os::unsetEnv("TEMP_FLAG")
  io::print(toString(os::hasEnv("TEMP_FLAG")))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "unsetEnv",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc: "The variable name to remove. Must be non-empty and free of embedded NUL bytes.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_unset_env),
        }],
    });
}
