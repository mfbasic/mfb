//! Allocation-free reporting of errors that escape a thread entry.

use std::collections::HashMap;

use crate::codegen::engine::builder::EmitCtx;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::{finalize_vreg_body_with_locals, Vregs};
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::push_symbol_address;
use crate::codegen::runtime::thread::{emit_thread_external_call, THREAD_TABLE_SYMBOL};
use crate::target::shared::abi;

pub(crate) const ERROR_REPORT_SYMBOL: &str = "_mfb_rt_report_thread_error";

/// C argument bank: thread identity, code, message String, optional ErrorLoc.
/// The thread table lock serializes complete reports. Callers must hold neither
/// it nor a control-block lock, and must keep message/source alive until return.
pub(crate) fn lower_error_report(
    threaded: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let mut vregs = Vregs::new();
    let thread = vregs.next();
    let code = vregs.next();
    let message = vregs.next();
    let source = vregs.next();
    let file = vregs.next();
    let offset = vregs.next();
    let number = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(thread, abi::c_arg(0)),
        abi::move_register(code, abi::c_arg(1)),
        abi::move_register(message, abi::c_arg(2)),
        abi::move_register(source, abi::c_arg(3)),
    ];
    let mut relocations = Vec::new();
    let mut ctx = EmitCtx {
        symbol: ERROR_REPORT_SYMBOL,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    };
    if threaded {
        report_lock(&mut ctx, "pthread_mutex_lock")?;
    }
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_THREAD_SYMBOL)?;
    write_number(&mut ctx, &mut vregs, thread, "thread", NumberFormat::Thread)?;
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_NEWLINE_SYMBOL)?;
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_PREFIX_SYMBOL)?;
    write_number(&mut ctx, &mut vregs, code, "code", NumberFormat::ErrorCode)?;
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_NEWLINE_SYMBOL)?;
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_MESSAGE_PREFIX_SYMBOL)?;
    write_object(&mut ctx, message)?;
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_NEWLINE_SYMBOL)?;
    ctx.instructions.extend([
        abi::compare_immediate(source, "0"),
        abi::branch_eq("report_no_source"),
    ]);
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_LOCATION_PREFIX_SYMBOL)?;
    ctx.instructions.extend([
        abi::load_u64(offset, source, 0),
        abi::add_registers(file, source, offset),
    ]);
    write_object(&mut ctx, file)?;
    for (field_offset, label) in [(8, "line"), (16, "column")] {
        write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_LOCATION_SEPARATOR_SYMBOL)?;
        ctx.instructions
            .push(abi::load_u64(number, source, field_offset));
        write_number(&mut ctx, &mut vregs, number, label, NumberFormat::Decimal)?;
    }
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_NEWLINE_SYMBOL)?;
    ctx.instructions.push(abi::label("report_no_source"));
    write_literal(&mut ctx, &mut vregs, ENTRY_ERROR_NEWLINE_SYMBOL)?;
    if threaded {
        report_lock(&mut ctx, "pthread_mutex_unlock")?;
    }
    ctx.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    // Every numeric field fits in this window (signed decimal or a pointer in
    // hex). Strings stream directly from their objects, without truncation.
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], 64);
    Ok(CodeFunction {
        name: "runtime.report_thread_error".to_string(),
        symbol: ERROR_REPORT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        stack_slots,
        instructions,
        relocations,
    })
}

fn report_lock(ctx: &mut EmitCtx, operation: &str) -> Result<(), String> {
    push_symbol_address(
        ctx.symbol,
        THREAD_TABLE_SYMBOL,
        abi::c_arg(0),
        ctx.instructions,
        ctx.relocations,
    );
    emit_thread_external_call(ctx, operation)
}

fn write_literal(ctx: &mut EmitCtx, vregs: &mut Vregs, symbol: &str) -> Result<(), String> {
    let object = vregs.next();
    push_symbol_address(
        ctx.symbol,
        symbol,
        object,
        ctx.instructions,
        ctx.relocations,
    );
    write_object(ctx, object)
}

fn write_object(ctx: &mut EmitCtx, object: impl Into<abi::Reg>) -> Result<(), String> {
    let object = object.into();
    ctx.instructions.extend([
        abi::load_u64(abi::string_length_register(), object, 0),
        abi::add_immediate(abi::string_data_register(), object, 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    ctx.platform.emit_write(
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )
}

#[derive(Clone, Copy)]
enum NumberFormat {
    Thread,
    ErrorCode,
    Decimal,
}

/// Render a number backwards in the local window. Decimal codes retain the
/// existing g-sss-nnnn grouping, including the behavior of short custom codes.
fn write_number(
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
    value: impl Into<abi::Reg>,
    label: &str,
    format: NumberFormat,
) -> Result<(), String> {
    let value = value.into();
    let (radix, min_digits, hyphenate) = match format {
        NumberFormat::Thread => (16, 8, false),
        NumberFormat::ErrorCode => (10, 1, true),
        NumberFormat::Decimal => (10, 1, false),
    };
    let remaining = vregs.next();
    let negative = vregs.next();
    let cursor = vregs.next();
    let base = vregs.next();
    let quotient = vregs.next();
    let digit = vregs.next();
    let count = vregs.next();
    let end = vregs.next();
    let abs = format!("report_{label}_absolute");
    let digits = format!("report_{label}_digits");
    let store = format!("report_{label}_store");
    let separator = format!("report_{label}_separator");
    let done = format!("report_{label}_done");
    let write = format!("report_{label}_write");
    ctx.instructions.extend([
        abi::move_register(remaining, value),
        abi::move_immediate(negative, "Integer", "0"),
    ]);
    if radix == 10 {
        ctx.instructions.extend([
            abi::compare_immediate(remaining, "0"),
            abi::branch_ge(&abs),
            abi::move_immediate(digit, "Integer", "0"),
            abi::subtract_registers(remaining, digit, remaining),
            abi::move_immediate(negative, "Integer", "1"),
            abi::label(&abs),
        ]);
    }
    ctx.instructions.extend([
        abi::add_immediate(end, abi::stack_pointer(), 64),
        abi::move_register(cursor, end),
        abi::move_immediate(base, "Integer", &radix.to_string()),
        abi::move_immediate(count, "Integer", "0"),
        abi::label(&digits),
        abi::unsigned_divide_registers(quotient, remaining, base),
        abi::multiply_subtract_registers(digit, quotient, base, remaining),
        abi::move_register(remaining, quotient),
        abi::add_immediate(digit, digit, 48),
    ]);
    if radix == 16 {
        ctx.instructions.extend([
            abi::compare_immediate(digit, "57"),
            abi::branch_le(&store),
            abi::add_immediate(digit, digit, 39),
            abi::label(&store),
        ]);
    }
    ctx.instructions.extend([
        abi::subtract_immediate(cursor, cursor, 1),
        abi::store_u8(digit, cursor, 0),
        abi::add_immediate(count, count, 1),
        abi::compare_immediate(count, &min_digits.to_string()),
        abi::branch_lt(&digits),
        abi::compare_immediate(remaining, "0"),
        abi::branch_eq(&done),
    ]);
    if hyphenate {
        ctx.instructions.extend([
            abi::compare_immediate(count, "4"),
            abi::branch_eq(&separator),
            abi::compare_immediate(count, "7"),
            abi::branch_ne(&digits),
            abi::label(&separator),
            abi::subtract_immediate(cursor, cursor, 1),
            abi::move_immediate(digit, "Integer", "45"),
            abi::store_u8(digit, cursor, 0),
        ]);
    }
    ctx.instructions.extend([
        abi::branch(&digits),
        abi::label(&done),
        abi::compare_immediate(negative, "0"),
        abi::branch_eq(&write),
        abi::subtract_immediate(cursor, cursor, 1),
        abi::move_immediate(digit, "Integer", "45"),
        abi::store_u8(digit, cursor, 0),
        abi::label(&write),
        abi::move_register(abi::string_data_register(), cursor),
        abi::subtract_registers(abi::string_length_register(), end, cursor),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    ctx.platform.emit_write(
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )
}
