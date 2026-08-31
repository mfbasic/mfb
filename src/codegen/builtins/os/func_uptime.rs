//! `os::uptime` — descriptor entry + native lowering.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

pub(crate) fn lower_uptime(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let fail = format!("{symbol}_fail");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let seconds = vregs.next();
    match ctx.platform.family() {
        PlatformFamily::Linux => {
            // sysinfo(struct sysinfo *info) fills the caller's buffer; `struct
            // sysinfo` starts with `long uptime` at offset 0, which is what the
            // load below reads back. The buffer is this frame (128 bytes reserved
            // for it), so its address MUST be staged into ARG[0] first — like the
            // macOS arm does for every `sysctl` argument.
            //
            // Without it, `sysinfo` was called with whatever the caller happened
            // to leave in ARG[0], and both outcomes were wrong: an unmapped value
            // returned -1/EFAULT, so the non-zero check raised ErrUnsupported and
            // `os::uptime` looked unimplemented on Linux; a *writable* one let
            // sysinfo scribble a 112-byte struct over an arbitrary address and
            // return 0, after which the load read a frame slot nothing had written
            // and reported a garbage uptime.
            //
            // `add_immediate` from the stack pointer is the right way to form it:
            // `finalize_frame` shifts sp-relative accesses up past the callee-saved
            // area, and it shifts this instruction with the load, so the pointer
            // passed and the slot read stay the same address.
            builder
                .instructions
                .push(abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), 0));
            ctx.platform.emit_external_call(
                "sysinfo",
                &symbol,
                ctx.platform_imports,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_ne(&fail),
                abi::load_u64(&seconds, abi::stack_pointer(), 0),
            ]);
            builder.stack_size = 128;
        }
        PlatformFamily::MacOS => {
            // sysctl({CTL_KERN,KERN_BOOTTIME}, &timeval, &len, NULL, 0), then
            // time(NULL) - boottime.tv_sec.
            const MIB: usize = 0;
            const TIMEVAL: usize = 16;
            const LEN: usize = 32;
            builder.instructions.extend([
                abi::move_immediate(&seconds, "Integer", "1"),
                abi::store_u32(&seconds, abi::stack_pointer(), MIB),
                abi::move_immediate(&seconds, "Integer", "21"),
                abi::store_u32(&seconds, abi::stack_pointer(), MIB + 4),
                abi::move_immediate(&seconds, "Integer", "16"),
                abi::store_u64(&seconds, abi::stack_pointer(), LEN),
                abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), MIB),
                abi::move_immediate(abi::c_arg(1), "Integer", "2"),
                abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), TIMEVAL),
                abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), LEN),
                abi::move_immediate(abi::c_arg(4), "Integer", "0"),
                abi::move_immediate(abi::c_arg(5), "Integer", "0"),
            ]);
            ctx.platform.emit_external_call(
                "sysctl",
                &symbol,
                ctx.platform_imports,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.extend([
                abi::compare_immediate(abi::c_return(0), "0"),
                abi::branch_ne(&fail),
                abi::move_immediate(abi::c_arg(0), "Integer", "0"),
            ]);
            ctx.platform.emit_external_call(
                "time",
                &symbol,
                ctx.platform_imports,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            builder.instructions.extend([
                abi::load_u64(&seconds, abi::stack_pointer(), TIMEVAL),
                abi::subtract_registers(&seconds, abi::c_return(0), &seconds),
            ]);
            builder.stack_size = 48;
        }
        PlatformFamily::Windows => {
            ctx.platform.emit_external_call(
                "GetTickCount64",
                &symbol,
                ctx.platform_imports,
                &mut builder.instructions,
                &mut builder.relocations,
            )?;
            // Milliseconds / 1000 without a division op: subtract 1000 in a loop.
            let loop_l = format!("{symbol}_uptime_div_loop");
            let done = format!("{symbol}_uptime_div_done");
            let ms = vregs.next();
            builder.instructions.extend([
                abi::move_register(&ms, abi::c_return(0)),
                abi::move_immediate(&seconds, "Integer", "0"),
                abi::label(&loop_l),
                abi::compare_immediate(&ms, "1000"),
                abi::branch_lt(&done),
                abi::subtract_immediate(&ms, &ms, 1000),
                abi::add_immediate(&seconds, &seconds, 1),
                abi::branch(&loop_l),
                abi::label(&done),
            ]);
        }
    }
    builder.instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, &seconds),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&fail),
    ]);
    raise_error_into(
        &symbol,
        "ErrUnsupported",
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);
    Ok(super::gen_shared::void_result("os.uptime"))
}

const INTRO: &str = r#"The operating-system uptime in seconds"#;
const DESC: &str = r#"`os::uptime` returns the host operating-system uptime as whole seconds. Linux
uses `sysinfo`, macOS reads `kern.boottime` through `sysctl`, and Windows uses
`GetTickCount64`."#;
const EX: &str = r#"Print the uptime:

```
IMPORT os
IMPORT io

SUB main()
  io::print(toString(os::uptime() >= 0))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "uptime",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec!["ErrUnsupported"],
            body: Body::abi_function(lower_uptime),
        }],
    });
}
