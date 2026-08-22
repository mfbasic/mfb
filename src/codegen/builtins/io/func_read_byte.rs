//! `io::readByte` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): [`lower_read_byte`] splices its vreg body
//! (from `emit_read_byte_body`) into the builder — the wrapper finalizes it
//! (crypto's shape). No adapter, no pre-finalized hatch.

use super::func_read_line::emit_stdin_byte_read;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;

/// `abi_function` body for `io::readByte` — read one raw byte from stdin (no
/// UTF-8 decoding), returned as a `Byte`.
pub(crate) fn lower_read_byte(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, frame_size) = emit_read_byte_body(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        ctx.build_mode.is_app(),
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = frame_size;
    Ok(ValueResult {
        type_: "Byte".to_string(),
        location: Operand::from("void"),
        text: "io.readByte".to_string(),
    })
}

const INTRO: &str = r#"Read one raw byte from standard input"#;
const DESC: &str = r#"`io::readByte` reads exactly one byte from standard input and returns it as a
`Byte` in the range 0 through 255. It takes no arguments and does not wait for a
newline.

**On a terminal the read is a single keypress.** For the duration of the call,
standard input is switched out of canonical mode and echo is suppressed
(`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0`), so one key satisfies the read with
no Return and nothing is displayed; the previous line discipline is restored
before the call returns. When standard input is not a terminal the stream is read
as is with no mode change.

Before blocking, any pending standard-output buffer is drained, so a prompt
written with `io::write` appears before the program waits. No decoding happens:
the byte is transferred verbatim, so a multi-byte character such as an emoji
arrives one byte at a time across successive calls and there is no `ErrEncoding`
to raise — this is the difference from `io::readChar`, which always returns one
whole Unicode scalar value. Use `io::readByte` for binary input or protocol
framing, and `io::readChar` for text.

End of input is reported as an error, not as a sentinel value such as `0` or
`-1`, which keeps every one of the 256 byte values usable as data. Use
`io::pollInput` to test for readiness when the program must not block. Standard
input is a per-thread broadcast log; a thread other than the main thread must
subscribe with `thread::openStdIn` before reading, or the call raises
`ErrInvalidContext`."#;
const EX: &str = r#"Read one byte and report its value:

```
IMPORT io

SUB main()
  LET b AS Byte = io::readByte()
  io::print(toString(b))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readByte",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Byte,
            errors: vec![],
            body: Body::abi_function(lower_read_byte),
        }],
    });
}

// --- stdin readByte emitter (relocated from native/) ---

/// Emit the readByte vreg body (pre-finalization): `(instructions, relocations,
/// frame_size)` for [`lower_read_byte`] to splice in; the wrapper finalizes.
fn emit_read_byte_body(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> Result<(Vec<CodeInstruction>, Vec<CodeRelocation>, usize), String> {
    const FRAME_SIZE: usize = 208;
    const BYTE_OFFSET: usize = 8;
    let terminal_slots = TerminalModeSlots {
        active: 16,
        saved_tag: 24,
        saved_value: 32,
        saved_message: 40,
        original: 48,
        modified: 120,
    };
    let eof = format!("{symbol}_eof");
    let input_error = format!("{symbol}_input_error");
    let invalid_context = format!("{symbol}_invalid_context");
    let read_retry = format!("{symbol}_read_retry");
    let read_resume = format!("{symbol}_read_resume");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    // Drain buffered stdout before blocking on input (plan-14-A §4.3 hook 2);
    // no-op when buffering is off, skipped in app mode (no stdout buffer).
    if !app_mode {
        instructions.push(abi::branch_link(STDOUT_DRAIN_SYMBOL));
        relocations.push(internal_branch(symbol, STDOUT_DRAIN_SYMBOL));
    }
    if app_mode {
        platform
            .emit_app_raw_input_mode(symbol, &mut instructions, &mut relocations)
            .ok_or_else(|| {
                format!(
                    "native target '{}' does not support app-mode raw input",
                    platform.target()
                )
            })??;
    }
    emit_configure_stdin_terminal(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &terminal_slots,
        abi::stack_pointer(),
        true,
        true,
        &input_error,
    )?;
    // plan-15: read the byte from the stdin broadcast log. EINTR/blocking are
    // handled inside `_mfb_rt_stdin_next_byte`; a 0-byte return is EOF.
    emit_stdin_byte_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTE_OFFSET,
        &read_retry,
        &read_resume,
        &input_error,
        &invalid_context,
    )?;
    instructions.extend([
        abi::branch_eq(&eof),
        abi::load_u8(RESULT_VALUE_REGISTER, abi::stack_pointer(), BYTE_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&eof),
    ]);
    raise_error_into(symbol, "ErrEndOfFile", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&input_error)]);
    raise_error_into(
        symbol,
        "ErrInputFailed",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&invalid_context)]);
    raise_error_into(
        symbol,
        "ErrInvalidContext",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_restore_stdin_terminal(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &terminal_slots,
    )?;
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME_SIZE))
}
