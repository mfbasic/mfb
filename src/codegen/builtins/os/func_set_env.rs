//! `os::setEnv` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_set_env`]).

use super::gen_env::{emit_env_lock, emit_env_unlock_return};
use super::gen_shared::{marshal_cstring, push_alloc_error, void_result, ERRNO_ENOMEM};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::setEnv(name, value)` — `setenv(name, value, 1)`, holding the env lock across
/// the call so a concurrent reader never observes a half-relocated `environ`
/// (bug-64). ENOMEM → `ErrOutOfMemory`; any other errno → `ErrInvalidArgument`.
pub(crate) fn lower_set_env(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let ok = format!("{symbol}_ok");
    let fail = format!("{symbol}_fail");
    let oom = format!("{symbol}_oom");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let name = vregs.next();
    let value = vregs.next();
    let cname = vregs.next();
    let cvalue = vregs.next();
    let errno = vregs.next();
    let mut instructions = vec![
        abi::move_register(&name, abi::c_arg(0)),
        abi::move_register(&value, abi::c_arg(1)),
    ];
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
    marshal_cstring(
        &symbol,
        &value,
        &cvalue,
        &alloc_error,
        &format!("{symbol}_value"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::move_register(abi::c_arg(0), &cname),
        abi::move_register(abi::c_arg(1), &cvalue),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
    ]);
    // Windows: SetEnvironmentVariableW(wideName, wideValue) via emit_env_set, which
    // marshals both and returns the POSIX convention (0 = success). plan-66-B.
    if ctx.platform.family() == PlatformFamily::Windows {
        ctx.platform.emit_env_set(
            &symbol,
            ctx.platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    } else {
        ctx.platform.emit_external_call(
            "setenv",
            &symbol,
            ctx.platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&fail),
        abi::label(&ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&fail),
    ]);
    // Distinguish ENOMEM (→ ErrOutOfMemory) from every other errno (→
    // ErrInvalidArgument: empty name, or a name containing '=').
    ctx.platform.emit_errno(
        &symbol,
        (&errno).into(),
        ctx.platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&errno, ERRNO_ENOMEM),
        abi::branch_eq(&oom),
    ]);
    raise_error_into(
        &symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&oom)]);
    push_alloc_error(&symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
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
    Ok(void_result("os.setEnv"))
}

const INTRO: &str = r#"Set or overwrite an environment variable"#;
const DESC: &str = r#"`os::setEnv` sets the environment variable named `name` to `value` in the live
process environment, overwriting any existing value. It is a SUB and returns
nothing. The change is visible to every later `os::getEnv`, `os::getEnvOr`,
`os::hasEnv`, and `os::environ` in the same process, and is inherited by child
processes spawned afterward. It maps to the host `setenv(name, value, 1)`.

`os::setEnv` mutates process-global state and is **not** synchronized against a
concurrent read in another `thread::` worker; avoid setting a variable while
another thread reads the environment. A `name` that is empty or contains `=` is
rejected with `ErrInvalidArgument`, since the host uses `=` to separate a name
from its value."#;
const EX: &str = r#"Set a variable and read it back:

```
IMPORT os
IMPORT io

SUB main()
  os::setEnv("GREETING", "hello")
  io::print(os::getEnv("GREETING"))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setEnv",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "name",
                    desc: "The variable name to set. Must be non-empty, free of embedded NUL bytes, and free of `=`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "value",
                    desc: "The value to store. Must be free of embedded NUL bytes.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_env),
        }],
    });
}
