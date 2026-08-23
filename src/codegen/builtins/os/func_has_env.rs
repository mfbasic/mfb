//! `os::hasEnv` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_has_env`]).

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

/// `os::hasEnv(name)` — whether an environment variable is set, as a `Boolean`. The
/// `getenv` probe is serialized against a concurrent `os::setEnv` relocating
/// `environ` (bug-64), holding the env lock across the probe.
pub(crate) fn lower_has_env(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let present = format!("{symbol}_present");
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
    // Windows: GetEnvironmentVariableW (via emit_env_get) leaves a non-zero UTF-8
    // value pointer when the variable exists, 0 when unset — the same
    // nonzero-means-present test as POSIX getenv (plan-66-B).
    if ctx.platform.family() == PlatformFamily::Windows {
        ctx.platform.emit_env_get(
            &symbol,
            ctx.platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    } else {
        ctx.platform.emit_external_call(
            "getenv",
            &symbol,
            ctx.platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        // plan-85: getenv's char* return is a C result (`rax`, `%retC`).
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_ne(&present),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&present),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
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
    Ok(void_result("os.hasEnv"))
}

const INTRO: &str = r#"Test whether an environment variable is set"#;
const DESC: &str = r#"`os::hasEnv` returns `TRUE` when the environment variable named `name` is
present in the live process environment and `FALSE` otherwise. It is the host
`getenv` call reduced to a non-NULL test, so it reflects both inherited variables
and any set earlier by `os::setEnv`. A variable set to the empty string still
counts as present.

`os::hasEnv` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects, and never raises."#;
const EX: &str = r#"Branch on the presence of a variable:

```
IMPORT os
IMPORT io

SUB main()
  IF os::hasEnv("CI") THEN
    io::print("running in CI")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hasEnv",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc:
                    "The variable name to test. Must be non-empty and free of embedded NUL bytes.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_has_env),
        }],
    });
}
