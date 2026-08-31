//! `process::poll` — descriptor entry.
//!
//! Per-member file (planning/migrate.md). `Implementation::Os`: the member's
//! per-platform OS-seam entry fns (`*_posix`/`*_win`) delegate to the arch-neutral
//! emission in `../native/{unix,windows}`, and the generic runtime-call dispatch
//! (`crate::codegen::os`) picks by `platform.family()`. This file carries the
//! descriptor, those entry fns, and the
//! docs.

// --- codegen tier imports (migration) ---
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
const INTRO: &str = r#"Test whether a child's output stream is readable within a timeout."#;
const DESC: &str = r#"`process::poll` reports whether a following read of a child's output stream can
proceed without blocking. It returns `TRUE` when the selected stream is readable —
**including** the case where the child has closed it and the stream is at end of
output, so a draining `process::receive`/`process::receiveBytes` can follow — and
`FALSE` when nothing became readable before the deadline. A `timeoutMs` of `0`
checks and returns immediately; a negative `timeoutMs` waits with no deadline at
all, until the stream is readable or the child exits.

Calling `poll` after `process::detach` raises `ErrResourceClosed`, because
detaching ends the handle. The stream is inspected
only; nothing is read, so a `TRUE` result leaves the bytes in place for the next
read.

`ms` is the wait bound in milliseconds. `0` is a non-blocking check that returns the
stream's current readiness immediately; a positive value waits up to that long; a
timeout that elapses with nothing readable returns `FALSE` (poll reports readiness
as a boolean and never raises `ErrTimeout`).


Without a `from` argument `poll` inspects the child's standard output; pass a
`Stream` value to choose standard output or standard error."#;
const EX: &str = r#"Read a line only if one is ready within 100 ms:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES child = process::spawn(["echo", "hello"])
  IF process::poll(child, 100) THEN
    io::print(process::receive(child))
  END IF
  RETURN 0
END FUNC
```

Check the child's standard error, waiting up to half a second:

```
IMPORT process
IMPORT io

FUNC main AS Integer
  RES sh = process::shell("echo oops 1>&2")
  IF process::poll(sh, 500, Stream.StdErr) THEN
    io::print(process::receive(sh, Stream.StdErr))
  END IF
  RETURN 0
END FUNC
```"#;

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `process::poll` — branches win/posix and calls this
/// member's own backend helper (with any alias discriminant via `ctx.call`), then
/// finalizes.
pub(crate) fn lower_poll(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        if ctx.platform.family() == crate::codegen::engine::types::PlatformFamily::Windows {
            lower_process_poll_helper_win(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
        } else {
            lower_process_poll_helper_posix(ctx.call, &symbol, ctx.platform_imports, ctx.platform)?
        };
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // The optional trailing `from AS Stream` widens arity to 3 and is NOT
    // default-padded: the 3-arg form is selected at codegen (`builder_values` →
    // `process.pollFrom`), and the emitter branches on the runtime-call name.
    pkg.add_function(RegistryFunction {
        name: "poll",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "p",
                    desc: "The child process handle. The handle stays open — this only tests readiness and reads no data. Also accepts the alternate named-argument spelling `process`.",
                    aliases: &["process"],
                    ty: ParameterType::named(super::PROCESS_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "ms",
                    desc: "The maximum time to wait, in milliseconds. `0` is an immediate non-blocking check; a positive value waits up to that long.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "from",
                    desc: "Optional. Which output stream to inspect: `Stream.StdOut` (the default) or `Stream.StdErr`.",
                    aliases: &[],
                    ty: ParameterType::named(super::STREAM_TYPE),
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function_aliased(lower_poll, &["pollFrom"]),
        }],
    });
}

pub(crate) fn lower_process_poll_helper_posix(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    let with_from = call == "process.pollFrom";
    const POLLFD_SLOT: usize = 0;
    let mut v = Vregs::new();
    let file = v.next();
    let ms = v.next();
    let fd = v.next();
    let n = v.next();
    let from = v.next();
    let closed_l = format!("{symbol}_closed");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let ready = format!("{symbol}_ready");
    let done = format!("{symbol}_done");

    let mut instructions = vec![
        abi::move_register(&file, abi::return_register()),
        abi::move_register(&ms, abi::c_arg(1)),
    ];
    if with_from {
        instructions.push(abi::move_register(&from, abi::c_arg(2)));
    }
    instructions.extend([
        abi::load_u64(&n, &file, RESOURCE_OFFSET_CLOSED),
        abi::compare_immediate(&n, "0"),
        abi::branch_ne(&closed_l),
    ]);
    if with_from {
        instructions.extend([
            abi::compare_immediate(&from, "0"),
            abi::branch_ne(&use_stderr),
            abi::load_u64(&fd, &file, PROC_STDOUT_R),
            abi::branch(&sel_done),
            abi::label(&use_stderr),
            abi::load_u64(&fd, &file, PROC_STDERR_R),
            abi::label(&sel_done),
        ]);
    } else {
        instructions.push(abi::load_u64(&fd, &file, PROC_STDOUT_R));
    }
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u32(&fd, abi::stack_pointer(), POLLFD_SLOT),
        abi::move_immediate(&n, "Integer", "1"), // POLLIN
        abi::store_u16(&n, abi::stack_pointer(), POLLFD_SLOT + 4),
        abi::store_u16(abi::ZERO, abi::stack_pointer(), POLLFD_SLOT + 6),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), POLLFD_SLOT),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::move_register(abi::c_arg(2), &ms),
    ]);
    platform.emit_external_call(
        "poll",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(&n, abi::c_return(0)),
        abi::compare_immediate(&n, "0"),
        abi::branch_gt(&ready),
        // 0 (timeout) or < 0 (error) -> not ready.
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 16))
}

pub(crate) fn lower_process_poll_helper_win(
    call: &str,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<ProcBodyParts, String> {
    let with_from = call == "process.pollFrom";
    const AVAIL: usize = 0x30;
    const FILE: usize = 0x38;
    const MS: usize = 0x40;
    const DEADLINE: usize = 0x48;
    const FD: usize = 0x50;
    const FROM: usize = 0x58;
    const FRAME: usize = 0x60;
    let sp = abi::stack_pointer();
    let closed_l = format!("{symbol}_closed");
    let use_stderr = format!("{symbol}_use_stderr");
    let sel_done = format!("{symbol}_sel_done");
    let poll_loop = format!("{symbol}_poll_loop");
    let ready = format!("{symbol}_ready");
    let not_ready = format!("{symbol}_not_ready");
    let done = format!("{symbol}_done");
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::return_register(), sp, FILE),
        abi::store_u64(abi::mfb_arg(1), sp, MS),
    ];
    if with_from {
        instructions.push(abi::store_u64(abi::mfb_arg(2), sp, FROM));
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
        // deadline = GetTickCount64() + ms
    ]);
    platform.emit_external_call(
        "GetTickCount64",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, MS),
        abi::add_registers(abi::mfb_arg(0), abi::c_return(0), abi::mfb_arg(1)),
        abi::store_u64(abi::mfb_arg(0), sp, DEADLINE),
        abi::label(&poll_loop),
        // PeekNamedPipe(fd, NULL, 0, NULL, &avail, NULL)
        abi::store_u64(abi::ZERO, sp, AVAIL),
        abi::add_immediate(abi::mfb_arg(0), sp, AVAIL),
        abi::store_u64(abi::mfb_arg(0), sp, 0x20), // 5th &avail
        abi::store_u64(abi::ZERO, sp, 0x28),       // 6th NULL
        abi::load_u64(abi::mfb_arg(0), sp, FD),
        abi::move_immediate(abi::mfb_arg(1), "Integer", "0"),
        abi::move_immediate(abi::mfb_arg(2), "Integer", "0"),
        abi::move_immediate(abi::mfb_arg(3), "Integer", "0"),
    ]);
    platform.emit_external_call(
        "PeekNamedPipe",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::c_return(0), abi::c_return(0)),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&ready), // FALSE = broken pipe → readable (EOF drain)
        abi::load_u32(abi::mfb_arg(0), sp, AVAIL),
        abi::compare_immediate(abi::mfb_arg(0), "0"),
        abi::branch_ne(&ready),
    ]);
    // Not ready yet: past the deadline? (now >= deadline → timeout).
    platform.emit_external_call(
        "GetTickCount64",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::mfb_arg(1), sp, DEADLINE),
        abi::compare_registers(abi::c_return(0), abi::mfb_arg(1)),
        abi::branch_ge(&not_ready),
        abi::move_immediate(abi::mfb_arg(0), "Integer", "1"),
    ]);
    platform.emit_external_call(
        "Sleep",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&poll_loop),
        abi::label(&ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed_l),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::add_stack(FRAME), abi::return_()]);
    Ok((instructions, relocations, 0))
}
