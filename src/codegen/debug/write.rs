//! Arena-free stderr writers for debug-report lines.
//!
//! The report runs after `_mfb_arena_destroy`, so nothing here may allocate or
//! read the arena register: a line is either a prebuilt string object or is
//! assembled in the calling helper's own stack window
//! ([`DEBUG_LINE_BUFFER_SIZE`] bytes at `sp`, reserved with
//! `finalize_vreg_body_with_locals`). Each line is exactly one `write` to fd 2 so a
//! concurrently running worker cannot split it.
//!
//! A window line is built right to left: the caller starts a cursor at
//! `sp + DEBUG_LINE_BUFFER_SIZE`, prepends its pieces ([`emit_prepend_decimal`],
//! [`emit_prepend_object`]), and writes `[cursor, sp + DEBUG_LINE_BUFFER_SIZE)` with
//! [`emit_write_window`].

use std::collections::HashMap;

use crate::codegen::engine::types::{
    CodeDataObject, CodeInstruction, CodeRelocation, CodegenPlatform,
};
use crate::codegen::engine::util::Vregs;
use crate::codegen::memory::data::{push_symbol_address, string_data_object};
use crate::target::shared::abi;

/// Stack window a helper assembling a line in place must reserve.
pub(super) const DEBUG_LINE_BUFFER_SIZE: usize = 128;

/// Widest `u64` in decimal (20 digits) plus the trailing newline.
const MAX_VALUE_TEXT: usize = 21;

/// The stderr file descriptor every report line goes to.
const STDERR_FD: &str = "2";

/// A report key rendered with its separating space, for [`emit_debug_key_value`].
///
/// Panics (an internal compiler error, not a user error) when the key could not
/// fit in the line window beside a full `u64`, or is not a single token.
pub(super) fn key_object(symbol: &str, key: &str) -> CodeDataObject {
    assert!(
        !key.is_empty() && !key.contains(char::is_whitespace),
        "debug report key `{key}` must be one non-empty token"
    );
    let text = format!("{key} ");
    assert!(
        text.len() + MAX_VALUE_TEXT <= DEBUG_LINE_BUFFER_SIZE,
        "debug report key `{key}` does not fit the {DEBUG_LINE_BUFFER_SIZE}-byte line window"
    );
    string_data_object(symbol, text)
}

/// A whole `key token\n` line whose value is known at compile time.
pub(super) fn constant_line_object(symbol: &str, key: &str, token: &str) -> CodeDataObject {
    assert!(
        !key.is_empty() && !key.contains(char::is_whitespace),
        "debug report key `{key}` must be one non-empty token"
    );
    assert!(
        !token.is_empty() && !token.contains(char::is_whitespace),
        "debug report value `{token}` for `{key}` must be one non-empty token"
    );
    string_data_object(symbol, format!("{key} {token}\n"))
}

/// Write the prebuilt line object `line_symbol` to stderr.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_debug_constant_line(
    from: &str,
    line_symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let object = vregs.next();
    push_symbol_address(from, line_symbol, &object, instructions, relocations);
    // Same register contract as the entry's `emit_write_string_object`: length at
    // [object+0], bytes at object+8, fd in the return register.
    instructions.extend([
        abi::load_u64(abi::string_length_register(), &object, 0),
        abi::add_immediate(abi::string_data_register(), &object, 8),
        abi::move_immediate(abi::return_register(), "Integer", STDERR_FD),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)
}

/// Move `cursor` back over the unsigned decimal digits of `value`, writing them
/// there. `value` is read through a copy and survives; `label` keeps this call's
/// labels distinct.
pub(super) fn emit_prepend_decimal(
    value: &str,
    cursor: &str,
    label: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let remaining = vregs.next();
    let ten = vregs.next();
    let quotient = vregs.next();
    let digit = vregs.next();
    let digits = format!("{label}_digits");
    instructions.extend([
        abi::move_register(&remaining, value),
        abi::move_immediate(&ten, "Integer", "10"),
        abi::label(&digits),
        abi::unsigned_divide_registers(&quotient, &remaining, &ten),
        abi::multiply_subtract_registers(&digit, &quotient, &ten, &remaining),
        abi::add_immediate(&digit, &digit, 48),
        abi::subtract_immediate(cursor, cursor, 1),
        abi::store_u8(&digit, cursor, 0),
        abi::move_register(&remaining, &quotient),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_ne(&digits),
    ]);
}

/// Move `cursor` back by the byte length of the `mfb.string.v1` object whose address
/// is in `object` (length at `[object+0]`, bytes from `object+8`), copying its bytes
/// there. `label` keeps this call's labels distinct.
pub(super) fn emit_prepend_object(
    object: &str,
    cursor: &str,
    label: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let length = vregs.next();
    let index = vregs.next();
    let byte = vregs.next();
    let source = vregs.next();
    let target = vregs.next();
    let copy = format!("{label}_copy");
    let copied = format!("{label}_copied");
    instructions.extend([
        abi::load_u64(&length, object, 0),
        abi::subtract_registers(cursor, cursor, &length),
        abi::move_register(&index, &length),
        abi::label(&copy),
        abi::compare_immediate(&index, "0"),
        abi::branch_eq(&copied),
        abi::subtract_immediate(&index, &index, 1),
        abi::add_registers(&source, object, &index),
        abi::load_u8(&byte, &source, 8),
        abi::add_registers(&target, cursor, &index),
        abi::store_u8(&byte, &target, 0),
        abi::branch(&copy),
        abi::label(&copied),
    ]);
}

/// Write `[cursor, sp + DEBUG_LINE_BUFFER_SIZE)` to stderr with one `write`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_write_window(
    from: &str,
    cursor: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let end = vregs.next();
    instructions.extend([
        abi::add_immediate(&end, abi::stack_pointer(), DEBUG_LINE_BUFFER_SIZE),
        abi::subtract_registers(abi::string_length_register(), &end, cursor),
        abi::move_register(abi::string_data_register(), cursor),
        abi::move_immediate(abi::return_register(), "Integer", STDERR_FD),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)
}

/// Write `<key> <value>\n` to stderr, where `key_symbol` is a [`key_object`] and
/// `value` holds an unsigned integer, as one line assembled in the caller's window.
/// `value` is read through a copy and survives. `tag` keeps this call's labels
/// distinct from any other in the same function.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_debug_key_value(
    from: &str,
    key_symbol: &str,
    value: &str,
    tag: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let cursor = vregs.next();
    let newline = vregs.next();
    let key = vregs.next();
    instructions.extend([
        abi::add_immediate(&cursor, abi::stack_pointer(), DEBUG_LINE_BUFFER_SIZE),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::move_immediate(&newline, "Integer", "10"),
        abi::store_u8(&newline, &cursor, 0),
    ]);
    emit_prepend_decimal(
        value,
        &cursor,
        &format!("{from}_{tag}_value"),
        instructions,
        vregs,
    );
    push_symbol_address(from, key_symbol, &key, instructions, relocations);
    emit_prepend_object(
        &key,
        &cursor,
        &format!("{from}_{tag}_key"),
        instructions,
        vregs,
    );
    emit_write_window(
        from,
        &cursor,
        platform_imports,
        platform,
        instructions,
        relocations,
        vregs,
    )
}
