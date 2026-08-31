//! `os::getEnv` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_get_env`]). The `getenv` + marshal body is
//! shared with `getEnvOr` (the two differ only by the fallback flag), so it lives in
//! [`super::gen_env`] and this per-member body calls it with `with_fallback = false`.

use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `os::getEnv(name)` — read an environment variable, raising `ErrNotFound` when it
/// is unset. Shares [`super::gen_env::lower_get_env`] with `getEnvOr`.
pub(crate) fn lower_get_env(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_env::lower_get_env(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result("os.getEnv"))
}

const INTRO: &str = r#"Read an environment variable, raising when it is unset"#;
const DESC: &str = r#"`os::getEnv` returns the value of the environment variable named `name` as it
appears in the live process environment, including any value written earlier by
`os::setEnv`. The lookup is the host `getenv` call; the returned bytes are copied
into a fresh `String`.

If the variable is not set, `os::getEnv` raises `ErrNotFound` rather than
returning an empty string, so a program can distinguish an unset variable from
one deliberately set to the empty string. Use `os::getEnvOr` to supply a fallback
instead of raising, or `os::hasEnv` to test presence without reading the value.

`os::getEnv` is **not pure**: its result depends on host and prior-`setEnv`
state. It reads process state only and has no side effects."#;
const EX: &str = r#"Read a variable that is expected to be present:

```
IMPORT os
IMPORT io

SUB main()
  LET home AS String = os::getEnv("HOME")
  io::print(home)
END SUB
```

Treat an unset variable as a recoverable condition:

```
IMPORT os
IMPORT io

SUB main()
  LET token = os::getEnv("API_TOKEN") TRAP(err)
    RECOVER ""
  END TRAP
  io::print(token)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getEnv",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc:
                    "The variable name to read. Must be non-empty and free of embedded NUL bytes.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_get_env),
        }],
    });
}
