use super::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// `os::name` / `os::arch` — return a fixed, target-selected `String` constant,
/// materialized directly into a fresh arena `String` (length header + bytes +
/// NUL) so the result is an ordinary owned value.
pub(super) fn lower_const_string(symbol: &str, value: &str) -> HelperResult {
    let alloc_ok = format!("{symbol}_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let bytes = value.as_bytes();
    let len = bytes.len();

    let mut vregs = Vregs::new();
    let block = vregs.next();
    let byte = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(abi::return_register(), "Integer", &(len + 9).to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = Vec::new();
    alloc_reloc(symbol, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&block, abi::RET[1]),
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

    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::pid` — `getpid()` as an `Integer` (a small positive value; the int
/// return is zero-extended by the W-register write, so no widening is needed).
pub(super) fn lower_pid(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "getpid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(RESULT_VALUE_REGISTER, abi::return_register()),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::cpuCount` — `sysconf(_SC_NPROCESSORS_ONLN)` as an `Integer`, clamped to
/// at least 1. `_SC_NPROCESSORS_ONLN` is 58 on Darwin and 84 on Linux.
pub(super) fn lower_cpu_count(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let sc_nprocessors_onln = match platform.family() {
        PlatformFamily::MacOS => "58",
        PlatformFamily::Linux => "84",
        // 47-D owns the Windows CPU count (GetSystemInfo), not sysconf.
        PlatformFamily::Windows => unreachable!("47-D owns the Windows processor count"),
    };
    let positive = format!("{symbol}_positive");
    let mut vregs = Vregs::new();
    let count = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_immediate(abi::ARG[0], "Integer", sc_nprocessors_onln),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "sysconf",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&count, abi::return_register()),
        // sysconf returns -1 (or 0) on failure or an indeterminate answer: clamp
        // to a minimum of 1 so callers always get a usable count.
        abi::compare_immediate(&count, "1"),
        abi::branch_ge(&positive),
        abi::move_immediate(&count, "Integer", "1"),
        abi::label(&positive),
        abi::move_register(RESULT_VALUE_REGISTER, &count),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::hostName` — `gethostname(buf, 256)` into an on-frame buffer, then a
/// `String` copy. HOST_NAME_MAX is 64 (Linux) / 255 (macOS), so 256 always
/// holds a NUL-terminated name.
pub(super) fn lower_host_name(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    const BUF: usize = 256;
    let ok = format!("{symbol}_ok");
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let buf = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::add_immediate(abi::ARG[0], abi::stack_pointer(), 0),
        abi::move_immediate(abi::ARG[1], "Integer", &BUF.to_string()),
    ];
    let mut relocations = Vec::new();
    platform.emit_libc_call(
        "gethostname",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&ok),
        abi::branch(&fail),
        abi::label(&ok),
        // Defensive NUL at the last byte, then build the String from the buffer.
        abi::add_immediate(&buf, abi::stack_pointer(), 0),
        abi::store_u8(abi::ZERO, &buf, BUF - 1),
    ]);
    build_string_from_cstr(
        symbol,
        &buf,
        &alloc_error,
        &format!("{symbol}_str"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut instructions, &[], BUF);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::userName` — `getpwuid(getuid())->pw_name` (`pw_name` is the first field
/// of `struct passwd` on every supported libc). Raises `ErrUnsupported` if the
/// uid has no passwd entry (e.g. a bare container uid).
pub(super) fn lower_user_name(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let have_pwd = format!("{symbol}_have_pwd");
    let have_name = format!("{symbol}_have_name");
    let fail = format!("{symbol}_fail");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");
    let mut vregs = Vregs::new();
    let pwname = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    // Hold the lock across `getpwuid` and the copy of its static `passwd`/`pw_name`
    // buffer, so a concurrent `getpwuid`/`getpwnam` cannot overwrite it mid-copy.
    // The env lock doubles as the process-global pwd lock (bug-64).
    emit_env_lock(&mut EmitCtx {
        symbol,
        platform_imports,
        platform,
        instructions: &mut instructions,
        relocations: &mut relocations,
    })?;
    platform.emit_libc_call(
        "getuid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    platform.emit_libc_call(
        "getpwuid",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&have_pwd),
        abi::branch(&fail),
        abi::label(&have_pwd),
        abi::load_u64(&pwname, abi::return_register(), 0), // pw_name @ offset 0
        abi::compare_immediate(&pwname, "0"),
        abi::branch_ne(&have_name),
        abi::branch(&fail),
        abi::label(&have_name),
    ]);
    build_string_from_cstr(
        symbol,
        &pwname,
        &alloc_error,
        &format!("{symbol}_str"),
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&fail)]);
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&alloc_error)]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.push(abi::label(&done));
    emit_env_unlock_return(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        &mut vregs,
    )?;
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}

/// `os::args` — build a `List OF String` from the entry-captured `argv`,
/// excluding `argv[0]` (the program name; D1). Reads the `_mfb_rt_os_argc` /
/// `_mfb_rt_os_argv` globals the program entry fills at startup.
pub(super) fn lower_args(symbol: &str) -> HelperResult {
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let count_str = format!("{symbol}_count_str");
    let count_str_done = format!("{symbol}_count_str_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let str_len = format!("{symbol}_str_len");
    let str_len_done = format!("{symbol}_str_len_done");
    let str_copy = format!("{symbol}_str_copy");
    let str_copy_done = format!("{symbol}_str_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut vregs = Vregs::new();
    let argc = vregs.next();
    let argv = vregs.next();
    let index = vregs.next();
    let count = vregs.next();
    let data_bytes = vregs.next();
    let arg_ptr = vregs.next();
    let scan = vregs.next();
    let byte = vregs.next();
    let collection = vregs.next();
    let entry_cursor = vregs.next();
    let data_cursor = vregs.next();
    let data_offset = vregs.next();
    let arg_len = vregs.next();
    let scratch = vregs.next();
    let src = vregs.next();

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    push_symbol_address(
        symbol,
        OS_ARGC_GLOBAL_SYMBOL,
        &argc,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&argc, &argc, 0));
    push_symbol_address(
        symbol,
        OS_ARGV_GLOBAL_SYMBOL,
        &argv,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&argv, &argv, 0));
    instructions.extend([
        // Pass 1: count args (from index 1) and their total byte length.
        abi::move_immediate(&count, "Integer", "0"),
        abi::move_immediate(&data_bytes, "Integer", "0"),
        abi::move_immediate(&index, "Integer", "1"),
        abi::label(&count_loop),
        abi::compare_registers(&index, &argc),
        abi::branch_ge(&count_done),
        abi::shift_left_immediate(&scratch, &index, 3),
        abi::add_registers(&scratch, &argv, &scratch),
        abi::load_u64(&arg_ptr, &scratch, 0),
        abi::move_register(&scan, &arg_ptr),
        abi::label(&count_str),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&count_str_done),
        abi::add_immediate(&data_bytes, &data_bytes, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&count_str),
        abi::label(&count_str_done),
        abi::add_immediate(&count, &count, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // size = HEADER + count*ENTRY_SIZE + data_bytes (a List has no buckets).
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&scratch, &scratch, &data_bytes),
        abi::add_immediate(abi::return_register(), &scratch, COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    alloc_reloc(symbol, &mut relocations);
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&alloc_error),
        abi::label(&alloc_ok),
        abi::move_register(&collection, abi::RET[1]),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KIND),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Byte", "1"),
        abi::store_u8(&scratch, &collection, COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &collection, COLLECTION_OFFSET_CAPACITY),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_bytes, &collection, COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate(&entry_cursor, &collection, COLLECTION_HEADER_SIZE),
        abi::move_immediate(&scratch, "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers(&scratch, &count, &scratch),
        abi::add_registers(&data_cursor, &entry_cursor, &scratch),
        abi::move_immediate(&data_offset, "Integer", "0"),
        // Pass 2: fill from index 1.
        abi::move_immediate(&index, "Integer", "1"),
        abi::label(&fill_loop),
        abi::compare_registers(&index, &argc),
        abi::branch_ge(&fill_done),
        abi::shift_left_immediate(&scratch, &index, 3),
        abi::add_registers(&scratch, &argv, &scratch),
        abi::load_u64(&arg_ptr, &scratch, 0),
        abi::move_register(&scan, &arg_ptr),
        abi::move_immediate(&arg_len, "Integer", "0"),
        abi::label(&str_len),
        abi::load_u8(&byte, &scan, 0),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&str_len_done),
        abi::add_immediate(&arg_len, &arg_len, 1),
        abi::add_immediate(&scan, &scan, 1),
        abi::branch(&str_len),
        abi::label(&str_len_done),
        abi::move_immediate(&scratch, "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8(&scratch, &entry_cursor, COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64(abi::ZERO, &entry_cursor, COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64(
            &data_offset,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ),
        abi::store_u64(
            &arg_len,
            &entry_cursor,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ),
        abi::move_register(&src, &arg_ptr),
        abi::move_immediate(&scratch, "Integer", "0"),
        abi::label(&str_copy),
        abi::compare_registers(&scratch, &arg_len),
        abi::branch_eq(&str_copy_done),
        abi::load_u8(&byte, &src, 0),
        abi::store_u8(&byte, &data_cursor, 0),
        abi::add_immediate(&src, &src, 1),
        abi::add_immediate(&data_cursor, &data_cursor, 1),
        abi::add_immediate(&scratch, &scratch, 1),
        abi::branch(&str_copy),
        abi::label(&str_copy_done),
        abi::add_registers(&data_offset, &data_offset, &arg_len),
        abi::add_immediate(&entry_cursor, &entry_cursor, COLLECTION_ENTRY_SIZE),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::move_register(RESULT_VALUE_REGISTER, &collection),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
    ]);
    push_alloc_error(symbol, &mut instructions, &mut relocations);
    instructions.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body(&mut instructions, &[]);
    Ok((frame, instructions, relocations, stack_slots))
}
