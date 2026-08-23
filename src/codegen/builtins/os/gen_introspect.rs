// --- codegen tier imports (migration) ---
use super::gen_shared::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use std::collections::HashMap;
/// `os::name` / `os::arch` — return a fixed, target-selected `String` constant,
/// materialized directly into a fresh arena `String` (length header + bytes +
/// NUL) so the result is an ordinary owned value.
pub(crate) fn lower_const_string(symbol: &str, value: &str) -> Result<OsBodyParts, String> {
    let alloc_ok = format!("{symbol}_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let bytes = value.as_bytes();
    let len = bytes.len();

    let mut vregs = Vregs::new();
    let block = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        abi::move_immediate(abi::return_register(), "Integer", &(len + 9).to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = Vec::new();
    alloc_reloc(symbol, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::mfb_return(1)),
        abi::move_immediate(&byte, "Integer", &len.to_string()),
        abi::store_u64(&byte, &block, 0),
    ]);
    for (i, b) in bytes.iter().enumerate() {
        instructions.push(abi::move_immediate(&byte, "Byte", &b.to_string()));
        instructions.push(abi::store_u8(&byte, &block, 8 + i));
    }
    instructions.extend([
        abi::move_immediate(&byte, "Byte", "0"),
        abi::store_u8(&byte, &block, 8 + len),
        abi::move_register(RESULT_VALUE_REGISTER, &block),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);

    Ok((instructions, relocations, 0))
}

/// `os::hostName` — `gethostname(buf, 256)` into an on-frame buffer, then a
/// `String` copy. HOST_NAME_MAX is 64 (Linux) / 255 (macOS), so 256 always
/// holds a NUL-terminated name.
/// Windows body shared by `os::hostName`/`userName`/`executablePath` (plan-66-B):
/// `platform.emit_os_wide_string(which)` leaves a UTF-8 value C-string pointer (0 on
/// failure) in the return register; build a `String` from it, or raise
/// `ErrUnsupported`. The `*W` query + UTF-16→UTF-8 marshal live in the Windows
/// backend; this reuses the shared String builder and error tails.
pub(crate) fn lower_os_wide_string_windows(
    symbol: &str,
    which: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<OsBodyParts, String> {
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let value = vregs.next();
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    platform.emit_os_wide_string(
        which,
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&value, abi::return_register()),
        abi::compare_immediate(&value, "0"),
        abi::branch_eq(&fail),
    ]);
    build_string_from_cstr(
        symbol,
        &value,
        &alloc_error,
        &format!("{symbol}_str"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    raise_error_into(
        symbol,
        "ErrUnsupported",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    Ok((instructions, relocations, 0))
}
