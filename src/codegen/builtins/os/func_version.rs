//! `os::version` — descriptor entry + native lowering.

use super::gen_shared::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

fn store_cstr(ins: &mut Vec<CodeInstruction>, offset: usize, text: &str) {
    for (index, byte) in text
        .as_bytes()
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .enumerate()
    {
        ins.extend([
            abi::move_immediate(abi::c_arg(0), "Byte", &byte.to_string()),
            abi::store_u8(abi::c_arg(0), abi::stack_pointer(), offset + index),
        ]);
    }
}

pub(crate) fn lower_version(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let value = vregs.next();
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();

    match ctx.platform.family() {
        PlatformFamily::Linux => {
            // Linux struct utsname has 65-byte fields; release is the third field.
            const UTS: usize = 0;
            const RELEASE: usize = 130;
            instructions.push(abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), UTS));
            ctx.platform.emit_external_call(
                "uname",
                &symbol,
                ctx.platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_ne(&fail),
                abi::add_immediate(&value, abi::stack_pointer(), RELEASE),
            ]);
            builder.stack_size = 390;
        }
        PlatformFamily::MacOS => {
            // kern.osproductversion is the user-facing macOS version, e.g. 14.5.
            const NAME: usize = 0;
            const BUF: usize = 32;
            const LEN: usize = 160;
            store_cstr(&mut instructions, NAME, "kern.osproductversion");
            instructions.extend([
                abi::move_immediate(&value, "Integer", "128"),
                abi::store_u64(&value, abi::stack_pointer(), LEN),
                abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), NAME),
                abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), BUF),
                abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), LEN),
                abi::move_immediate(abi::c_arg(3), "Integer", "0"),
                abi::move_immediate(abi::c_arg(4), "Integer", "0"),
            ]);
            ctx.platform.emit_external_call(
                "sysctlbyname",
                &symbol,
                ctx.platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_ne(&fail),
                abi::add_immediate(&value, abi::stack_pointer(), BUF),
            ]);
            builder.stack_size = 168;
        }
        PlatformFamily::Windows => {
            // RtlGetVersion fills RTL_OSVERSIONINFOW; dwBuildNumber is at offset 12.
            const INFO: usize = 0x40;
            const BUF: usize = 0;
            let n = vregs.next();
            let digit = vregs.next();
            let write = vregs.next();
            let div_loop = |ins: &mut Vec<CodeInstruction>,
                            n: &str,
                            digit: &str,
                            divisor: usize,
                            suffix: &str| {
                let loop_l = format!("{symbol}_version_div_{suffix}");
                let done_l = format!("{symbol}_version_div_done_{suffix}");
                ins.extend([
                    abi::move_immediate(digit, "Integer", "48"),
                    abi::label(&loop_l),
                    abi::compare_immediate(n, &divisor.to_string()),
                    abi::branch_lt(&done_l),
                    abi::subtract_immediate(n, n, divisor),
                    abi::add_immediate(digit, digit, 1),
                    abi::branch(&loop_l),
                    abi::label(&done_l),
                ]);
            };
            instructions.extend([
                abi::move_immediate(&n, "Integer", "284"),
                abi::store_u32(&n, abi::stack_pointer(), INFO),
                abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), INFO),
            ]);
            ctx.platform.emit_external_call(
                "RtlGetVersion",
                &symbol,
                ctx.platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_ne(&fail),
                abi::load_u32(&n, abi::stack_pointer(), INFO + 12),
                abi::add_immediate(&write, abi::stack_pointer(), BUF),
            ]);
            for (divisor, suffix) in [(10000, "10000"), (1000, "1000"), (100, "100"), (10, "10")] {
                div_loop(&mut instructions, &n, &digit, divisor, suffix);
                instructions.extend([
                    abi::store_u8(&digit, &write, 0),
                    abi::add_immediate(&write, &write, 1),
                ]);
            }
            instructions.extend([
                abi::add_immediate(&digit, &n, 48),
                abi::store_u8(&digit, &write, 0),
                abi::add_immediate(&write, &write, 1),
                abi::store_u8(abi::ZERO, &write, 0),
                abi::add_immediate(&value, abi::stack_pointer(), BUF),
            ]);
            builder.stack_size = 0x160;
        }
    }

    build_string_from_cstr(
        &symbol,
        &value,
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
    Ok(void_result("os.version"))
}

const INTRO: &str = r#"The operating-system version string"#;
const DESC: &str = r#"`os::version` returns the host operating-system version: the kernel release on
Linux, the user-facing product version on macOS, and the Windows build number on
Windows."#;
const EX: &str = r#"Print the OS version:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::version())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "version",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec!["ErrUnsupported"],
            body: Body::abi_function(lower_version),
        }],
    });
}
