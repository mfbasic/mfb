//! `process::receive` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor and those entry fns.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use std::collections::HashMap;

use crate::codegen::error::emission::emit_fail;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_shared::*;
const INTRO: &str = r#"Read one newline-terminated line of text from a child's output."#;
const DESC: &str = r#"`process::receive` reads one line from a child's output stream and returns it as a
`String`, **including** the trailing newline. It reads until it sees a `'\n'`,
never over-reading past the line boundary, so successive calls return successive
lines. Without a `from` argument it reads the child's standard output; pass a
`Stream` value to choose standard output or standard error explicitly.



The call blocks until a full line is available or the stream ends. At end of stream
it **drains before reporting closed**: any bytes accumulated since the last newline
are returned as a final (newline-less) line, and only a subsequent read that finds
end of stream with nothing buffered raises `ErrResourceClosed`. A consumer therefore
loops, reading lines until `ErrResourceClosed` marks the end of the output.


The returned line is validated as UTF-8; output that is not valid UTF-8 raises
`ErrEncoding`. Use `process::receiveBytes` for binary output or output whose
encoding is unknown. Very long lines are capped at 1 MiB: a line reaching that
length is returned as-is without waiting for a newline."#;
const EX: &str = r#"Read one line of a child's standard output:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  io::print(process::receive(child))
  RETURN 0
END FUNC
```

Read a diagnostic line from the child's standard error:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo oops 1>&2")
  io::print(process::receive(sh, Stream.StdErr))
  RETURN 0
END FUNC
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::receive` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_receive(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = if ctx.platform.family()
        == crate::codegen::engine::types::PlatformFamily::Windows
    {
        lower_process_receive_helper_win(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
    } else {
        lower_process_receive_helper_posix(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
    };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // The optional trailing `from AS Stream` widens arity to 2 and is NOT
    // default-padded: the 2-arg form is selected at codegen (`builder_values` →
    // `process.receiveFrom`), and the emitter branches on the runtime-call name.
    pkg.add_function(RegistryFunction {
        name: "receive",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "p",
                    desc: "The child process handle to read from. Also accepts the alternate named-argument spelling `process`.",
                    aliases: &["process"],
                    ty: ParameterType::named(super::PROCESS_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "from",
                    desc: "Optional. Which output stream to read: `Stream.StdOut` (the default) or `Stream.StdErr`.",
                    aliases: &[],
                    ty: ParameterType::named(super::STREAM_TYPE),
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function_aliased(lower_receive, &["receiveFrom"]),
        }],
    });
}

pub(crate) fn lower_process_receive_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    let with_from = call == "process.receiveFrom";
    const FD: usize = 0;
    const LINEP: usize = 8; // accumulator pointer (for the result build)
    const N: usize = 16; // accumulated length
    const STR: usize = 24; // built String ptr
    const CAP: usize = 1048576; // 1 MiB max line
    const EINTR: &str = "4";

    let closed = format!("{symbol}_closed");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let read_loop = format!("{symbol}_read_loop");
    let read_fail = format!("{symbol}_read_fail");
    let got_byte = format!("{symbol}_got_byte");
    let eof_check = format!("{symbol}_eof_check");
    let build = format!("{symbol}_build");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let reg9 = vregs.next();
    let reg10 = vregs.next();
    let reg11 = vregs.next();
    let reg12 = vregs.next();

    let mut instructions = vec![
        abi::load_u64(&reg9, abi::return_register(), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&reg9, "0"),
        abi::branch_ne(&closed),
    ];
    if with_from {
        instructions.extend([
            abi::compare_immediate(abi::c_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64(&reg9, abi::return_register(), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64(&reg9, abi::return_register(), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64(&reg9, abi::return_register(), PROC_STDOUT_R));
    }
    instructions.extend([
        abi::store_u64(&reg9, abi::stack_pointer(), FD),
        // Accumulator buffer.
        abi::move_immediate(abi::return_register(), "Integer", &CAP.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    let mut relocations = Vec::new();
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), LINEP),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), N),
        // read one byte into acc[filled].
        abi::label(&read_loop),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), FD),
        abi::load_u64(&reg9, abi::stack_pointer(), LINEP),
        abi::load_u64(&reg10, abi::stack_pointer(), N),
        abi::add_registers(abi::c_arg(1), &reg9, &reg10),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "read",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_lt(&read_fail),
        abi::branch_gt(&got_byte),
        // r == 0: EOF.
        abi::label(&eof_check),
        abi::load_u64(&reg10, abi::stack_pointer(), N),
        abi::compare_immediate(&reg10, "0"),
        abi::branch_eq(&closed),
        abi::branch(&build),
        abi::label(&got_byte),
        // filled += 1; check the byte just read for '\n'.
        abi::load_u64(&reg9, abi::stack_pointer(), LINEP),
        abi::load_u64(&reg10, abi::stack_pointer(), N),
        abi::add_registers(&reg11, &reg9, &reg10),
        abi::load_u8(&reg12, &reg11, 0),
        abi::add_immediate(&reg10, &reg10, 1),
        abi::store_u64(&reg10, abi::stack_pointer(), N),
        abi::compare_immediate(&reg12, "10"), // '\n'
        abi::branch_eq(&build),
        abi::move_immediate(&reg11, "Integer", &CAP.to_string()),
        abi::compare_registers(&reg10, &reg11),
        abi::branch_eq(&build), // line too long -> return what we have
        abi::branch(&read_loop),
        abi::label(&read_fail),
    ]);
    platform.emit_errno(
        symbol,
        (&reg9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&reg9, EINTR),
        abi::branch_eq(&read_loop),
        abi::branch(&closed),
        abi::label(&build),
    ]);
    crate::codegen::os::socket::shared::emit_string_result_build(
        symbol,
        LINEP,
        N,
        STR,
        &str_copy,
        &str_done,
        &alloc_fail,
        &encoding_error,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
    ]);
    emit_fail(
        symbol,
        "ErrEncoding",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 48))
}

pub(crate) fn lower_process_receive_helper_win(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    let with_from = call == "process.receiveFrom";
    const NREAD: usize = 0x28;
    const FILE: usize = 0x30;
    const FD: usize = 0x38;
    const ACC: usize = 0x40;
    const N: usize = 0x48;
    const STR: usize = 0x50;
    const I: usize = 0x58;
    const FROM: usize = 0x60;
    const FRAME: usize = 0x70;
    const CAP: usize = 1048576; // 1 MiB max line
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let read_loop = format!("{symbol}_read_loop");
    let eof = format!("{symbol}_eof");
    let build = format!("{symbol}_build");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
    ];
    if with_from {
        instructions.push(abi::store_u64(abi::mfb_arg(1), sp, FROM));
    }
    instructions.extend([
        abi::load_u64(abi::mfb_arg(0), sp, FILE),
        abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(abi::mfb_arg(1), "0"),
        abi::branch_ne(&closed_l),
    ]);
    if with_from {
        instructions.extend([
            abi::load_u64(abi::mfb_arg(1), sp, FROM),
            abi::compare_immediate(abi::mfb_arg(1), "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64(abi::mfb_arg(1), abi::mfb_arg(0), PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64(
            abi::mfb_arg(1),
            abi::mfb_arg(0),
            PROC_STDOUT_R,
        ));
    }
    instructions.extend([
        abi::store_u64(abi::mfb_arg(1), sp, FD),
        // accumulator = arena_alloc(CAP, 1)
        abi::move_immediate(abi::return_register(), "Integer", &CAP.to_string()),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, ACC),
        abi::store_u64(abi::ZERO, sp, N),
        abi::label(&read_loop),
        // ReadFile(fd, acc + n, 1, &nread, NULL)
        abi::load_u64(abi::mfb_arg(0), sp, FD),
        abi::load_u64(abi::mfb_arg(1), sp, ACC),
        abi::load_u64(abi::mfb_arg(2), sp, N),
        abi::add_registers(abi::mfb_arg(1), abi::mfb_arg(1), abi::mfb_arg(2)),
        abi::move_immediate(abi::mfb_arg(2), "Integer", "1"),
        abi::add_immediate(abi::mfb_arg(3), sp, NREAD),
        abi::store_u64(abi::ZERO, sp, 0x20),
    ]);
    platform.emit_external_call(
        "ReadFile",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&eof), // ReadFile FALSE = broken pipe / EOF
        abi::load_u32(abi::mfb_arg(0), sp, NREAD),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&eof),
        // got a byte: n += 1; check for '\n' or a full line.
        abi::load_u64(abi::mfb_arg(0), sp, ACC),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::add_registers(abi::mfb_arg(2), abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::load_u8(abi::mfb_arg(3), abi::mfb_arg(2), 0),
        abi::add_immediate(abi::mfb_arg(1), abi::mfb_arg(1), 1),
        abi::store_u64(abi::mfb_arg(1), sp, N),
        abi::compare_immediate(abi::mfb_arg(3), "10"),
        abi::branch_eq(&build),
        abi::move_immediate(abi::mfb_arg(2), "Integer", &CAP.to_string()),
        abi::compare_registers(abi::mfb_arg(1), abi::mfb_arg(2)),
        abi::branch_eq(&build),
        abi::branch(&read_loop),
        abi::label(&eof),
        abi::load_u64(abi::mfb_arg(0), sp, N),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_eq(&closed_l),
        abi::label(&build),
        // String obj = arena_alloc(n + 9, 8): length@0, bytes@8, NUL.
        abi::load_u64(abi::mfb_arg(0), sp, N),
        abi::add_immediate(abi::return_register(), abi::mfb_arg(0), 9),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), sp, STR),
        abi::move_register(abi::mfb_arg(0), abi::mfb_return(1)),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), 0),
        // copy n bytes: dst = str + 8 (mfb_arg(2)), src = acc (mfb_arg(3))
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(0), 8),
        abi::load_u64(abi::mfb_arg(3), sp, ACC),
        abi::store_u64(abi::ZERO, sp, I),
        abi::label(&copy_loop),
        abi::load_u64(abi::mfb_arg(0), sp, I),
        abi::load_u64(abi::mfb_arg(1), sp, N),
        abi::compare_registers(abi::mfb_arg(0), abi::mfb_arg(1)),
        abi::branch_eq(&copy_done),
        abi::load_u8(abi::mfb_arg(0), abi::mfb_arg(3), 0),
        abi::store_u8(abi::mfb_arg(0), abi::mfb_arg(2), 0),
        abi::add_immediate(abi::mfb_arg(2), abi::mfb_arg(2), 1),
        abi::add_immediate(abi::mfb_arg(3), abi::mfb_arg(3), 1),
        abi::load_u64(abi::mfb_arg(0), sp, I),
        abi::add_immediate(abi::mfb_arg(0), abi::mfb_arg(0), 1),
        abi::store_u64(abi::mfb_arg(0), sp, I),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, abi::mfb_arg(2), 0), // NUL-terminate
        // validate_utf8(str + 8, n)
        abi::load_u64(abi::mfb_arg(0), sp, STR),
        abi::add_immediate(abi::return_register(), abi::mfb_arg(0), 8),
        abi::load_u64(abi::c_arg(1), sp, N),
    ]);
    crate::codegen::string::validate::emit_call_validate_utf8(
        symbol,
        &encoding_error,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, sp, STR),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
    ]);
    emit_fail(
        symbol,
        "ErrEncoding",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed_l));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    Ok((instructions, relocations, 0))
}
