//! `os::hostName` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_host_name`]).

use super::gen_introspect::lower_os_wide_string_windows;
use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::hostName` — `gethostname(buf, 256)` into an on-frame buffer, then a `String`
/// copy. `HOST_NAME_MAX` is 64 (Linux) / 255 (macOS), so 256 always holds a
/// NUL-terminated name. Windows uses the shared UTF-16 wide-string path.
pub(crate) fn lower_host_name(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if ctx.platform.family() == PlatformFamily::Windows {
        let (instructions, relocations, stack_size) =
            lower_os_wide_string_windows(&symbol, "hostName", ctx.platform_imports, ctx.platform)?;
        builder.instructions.extend(instructions);
        builder.relocations.extend(relocations);
        builder.stack_size = stack_size;
        return Ok(void_result("os.hostName"));
    }
    const BUF: usize = 256;
    let ok = format!("{symbol}_ok");
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let buf = vregs.next();
    let mut instructions = vec![
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), 0),
        abi::move_immediate(abi::c_arg(1), "Integer", &BUF.to_string()),
    ];
    let mut relocations = Vec::new();
    ctx.platform.emit_external_call(
        "gethostname",
        &symbol,
        ctx.platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // plan-85: gethostname's return is a C result (`rax`, `%retC`).
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&ok),
        abi::branch(&fail),
        abi::label(&ok),
        // Defensive NUL at the last byte, then build the String from the buffer.
        abi::add_immediate(&buf, abi::stack_pointer(), 0),
        abi::store_u8(abi::ZERO, &buf, BUF - 1),
    ]);
    build_string_from_cstr(
        &symbol,
        &buf,
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
    instructions.extend([abi::label(&done), abi::return_()]);
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = BUF;
    Ok(void_result("os.hostName"))
}

const INTRO: &str = r#"The host's network name"#;
const DESC: &str = r#"`os::hostName` returns the host's network name via the host `gethostname` call,
copied into an owned `String`. The name is whatever the host is configured to
report (often the short hostname).

If the host cannot supply the name, `os::hostName` raises `ErrUnsupported`. It
reads host state only and has no side effects."#;
const EX: &str = r#"Print the host name:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::hostName())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "hostName",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_host_name),
        }],
    });
}
