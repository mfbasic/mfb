//! `io::readChar` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): [`lower_read_char`] emits its vreg body
//! directly into the builder — the wrapper finalizes it (crypto's shape). The
//! shared UTF-8 sequence reader lives in [`super::gen_read_family`]. No adapter, no
//! pre-finalized hatch.

use super::gen_read_family::{emit_stdin_byte_read, emit_utf8_sequence_read, Utf8SeqLabels};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Read one whole Unicode scalar value from standard input"#;
const DESC: &str = r#"`io::readChar` reads exactly one Unicode scalar value from standard input and
returns it as a one-character `String`. It reads the lead byte, derives the
sequence length from it, and reads the one to three continuation bytes that
complete the scalar. It takes no arguments and does not wait for a newline.

**On a terminal the read is a single keypress.** For the duration of the call,
standard input is switched out of canonical mode and echo is suppressed
(`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0`), so one key satisfies the read with
no Return and nothing is displayed; the previous line discipline is restored
before the call returns. When standard input is not a terminal the stream is read
as is with no mode change.

Before blocking, any pending standard-output buffer is drained, so a prompt
written with `io::write` appears before the program waits. Decoding is strict
UTF-8, not lenient: an ill-formed sequence raises `ErrEncoding` rather than
yielding a replacement character, and so does a sequence cut short by end of
input. This returns one *scalar value*, not one user-perceived character: a
grapheme cluster made of several scalars takes that many calls. Compare
`io::readByte`, which returns raw bytes with no decoding at all.

End of input is reported as an error, not as an empty result. Use `io::pollInput`
to test for readiness when the program must not block. Standard input is a
per-thread broadcast log; a thread other than the main thread must subscribe with
`thread::openStdIn` before reading, or the call raises `ErrInvalidContext`."#;
const EX: &str = r#"Wait for any keypress to continue:

```
IMPORT io

SUB main()
  io::write("Press any key to continue...")
  LET ignored AS String = io::readChar()
  io::print("")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readChar",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_read_char),
        }],
    });
}

// --- stdin readChar lowering (relocated from native/) ---

/// `abi_function` body for `io::readChar` — read one whole Unicode scalar value
/// (one UTF-8 sequence) from stdin, returned as a one-character `String`. Emits
/// its vreg body directly into the builder; the wrapper finalizes.
pub(crate) fn lower_read_char(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol_owned = builder.current_symbol.clone();
    let symbol: &str = &symbol_owned;
    let platform_imports = ctx.platform_imports;
    let platform = ctx.platform;
    let app_mode = ctx.build_mode.is_app();
    const FRAME_SIZE: usize = 224;
    const BYTES_OFFSET: usize = 8;
    const LEN_OFFSET: usize = 16;
    const RESULT_OFFSET: usize = 24;
    let terminal_slots = TerminalModeSlots {
        active: 32,
        saved_tag: 40,
        saved_value: 48,
        saved_message: 56,
        original: 64,
        modified: 136,
    };
    let read_second = format!("{symbol}_read_second");
    let read_third = format!("{symbol}_read_third");
    let read_fourth = format!("{symbol}_read_fourth");
    let got_len = format!("{symbol}_got_len");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let eof = format!("{symbol}_eof");
    let input_error = format!("{symbol}_input_error");
    let invalid_context = format!("{symbol}_invalid_context");
    let encoding_error = format!("{symbol}_encoding_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let read_retry = format!("{symbol}_read_retry");
    let read_resume = format!("{symbol}_read_resume");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
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
    // plan-15: read the lead byte from the stdin broadcast log; a 0-byte return is
    // EOF. EINTR/blocking are handled inside `_mfb_rt_stdin_next_byte`.
    emit_stdin_byte_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        app_mode,
        BYTES_OFFSET,
        &read_retry,
        &read_resume,
        &input_error,
        &invalid_context,
    )?;
    let three_not_e0 = format!("{symbol}_three_not_e0");
    let three_general = format!("{symbol}_three_general");
    let three_second_ok = format!("{symbol}_three_second_ok");
    let four_not_f0 = format!("{symbol}_four_not_f0");
    let four_general = format!("{symbol}_four_general");
    let four_second_ok = format!("{symbol}_four_second_ok");
    let seq_labels = Utf8SeqLabels {
        eof: &eof,
        read_second: &read_second,
        read_third: &read_third,
        read_fourth: &read_fourth,
        three_not_e0: &three_not_e0,
        three_general: &three_general,
        three_second_ok: &three_second_ok,
        four_not_f0: &four_not_f0,
        four_general: &four_general,
        four_second_ok: &four_second_ok,
        encoding_error: &encoding_error,
        input_error: &input_error,
        cont: &got_len,
    };
    emit_utf8_sequence_read(
        symbol,
        platform_imports,
        platform,
        app_mode,
        &seq_labels,
        BYTES_OFFSET,
        LEN_OFFSET,
        None,
        &mut vregs,
        &mut instructions,
        &mut relocations,
    )?;
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), &v10, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64(&v10, abi::mfb_return(1), 0),
        abi::add_immediate(&v11, abi::mfb_return(1), 8),
        abi::add_immediate(&v12, abi::stack_pointer(), BYTES_OFFSET),
        abi::label(&copy_loop),
        abi::compare_immediate(&v10, "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8(&v13, &v12, 0),
        abi::store_u8(&v13, &v11, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::subtract_immediate(&v10, &v10, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &v11, 0),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
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
    instructions.extend([abi::branch(&done), abi::label(&encoding_error)]);
    raise_error_into(symbol, "ErrEncoding", &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    raise_error_into(
        symbol,
        "ErrOutOfMemory",
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
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = FRAME_SIZE;
    Ok(ValueResult {
        origin: None,
        type_: "String".to_string(),
        location: Operand::from("void"),
        text: "io.readChar".to_string(),
    })
}
