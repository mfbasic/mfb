//! Arena-free stderr writers for debug-report lines.
//!
//! The report runs after `_mfb_arena_destroy`, so nothing here may allocate or
//! read the arena register: a line is either a prebuilt string object or is
//! assembled in the calling helper's own stack window
//! ([`DEBUG_LINE_BUFFER_SIZE`] bytes at `sp`, reserved with
//! `finalize_vreg_body_with_locals`). Each line is exactly one `write` to fd 2 so a
//! concurrently running worker cannot split it.

use std::collections::HashMap;

use crate::codegen::engine::types::{
    CodeDataObject, CodeInstruction, CodeRelocation, CodegenPlatform,
};
use crate::codegen::engine::util::Vregs;
use crate::codegen::memory::data::{push_symbol_address, string_data_object};
use crate::target::shared::abi;

/// Stack window a helper calling [`emit_debug_key_value`] must reserve.
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

/// Write `<key> <value>\n` to stderr, where `key_symbol` is a [`key_object`] and
/// `value` holds an unsigned integer.
///
/// The line is assembled right-to-left in the caller's stack window: the decimal
/// digits and newline first, ending at `sp + DEBUG_LINE_BUFFER_SIZE`, then the key
/// bytes copied in front of them, so one `write` covers the whole line. `value`
/// is read through a copy and survives. `tag` keeps this call's labels distinct
/// from any other in the same function.
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
    let remaining = vregs.next();
    let cursor = vregs.next();
    let ten = vregs.next();
    let quotient = vregs.next();
    let byte = vregs.next();
    let key = vregs.next();
    let key_length = vregs.next();
    let index = vregs.next();
    let source = vregs.next();
    let target = vregs.next();
    let end = vregs.next();
    let digits = format!("{from}_{tag}_digits");
    let copy = format!("{from}_{tag}_copy");
    let copied = format!("{from}_{tag}_copied");
    instructions.extend([
        abi::move_register(&remaining, value),
        abi::add_immediate(&cursor, abi::stack_pointer(), DEBUG_LINE_BUFFER_SIZE),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::move_immediate(&byte, "Integer", "10"),
        abi::store_u8(&byte, &cursor, 0),
        abi::move_immediate(&ten, "Integer", "10"),
        abi::label(&digits),
        abi::unsigned_divide_registers(&quotient, &remaining, &ten),
        abi::multiply_subtract_registers(&byte, &quotient, &ten, &remaining),
        abi::add_immediate(&byte, &byte, 48),
        abi::subtract_immediate(&cursor, &cursor, 1),
        abi::store_u8(&byte, &cursor, 0),
        abi::move_register(&remaining, &quotient),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_ne(&digits),
    ]);
    push_symbol_address(from, key_symbol, &key, instructions, relocations);
    instructions.extend([
        abi::load_u64(&key_length, &key, 0),
        abi::add_immediate(&key, &key, 8),
        abi::subtract_registers(&cursor, &cursor, &key_length),
        abi::move_register(&index, &key_length),
        abi::label(&copy),
        abi::compare_immediate(&index, "0"),
        abi::branch_eq(&copied),
        abi::subtract_immediate(&index, &index, 1),
        abi::add_registers(&source, &key, &index),
        abi::load_u8(&byte, &source, 0),
        abi::add_registers(&target, &cursor, &index),
        abi::store_u8(&byte, &target, 0),
        abi::branch(&copy),
        abi::label(&copied),
        abi::add_immediate(&end, abi::stack_pointer(), DEBUG_LINE_BUFFER_SIZE),
        abi::subtract_registers(abi::string_length_register(), &end, &cursor),
        abi::move_register(abi::string_data_register(), &cursor),
        abi::move_immediate(abi::return_register(), "Integer", STDERR_FD),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)
}
