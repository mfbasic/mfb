//! `io::pollInput` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): [`lower_poll_input`] emits its vreg body
//! directly into the builder — the wrapper finalizes it (crypto's shape). No
//! adapter, no pre-finalized hatch.

use super::gen_read_family::emit_stdin_byte_read;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::mouse::clock::MOUSE_CLOCK_SCRATCH_BYTES;
use crate::codegen::io::mouse::decode as mouse_decode;
use crate::codegen::io::stdin::*;
use crate::codegen::memory::data::*;
use crate::codegen::os::syscall::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `abi_function` body for `io::pollInput` — test whether stdin has input ready
/// (optionally waiting up to a timeout), returned as a `Boolean`, consuming none.
/// Emits its vreg body directly into the builder; the wrapper finalizes.
pub(crate) fn lower_poll_input(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    const POLLIN_PACKED_FD0: &str = "4294967296";
    const BASE_FRAME_SIZE: usize = 48;
    const POLLFD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 32;
    // plan-94-B §3d. The verification reads one byte to find out whether the ready
    // bytes are a character or a mouse report, so it needs somewhere to put that
    // byte and somewhere for the decoder's clock read. Appended past the base
    // frame, and only for a mouse program, so no existing slot moves and a
    // non-mouse `pollInput` keeps its exact frame.
    const MOUSE_BYTE_OFFSET: usize = BASE_FRAME_SIZE;
    const MOUSE_CLOCK_OFFSET: usize = BASE_FRAME_SIZE + 8;

    let symbol_owned = builder.current_symbol.clone();
    let symbol: &str = &symbol_owned;
    let platform_imports = ctx.platform_imports;
    let platform = ctx.platform;
    let app_mode = ctx.build_mode.is_app();

    let poll_error = format!("{symbol}_poll_error");
    let poll_invalid = format!("{symbol}_poll_invalid");
    let poll_eintr_check = format!("{symbol}_poll_eintr");
    let poll_ready = format!("{symbol}_poll_ready");
    let poll_infinite = format!("{symbol}_poll_infinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let os_poll = format!("{symbol}_os_poll");
    // Where the mouse verification goes back to when the ready bytes turned out to
    // be a report: re-ask readiness, with the timeout already forced to zero.
    let ready_recheck = format!("{symbol}_ready_recheck");
    let report_ready = format!("{symbol}_report_ready");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    // Save the caller's timeout before the log-ready check clobbers x0.
    instructions.push(abi::store_u64(
        abi::return_register(),
        abi::stack_pointer(),
        TIMEOUT_OFFSET,
    ));
    // plan-73-F: normalize `timeoutMs` to the plan-73 timeout convention BEFORE any
    // work, then stash the OS-ready value back to TIMEOUT_OFFSET (os_poll reloads it
    // on every EINTR retry). An OMITTED timeout is padded with the unbounded sentinel
    // (i64::MIN) → poll(2) with a -1 (block-forever) timeout; `0` = one immediate
    // readiness check; `> 0` = bounded, clamped to INT_MAX so a bit-31 value is not
    // read by poll's C `int` as a negative/block timeout (bug-239 class); any other
    // negative is `ErrInvalidArgument`. Before plan-73 the raw value went straight to
    // poll(2) (negative = block, omit padded with 0 = non-blocking), the exact
    // inversion this convention removes.
    let v10 = vregs.next();
    let v11 = vregs.next();
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v11, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v10, &v11),
        abi::branch_eq(&poll_infinite),
        abi::compare_immediate(&v10, "0"),
        abi::branch_lt(&poll_invalid),
        abi::move_immediate(&v11, "Integer", "2147483647"),
        abi::compare_registers(&v10, &v11),
        abi::branch_le(&timeout_ok),
        abi::move_register(&v10, &v11),
        abi::branch(&timeout_ok),
        abi::label(&poll_infinite),
        abi::bitwise_not(&v10, abi::ZERO),
        abi::label(&timeout_ok),
        abi::store_u64(&v10, abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    // The re-check entry point exists only for the mouse verification below. A
    // label is an instruction in the emitted stream, so pushing it unconditionally
    // would put it in every program's `io::pollInput` — which is exactly the
    // byte-identity claim this sub-plan makes about programs that never mention
    // the mouse, and exactly how it was first broken (five `io` fixtures diffed).
    if let Some(mouse_state_offset) = ctx.mouse_state_offset {
        instructions.push(abi::label(&ready_recheck));
        // Bytes the decoder owes the program (the tail of a flushed escape prefix
        // — the `[A` after an arrow key's ESC) live in the decoder, not in the log
        // or the OS, so neither readiness check below can see them. They are
        // ready by definition: answer before waiting on anything (bug-669).
        let mut ectx = EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        };
        mouse_decode::emit_branch_if_owed(&report_ready, mouse_state_offset, &mut ectx, &mut vregs);
    }
    // plan-15 §4.4: a byte already staged for this thread in the broadcast log is
    // invisible to `poll(fd 0)`, so check the log first (ready => report TRUE) and
    // only `poll(fd 0)` when the log has nothing for us. App mode reads the window
    // pipe (no broadcast log), so it skips straight to `poll(fd 0)`.
    if !app_mode {
        emit_stdin_poll_ready_check(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            &poll_ready,
            &os_poll,
        )?;
    }
    let v9 = vregs.next();
    instructions.extend([
        abi::label(&os_poll),
        abi::move_immediate(&v9, "Integer", POLLIN_PACKED_FD0),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
    ]);

    instructions.push(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        TIMEOUT_OFFSET,
    ));

    instructions.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    platform.emit_poll_input(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_eintr_check),
        abi::branch_gt(&poll_ready),
    ]);
    // Nothing more arrived. If the decoder is still holding an escape prefix, the
    // silence has settled it: it was never a mouse report, so its bytes are the
    // program's and a read will return them now (bug-669).
    if let Some(mouse_state_offset) = ctx.mouse_state_offset {
        let not_held = format!("{symbol}_mouse_not_held");
        let mut ectx = EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        };
        mouse_decode::emit_branch_unless_prefix_held(
            &not_held,
            mouse_state_offset,
            &mut ectx,
            &mut vregs,
        );
        mouse_decode::emit_release_prefix(mouse_state_offset, &mut ectx, &mut vregs);
        ectx.instructions.push(abi::branch(&report_ready));
        ectx.instructions.push(abi::label(&not_held));
    }
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_ready),
    ]);
    // plan-94-B §3d: with mouse reporting on, "bytes are ready" no longer implies
    // "a character is ready" — the pending bytes may be a report the pump will
    // swallow whole, leaving the `readChar` this TRUE invited to block on a
    // terminal that has gone quiet.
    //
    // The alternative was to document that TRUE means "bytes", not "a character".
    // That is cheaper and it is a lie: every existing caller reads TRUE as a
    // promise that the next read returns. A readiness predicate that no longer
    // predicts readiness is a worse defect than the one it replaces, so this runs
    // the bytes through the same decoder the read path uses and answers the
    // question it claims to answer.
    if let Some(mouse_state_offset) = ctx.mouse_state_offset {
        emit_mouse_ready_verify(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: &mut instructions,
                relocations: &mut relocations,
            },
            app_mode,
            mouse_state_offset,
            MOUSE_BYTE_OFFSET,
            MOUSE_CLOCK_OFFSET,
            TIMEOUT_OFFSET,
            &report_ready,
            &ready_recheck,
        )?;
    }
    if ctx.mouse_state_offset.is_some() {
        instructions.push(abi::label(&report_ready));
    }
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_eintr_check),
    ]);
    // bug-314 H1: a negative return used to go straight to ErrInput. Every other
    // blocking primitive retries EINTR -- read/write/seek (bug-62) and net poll
    // (bug-115) -- but fd-0 poll was left unwrapped, so any handled signal
    // (SIGWINCH in a TUI, SIGCHLD, the console SIGINT/SIGTERM handler where the
    // program continues) interrupting a blocked `io::pollInput()` surfaced as a
    // spurious ErrInput instead of ready/not-ready. Retry at `os_poll`, which
    // re-arms the pollfd from scratch.
    emit_eintr_retry_or_error(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        abi::return_register(),
        false,
        &os_poll,
        &poll_error,
    )?;
    // plan-73-F: a negative timeout other than the unbounded sentinel is
    // `ErrInvalidArgument`. Placed before `poll_error` and terminated with a branch to
    // `done` so `poll_error` still falls through to `done` (byte-identical to before).
    instructions.push(abi::label(&poll_invalid));
    raise_error_into(
        symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&done));
    instructions.push(abi::label(&poll_error));
    raise_error_into(
        symbol,
        "ErrInputFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    instructions.push(abi::return_());
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = if ctx.mouse_state_offset.is_some() {
        MOUSE_CLOCK_OFFSET + MOUSE_CLOCK_SCRATCH_BYTES
    } else {
        BASE_FRAME_SIZE
    };
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from("void"),
        text: "io.pollInput".to_string(),
    })
}

const INTRO: &str =
    r#"Test whether standard input is ready to read, optionally waiting up to a timeout"#;
const DESC: &str = r#"`io::pollInput` reports whether a following read of standard input can proceed
without blocking. It returns `TRUE` when input is ready and `FALSE` when the wait
elapses first, and it **reads nothing** — the bytes are still there for
`io::readLine`, `io::readChar`, `io::readByte`, or `io::input`.

`timeoutMs` bounds the wait, in milliseconds, following the language timeout
convention. When it is **omitted, `pollInput` blocks** until standard input
becomes ready and then returns `TRUE` (omit = unbounded). `0` is a non-blocking
check that returns immediately with the current readiness. A positive value waits
up to that long. A negative `timeoutMs` is rejected with `ErrInvalidArgument`.
A value above `2147483647` waits no longer than that, roughly 24 days.

Input already waiting for this thread reports `TRUE` immediately; otherwise the
call waits on standard input itself. A thread that has not subscribed with
`thread::openStdIn` may still call `io::pollInput` — unlike the read calls, it
does not raise `ErrInvalidContext`.

**`TRUE` means at least one byte is ready — not that the next read completes.**
Two cases to know about:

- **End of input counts as ready.** `io::pollInput` returns `TRUE` and the
  following read raises `ErrEndOfFile`.
- **A partial character still waits.** `io::readChar` returns a whole Unicode
  scalar, so if only the first byte of a multi-byte sequence has arrived, the
  read blocks for the rest even though `io::pollInput` said `TRUE`.

`io::readByte` is the one read that a `TRUE` result does fully guarantee. A
signal delivered while waiting is not an error; the wait is resumed."#;
const EX: &str = r#"Read a line only when one is already pending (pass `0` for the immediate check —
omitting the timeout would instead block until input is ready):

```
IMPORT io

SUB main()
  IF io::pollInput(0) THEN
    io::print(io::readLine())
  END IF
END SUB
```

Wait up to a second for a keypress:

```
IMPORT io

SUB main()
  IF io::pollInput(1000) THEN
    io::print(io::readChar())
  ELSE
    io::print("timeout")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pollInput",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "timeoutMs",
                desc: "Optional. Omit to block until standard input is ready; `0` is an immediate non-blocking check; a positive value waits up to that many milliseconds, clamped to `2147483647`. Must not be negative.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::Optional,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_poll_input),
        }],
    });
}

/// Decide whether the ready bytes really are a character (plan-94-B §3d).
///
/// Branches to `report_ready` when the next `readChar` will return without
/// blocking, and to `ready_recheck` — with the timeout forced to zero — when what
/// was ready turned out to be a mouse report and the question has to be asked
/// again.
///
/// **The re-check is non-blocking on purpose.** Looping with the caller's original
/// timeout would let `pollInput(100)` wait 100 ms per report, unbounded, while a
/// user drags the mouse. Zero is also the honest answer to what has actually been
/// established: the caller's wait already happened, what arrived was mouse, and
/// nothing else is ready *now*.
#[allow(clippy::too_many_arguments)]
fn emit_mouse_ready_verify(
    ctx: &mut EmitCtx,
    app_mode: bool,
    mouse_state_offset: usize,
    byte_offset: usize,
    clock_offset: usize,
    timeout_offset: usize,
    report_ready: &str,
    ready_recheck: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let mut vregs = Vregs::new();
    let mode_addr = vregs.next();
    let mode = vregs.next();
    let out_byte = vregs.next();
    let count = vregs.next();
    let byte = vregs.next();
    let action = vregs.next();
    let zero = vregs.next();
    let consumed = format!("{symbol}_mouse_verify_consumed");
    let read_retry = format!("{symbol}_mouse_verify_retry");
    let read_resume = format!("{symbol}_mouse_verify_resume");
    let read_gave_up = format!("{symbol}_mouse_verify_gave_up");

    // Mouse reporting off: nothing can swallow the ready bytes, so the original
    // answer stands.
    push_symbol_address(
        symbol,
        MOUSE_MODE_SYMBOL,
        &mode_addr,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::load_u64(&mode, &mode_addr, 0));
    ctx.instructions.push(abi::compare_immediate(&mode, "0"));
    ctx.instructions.push(abi::branch_eq(report_ready));

    // A byte already owed to the program is a character ready by definition, and
    // checking first is also what makes the read below safe to issue. Peek, don't
    // drain: draining would hand the owed byte to nobody (bug-669).
    mouse_decode::emit_branch_if_owed(report_ready, mouse_state_offset, ctx, &mut vregs);

    // Take one byte. Readiness has already said one is there, so this does not
    // block; `mouse: None` keeps the pump out of it — running the pump here would
    // consume a whole report and lose the byte this function exists to inspect.
    emit_stdin_byte_read(
        ctx,
        app_mode,
        byte_offset,
        &read_retry,
        &read_resume,
        // An input error or an unsubscribed thread is not this function's to
        // report: `pollInput` has never raised for either, and the follow-up read
        // raises the same thing without blocking — which is what TRUE promises.
        &read_gave_up,
        &read_gave_up,
        None,
    )?;
    ctx.instructions
        .push(abi::move_register(&count, abi::return_register()));
    ctx.instructions.push(abi::compare_immediate(&count, "0"));
    // EOF: a read returns immediately at EOF, so TRUE is still true.
    ctx.instructions.push(abi::branch_le(report_ready));

    ctx.instructions
        .push(abi::load_u8(&byte, abi::stack_pointer(), byte_offset));
    mouse_decode::emit_decode_byte(
        &byte,
        &action,
        &out_byte,
        mouse_state_offset,
        clock_offset,
        ctx,
        &mut vregs,
    )?;
    ctx.instructions.push(abi::compare_immediate(
        &action,
        &mouse_decode::DECODE_PASS.to_string(),
    ));
    ctx.instructions.push(abi::branch_ne(&consumed));
    // The byte is the program's. Put it back undelivered so the `readChar` this
    // TRUE invites hands it over — `pollInput` consumes nothing, and that contract
    // survives the inspection.
    mouse_decode::emit_pushback_byte(&out_byte, mouse_state_offset, ctx, &mut vregs);
    ctx.instructions.push(abi::branch(report_ready));

    // Consumed. A finished report (or one byte further into a report) re-asks
    // readiness without waiting; but when the byte was BUFFERED the decoder is now
    // holding a prefix that only the next byte can settle, so the re-check waits
    // the escape delay for it. If nothing comes, the not-ready arm releases the
    // prefix as keystrokes (bug-669) — that is how a lone Esc gets reported.
    let recheck_store = format!("{symbol}_mouse_verify_recheck_store");
    ctx.instructions.extend([
        abi::label(&consumed),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::compare_immediate(&action, &mouse_decode::DECODE_BUFFERED.to_string()),
        abi::branch_ne(&recheck_store),
        abi::move_immediate(&zero, "Integer", &mouse_decode::ESCAPE_DELAY_MS.to_string()),
        abi::label(&recheck_store),
        abi::store_u64(&zero, abi::stack_pointer(), timeout_offset),
        abi::branch(ready_recheck),
        abi::label(&read_gave_up),
        abi::branch(report_ready),
    ]);
    Ok(())
}
