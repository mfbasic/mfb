//! Shared clean-room stdin-read machinery for the `io` reader members.
//!
//! The UTF-8 lead-byte decoder (`emit_utf8_sequence_read` + its `Utf8SeqLabels`),
//! the per-byte reader (`emit_stdin_byte_read`), and the continuation reader
//! (`emit_continuation_read`) are reused by `io::readByte`, `io::readChar`, and the
//! line reader (`super::gen_read_line_family`) — so they live here rather than in
//! any one member file (`func_read_byte`/`func_read_char` and `gen_read_line_family`
//! all `use super::gen_read_family::…`).

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::io::stdin::*;
use crate::codegen::os::syscall::*;
use crate::target::shared::abi;
use std::collections::HashMap;
/// The label set for the shared UTF-8 lead-byte decoder (bug-331 §F). The two
/// callers (`readChar`/`readLine`) drifted to different label *names*, not just a
/// prefix, so every label is passed explicitly to keep each site's `.ncode`/`.mir`
/// goldens byte-identical.
pub(crate) struct Utf8SeqLabels<'a> {
    pub(crate) eof: &'a str,
    pub(crate) read_second: &'a str,
    pub(crate) read_third: &'a str,
    pub(crate) read_fourth: &'a str,
    pub(crate) three_not_e0: &'a str,
    pub(crate) three_general: &'a str,
    pub(crate) three_second_ok: &'a str,
    pub(crate) four_not_f0: &'a str,
    pub(crate) four_general: &'a str,
    pub(crate) four_second_ok: &'a str,
    pub(crate) encoding_error: &'a str,
    pub(crate) input_error: &'a str,
    pub(crate) cont: &'a str,
}

/// Emit one stdin byte read for a read helper, choosing the source by mode. In
/// console mode (`!app_mode`) the byte comes from the stdin broadcast log
/// (`_mfb_rt_stdin_next_byte`, plan-15). In app mode stdin is the window input
/// pipe, not fd 0, so the log is not built — keep the direct per-byte
/// `read(0,…,1)` + EINTR guard. Both paths push `retry_label` (the loop/retry head)
/// and leave the `x0 vs 0` flags live for the caller's follow-on `branch_eq`.
pub(crate) fn emit_stdin_byte_read(
    ctx: &mut EmitCtx,
    app_mode: bool,
    byte_offset: usize,
    retry_label: &str,
    resume_label: &str,
    input_error: &str,
    invalid_context: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    if app_mode {
        ctx.instructions.extend([
            abi::label(retry_label),
            // fd 0 (stdin) goes in ARG[0], the read() first-arg register that
            // emit_read_file reads it from. Using return_register() worked only by
            // accident on aarch64 (x0 == ARG[0]); on Win64 return_register() is rax,
            // not ARG[0]=rcx, so emit_read_file read garbage as the fd (plan-66-J-4).
            abi::move_immediate(abi::c_arg(0), "Integer", "0"),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), byte_offset),
            abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        ]);
        platform.emit_read_file(symbol, platform_imports, ctx.instructions, ctx.relocations)?;
        ctx.instructions
            .push(abi::compare_immediate(abi::return_register(), "0"));
        emit_single_op_eintr_guard(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: ctx.instructions,
                relocations: ctx.relocations,
            },
            retry_label,
            resume_label,
            input_error,
        )?;
    } else {
        ctx.instructions.push(abi::label(retry_label));
        emit_stdin_next_byte(
            symbol,
            byte_offset,
            retry_label,
            input_error,
            invalid_context,
            ctx.instructions,
            ctx.relocations,
        );
    }
    Ok(())
}

/// Emit one EINTR-guarded UTF-8 continuation-byte `read` for `io::readChar` /
/// `io::readLine` (bug-97.2). A signal delivered mid-multibyte-sequence returns
/// `-1`/`EINTR`; before this the bare `compare/branch_lt(<input_error>)` treated
/// that as a fatal input error (discarding `readLine`'s partial line). This
/// replicates the lead-read guard: `retry_label` re-issues the identical 1-byte
/// read into `stack[byte_offset]`, and the guard leaves the `cmp x0, 0` flags
/// live so the caller's follow-on `branch_eq(<encoding_error>)` (a 0-byte read
/// mid-sequence is a truncated sequence) fuses on every backend. Reads always go
/// through libc, so the guard uses the `errno`-accessor convention (both read
/// helpers already import it for the lead read).
fn emit_continuation_read(
    ctx: &mut EmitCtx,
    app_mode: bool,
    byte_offset: usize,
    retry_label: &str,
    resume_label: &str,
    input_error: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // plan-15: in console mode the continuation byte comes from the stdin broadcast
    // log (`_mfb_rt_stdin_next_byte`); in app mode it is a direct per-byte read of
    // the window pipe. A continuation byte from an unsubscribed thread is the same
    // ErrInvalidContext as the lead byte, routed to the helper's shared handler.
    emit_stdin_byte_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        app_mode,
        byte_offset,
        retry_label,
        resume_label,
        input_error,
        &format!("{symbol}_invalid_context"),
    )
}

/// Emit the shared 1/2/3/4-byte UTF-8 lead-byte decoder (bug-331 §F): validate the
/// sequence (overlong/surrogate rejection), read continuation bytes, and store the
/// sequence length at `sp + len_offset`, ending at the caller's `cont` label. The
/// sole behavioural delta is `on_lf`: `readLine` passes its `trim_cr` label so a
/// lone LF short-circuits; `readChar` passes `None`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_utf8_sequence_read(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
    l: &Utf8SeqLabels,
    bytes_offset: usize,
    len_offset: usize,
    on_lf: Option<&str>,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let v10 = vregs.next();
    let v11 = vregs.next();
    instructions.extend([
        abi::branch_eq(l.eof),
        abi::load_u8(&v10, abi::stack_pointer(), bytes_offset),
    ]);
    if let Some(lf) = on_lf {
        instructions.push(abi::compare_immediate(&v10, "10"));
        instructions.push(abi::branch_eq(lf));
    }
    instructions.extend([
        abi::compare_immediate(&v10, "127"),
        abi::branch_hi(l.read_second),
        abi::move_immediate(&v11, "Integer", "1"),
        abi::store_u64(&v11, abi::stack_pointer(), len_offset),
        abi::branch(l.cont),
        abi::label(l.read_second),
        abi::compare_immediate(&v10, "194"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v10, "223"),
        abi::branch_hi(l.read_third),
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *instructions,
            relocations: &mut *relocations,
        },
        app_mode,
        bytes_offset + 1,
        &format!("{symbol}_cont1_retry"),
        &format!("{symbol}_cont1_resume"),
        l.input_error,
    )?;
    instructions.extend([
        abi::branch_eq(l.encoding_error),
        abi::load_u8(&v11, abi::stack_pointer(), bytes_offset + 1),
        abi::compare_immediate(&v11, "128"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "191"),
        abi::branch_hi(l.encoding_error),
        abi::move_immediate(&v11, "Integer", "2"),
        abi::store_u64(&v11, abi::stack_pointer(), len_offset),
        abi::branch(l.cont),
        abi::label(l.read_third),
        abi::compare_immediate(&v10, "239"),
        abi::branch_hi(l.read_fourth),
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *instructions,
            relocations: &mut *relocations,
        },
        app_mode,
        bytes_offset + 1,
        &format!("{symbol}_cont2_retry"),
        &format!("{symbol}_cont2_resume"),
        l.input_error,
    )?;
    instructions.extend([
        abi::branch_eq(l.encoding_error),
        abi::load_u8(&v11, abi::stack_pointer(), bytes_offset + 1),
        abi::compare_immediate(&v10, "224"),
        abi::branch_ne(l.three_not_e0),
        abi::compare_immediate(&v11, "160"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "191"),
        abi::branch_hi(l.encoding_error),
        abi::branch(l.three_second_ok),
        abi::label(l.three_not_e0),
        abi::compare_immediate(&v10, "237"),
        abi::branch_ne(l.three_general),
        abi::compare_immediate(&v11, "128"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "159"),
        abi::branch_hi(l.encoding_error),
        abi::branch(l.three_second_ok),
        abi::label(l.three_general),
        abi::compare_immediate(&v11, "128"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "191"),
        abi::branch_hi(l.encoding_error),
        abi::label(l.three_second_ok),
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *instructions,
            relocations: &mut *relocations,
        },
        app_mode,
        bytes_offset + 2,
        &format!("{symbol}_cont3_retry"),
        &format!("{symbol}_cont3_resume"),
        l.input_error,
    )?;
    instructions.extend([
        abi::branch_eq(l.encoding_error),
        abi::load_u8(&v11, abi::stack_pointer(), bytes_offset + 2),
        abi::compare_immediate(&v11, "128"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "191"),
        abi::branch_hi(l.encoding_error),
        abi::move_immediate(&v11, "Integer", "3"),
        abi::store_u64(&v11, abi::stack_pointer(), len_offset),
        abi::branch(l.cont),
        abi::label(l.read_fourth),
        abi::compare_immediate(&v10, "240"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v10, "244"),
        abi::branch_hi(l.encoding_error),
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *instructions,
            relocations: &mut *relocations,
        },
        app_mode,
        bytes_offset + 1,
        &format!("{symbol}_cont4_retry"),
        &format!("{symbol}_cont4_resume"),
        l.input_error,
    )?;
    instructions.extend([
        abi::branch_eq(l.encoding_error),
        abi::load_u8(&v11, abi::stack_pointer(), bytes_offset + 1),
        abi::compare_immediate(&v10, "240"),
        abi::branch_ne(l.four_not_f0),
        abi::compare_immediate(&v11, "144"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "191"),
        abi::branch_hi(l.encoding_error),
        abi::branch(l.four_second_ok),
        abi::label(l.four_not_f0),
        abi::compare_immediate(&v10, "244"),
        abi::branch_ne(l.four_general),
        abi::compare_immediate(&v11, "128"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "143"),
        abi::branch_hi(l.encoding_error),
        abi::branch(l.four_second_ok),
        abi::label(l.four_general),
        abi::compare_immediate(&v11, "128"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "191"),
        abi::branch_hi(l.encoding_error),
        abi::label(l.four_second_ok),
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *instructions,
            relocations: &mut *relocations,
        },
        app_mode,
        bytes_offset + 2,
        &format!("{symbol}_cont5_retry"),
        &format!("{symbol}_cont5_resume"),
        l.input_error,
    )?;
    instructions.extend([
        abi::branch_eq(l.encoding_error),
        abi::load_u8(&v11, abi::stack_pointer(), bytes_offset + 2),
        abi::compare_immediate(&v11, "128"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "191"),
        abi::branch_hi(l.encoding_error),
    ]);
    emit_continuation_read(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut *instructions,
            relocations: &mut *relocations,
        },
        app_mode,
        bytes_offset + 3,
        &format!("{symbol}_cont6_retry"),
        &format!("{symbol}_cont6_resume"),
        l.input_error,
    )?;
    instructions.extend([
        abi::branch_eq(l.encoding_error),
        abi::load_u8(&v11, abi::stack_pointer(), bytes_offset + 3),
        abi::compare_immediate(&v11, "128"),
        abi::branch_lo(l.encoding_error),
        abi::compare_immediate(&v11, "191"),
        abi::branch_hi(l.encoding_error),
        abi::move_immediate(&v11, "Integer", "4"),
        abi::store_u64(&v11, abi::stack_pointer(), len_offset),
        abi::label(l.cont),
    ]);
    Ok(())
}
