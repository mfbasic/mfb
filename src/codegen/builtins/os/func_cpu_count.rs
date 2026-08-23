//! `os::cpuCount` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_cpu_count`]).

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `os::cpuCount` — `sysconf(_SC_NPROCESSORS_ONLN)` as an `Integer`, clamped to at
/// least 1. `_SC_NPROCESSORS_ONLN` is 58 on Darwin and 84 on Linux; Windows has no
/// `sysconf` so it reads `GetSystemInfo(&si).dwNumberOfProcessors` (plan-66-B).
pub(crate) fn lower_cpu_count(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let stack_size;
    let positive = format!("{symbol}_positive");
    let count = {
        let mut vregs = Vregs::new();
        vregs.next()
    };
    if ctx.platform.family() == PlatformFamily::Windows {
        // Windows has no sysconf; GetSystemInfo(&si) fills a SYSTEM_INFO whose
        // `dwNumberOfProcessors` (DWORD) sits at offset 0x20. It is always >= 1, but
        // keep the shared clamp for uniformity. plan-66-B.
        const SYSTEM_INFO_SIZE: usize = 48;
        const DW_NUMBER_OF_PROCESSORS_OFFSET: usize = 0x20;
        instructions.push(abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), 0)); // &SYSTEM_INFO
        ctx.platform.emit_external_call(
            "GetSystemInfo",
            &symbol,
            ctx.platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u32(&count, abi::stack_pointer(), DW_NUMBER_OF_PROCESSORS_OFFSET),
            abi::compare_immediate(&count, "1"),
            abi::branch_ge(&positive),
            abi::move_immediate(&count, "Integer", "1"),
            abi::label(&positive),
            abi::move_register(RESULT_VALUE_REGISTER, &count),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::return_(),
        ]);
        stack_size = SYSTEM_INFO_SIZE;
    } else {
        let sc_nprocessors_onln = match ctx.platform.family() {
            PlatformFamily::MacOS => "58",
            PlatformFamily::Linux => "84",
            PlatformFamily::Windows => {
                unreachable!("plan-66-B routes Windows cpuCount to GetSystemInfo")
            }
        };
        instructions.push(abi::move_immediate(
            abi::c_arg(0),
            "Integer",
            sc_nprocessors_onln,
        ));
        ctx.platform.emit_external_call(
            "sysconf",
            &symbol,
            ctx.platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // plan-85: sysconf's return is a C result (`rax`, `%retC`); read from the
            // C-return register (byte-identical `x0` on AArch64/RISC-V).
            abi::move_register(&count, abi::c_return(0)),
            // sysconf returns -1 (or 0) on failure or an indeterminate answer: clamp
            // to a minimum of 1 so callers always get a usable count.
            abi::compare_immediate(&count, "1"),
            abi::branch_ge(&positive),
            abi::move_immediate(&count, "Integer", "1"),
            abi::label(&positive),
            abi::move_register(RESULT_VALUE_REGISTER, &count),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::return_(),
        ]);
        stack_size = 0;
    }
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(ValueResult {
        origin: None,
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: "os.cpuCount".to_string(),
    })
}

const INTRO: &str = r#"The number of online logical CPUs"#;
const DESC: &str = r#"`os::cpuCount` returns the number of online logical CPUs as reported by the host
`sysconf(_SC_NPROCESSORS_ONLN)`. The result is clamped to a minimum of 1, so a
caller always gets a usable count even if the host cannot determine the true
value.

Use it to size a `thread::` worker pool. The value reflects CPUs online at the
moment of the call and may in principle change over a long-running process on a
host that hot-plugs CPUs."#;
const EX: &str = r#"Print the CPU count:

```
IMPORT os
IMPORT io

SUB main()
  io::print(toString(os::cpuCount()))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "cpuCount",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_cpu_count),
        }],
    });
}
