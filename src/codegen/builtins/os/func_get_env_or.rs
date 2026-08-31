//! `os::getEnvOr` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_get_env_or`]) — shares
//! [`super::gen_env::lower_get_env`] with `getEnv` (fallback flag `true`).

use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `os::getEnvOr(name, fallback)` — read an environment variable, returning `fallback`
/// when it is unset. Shares [`super::gen_env::lower_get_env`] with `getEnv`.
pub(crate) fn lower_get_env_or(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_env::lower_get_env(&symbol, ctx.platform_imports, ctx.platform, true)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result("os.getEnvOr"))
}

const INTRO: &str = r#"Read an environment variable, or a fallback when it is unset"#;
const DESC: &str = r#"`os::getEnvOr` returns the value of the environment variable named `name` when it
is set, and otherwise returns `fallback`. It never raises for a missing variable,
mirroring `collections::getOr(map, key, fallback)`. The lookup reflects the live
environment, including values written earlier by `os::setEnv`.

Both the found value and the fallback are returned as fresh `String`
values. Because absence yields `fallback` rather than a raised error, a variable
set to the empty string and an unset variable are indistinguishable through this
function; use `os::hasEnv` or `os::getEnv` when that distinction matters.

`os::getEnvOr` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects."#;
const EX: &str = r#"Read an optional variable with a default:

```
IMPORT os
IMPORT io

SUB main()
  LET level AS String = os::getEnvOr("LOG_LEVEL", "info")
  io::print(level)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getEnvOr",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "name",
                    desc: "The variable name to read. Must be non-empty and free of embedded NUL bytes.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "fallback",
                    desc: "The value returned when `name` is not set.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_get_env_or),
        }],
    });
}
