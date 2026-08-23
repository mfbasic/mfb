//! `os::userName` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_user_name`]).

use super::gen_env::{emit_env_lock, emit_env_unlock_return};
use super::gen_introspect::lower_os_wide_string_windows;
use super::gen_shared::{build_string_from_cstr, push_alloc_error, void_result};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::userName` — `getpwuid(getuid())->pw_name` (`pw_name` is the first field of
/// `struct passwd` on every supported libc). Raises `ErrUnsupported` if the uid has
/// no passwd entry (e.g. a bare container uid). Windows uses the shared UTF-16
/// wide-string path. The env lock doubles as the pwd lock across the static-buffer
/// copy (bug-64).
pub(crate) fn lower_user_name(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if ctx.platform.family() == PlatformFamily::Windows {
        // Windows has no getpwuid; GetUserNameW is self-contained (writes a caller
        // buffer, no shared static), so it needs no env/pwd lock (plan-66-B).
        let (instructions, relocations, stack_size) =
            lower_os_wide_string_windows(&symbol, "userName", ctx.platform_imports, ctx.platform)?;
        builder.instructions.extend(instructions);
        builder.relocations.extend(relocations);
        builder.stack_size = stack_size;
        return Ok(void_result("os.userName"));
    }
    let have_pwd = format!("{symbol}_have_pwd");
    let have_name = format!("{symbol}_have_name");
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let pwname = vregs.next();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    // Hold the lock across `getpwuid` and the copy of its static `passwd`/`pw_name`
    // buffer, so a concurrent `getpwuid`/`getpwnam` cannot overwrite it mid-copy.
    emit_env_lock(&mut EmitCtx {
        symbol: symbol.as_str(),
        platform_imports: ctx.platform_imports,
        platform: ctx.platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    ctx.platform.emit_external_call(
        "getuid",
        &symbol,
        ctx.platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    ctx.platform.emit_external_call(
        "getpwuid",
        &symbol,
        ctx.platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&have_pwd),
        abi::branch(&fail),
        abi::label(&have_pwd),
        abi::load_u64(&pwname, abi::return_register(), 0), // pw_name @ offset 0
        abi::compare_immediate(&pwname, "0"),
        abi::branch_ne(&have_name),
        abi::branch(&fail),
        abi::label(&have_name),
    ]);
    build_string_from_cstr(
        &symbol,
        &pwname,
        &alloc_error,
        &format!("{symbol}_str"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    raise_error_into(
        &symbol,
        "ErrUnsupported",
        &mut instructions,
        &mut relocations,
    );
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
    Ok(void_result("os.userName"))
}

const INTRO: &str = r#"The effective user's login name"#;
const DESC: &str = r#"`os::userName` returns the login name of the effective user, resolved through
`getpwuid(getuid())` and copied into an owned `String`. Using the passwd database
rather than the controlling terminal means it works without a login session (for
example under a service manager).

If the effective uid has no passwd entry (as on a bare container uid),
`os::userName` raises `ErrUnsupported`. It reads host state only and has no side
effects."#;
const EX: &str = r#"Print the user name:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::userName())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "userName",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_user_name),
        }],
    });
}
