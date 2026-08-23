//! `os::executablePath` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_executable_path`]).

use super::gen_introspect::lower_os_wide_string_windows;
use super::gen_paths::emit_executable_path_into;
use super::gen_shared::{
    build_string_from_cstr, build_string_from_len, push_alloc_error, void_result,
    EXE_PATH_FRAME_LOCALS,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::executablePath` — the absolute path of the running binary. Acquires the path
/// via the shared [`emit_executable_path_into`] (plan-55-B §4.1) and builds an owned
/// `String`: NUL-terminated on macOS, byte-counted on Linux; Windows uses the shared
/// UTF-16 wide-string path.
pub(crate) fn lower_executable_path(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if ctx.platform.family() == PlatformFamily::Windows {
        // GetModuleFileNameW(NULL, …) + UTF-16→UTF-8 marshal (plan-66-B).
        let (instructions, relocations, stack_size) = lower_os_wide_string_windows(
            &symbol,
            "executablePath",
            ctx.platform_imports,
            ctx.platform,
        )?;
        builder.instructions.extend(instructions);
        builder.relocations.extend(relocations);
        builder.stack_size = stack_size;
        return Ok(void_result("os.executablePath"));
    }
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let (buf, count) = emit_executable_path_into(
        &mut EmitCtx {
            symbol: symbol.as_str(),
            platform_imports: ctx.platform_imports,
            platform: ctx.platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &fail,
        &mut vregs,
    )?;
    match count {
        // Linux: `readlink` reported the byte count; the buffer has no NUL.
        Some(count) => build_string_from_len(
            &symbol,
            &buf,
            &count,
            &alloc_error,
            &format!("{symbol}_str"),
            &mut vregs,
            &mut instructions,
            &mut relocations,
        ),
        // macOS: the buffer is NUL-terminated.
        None => build_string_from_cstr(
            &symbol,
            &buf,
            &alloc_error,
            &format!("{symbol}_str"),
            &mut vregs,
            &mut instructions,
            &mut relocations,
        ),
    }
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    raise_error_into(
        &symbol,
        "ErrUnsupported",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(&symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = EXE_PATH_FRAME_LOCALS;
    Ok(void_result("os.executablePath"))
}

const INTRO: &str = r#"The path to the running executable"#;
const DESC: &str = r#"`os::executablePath` returns the filesystem path of the running binary as an
owned `String`. On macOS it uses `_NSGetExecutablePath`; on Linux it reads the
`/proc/self/exe` symlink with `readlink`, which yields the absolute, symlink-
resolved path.

Use it to locate resources beside the executable, or to report the program's own
path. If the host cannot determine the path, `os::executablePath` raises
`ErrUnsupported`. It reads host state only and has no side effects."#;
const EX: &str = r#"Print the executable path:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::executablePath())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "executablePath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_executable_path),
        }],
    });
}
