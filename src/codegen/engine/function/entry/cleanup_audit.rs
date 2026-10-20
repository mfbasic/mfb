//! plan-176-U: the cleanup-failure audit report and its stderr writers. The program entry (`lower_program_entry`) lives in
//! `engine/function/entry/mod.rs`.

use super::*;

pub(super) fn emit_cleanup_failure_audit_report(ctx: &mut EmitCtx) -> Result<(), String> {
    // `from` is gone: it named the emitting symbol, which is `ctx.symbol`.
    let from = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let done = "entry_cleanup_failure_audit_done";
    ctx.instructions.extend([
        abi::load_u64(
            abi::SCRATCH[0],
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ),
        abi::compare_immediate(abi::SCRATCH[0], "0"),
        abi::branch_eq(done),
    ]);
    emit_write_string_object(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        CLEANUP_FAILURE_PREFIX_SYMBOL,
    )?;
    ctx.instructions.push(abi::load_u64(
        abi::SCRATCH[0],
        ARENA_STATE_REGISTER,
        ARENA_CLEANUP_FAILURE_CODE_OFFSET,
    ));
    ctx.instructions
        .push(abi::store_u64(abi::SCRATCH[0], ARENA_STATE_REGISTER, 32));
    emit_write_integer_to_stderr_with_labels(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        "entry_cleanup_failure_code",
        true,
    )?;
    // The cleanup audit retains `Cleanup failure: <code>\n`
    // with the code in canonical hyphenated form, then the message on its own
    // line.
    emit_write_string_object(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ENTRY_ERROR_NEWLINE_SYMBOL,
    )?;
    ctx.instructions.extend([
        abi::load_u64(
            abi::SCRATCH[10],
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
        ),
        abi::load_u64(abi::string_length_register(), abi::SCRATCH[10], 0),
        abi::add_immediate(abi::string_data_register(), abi::SCRATCH[10], 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, ctx.instructions, ctx.relocations)?;
    emit_write_string_object(
        &mut EmitCtx {
            symbol: from,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        ENTRY_ERROR_NEWLINE_SYMBOL,
    )?;
    ctx.instructions.push(abi::label(done));
    Ok(())
}

fn emit_write_string_object(ctx: &mut EmitCtx, data_symbol: &str) -> Result<(), String> {
    // `ctx.symbol` is the emitting symbol (each relocation's `from`);
    // `data_symbol` is the string object being addressed (its `to`).
    let from = ctx.symbol;
    let symbol = data_symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    ctx.instructions.extend([
        abi::load_page_address(abi::SCRATCH[11], symbol),
        abi::add_page_offset(abi::SCRATCH[11], abi::SCRATCH[11], symbol),
        abi::load_u64(abi::string_length_register(), abi::SCRATCH[11], 0),
        abi::add_immediate(abi::string_data_register(), abi::SCRATCH[11], 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    ctx.relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
    platform.emit_write(from, platform_imports, ctx.instructions, ctx.relocations)
}

fn emit_write_integer_to_stderr_with_labels(
    ctx: &mut EmitCtx,
    label_prefix: &str,
    hyphenate: bool,
) -> Result<(), String> {
    let from = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let absolute_ready_label = format!("{label_prefix}_absolute_ready");
    let digit_loop_label = format!("{label_prefix}_digit_loop");
    let digits_done_label = format!("{label_prefix}_digits_done");
    let write_label = format!("{label_prefix}_write");
    let hyphen_label = format!("{label_prefix}_hyphen");
    ctx.instructions.extend([
        abi::subtract_stack(64),
        abi::load_u64(abi::SCRATCH[11], ARENA_STATE_REGISTER, 32),
        // Record the value's original sign in x28 *before* negating: x21 is
        // overwritten (absolute value, then consumed by the digit loop), so the
        // minus test below must read this saved flag, not the value register
        // (bug-70: the test read x19/arena_base — always non-negative — so the
        // minus branch was dead). x28 is untouched by the digit loop.
        abi::move_immediate(abi::SCRATCH[18], "Integer", "0"),
        abi::compare_immediate(abi::SCRATCH[11], "0"),
        abi::branch_ge(&absolute_ready_label),
        abi::move_immediate(abi::SCRATCH[12], "Integer", "0"),
        abi::subtract_registers(abi::SCRATCH[11], abi::SCRATCH[12], abi::SCRATCH[11]),
        abi::move_immediate(abi::SCRATCH[18], "Integer", "1"),
        abi::label(&absolute_ready_label),
        abi::add_immediate(abi::SCRATCH[13], abi::stack_pointer(), 64),
        abi::move_immediate(abi::SCRATCH[14], "Integer", "10"),
    ]);
    // Canonical error-code formatting (doc `diagnostics 02_error-codes.md`): the
    // cleanup-failure banner prints the code in its hyphenated `G-SSS-EEEE` form,
    // so the digit loop keeps a running digit count and injects a `-` after the
    // 4th and 7th digit-from-the-right — but only while more digits remain, so an
    // arbitrary short `FAIL` code degrades to a plain number rather than gaining a
    // stray leading/trailing hyphen. The cleanup-failure audit passes
    // `hyphenate = false` and stays on the bare integer (byte-identical to the
    // pre-change lowering). The counter lives in SCRATCH[12] (AArch64 x22 / x86
    // rdi / riscv scratch): the loop body never touches that register and the
    // divide's implicit rax:rdx clobber does not reach it, so it persists across
    // every iteration. The hyphen store below reuses SCRATCH[16] (the just-freed
    // digit register) for the `-` byte so it does not disturb the counter.
    if hyphenate {
        ctx.instructions
            .push(abi::move_immediate(abi::SCRATCH[12], "Integer", "0"));
    }
    ctx.instructions.extend([
        abi::compare_immediate(abi::SCRATCH[11], "0"),
        abi::branch_ne(&digit_loop_label),
        abi::subtract_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
        abi::move_immediate(abi::SCRATCH[12], "Integer", "48"),
        abi::store_u8(abi::SCRATCH[12], abi::SCRATCH[13], 0),
        abi::branch(&digits_done_label),
        abi::label(&digit_loop_label),
        abi::unsigned_divide_registers(abi::SCRATCH[15], abi::SCRATCH[11], abi::SCRATCH[14]),
        abi::multiply_subtract_registers(
            abi::SCRATCH[16],
            abi::SCRATCH[15],
            abi::SCRATCH[14],
            abi::SCRATCH[11],
        ),
        abi::add_immediate(abi::SCRATCH[16], abi::SCRATCH[16], 48),
        abi::subtract_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
        abi::store_u8(abi::SCRATCH[16], abi::SCRATCH[13], 0),
        abi::move_register(abi::SCRATCH[11], abi::SCRATCH[15]),
    ]);
    if hyphenate {
        ctx.instructions.extend([
            abi::add_immediate(abi::SCRATCH[12], abi::SCRATCH[12], 1),
            abi::compare_immediate(abi::SCRATCH[11], "0"),
            abi::branch_eq(&digits_done_label),
            abi::compare_immediate(abi::SCRATCH[12], "4"),
            abi::branch_eq(&hyphen_label),
            abi::compare_immediate(abi::SCRATCH[12], "7"),
            abi::branch_eq(&hyphen_label),
            abi::branch(&digit_loop_label),
            abi::label(&hyphen_label),
            abi::subtract_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
            abi::move_immediate(abi::SCRATCH[16], "Integer", "45"),
            abi::store_u8(abi::SCRATCH[16], abi::SCRATCH[13], 0),
            abi::branch(&digit_loop_label),
            abi::label(&digits_done_label),
        ]);
    } else {
        ctx.instructions.extend([
            abi::compare_immediate(abi::SCRATCH[11], "0"),
            abi::branch_ne(&digit_loop_label),
            abi::label(&digits_done_label),
        ]);
    }
    ctx.instructions.extend([
        abi::compare_immediate(abi::SCRATCH[18], "0"),
        abi::branch_eq(&write_label),
        abi::subtract_immediate(abi::SCRATCH[13], abi::SCRATCH[13], 1),
        abi::move_immediate(abi::SCRATCH[12], "Integer", "45"),
        abi::store_u8(abi::SCRATCH[12], abi::SCRATCH[13], 0),
        abi::label(&write_label),
        abi::add_immediate(abi::SCRATCH[17], abi::stack_pointer(), 64),
        // plan-99: capture the buffer cursor as the data pointer BEFORE computing
        // the length. The length's destination and the cursor can be the SAME
        // machine register: `string_length_register()` is MFB argument 2, which the
        // Win64 bank realizes as `r8`, and `SCRATCH[13]` (`x23`) realizes as `r8`
        // too (`map_scratch_register`, whose plan-47-B note predicted exactly this
        // "low scratch overlaps the Win64 argument registers" hazard). Computing
        // the length first overwrote the cursor, so the write was handed the digit
        // COUNT as its buffer address: on box 2230 an uncaught runtime error printed
        // `Error: ` with an empty code line — the message line, which uses a
        // different staging path, was fine. Reading the cursor first makes the order
        // safe on every ABI; the length's `dst == rhs` case is already encoded
        // correctly (`alu3` negates and adds).
        abi::move_register(abi::string_data_register(), abi::SCRATCH[13]),
        abi::subtract_registers(
            abi::string_length_register(),
            abi::SCRATCH[17],
            abi::SCRATCH[13],
        ),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, ctx.instructions, ctx.relocations)?;
    ctx.instructions.push(abi::add_stack(64));
    Ok(())
}
